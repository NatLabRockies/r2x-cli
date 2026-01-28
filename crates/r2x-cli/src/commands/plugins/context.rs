use crate::config_manager::Config;
use crate::plugins::PluginError;
use r2x_manifest::{Manifest, PackageLocator};
use r2x_python::resolve_site_package_path;
use std::path::PathBuf;

pub struct PluginContext {
    pub config: Config,
    pub manifest: Manifest,
    pub uv_path: String,
    pub venv_path: String,
    pub python_path: String,
    pub locator: PackageLocator,
}

impl PluginContext {
    pub fn load() -> Result<Self, PluginError> {
        let mut config = Config::load()
            .map_err(|e| PluginError::Config(format!("Failed to load config: {e}")))?;

        config
            .ensure_uv_path()
            .map_err(|e| PluginError::Config(format!("Failed to setup uv: {e}")))?;
        config
            .ensure_cache_path()
            .map_err(|e| PluginError::Config(format!("Failed to setup cache: {e}")))?;
        config
            .ensure_venv_path()
            .map_err(|e| PluginError::Config(format!("Failed to setup venv: {e}")))?;

        let uv_path = config
            .uv_path
            .as_ref()
            .cloned()
            .ok_or_else(|| PluginError::Config("uv path not configured".to_string()))?;

        let venv_path = config.get_venv_path();
        let python_path = config.get_venv_python_path();

        let site_packages = resolve_site_package_path(&PathBuf::from(&venv_path))
            .map_err(|e| PluginError::Python(format!("Failed to resolve site-packages: {e}")))?;

        let uv_cache_dir = resolve_uv_cache_dir();
        let locator = PackageLocator::new(site_packages, uv_cache_dir).map_err(|e| {
            PluginError::Locator(format!("Failed to initialize package locator: {e}"))
        })?;

        let manifest = Manifest::load()?;

        Ok(PluginContext {
            config,
            manifest,
            uv_path,
            venv_path,
            python_path,
            locator,
        })
    }
}

fn resolve_uv_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("UV_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|cache| cache.join("uv")));
    base.map(|root| root.join("archive-v0"))
        .filter(|path| path.exists())
}
