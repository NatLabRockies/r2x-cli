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
use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use r2x_config::Config;
use r2x_logger as logger;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Python bridge for plugin execution
pub struct Bridge {
    /// Placeholder field for future extension
    _marker: (),
}

/// Global bridge singleton
static BRIDGE_INSTANCE: OnceCell<Result<Bridge, BridgeError>> = OnceCell::new();

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
        let venv_path = PathBuf::from(config.get_venv_path());

        if !venv_path.exists() {
            // Create venv using the compiled Python version
            Self::create_venv(&config, &venv_path)?;
        }

        // Resolve PYTHONHOME from venv's pyvenv.cfg
        let python_home = resolve_python_home(&venv_path)?;
        env::set_var("PYTHONHOME", &python_home);
        logger::debug(&format!("Set PYTHONHOME={}", python_home.display()));

        // Get site-packages path
        let site_packages = resolve_site_package_path(&venv_path)?;

        // Add site-packages to PYTHONPATH
        Self::configure_python_path(&site_packages);

        // Check if Python library is available before initializing
        check_python_library_available()?;

        // Initialize PyO3
        logger::debug("Initializing PyO3...");
        let pyo3_start = std::time::Instant::now();
        pyo3::Python::initialize();
        logger::debug(&format!(
            "pyo3::Python::initialize took: {:?}",
            pyo3_start.elapsed()
        ));

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

        logger::debug(&format!(
            "Total bridge initialization took: {:?}",
            start_time.elapsed()
        ));

        Ok(Bridge { _marker: () })
    }

    /// Create a virtual environment
    ///
    /// Uses the compiled Python version to ensure compatibility with PyO3.
    fn create_venv(config: &Config, venv_path: &PathBuf) -> Result<(), BridgeError> {
        logger::step(&format!(
            "Creating Python virtual environment at: {}",
            venv_path.display()
        ));

        let python_version = get_compiled_python_version();

        // Try uv first
        if let Some(ref uv_path) = config.uv_path {
            let output = Command::new(uv_path)
                .arg("venv")
                .arg(venv_path)
                .arg("--python")
                .arg(&python_version)
                .output()?;

            if output.status.success() {
                logger::success("Virtual environment created successfully");
                return Ok(());
            }

            let stderr = String::from_utf8_lossy(&output.stderr);
            logger::debug(&format!("uv venv failed: {}", stderr));
        }

        // Fallback to python3 -m venv
        let python_cmd = format!("python{}", python_version);
        let output = Command::new(&python_cmd)
            .args(["-m", "venv"])
            .arg(venv_path)
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                logger::success("Virtual environment created successfully");
                return Ok(());
            }
        }

        // Try generic python3
        let output = Command::new("python3")
            .args(["-m", "venv"])
            .arg(venv_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BridgeError::Initialization(format!(
                "Failed to create virtual environment: {}",
                stderr
            )));
        }

        logger::success("Virtual environment created successfully");
        Ok(())
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
            logger::debug(&format!(
                "Updated PYTHONPATH to include {}",
                site_packages.display()
            ));
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

            let file_ops = PyModule::import(py, "r2x_core.utils.file_operations").map_err(|e| {
                BridgeError::Python(format!(
                    "Failed to import r2x_core.utils.file_operations: {}",
                    e
                ))
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
    fn configure_python_logging() -> Result<(), BridgeError> {
        let log_python = logger::get_log_python();
        if !log_python {
            return Ok(());
        }

        let verbosity = logger::get_verbosity();
        logger::debug(&format!(
            "Configuring Python logging with verbosity={}",
            verbosity
        ));

        pyo3::Python::attach(|py| {
            let logger_module = PyModule::import(py, "r2x_core.logger").map_err(|e| {
                logger::warn(&format!("Failed to import r2x_core.logger: {}", e));
                BridgeError::Import("r2x_core.logger".to_string(), format!("{}", e))
            })?;
            let setup_logging = logger_module.getattr("setup_logging").map_err(|e| {
                logger::warn(&format!("Failed to get setup_logging function: {}", e));
                BridgeError::Python(format!("setup_logging not found: {}", e))
            })?;
            setup_logging.call1((verbosity,))?;

            let loguru = PyModule::import(py, "loguru")?;
            let logger_obj = loguru.getattr("logger")?;
            logger_obj.call_method1("enable", ("r2x_core",))?;
            logger_obj.call_method1("enable", ("r2x_reeds",))?;
            logger_obj.call_method1("enable", ("r2x_plexos",))?;
            logger_obj.call_method1("enable", ("r2x_sienna",))?;

            Ok::<(), BridgeError>(())
        })
    }

    /// Reconfigure Python logging for a specific plugin
    pub fn reconfigure_logging_for_plugin(_plugin_name: &str) -> Result<(), BridgeError> {
        Self::configure_python_logging()
    }
}

/// Resolve PYTHONHOME from the venv's pyvenv.cfg file
///
/// The pyvenv.cfg file contains:
/// ```text
/// home = /path/to/python/installation
/// include-system-site-packages = false
/// version = 3.12.1
/// ```
///
/// The `home` field points to the Python installation's bin directory,
/// so we return its parent as PYTHONHOME.
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
        if line.starts_with("home") {
            if let Some((_key, value)) = line.split_once('=') {
                let home_bin = PathBuf::from(value.trim());
                // The 'home' field points to the bin directory, return its parent
                if let Some(parent) = home_bin.parent() {
                    logger::debug(&format!(
                        "Resolved PYTHONHOME from pyvenv.cfg: {}",
                        parent.display()
                    ));
                    return Ok(parent.to_path_buf());
                }
                // If no parent, use the path directly (unusual case)
                return Ok(home_bin);
            }
        }
    }

    Err(BridgeError::Initialization(format!(
        "Could not find 'home' in pyvenv.cfg: {}",
        pyvenv_cfg.display()
    )))
}

/// Get the Python version that PyO3 was compiled against
///
/// Returns the version string (e.g., "3.12") based on PyO3's abi3 feature.
/// This should match the PYO3_PYTHON environment variable used during build.
fn get_compiled_python_version() -> String {
    // PyO3 with abi3-py311 is compatible with Python 3.11+
    // The actual version depends on PYO3_PYTHON at build time
    // Default to 3.12 which is the version in the justfile
    "3.12".to_string()
}

/// Check if Python library is available before attempting to initialize PyO3.
///
/// This provides better error messages than the cryptic dyld errors on macOS
/// or DLL loading errors on Windows.
fn check_python_library_available() -> Result<(), BridgeError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let python_version = get_compiled_python_version();

        #[cfg(target_os = "macos")]
        let (lib_names, search_paths, env_var) = (
            vec![format!("libpython{}.dylib", python_version)],
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
                format!("libpython{}.so", python_version),
                format!("libpython{}.so.1.0", python_version),
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
        if let Some(lib_dir) = find_python_lib_via_uv(&python_version, &lib_names) {
            prepend_to_env_path(env_var, &lib_dir);
            logger::debug(&format!(
                "Set {} to include: {}",
                env_var,
                lib_dir.display()
            ));
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
        if let Err(e) = setup_windows_dll_path() {
            logger::debug(&format!("Windows DLL path setup note: {}", e));
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
                logger::debug(&format!("Found Python library at: {}", lib_path.display()));
                return true;
            }
        }
    }
    false
}

/// Try to find Python library via uv python find command.
/// Returns the lib directory path if found.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn find_python_lib_via_uv(python_version: &str, lib_names: &[String]) -> Option<PathBuf> {
    let output = Command::new("uv")
        .args(["python", "find", python_version])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let python_path = String::from_utf8_lossy(&output.stdout);
    let python_path = python_path.trim();

    // Python binary is in bin/, lib is in ../lib/
    let lib_dir = PathBuf::from(python_path).parent()?.parent()?.join("lib");

    for lib_name in lib_names {
        let lib_path = lib_dir.join(lib_name);
        if lib_path.exists() {
            logger::debug(&format!(
                "Found Python library via uv: {}",
                lib_path.display()
            ));
            return Some(lib_dir);
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
fn setup_windows_dll_path() -> Result<(), BridgeError> {
    let python_version = get_compiled_python_version();
    let dll_name = format!("python{}.dll", python_version.replace(".", ""));

    // Try to find Python via uv first
    let output = Command::new("uv")
        .args(["python", "find", &python_version])
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
                        logger::debug(&format!(
                            "Added {} to PATH for Python DLL discovery",
                            parent.display()
                        ));
                        return Ok(());
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
                        logger::debug(&format!("Found Python DLL at: {}", dll_path.display()));
                        return Ok(());
                    }
                }
            }
        }
    }

    Err(BridgeError::PythonLibraryNotFound(format!(
        "Could not find {}.\n\n\
        This binary requires Python {} to be installed.\n\n\
        To fix this on Windows:\n\
        1. Install Python via uv: uv python install {}\n\
        2. Or download from https://www.python.org/downloads/\n\
        3. Ensure Python is in your PATH\n\n\
        If you installed Python via uv, try running:\n\
           uv python find {}",
        dll_name, python_version, python_version, python_version
    )))
}

/// Configure the Python virtual environment (legacy API compatibility)
pub fn configure_python_venv() -> Result<PythonEnvCompat, BridgeError> {
    let config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    let venv_path = PathBuf::from(config.get_venv_path());

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
    use super::*;

    #[test]
    fn test_bridge_struct() {
        // Test that Bridge can be created
        let _bridge = Bridge { _marker: () };
    }

    #[test]
    fn test_get_compiled_python_version() {
        let version = get_compiled_python_version();
        assert!(version.starts_with("3."));
    }
}
