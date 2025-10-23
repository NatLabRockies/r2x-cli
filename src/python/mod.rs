//! Python integration module for r2x

mod interpreter;
pub mod plugin;
pub mod plugin_cache;
pub mod uv;
pub mod venv;

use crate::Result;
use std::path::PathBuf;
use std::sync::OnceLock;

static VENV_PATH: OnceLock<PathBuf> = OnceLock::new();
static PYTHON_INITIALIZED: OnceLock<()> = OnceLock::new();

fn load_config() -> crate::config::Config {
    let config_path = dirs::cache_dir().map(|d| d.join("r2x").join("config.toml"));

    if let Some(path) = config_path {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(config) = toml::from_str(&content) {
                    return config;
                }
            }
        }
    }

    crate::config::Config::default()
}

pub fn prepare_environment() -> Result<()> {
    let config = load_config();
    let python_version = &config.python.version;

    let uv_path = uv::ensure_uv()?;
    uv::ensure_python(&uv_path, python_version)?;
    let venv_path = venv::ensure_venv(&uv_path)?;

    VENV_PATH.set(venv_path.clone()).ok();
    interpreter::setup_environment(&venv_path)?;

    Ok(())
}

pub fn init() -> Result<()> {
    use tracing::debug;

    // Check if already initialized
    if PYTHON_INITIALIZED.get().is_some() {
        debug!("Python already initialized, skipping verification");
        return Ok(());
    }

    debug!("Initializing Python for the first time");

    let venv_path = VENV_PATH.get().ok_or_else(|| {
        crate::R2xError::PythonInit(
            "prepare_environment() must be called before init()".to_string(),
        )
    })?;

    interpreter::verify_python(venv_path)?;

    // Mark as initialized
    PYTHON_INITIALIZED.set(()).ok();
    debug!("Python initialization complete");

    Ok(())
}
