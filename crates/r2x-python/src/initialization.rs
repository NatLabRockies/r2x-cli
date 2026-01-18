//! Python environment initialization and setup
//!
//! This module handles all Python interpreter initialization, virtual environment
//! configuration, and environment setup required before the bridge can be used.

use super::utils::{resolve_python_path, resolve_site_package_path};
use crate::errors::BridgeError;
use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use r2x_config::Config;
use r2x_logger as logger;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const PYTHON_BIN_DIR_NAME: &str = "Scripts";
#[cfg(not(windows))]
const PYTHON_BIN_DIR_NAME: &str = "bin";

/// Get the Python version this binary was compiled against
/// Returns version string like "3.12"
/// This requires querying the Python interpreter, so it can only be called
/// after Python initialization or during runtime checks
fn get_compiled_python_version() -> Result<String, BridgeError> {
    // Use the PYO3_PYTHON environment variable set at build time if available
    // Otherwise, we need to query Python at runtime
    if let Ok(pyo3_python) = env::var("PYO3_PYTHON") {
        // Try to extract version from path like "python3.12"
        if let Some(version) = pyo3_python.split('/').last() {
            if let Some(ver) = version.strip_prefix("python") {
                if ver.matches('.').count() >= 1 {
                    let parts: Vec<&str> = ver.split('.').take(2).collect();
                    if parts.len() == 2 {
                        return Ok(format!("{}.{}", parts[0], parts[1]));
                    }
                }
            }
        }
    }

    // Fallback: default to 3.12 which is our standard version
    Ok("3.12".to_string())
}

/// Ensure the compiled Python version is installed via uv
/// If not found, automatically install it with user notification
fn ensure_python_installed(config: &Config) -> Result<(), BridgeError> {
    let uv_path = config
        .uv_path
        .as_ref()
        .ok_or_else(|| BridgeError::Initialization("UV path not configured".to_string()))?;

    let compiled_version = get_compiled_python_version()?;

    logger::debug(&format!(
        "Checking if Python {} is installed",
        compiled_version
    ));

    // Check if Python is already installed
    let check_output = Command::new(uv_path)
        .arg("python")
        .arg("list")
        .arg("--only-installed")
        .arg(&compiled_version)
        .output()
        .map_err(|e| {
            BridgeError::Initialization(format!("Failed to check Python installation: {}", e))
        })?;

    if check_output.status.success() {
        let stdout = String::from_utf8_lossy(&check_output.stdout);
        // Check if the output contains the version (means it's installed)
        if stdout.contains(&compiled_version) {
            logger::debug(&format!("Python {} is already installed", compiled_version));
            return Ok(());
        }
    }

    // Python not found, install it
    logger::step(&format!(
        "Installing Python {} (required by this binary)...",
        compiled_version
    ));
    logger::info(&format!(
        "This binary was compiled against Python {}. Installing now.",
        compiled_version
    ));

    let install_output = Command::new(uv_path)
        .arg("python")
        .arg("install")
        .arg(&compiled_version)
        .output()
        .map_err(|e| {
            BridgeError::Initialization(format!(
                "Failed to install Python {}: {}",
                compiled_version, e
            ))
        })?;

    logger::capture_output(
        &format!("uv python install {}", compiled_version),
        &install_output,
    );

    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        return Err(BridgeError::Initialization(format!(
            "Failed to install Python {}.\n\
            Please ensure uv can access the network.\n\
            Error: {}",
            compiled_version, stderr
        )));
    }

    logger::success(&format!(
        "Python {} installed successfully",
        compiled_version
    ));
    Ok(())
}

pub struct Bridge {}

#[derive(Debug, Clone)]
pub struct PythonEnvironment {
    pub interpreter: PathBuf,
    pub python_home: Option<PathBuf>,
}

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

        let python_env = configure_python_venv()?;
        let python_path = python_env.interpreter.clone();

        let mut config = Config::load()
            .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;
        let cache_path = config.ensure_cache_path().map_err(|e| {
            BridgeError::Initialization(format!("Failed to ensure cache path: {}", e))
        })?;
        let venv_path = PathBuf::from(config.get_venv_path());
        let site_packages = resolve_site_package_path(&venv_path)?;
        if let Some(ref home) = python_env.python_home {
            configure_embedded_python_env(home, &site_packages);
        } else {
            logger::debug("Using default embedded Python search paths");
        }

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
        Self::configure_python_logging_with_plugin(None)
    }

    /// Configure Python loguru logging with optional plugin context
    fn configure_python_logging_with_plugin(plugin_name: Option<&str>) -> Result<(), BridgeError> {
        let log_file = logger::get_log_path_string();
        let verbosity = logger::get_verbosity();
        let log_level = match verbosity {
            0 => "WARNING",
            1 => "INFO",
            2 => "DEBUG",
            _ => "TRACE",
        };

        // Format to match Rust logger with plugin context
        let fmt = if let Some(name) = plugin_name {
            format!(
                "[{{time:YYYY-MM-DD HH:mm:ss}}] [PYTHON] [{}] {{level: <8}} {{message}}",
                name
            )
        } else {
            "[{time:YYYY-MM-DD HH:mm:ss}] [PYTHON] {level: <8} {message}".to_string()
        };

        // Check if Python logs should be shown on console
        let enable_console = logger::get_log_python();

        // Check if stdout should be suppressed from logs
        let no_stdout = logger::get_no_stdout();

        logger::debug(&format!(
            "Configuring Python logging with level={}, file={}, enable_console={}, no_stdout={}, plugin={}",
            log_level, log_file, enable_console, no_stdout, plugin_name.unwrap_or("none")
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
            kwargs.set_item("fmt", &fmt)?;
            kwargs.set_item("enable_console_log", enable_console)?;
            kwargs.set_item("suppress_stdout", no_stdout)?;
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

    /// Reconfigure Python logging for a specific plugin
    pub fn reconfigure_logging_for_plugin(plugin_name: &str) -> Result<(), BridgeError> {
        Self::configure_python_logging_with_plugin(Some(plugin_name))
    }
}

/// Helper: get python3.X directory inside venv lib/
/// Detect the Python version from the embedded interpreter and store it in config
///
/// This function:
/// 1. Gets the Python version from sys.version_info (the actual running version)
/// 2. Compares it with the compiled version from PyO3
/// 3. Warns if there's a mismatch (which could cause runtime issues)
/// 4. Stores the compiled version in config for future use
fn detect_and_store_python_version() -> Result<(), BridgeError> {
    let mut config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    // Get the compiled version
    let compiled_version = get_compiled_python_version()?;

    // Get Python version from sys.version_info (the actual running version)
    let runtime_version = pyo3::Python::attach(|py| {
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

    logger::debug(&format!(
        "Python versions - compiled: {}, runtime: {}",
        compiled_version, runtime_version
    ));

    // Verify that runtime matches compiled version
    if runtime_version != compiled_version {
        logger::warn(&format!(
            "Python version mismatch! Binary compiled with {}, but running {}. This may cause undefined behavior.",
            compiled_version, runtime_version
        ));
    }

    // Store/update the compiled version in config
    if config.python_version.as_deref() != Some(&compiled_version) {
        config.python_version = Some(compiled_version.clone());
        config
            .save()
            .map_err(|e| BridgeError::Initialization(format!("Failed to save config: {}", e)))?;
        logger::debug(&format!(
            "Stored compiled Python version {} in config",
            compiled_version
        ));
    }

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
pub fn configure_python_venv() -> Result<PythonEnvironment, BridgeError> {
    let mut config = Config::load()
        .map_err(|e| BridgeError::Initialization(format!("Failed to load config: {}", e)))?;

    // Get the compiled Python version - this is what we MUST use
    let compiled_version = get_compiled_python_version()?;

    // Update config to use the compiled version if it's different
    if config.python_version.as_deref() != Some(&compiled_version) {
        logger::debug(&format!(
            "Updating config to use compiled Python version: {}",
            compiled_version
        ));
        config.python_version = Some(compiled_version.clone());
        config
            .save()
            .map_err(|e| BridgeError::Initialization(format!("Failed to save config: {}", e)))?;
    }

    // Ensure Python is installed before trying to create venv
    ensure_python_installed(&config)?;

    let venv_path = PathBuf::from(config.get_venv_path());

    let python_path_result = resolve_python_path(&venv_path);

    if python_path_result.is_err() {
        logger::debug("Could not resolve Python path");
    }

    let mut python_path = python_path_result.unwrap_or_else(|_| PathBuf::new());

    // Create venv only when it doesn't exist
    if !venv_path.exists() {
        logger::step(&format!(
            "Creating Python virtual environment at: {}",
            venv_path.display()
        ));

        let uv_path = config
            .ensure_uv_path()
            .map_err(|e| BridgeError::Initialization(format!("Failed to ensure uv: {}", e)))?;

        logger::info(&format!(
            "Using Python {} (version this binary was compiled with)",
            compiled_version
        ));

        let output = Command::new(&uv_path)
            .arg("venv")
            .arg(&venv_path)
            .arg("--python")
            .arg(&compiled_version)
            .output()?;

        logger::capture_output(&format!("uv venv --python {}", compiled_version), &output);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BridgeError::Initialization(format!(
                "Failed to create Python virtual environment with Python {}.\n\
                Error: {}",
                compiled_version, stderr
            )));
        }

        python_path = resolve_python_path(&venv_path).unwrap_or_else(|_| PathBuf::new());

        if python_path.as_os_str().is_empty() || !python_path.exists() {
            if let Ok(entries) = std::fs::read_dir(venv_path.join(PYTHON_BIN_DIR_NAME)) {
                let names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                logger::debug(&format!("Venv bin contents after creation: {:?}", names));
            }
            return Err(BridgeError::Initialization(
                "Failed to locate Python executable after creating venv".to_string(),
            ));
        }
    }

    if python_path.as_os_str().is_empty() || !python_path.exists() {
        logger::warn(
            "Python binary not found in configured venv; attempting uv-managed Python fallback",
        );
        if let Some((fallback, home)) = find_uv_python(&config)? {
            return Ok(PythonEnvironment {
                interpreter: fallback,
                python_home: Some(home),
            });
        }

        return Err(BridgeError::Initialization(format!(
            "Failed to locate a usable Python interpreter.\n\
            This binary requires Python {}.",
            compiled_version
        )));
    }

    let python_home = resolve_python_home(&venv_path);

    Ok(PythonEnvironment {
        interpreter: python_path,
        python_home,
    })
}

fn configure_embedded_python_env(python_home: &Path, site_packages: &Path) {
    let home = python_home.to_string_lossy().to_string();
    env::set_var("PYTHONHOME", &home);
    logger::debug(&format!("Set PYTHONHOME={}", home));

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

fn resolve_python_home(venv_path: &Path) -> Option<PathBuf> {
    let cfg_path = venv_path.join("pyvenv.cfg");
    let contents = fs::read_to_string(&cfg_path).ok()?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("home") {
            let parts: Vec<_> = trimmed.splitn(2, '=').collect();
            if parts.len() == 2 {
                let mut path = PathBuf::from(parts[1].trim());
                if path.ends_with("bin") || path.ends_with("Scripts") {
                    path = path.parent().map(PathBuf::from).unwrap_or(path);
                }
                logger::debug(&format!(
                    "Resolved base Python home {} from {}",
                    path.display(),
                    cfg_path.display()
                ));
                return Some(path);
            }
        }
    }
    logger::debug(&format!(
        "Failed to resolve base Python home from {}",
        cfg_path.display()
    ));
    None
}

/// Find and use uv-managed Python instead of system Python
/// This avoids conflicts with system Python installations and the Windows Store popup
fn find_uv_python(config: &Config) -> Result<Option<(PathBuf, PathBuf)>, BridgeError> {
    let uv_path = config
        .uv_path
        .as_ref()
        .ok_or_else(|| BridgeError::Initialization("UV path not configured".to_string()))?;

    // Use the compiled Python version - this is what the binary requires
    let compiled_version = get_compiled_python_version()?;

    logger::debug(&format!(
        "Attempting to use uv-managed Python {} (compiled version)",
        compiled_version
    ));

    // Use `uv run python` to get the Python path
    // This ensures we use uv's managed Python installations
    let output = Command::new(uv_path)
        .arg("run")
        .arg("--python")
        .arg(&compiled_version)
        .arg("python")
        .arg("-c")
        .arg("import sys; print(sys.executable); print(sys.base_prefix)")
        .output()
        .map_err(|e| BridgeError::Initialization(format!("Failed to run uv python: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        logger::debug(&format!(
            "Failed to probe uv-managed Python {} (status {:?}): {}",
            compiled_version,
            output.status.code(),
            stderr
        ));
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut lines: Vec<String> = stdout.lines().map(|s| s.trim().to_string()).collect();

    if lines.len() < 2 {
        logger::debug("Unexpected output from uv python probe");
        return Ok(None);
    }

    let prefix = PathBuf::from(lines.pop().unwrap_or_default());
    let executable = PathBuf::from(lines.pop().unwrap_or_default());

    if !executable.exists() {
        logger::debug(&format!(
            "UV-managed Python executable does not exist: {}",
            executable.display()
        ));
        return Ok(None);
    }

    logger::info(&format!(
        "Using uv-managed Python {} at {}",
        compiled_version,
        executable.display()
    ));

    Ok(Some((executable, prefix)))
}
