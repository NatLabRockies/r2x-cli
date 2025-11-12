//! Python environment initialization and setup
//!
//! This module handles all Python interpreter initialization, virtual environment
//! configuration, and environment setup required before the bridge can be used.

use super::utils::*;
use crate::errors::BridgeError;
use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use r2x_config::Config;
use r2x_logger as logger;
use std::path::PathBuf;
use std::process::Command;

pub struct Bridge {}

static BRIDGE_INSTANCE: OnceCell<Result<Bridge, BridgeError>> = OnceCell::new();

impl Bridge {
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
    /// - Configure venv/python environment
    /// - Add venv site-packages to sys.path
    /// - Ensure r2x-core is installed
    fn initialize() -> Result<Bridge, BridgeError> {
        let start_time = std::time::Instant::now();

        let python_path = configure_python_venv()?;

        let mut config = Config::load()
            .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;
        let cache_path = config.ensure_cache_path().map_err(|e| {
            BridgeError::Initialization(format!("Failed to ensure cache path: {}", e))
        })?;

        logger::debug(&format!(
            "Initializing Python bridge with: {}",
            python_path.display()
        ));

        let pyo3_start = std::time::Instant::now();
        pyo3::Python::initialize();
        logger::debug(&format!(
            "pyo3::Python::initialize took: {:?}",
            pyo3_start.elapsed()
        ));

        // Enable Python bytecode generation for faster subsequent imports
        // This overrides PYTHONDONTWRITEBYTECODE if set in the environment
        pyo3::Python::attach(|py| {
            let sys = PyModule::import(py, "sys")
                .map_err(|e| BridgeError::Python(format!("Failed to import sys module: {}", e)))?;
            sys.setattr("dont_write_bytecode", false).map_err(|e| {
                BridgeError::Python(format!("Failed to enable bytecode generation: {}", e))
            })?;
            Ok::<(), BridgeError>(())
        })?;
        logger::debug("Enabled Python bytecode generation");

        // Add site-packages from venv to sys.path so imports work as expected
        let venv_path = PathBuf::from(config.get_venv_path());

        let lib_dir = venv_path.join(PYTHON_LIB_DIR);
        logger::debug(&format!(
            "lib_dir: {}, exists: {}",
            lib_dir.display(),
            lib_dir.exists()
        ));
        if !lib_dir.exists() {
            return Err(BridgeError::VenvNotFound(venv_path.to_path_buf()));
        }

        // Find the python3.X directory inside lib/
        use std::fs;
        let python_version_dir = fs::read_dir(&lib_dir)
            .map_err(|e| {
                BridgeError::Initialization(format!("Failed to read lib directory: {}", e))
            })?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("python"))
            .ok_or_else(|| {
                BridgeError::Initialization("No python3.X directory found in venv/lib".to_string())
            })?;

        let site_packages = python_version_dir.path().join(SITE_PACKAGES);
        logger::debug(&format!(
            "site_packages: {}, exists: {}",
            site_packages.display(),
            site_packages.exists()
        ));

        pyo3::Python::attach(|py| {
            let site = PyModule::import(py, "site")
                .map_err(|e| BridgeError::Python(format!("Failed to import site module: {}", e)))?;
            site.call_method1("addsitedir", (site_packages.to_str().unwrap(),))
                .map_err(|e| BridgeError::Python(format!("Failed to add site directory: {}", e)))?;
            Ok::<(), BridgeError>(())
        })?;

        let sitedir_start = std::time::Instant::now();
        logger::debug(&format!(
            "Site packages setup completed in: {:?}",
            sitedir_start.elapsed()
        ));

        // Detect and store the compiled Python version in config if not already set
        let version_start = std::time::Instant::now();
        detect_and_store_python_version()?;
        logger::debug(&format!(
            "Python version detection took: {:?}",
            version_start.elapsed()
        ));

        configure_python_cache(&cache_path)?;

        // r2x_core is now installed during venv creation, so no need to check here

        // Configure Python loguru to write to the same log file as Rust
        // Python logs always go to file, --log-python flag controls console output
        logger::debug("Starting Python logging configuration...");
        if let Err(e) = Self::configure_python_logging() {
            logger::warn(&format!("Python logging configuration failed: {}", e));
        }
        logger::debug("Python logging configuration completed");

        logger::debug(&format!(
            "Total bridge initialization took: {:?}",
            start_time.elapsed()
        ));
        Ok(Bridge {})
    }

    /// Configure Python loguru logging to integrate with Rust logger
    fn configure_python_logging() -> Result<(), BridgeError> {
        let log_file = logger::get_log_path_string();
        let verbosity = logger::get_verbosity();
        let log_level = match verbosity {
            0 => "WARNING",
            1 => "INFO",
            2 => "DEBUG",
            _ => "TRACE",
        };

        // Format to match Rust logger: [YYYY-MM-DD HH:MM:SS] [PYTHON] LEVEL message
        let fmt = "[{time:YYYY-MM-DD HH:mm:ss}] [PYTHON] {level: <8} {message}";

        // Check if Python logs should be shown on console
        let enable_console = logger::get_log_python();

        logger::debug(&format!(
            "Configuring Python logging with level={}, file={}, enable_console={}",
            log_level, log_file, enable_console
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
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("level", log_level)?;
            kwargs.set_item("log_file", &log_file)?;
            kwargs.set_item("fmt", fmt)?;
            kwargs.set_item("enable_console_log", enable_console)?;
            setup_logging.call((), Some(&kwargs))?;

            // Explicitly enable logging for r2x modules
            let loguru = PyModule::import(py, "loguru")?;
            let logger = loguru.getattr("logger")?;
            logger.call_method1("enable", ("r2x_core",))?;
            logger.call_method1("enable", ("r2x_reeds",))?;
            logger.call_method1("enable", ("r2x_plexos",))?;
            logger.call_method1("enable", ("r2x_sienna",))?;

            Ok::<(), BridgeError>(())
        })
    }
}

/// Helper: get python3.X directory inside venv lib/
/// Detect the Python version from the embedded interpreter and store it in config
///
/// This function:
/// 1. Gets the Python version from sys.version_info (the compiled/embedded version)
/// 2. Compares it with what's stored in config
/// 3. If missing or mismatched, updates config to the actual version
/// 4. Logs warnings if there's a mismatch (indicates config was manually edited)
fn detect_and_store_python_version() -> Result<(), BridgeError> {
    let mut config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    // Get Python version from sys.version_info (the actual compiled version)
    let version_str = pyo3::Python::attach(|py| {
        let sys = PyModule::import(py, "sys")
            .map_err(|e| BridgeError::Python(format!("Failed to import sys: {}", e)))?;
        let version_info = sys
            .getattr("version_info")
            .map_err(|e| BridgeError::Python(format!("Failed to get version_info: {}", e)))?;

        let major = version_info
            .getattr("major")
            .map_err(|e| BridgeError::Python(format!("Failed to get major: {}", e)))?
            .extract::<i32>()
            .map_err(|e| BridgeError::Python(format!("Failed to extract major: {}", e)))?;

        let minor = version_info
            .getattr("minor")
            .map_err(|e| BridgeError::Python(format!("Failed to get minor: {}", e)))?
            .extract::<i32>()
            .map_err(|e| BridgeError::Python(format!("Failed to extract minor: {}", e)))?;

        Ok::<String, BridgeError>(format!("{}.{}", major, minor))
    })?;

    logger::debug(&format!("Detected Python version: {}", version_str));

    // Check if config version matches detected version
    if let Some(ref config_version) = config.python_version {
        if config_version == &version_str {
            // Versions match, nothing to do
            return Ok(());
        } else {
            // Mismatch detected - config was likely manually edited
            logger::warn(&format!(
                "Python version mismatch: binary was compiled with {}, but config shows {}. Updating config to match compiled version.",
                version_str, config_version
            ));
        }
    } else {
        // First time detection
        logger::debug("First time detecting Python version for this binary");
    }

    // Store/update the actual compiled version in config
    config.python_version = Some(version_str.clone());
    config
        .save()
        .map_err(|e| BridgeError::Initialization(format!("Failed to save config: {}", e)))?;

    logger::info(&format!("Python version {} stored in config", version_str));

    Ok(())
}

fn configure_python_cache(cache_path: &str) -> Result<(), BridgeError> {
    std::fs::create_dir_all(cache_path).map_err(|e| {
        BridgeError::Initialization(format!("Failed to create cache directory: {}", e))
    })?;
    std::env::set_var("R2X_CACHE_PATH", cache_path);

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
            .map_err(|e| BridgeError::Python(format!("Failed to override cache path: {}", e)))?;

        Ok::<(), BridgeError>(())
    })?;

    Ok(())
}

/// Configure the Python virtual environment before PyO3 initialization
pub fn configure_python_venv() -> Result<PathBuf, BridgeError> {
    let mut config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    let venv_path = PathBuf::from(config.get_venv_path());

    let python_path = venv_path.join(PYTHON_BIN_DIR).join(PYTHON_EXE);

    // Create venv if it doesn't exist
    if !venv_path.exists() || !python_path.exists() {
        logger::step(&format!(
            "Creating Python virtual environment at: {}",
            venv_path.display()
        ));

        let uv_path = config
            .ensure_uv_path()
            .map_err(|e| BridgeError::Initialization(format!("Failed to ensure uv: {}", e)))?;

        // Use the Python version from config, or default to 3.12
        let python_version = config.python_version.as_deref().unwrap_or("3.12");

        let output = Command::new(&uv_path)
            .arg("venv")
            .arg(&venv_path)
            .arg("--python")
            .arg(python_version)
            .output()?;

        logger::capture_output(&format!("uv venv --python {}", python_version), &output);

        if !output.status.success() {
            return Err(BridgeError::Initialization(
                "Failed to create Python virtual environment".to_string(),
            ));
        }
    }

    Ok(python_path)
}
