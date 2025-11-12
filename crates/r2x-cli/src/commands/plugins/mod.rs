use crate::config_manager::Config;
use crate::logger;

pub mod clean;
pub mod install;
pub mod list;
pub mod remove;
pub mod sync;

pub use clean::clean_manifest;
pub use install::{install_plugin, show_install_help, GitOptions};
pub use list::list_plugins;
pub use remove::remove_plugin;
pub use sync::sync_manifest;

pub(super) fn setup_config() -> Result<(String, String, String), String> {
    let mut config = Config::load().map_err(|e| {
        logger::error(&format!("Failed to load config: {}", e));
        format!("Failed to load config: {}", e)
    })?;

    config.ensure_uv_path().map_err(|e| {
        logger::error(&format!("Failed to setup uv: {}", e));
        format!("Failed to setup uv: {}", e)
    })?;
    config.ensure_cache_path().map_err(|e| {
        logger::error(&format!("Failed to setup cache: {}", e));
        format!("Failed to setup cache: {}", e)
    })?;
    config.ensure_venv_path().map_err(|e| {
        logger::error(&format!("Failed to setup venv: {}", e));
        format!("Failed to setup venv: {}", e)
    })?;

    let uv_path = config
        .uv_path
        .as_ref()
        .cloned()
        .ok_or_else(|| "uv path not configured".to_string())?;
    let venv_path = config.get_venv_path();
    let python_path = config.get_venv_python_path();

    Ok((uv_path, venv_path, python_path))
}
