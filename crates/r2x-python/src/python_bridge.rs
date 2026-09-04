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
use r2x_config::Config;
use r2x_logger as logger;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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

/// Terminate the process safely when Python may be initialized.
pub fn process_exit(code: i32) -> ! {
    if BRIDGE_INSTANCE.get().is_some() {
        // Give the main thread a thread state so Py_Finalize()'s
        // PyEval_SaveThread() call can succeed.
        Python::attach(|_py| -> ! { std::process::exit(code) });
    }
    std::process::exit(code)
}

impl Bridge {
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self { _marker: () }
    }

    /// Get or initialize the bridge singleton
    pub fn get() -> Result<&'static Bridge, BridgeError> {
        match BRIDGE_INSTANCE.get_or_init(Bridge::initialize) {
            Ok(bridge) => Ok(bridge),
            Err(e) => Err(BridgeError::Initialization(format!("{}", e))),
        }
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
    fn enable_loguru_modules(py: Python, modules: &[&str]) -> Result<(), BridgeError> {
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
#[path = "python_bridge/tests.rs"]
mod tests;
