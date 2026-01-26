//! Python bridge initialization with runtime discovery
//!
//! This module handles lazy initialization of the Python bridge using
//! runtime discovery of Python installations. It uses OnceCell for
//! thread-safe singleton initialization.

use crate::errors::BridgeError;
use crate::python_discovery::PythonEnvironment;
use crate::python_loader::PythonLoader;
use crate::utils::resolve_site_package_path;
use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use r2x_config::Config;
use r2x_logger as logger;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Python bridge for plugin execution
pub struct Bridge {
    /// Keep the loader alive to prevent library unload
    _loader: Option<PythonLoader>,
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

        PythonEnvironment::discover(&config).is_ok()
    }

    /// Initialize Python interpreter and configure environment
    ///
    /// This performs:
    /// 1. Discover Python installation (uv-managed or system)
    /// 2. Load Python shared library dynamically
    /// 3. Set PYTHONHOME and initialize PyO3
    /// 4. Configure venv and site-packages
    fn initialize() -> Result<Bridge, BridgeError> {
        let start_time = std::time::Instant::now();

        let mut config = Config::load()
            .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

        // Discover Python installation
        logger::debug("Discovering Python installation...");
        let python_env = PythonEnvironment::discover(&config)?;

        // Load Python shared library
        logger::debug(&format!(
            "Loading Python library from: {}",
            python_env.lib_path.display()
        ));
        let loader = PythonLoader::load(&python_env.lib_path)?;

        // Set environment before PyO3 initialization
        env::set_var("PYTHONHOME", &python_env.prefix);
        logger::debug(&format!("Set PYTHONHOME={}", python_env.prefix.display()));

        // Store Python version in config
        let version_str = format!("{}.{}", python_env.version.0, python_env.version.1);
        if config.python_version.as_deref() != Some(&version_str) {
            config.python_version = Some(version_str.clone());
            let _ = config.save();
        }

        // Ensure venv exists and configure
        let venv_path = PathBuf::from(config.get_venv_path());
        let venv_exists = venv_path.exists();

        if !venv_exists {
            // Create venv using uv or discovered Python
            Self::create_venv(&config, &python_env, &venv_path)?;
        }

        // Get site-packages path
        let site_packages = resolve_site_package_path(&venv_path)?;

        // Add site-packages to PYTHONPATH
        Self::configure_python_path(&site_packages);

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
            site.call_method1("addsitedir", (site_packages.to_str().unwrap(),))
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

        Ok(Bridge {
            _loader: Some(loader),
        })
    }

    /// Create a virtual environment using uv or Python directly
    fn create_venv(
        config: &Config,
        python_env: &PythonEnvironment,
        venv_path: &PathBuf,
    ) -> Result<(), BridgeError> {
        logger::step(&format!(
            "Creating Python virtual environment at: {}",
            venv_path.display()
        ));

        let version_str = format!("{}.{}", python_env.version.0, python_env.version.1);

        // Try uv first
        if let Some(ref uv_path) = config.uv_path {
            let output = Command::new(uv_path)
                .arg("venv")
                .arg(venv_path)
                .arg("--python")
                .arg(&version_str)
                .output()?;

            if output.status.success() {
                logger::success("Virtual environment created successfully");
                return Ok(());
            }

            let stderr = String::from_utf8_lossy(&output.stderr);
            logger::debug(&format!("uv venv failed: {}", stderr));
        }

        // Fallback to Python -m venv
        let output = Command::new(&python_env.executable)
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
            let filename = std::ffi::CString::new("r2x_cache_patch.py").unwrap();
            let module_name = std::ffi::CString::new("r2x_cache_patch").unwrap();
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

/// Configure the Python virtual environment (legacy API compatibility)
pub fn configure_python_venv() -> Result<PythonEnvCompat, BridgeError> {
    let config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    let python_env = PythonEnvironment::discover(&config)?;

    Ok(PythonEnvCompat {
        interpreter: python_env.executable,
        python_home: Some(python_env.prefix),
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
        // Test that Bridge can be created with None loader
        let _bridge = Bridge { _loader: None };
    }
}
