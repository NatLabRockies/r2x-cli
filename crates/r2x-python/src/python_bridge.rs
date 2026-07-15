//! Python bridge initialization with venv-based configuration
//!
//! This module handles lazy initialization of the Python bridge using
//! the virtual environment's configuration. It uses OnceCell for
//! thread-safe singleton initialization.
//!
//! ## PYTHONHOME Resolution
//!
//! PYTHONHOME is resolved from the venv's `pyvenv.cfg` file, which contains
//! the `home` field pointing to the Python installation used to create the venv.
//! This ensures PyO3 (linked at build time) uses a compatible Python environment.

use crate::errors::BridgeError;
use crate::utils::{resolve_python_path, resolve_site_package_path};
use once_cell::sync::{Lazy, OnceCell};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use r2x_config::{Config, PythonRuntimeVersion};
use r2x_logger as logger;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// The Python bridge for plugin execution
pub struct Bridge {
    /// Placeholder field for future extension
    _marker: (),
}

/// Global bridge singleton
static BRIDGE_INSTANCE: OnceCell<Result<Bridge, BridgeError>> = OnceCell::new();
static POST_IMPORT_LOG_MODULES_ENABLED: Lazy<Mutex<HashSet<String>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

impl Bridge {
    /// Get or initialize the bridge singleton
    pub fn get() -> Result<&'static Bridge, BridgeError> {
        match BRIDGE_INSTANCE.get_or_init(Bridge::initialize) {
            Ok(bridge) => Ok(bridge),
            Err(e) => Err(BridgeError::Initialization(format!("{}", e))),
        }
    }

    /// Check if Python is available without initializing
    pub fn is_python_available() -> bool {
        let config = match Config::load() {
            Ok(c) => c,
            Err(_) => return false,
        };

        // Check if venv exists and has valid pyvenv.cfg
        let venv_path = PathBuf::from(config.get_venv_path());
        venv_path.join("pyvenv.cfg").exists()
    }

    /// Initialize Python interpreter and configure environment
    ///
    /// This performs:
    /// 1. Ensure venv exists (create if needed)
    /// 2. Resolve PYTHONHOME from venv's pyvenv.cfg
    /// 3. Set PYTHONHOME and initialize PyO3
    /// 4. Configure site-packages
    fn initialize() -> Result<Bridge, BridgeError> {
        let start_time = std::time::Instant::now();

        let mut config = Config::load()
            .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

        // Ensure venv exists
        let venv_path = ensure_configured_venv(&mut config)?;

        // Resolve PYTHONHOME from venv's pyvenv.cfg
        let python_home = resolve_python_home(&venv_path)?;
        env::set_var("PYTHONHOME", &python_home);
        logger::debug_lazy(|| format!("Set PYTHONHOME={}", python_home.display()));

        // Get site-packages path
        let site_packages = resolve_site_package_path(&venv_path)?;

        // Add site-packages to PYTHONPATH
        Self::configure_python_path(&site_packages);

        // Check if Python library is available before initializing
        let python_version = runtime_python_version(&config)?;
        check_python_library_available(&python_version)?;

        // Initialize PyO3
        logger::debug("Initializing PyO3...");
        let pyo3_start = std::time::Instant::now();
        pyo3::Python::initialize();
        logger::debug_lazy(|| format!("pyo3::Python::initialize took: {:?}", pyo3_start.elapsed()));

        // Enable bytecode generation
        pyo3::Python::attach(|py| {
            let sys = PyModule::import(py, "sys")
                .map_err(|e| BridgeError::Python(format!("Failed to import sys module: {}", e)))?;
            sys.setattr("dont_write_bytecode", false).map_err(|e| {
                BridgeError::Python(format!("Failed to enable bytecode generation: {}", e))
            })?;
            Ok::<(), BridgeError>(())
        })?;
        logger::debug("Enabled Python bytecode generation");

        // Add venv site-packages to sys.path
        pyo3::Python::attach(|py| {
            let site = PyModule::import(py, "site")
                .map_err(|e| BridgeError::Python(format!("Failed to import site module: {}", e)))?;
            site.call_method1("addsitedir", (site_packages.to_string_lossy().as_ref(),))
                .map_err(|e| BridgeError::Python(format!("Failed to add site directory: {}", e)))?;
            Ok::<(), BridgeError>(())
        })?;

        // Configure cache path
        let cache_path = config.ensure_cache_path().map_err(|e| {
            BridgeError::Initialization(format!("Failed to ensure cache path: {}", e))
        })?;
        Self::configure_python_cache(&cache_path)?;

        // Configure Python logging
        if let Err(e) = Self::configure_python_logging() {
            logger::warn(&format!("Python logging configuration failed: {}", e));
        }

        logger::debug_lazy(|| {
            format!(
                "Total bridge initialization took: {:?}",
                start_time.elapsed()
            )
        });

        Ok(Bridge { _marker: () })
    }

    /// Configure PYTHONPATH to include site-packages
    fn configure_python_path(site_packages: &Path) {
        let mut paths = vec![site_packages.to_path_buf()];
        if let Some(existing) = env::var_os("PYTHONPATH") {
            if !existing.is_empty() {
                paths.extend(env::split_paths(&existing));
            }
        }
        if let Ok(joined) = env::join_paths(paths) {
            env::set_var("PYTHONPATH", &joined);
            logger::debug_lazy(|| {
                format!("Updated PYTHONPATH to include {}", site_packages.display())
            });
        }
    }

    /// Configure Python cache path override
    fn configure_python_cache(cache_path: &str) -> Result<(), BridgeError> {
        std::fs::create_dir_all(cache_path).map_err(|e| {
            BridgeError::Initialization(format!("Failed to create cache directory: {}", e))
        })?;
        env::set_var("R2X_CACHE_PATH", cache_path);

        let cache_path_escaped = cache_path.replace('\\', "\\\\");
        pyo3::Python::attach(|py| {
            let patch_code = format!(
                r#"from pathlib import Path
_R2X_CACHE_PATH = Path(r"{cache}")

def _r2x_cache_path_override():
    return _R2X_CACHE_PATH
"#,
                cache = cache_path_escaped
            );

            let code_cstr = std::ffi::CString::new(patch_code).map_err(|e| {
                BridgeError::Python(format!("Failed to prepare cache override script: {}", e))
            })?;
            let filename = std::ffi::CString::new("r2x_cache_patch.py")
                .map_err(|e| BridgeError::Python(format!("Failed to create filename: {}", e)))?;
            let module_name = std::ffi::CString::new("r2x_cache_patch")
                .map_err(|e| BridgeError::Python(format!("Failed to create module name: {}", e)))?;
            let patch_module = PyModule::from_code(
                py,
                code_cstr.as_c_str(),
                filename.as_c_str(),
                module_name.as_c_str(),
            )
            .map_err(|e| BridgeError::Python(format!("Failed to build cache override: {}", e)))?;

            let override_fn = patch_module
                .getattr("_r2x_cache_path_override")
                .map_err(|e| {
                    BridgeError::Python(format!("Failed to obtain cache override function: {}", e))
                })?;

            let file_ops = PyModule::import(py, "r2x_core.utils.files").map_err(|e| {
                BridgeError::Python(format!("Failed to import r2x_core.utils.files: {}", e))
            })?;

            file_ops
                .setattr("get_r2x_cache_path", override_fn)
                .map_err(|e| {
                    BridgeError::Python(format!("Failed to override cache path: {}", e))
                })?;

            Ok::<(), BridgeError>(())
        })?;

        Ok(())
    }

    /// Configure Python loguru logging
    ///
    /// Always configures a file sink pointing to the shared r2x.log.
    /// Optionally adds a console sink when --log-python is active.
    fn configure_python_logging() -> Result<(), BridgeError> {
        let verbosity = logger::get_verbosity();
        let log_python = logger::get_log_python();
        let log_file = logger::get_log_path_string();

        logger::debug_lazy(|| {
            format!(
                "Configuring Python logging with verbosity={}, log_python={}, log_file={}",
                verbosity, log_python, log_file
            )
        });

        pyo3::Python::attach(|py| {
            let logger_module = PyModule::import(py, "r2x_core.logger").map_err(|e| {
                BridgeError::Import("r2x_core.logger".to_string(), format!("{}", e))
            })?;
            let setup_logging = logger_module
                .getattr("setup_logging")
                .map_err(|e| BridgeError::Python(format!("setup_logging not found: {}", e)))?;

            let kwargs = PyDict::new(py);
            if !log_file.is_empty() {
                kwargs.set_item("log_file", &log_file)?;
            }
            kwargs.set_item("log_to_console", log_python)?;
            setup_logging.call((verbosity,), Some(&kwargs))?;

            Self::enable_loguru_modules(
                py,
                &[
                    "r2x_core",
                    "r2x_reeds",
                    "r2x_plexos",
                    "r2x_sienna",
                    "r2x_nodal",
                ],
            )
        })
    }

    /// Enable loguru logging for a list of Python modules
    pub(crate) fn enable_loguru_modules(py: Python, modules: &[&str]) -> Result<(), BridgeError> {
        let loguru = PyModule::import(py, "loguru")?;
        let logger_obj = loguru.getattr("logger")?;

        for module in modules {
            logger_obj.call_method1("enable", (module,))?;
        }

        Ok(())
    }

    /// Enable loguru for plugin modules after Python has imported them.
    ///
    /// Many plugin packages call `logger.disable(__name__)` from their
    /// `__init__.py`; enabling before import can be overwritten. Once a package
    /// has been imported and re-enabled, later plugin invocations from that same
    /// package do not need to import loguru or call enable again.
    pub(crate) fn enable_loguru_modules_after_import(
        py: Python,
        modules: &[&str],
    ) -> Result<(), BridgeError> {
        let pending = pending_post_import_log_modules(modules);

        if pending.is_empty() {
            return Ok(());
        }

        let pending_refs = pending.iter().map(String::as_str).collect::<Vec<_>>();
        Self::enable_loguru_modules(py, &pending_refs)?;

        mark_post_import_log_modules_enabled(&pending);

        Ok(())
    }
}

fn ensure_configured_venv(config: &mut Config) -> Result<PathBuf, BridgeError> {
    let venv_path = config.ensure_venv_path().map_err(|error| {
        BridgeError::Initialization(format!(
            "Failed to ensure Python virtual environment: {}",
            error
        ))
    })?;
    Ok(PathBuf::from(venv_path))
}

fn pending_post_import_log_modules(modules: &[&str]) -> Vec<String> {
    let enabled = POST_IMPORT_LOG_MODULES_ENABLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    modules
        .iter()
        .copied()
        .filter(|module| !enabled.contains(*module))
        .map(str::to_string)
        .collect()
}

fn mark_post_import_log_modules_enabled(modules: &[String]) {
    let mut enabled = POST_IMPORT_LOG_MODULES_ENABLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    enabled.extend(modules.iter().cloned());
}

/// Resolve PYTHONHOME from the venv's pyvenv.cfg file.
///
/// `home` in `pyvenv.cfg` is not fully consistent across creators/platforms:
/// it may point at a prefix, a launcher dir (`bin`/`Scripts`), or an executable.
/// We normalize it into a stable Python prefix for embedded startup.
fn resolve_python_home(venv_path: &Path) -> Result<PathBuf, BridgeError> {
    let pyvenv_cfg = venv_path.join("pyvenv.cfg");

    if !pyvenv_cfg.exists() {
        return Err(BridgeError::Initialization(format!(
            "pyvenv.cfg not found in venv: {}",
            venv_path.display()
        )));
    }

    let content = fs::read_to_string(&pyvenv_cfg)
        .map_err(|e| BridgeError::Initialization(format!("Failed to read pyvenv.cfg: {}", e)))?;

    for line in content.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case("home") {
                let home_value = PathBuf::from(value.trim());
                let python_home = normalize_python_home(&home_value);
                logger::debug_lazy(|| {
                    format!(
                        "Resolved PYTHONHOME from pyvenv.cfg home={} -> {}",
                        home_value.display(),
                        python_home.display()
                    )
                });
                return Ok(python_home);
            }
        }
    }

    Err(BridgeError::Initialization(format!(
        "Could not find 'home' in pyvenv.cfg: {}",
        pyvenv_cfg.display()
    )))
}

fn normalize_python_home(home_value: &Path) -> PathBuf {
    let Some(last_segment) = home_value.file_name().and_then(|name| name.to_str()) else {
        return home_value.to_path_buf();
    };

    if is_python_executable_name(last_segment)
        || last_segment.eq_ignore_ascii_case("bin")
        || last_segment.eq_ignore_ascii_case("scripts")
    {
        if let Some(parent) = home_value.parent() {
            return parent.to_path_buf();
        }
    }

    home_value.to_path_buf()
}

fn is_python_executable_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case("python")
        || name.eq_ignore_ascii_case("python.exe")
        || name.eq_ignore_ascii_case("python3")
        || name.eq_ignore_ascii_case("python3.exe")
    {
        return true;
    }

    let lower = name.to_ascii_lowercase();
    if let Some(suffix) = lower.strip_prefix("python") {
        let suffix = suffix.strip_suffix(".exe").unwrap_or(suffix);
        if let Some(version) = suffix.strip_prefix('3') {
            if version.is_empty() {
                return true;
            }
            if let Some(dotless) = version.strip_prefix('.') {
                return !dotless.is_empty() && dotless.chars().all(|ch| ch.is_ascii_digit());
            }
            return version.chars().all(|ch| ch.is_ascii_digit());
        }
    }

    false
}

/// Select the Python runtime version for venv creation and library discovery.
fn runtime_python_version(config: &Config) -> Result<PythonRuntimeVersion, BridgeError> {
    config.runtime_python_version().map_err(|error| {
        BridgeError::Initialization(format!("Invalid configured Python version: {}", error))
    })
}

/// Check if Python library is available before attempting to initialize PyO3.
///
/// This provides better error messages than the cryptic dyld errors on macOS
/// or DLL loading errors on Windows.
fn check_python_library_available(
    python_version: &PythonRuntimeVersion,
) -> Result<(), BridgeError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[cfg(target_os = "macos")]
        let (lib_names, search_paths, env_var) = (
            vec![format!("libpython{}.dylib", python_version.abi())],
            &[
                "/opt/homebrew/lib",
                "/usr/local/lib",
                "/Library/Frameworks/Python.framework/Versions/Current/lib",
            ][..],
            "DYLD_LIBRARY_PATH",
        );

        #[cfg(target_os = "linux")]
        let (lib_names, search_paths, env_var) = (
            vec![
                format!("libpython{}.so", python_version.abi()),
                format!("libpython{}.so.1.0", python_version.abi()),
            ],
            &[
                "/usr/lib",
                "/usr/lib64",
                "/usr/local/lib",
                "/usr/local/lib64",
            ][..],
            "LD_LIBRARY_PATH",
        );

        // Check environment variable paths first
        if let Ok(paths) = env::var(env_var) {
            if find_lib_in_paths(paths.split(':'), &lib_names) {
                return Ok(());
            }
        }

        // Check standard system locations
        if find_lib_in_paths(search_paths.iter().copied(), &lib_names) {
            return Ok(());
        }

        // Try to find Python via uv and set up the library path
        if let Some(lib_dir) = find_python_lib_via_uv(python_version, &lib_names) {
            prepend_to_env_path(env_var, &lib_dir);
            logger::debug_lazy(|| format!("Set {} to include: {}", env_var, lib_dir.display()));
            return Ok(());
        }

        // Library not found in expected locations, but don't fail -
        // let PyO3 try to load it via rpath or other mechanisms.
        logger::debug("Python library not found in standard locations, relying on rpath");
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, try to set up the DLL path (best effort)
        if let Err(e) = setup_windows_dll_path(python_version) {
            logger::debug_lazy(|| format!("Windows DLL path setup note: {}", e));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // For other platforms, just proceed and let PyO3 handle it
        Ok(())
    }
}

/// Search for any of the library names in the given paths.
/// Returns true if found, logging the discovery.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn find_lib_in_paths<I, S>(paths: I, lib_names: &[String]) -> bool
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    for path in paths {
        for lib_name in lib_names {
            let lib_path = PathBuf::from(path.as_ref()).join(lib_name);
            if lib_path.exists() {
                logger::debug_lazy(|| format!("Found Python library at: {}", lib_path.display()));
                return true;
            }
        }
    }
    false
}

/// Try to find Python library via uv python find command.
/// Returns the lib directory path if found.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn find_python_lib_via_uv(
    python_version: &PythonRuntimeVersion,
    lib_names: &[String],
) -> Option<PathBuf> {
    for python_query in python_version.query_candidates() {
        let output = Command::new("uv")
            .args(["python", "find", python_query])
            .output()
            .ok()?;

        if !output.status.success() {
            continue;
        }

        let python_path = String::from_utf8_lossy(&output.stdout);
        let python_path = python_path.trim();

        // Python binary is in bin/, lib is in ../lib/
        let lib_dir = PathBuf::from(python_path).parent()?.parent()?.join("lib");

        for lib_name in lib_names {
            let lib_path = lib_dir.join(lib_name);
            if lib_path.exists() {
                logger::debug_lazy(|| {
                    format!("Found Python library via uv: {}", lib_path.display())
                });
                return Some(lib_dir);
            }
        }
    }

    None
}

/// Prepend a directory to an environment path variable.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn prepend_to_env_path(env_var: &str, dir: &Path) {
    if let Some(existing) = env::var_os(env_var) {
        let mut paths = env::split_paths(&existing).collect::<Vec<_>>();
        paths.insert(0, dir.to_path_buf());
        if let Ok(new_path) = env::join_paths(&paths) {
            env::set_var(env_var, new_path);
        }
    } else {
        env::set_var(env_var, dir);
    }
}

/// Setup Windows DLL search path for Python
#[cfg(target_os = "windows")]
fn setup_windows_dll_path(python_version: &PythonRuntimeVersion) -> Result<(), BridgeError> {
    let dll_name = format!("python{}.dll", python_version.abi().replace('.', ""));

    // Try to find Python via uv first
    for python_query in python_version.query_candidates() {
        let output = Command::new("uv")
            .args(["python", "find", python_query])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let python_path = String::from_utf8_lossy(&output.stdout);
                let python_path = python_path.trim();
                if let Some(parent) = PathBuf::from(python_path).parent() {
                    // On Windows, Python DLL is usually in the same directory as python.exe
                    let dll_path = parent.join(&dll_name);
                    if dll_path.exists() {
                        // Add the directory to PATH so Windows can find the DLL
                        if let Ok(current_path) = env::var("PATH") {
                            let new_path = format!("{};{}", parent.display(), current_path);
                            env::set_var("PATH", &new_path);
                            logger::debug_lazy(|| {
                                format!(
                                    "Added {} to PATH for Python DLL discovery",
                                    parent.display()
                                )
                            });
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // Try to find Python in PATH
    if let Ok(output) = Command::new("where").arg("python").output() {
        if output.status.success() {
            let python_path = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = python_path.lines().next() {
                if let Some(parent) = PathBuf::from(first_line.trim()).parent() {
                    let dll_path = parent.join(&dll_name);
                    if dll_path.exists() {
                        logger::debug_lazy(|| {
                            format!("Found Python DLL at: {}", dll_path.display())
                        });
                        return Ok(());
                    }
                }
            }
        }
    }

    let find_hint = python_version.find_hint();
    let install_hint = python_version.install_hint();
    Err(BridgeError::PythonLibraryNotFound(format!(
        "Could not find {}.\n\n\
        This binary requires Python {} to be installed.\n\n\
        To fix this on Windows:\n\
        1. Install Python via uv: {}\n\
        2. Or download from https://www.python.org/downloads/\n\
        3. Ensure Python is in your PATH\n\n\
        If you installed Python via uv, try running:\n\
           {}",
        dll_name,
        python_version.requested(),
        install_hint,
        find_hint
    )))
}

/// Configure the Python virtual environment (legacy API compatibility)
pub fn configure_python_venv() -> Result<PythonEnvCompat, BridgeError> {
    let mut config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    let venv_path = ensure_configured_venv(&mut config)?;
    let interpreter = resolve_python_path(&venv_path)?;
    let python_home = resolve_python_home(&venv_path).ok();

    Ok(PythonEnvCompat {
        interpreter,
        python_home,
    })
}

/// Legacy compatibility struct for PythonEnvironment
#[derive(Debug, Clone)]
pub struct PythonEnvCompat {
    pub interpreter: PathBuf,
    pub python_home: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use crate::python_bridge::*;
    use r2x_config::{default_python_version, PythonRuntimeVersion};
    #[cfg(unix)]
    use std::env;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[cfg(unix)]
    static PATH_TEST_LOCK: once_cell::sync::Lazy<std::sync::Mutex<()>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(()));

    #[test]
    fn test_bridge_struct() {
        // Test that Bridge can be created
        let _bridge = Bridge { _marker: () };
    }

    #[test]
    fn test_runtime_python_version_defaults_to_build_python_version() {
        let config = Config::default();
        assert_eq!(
            runtime_python_version(&config)
                .ok()
                .map(|version| (version.requested().to_string(), version.abi().to_string())),
            Some((
                default_python_version().to_string(),
                default_python_version().to_string()
            ))
        );
    }

    #[test]
    fn test_runtime_python_version_uses_configured_version() {
        let config = Config {
            python_version: Some("3.13".to_string()),
            ..Config::default()
        };
        assert_eq!(
            runtime_python_version(&config)
                .ok()
                .map(|version| (version.requested().to_string(), version.abi().to_string())),
            Some(("3.13".to_string(), "3.13".to_string()))
        );
    }

    #[test]
    fn test_runtime_python_version_keeps_patch_request_but_uses_minor_abi() {
        let config = Config {
            python_version: Some("3.13.1".to_string()),
            ..Config::default()
        };
        assert_eq!(
            runtime_python_version(&config)
                .ok()
                .map(|version| (version.requested().to_string(), version.abi().to_string())),
            Some(("3.13.1".to_string(), "3.13".to_string()))
        );
    }

    #[test]
    fn test_runtime_python_version_ignores_blank_config_value() {
        let config = Config {
            python_version: Some("  ".to_string()),
            ..Config::default()
        };
        assert_eq!(
            runtime_python_version(&config)
                .ok()
                .map(|version| (version.requested().to_string(), version.abi().to_string())),
            Some((
                default_python_version().to_string(),
                default_python_version().to_string()
            ))
        );
    }

    #[test]
    fn test_runtime_python_version_rejects_unsupported_configured_version() {
        let config = Config {
            python_version: Some("3.10".to_string()),
            ..Config::default()
        };

        assert!(runtime_python_version(&config).is_err());
    }

    #[test]
    fn test_post_import_log_module_cache_skips_previously_enabled_modules() {
        let module_a = "r2x_test_cache_alpha";
        let module_b = "r2x_test_cache_beta";

        let pending = pending_post_import_log_modules(&[module_a, module_b]);
        assert!(pending.contains(&module_a.to_string()));
        assert!(pending.contains(&module_b.to_string()));

        mark_post_import_log_modules_enabled(&[module_a.to_string()]);

        let pending = pending_post_import_log_modules(&[module_a, module_b]);
        assert!(!pending.contains(&module_a.to_string()));
        assert!(pending.contains(&module_b.to_string()));
    }

    #[test]
    fn test_is_python_executable_name_variants() {
        assert!(is_python_executable_name("python"));
        assert!(is_python_executable_name("python.exe"));
        assert!(is_python_executable_name("python3"));
        assert!(is_python_executable_name("python3.exe"));
        assert!(is_python_executable_name("python3.12"));
        assert!(is_python_executable_name("python3.12.exe"));
        assert!(is_python_executable_name("PYTHON3.13.EXE"));
        assert!(!is_python_executable_name("pythonw.exe"));
        assert!(!is_python_executable_name("python-3.12.exe"));
    }

    #[test]
    fn test_normalize_python_home_bin_dir() {
        let home = PathBuf::from("/opt/python/bin");
        assert_eq!(normalize_python_home(&home), PathBuf::from("/opt/python"));
    }

    #[test]
    fn test_normalize_python_home_scripts_dir() {
        let home = PathBuf::from("/opt/python/Scripts");
        assert_eq!(normalize_python_home(&home), PathBuf::from("/opt/python"));
    }

    #[test]
    fn test_normalize_python_home_python_executable() {
        let home = PathBuf::from("/opt/python/python3.12");
        assert_eq!(normalize_python_home(&home), PathBuf::from("/opt/python"));
    }

    #[test]
    fn test_normalize_python_home_prefix_value() {
        let home = PathBuf::from("/opt/python/cpython-3.12.9-windows-x86_64-none");
        assert_eq!(normalize_python_home(&home), home);
    }

    #[test]
    fn test_resolve_python_home_preserves_prefix_from_pyvenv_cfg() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let venv_path = temp_dir.path().join(".venv");
        if fs::create_dir_all(&venv_path).is_err() {
            return;
        }

        let expected_prefix = temp_dir.path().join("uv-python-prefix");
        let pyvenv_cfg = format!("home = {}\n", expected_prefix.to_string_lossy());
        if fs::write(venv_path.join("pyvenv.cfg"), pyvenv_cfg).is_err() {
            return;
        }

        let result = resolve_python_home(&venv_path);
        assert!(result.is_ok());
        assert!(result.is_ok_and(|path| path == expected_prefix));
    }

    #[test]
    fn test_resolve_python_home_converts_bin_home_to_prefix() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let venv_path = temp_dir.path().join(".venv");
        if fs::create_dir_all(&venv_path).is_err() {
            return;
        }

        let expected_prefix = temp_dir.path().join("python-prefix");
        let home_bin = expected_prefix.join("bin");
        let pyvenv_cfg = format!("home = {}\n", home_bin.to_string_lossy());
        if fs::write(venv_path.join("pyvenv.cfg"), pyvenv_cfg).is_err() {
            return;
        }

        let result = resolve_python_home(&venv_path);
        assert!(result.is_ok());
        assert!(result.is_ok_and(|path| path == expected_prefix));
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_find_python_lib_via_uv_falls_back_from_patch_to_abi_query() {
        let Ok(_lock) = PATH_TEST_LOCK.lock() else {
            return;
        };
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };

        let python_prefix = temp_dir.path().join("cpython-3.13");
        let python_bin = python_prefix.join("bin").join("python3.13");
        let lib_dir = python_prefix.join("lib");
        if fs::create_dir_all(python_bin.parent().unwrap_or(temp_dir.path())).is_err() {
            return;
        }
        if fs::create_dir_all(&lib_dir).is_err() {
            return;
        }
        if fs::write(&python_bin, "").is_err() {
            return;
        }

        let lib_name = if cfg!(target_os = "macos") {
            "libpython3.13.dylib"
        } else {
            "libpython3.13.so"
        };
        if fs::write(lib_dir.join(lib_name), "").is_err() {
            return;
        }

        let uv = temp_dir.path().join("uv");
        if fs::write(
            &uv,
            format!(
                "#!/usr/bin/env sh\nif [ \"$1\" = \"python\" ] && [ \"$2\" = \"find\" ] && [ \"$3\" = \"3.13.1\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"python\" ] && [ \"$2\" = \"find\" ] && [ \"$3\" = \"3.13\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\nexit 1\n",
                python_bin.display()
            ),
        )
        .is_err()
        {
            return;
        }
        let Ok(metadata) = fs::metadata(&uv) else {
            return;
        };
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        if fs::set_permissions(&uv, permissions).is_err() {
            return;
        }

        let original_path = env::var_os("PATH");
        let mut path_entries = vec![temp_dir.path().to_path_buf()];
        if let Some(existing) = original_path.as_ref() {
            path_entries.extend(env::split_paths(existing));
        }
        let Ok(new_path) = env::join_paths(path_entries) else {
            return;
        };
        env::set_var("PATH", &new_path);

        let Ok(version) = PythonRuntimeVersion::parse("3.13.1") else {
            return;
        };
        let found = find_python_lib_via_uv(&version, &[lib_name.to_string()]);

        if let Some(path) = original_path {
            env::set_var("PATH", path);
        } else {
            env::remove_var("PATH");
        }

        assert_eq!(found, Some(lib_dir));
    }
}
