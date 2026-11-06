use crate::plugins::error::PluginError;
use r2x_config::Config;
use r2x_manifest::package_discovery::PackageLocator;
use r2x_manifest::types::Manifest;
use r2x_python::utils::resolve_site_package_path;
use std::path::PathBuf;

pub struct PluginContext {
    pub(crate) config: Config,
    pub(crate) manifest: Manifest,
    pub(crate) uv_path: String,
    pub(crate) venv_path: String,
    pub(crate) python_path: String,
    pub(crate) locator: PackageLocator,
}

#[derive(Clone, Copy)]
enum SetupMode {
    Ensure,
    Existing,
}

impl PluginContext {
    pub fn load() -> Result<Self, PluginError> {
        Self::load_with_mode(SetupMode::Ensure)
    }

    pub fn load_existing() -> Result<Self, PluginError> {
        Self::load_with_mode(SetupMode::Existing)
    }

    fn load_with_mode(mode: SetupMode) -> Result<Self, PluginError> {
        let mut config = Config::load()
            .map_err(|e| PluginError::Config(format!("Failed to load config: {e}")))?;

        let (uv_path, venv_path) = match mode {
            SetupMode::Ensure => {
                let uv = config
                    .ensure_uv_path()
                    .map_err(|e| PluginError::Config(format!("Failed to setup uv: {e}")))?;
                config
                    .ensure_cache_path()
                    .map_err(|e| PluginError::Config(format!("Failed to setup cache: {e}")))?;
                let venv = config
                    .ensure_venv_path()
                    .map_err(|e| PluginError::Config(format!("Failed to setup venv: {e}")))?;
                (uv, venv)
            }
            SetupMode::Existing => {
                let uv = config
                    .existing_uv_path()
                    .map_err(|e| PluginError::EnvironmentUnavailable(e.to_string()))?;
                let venv = config
                    .existing_venv_path()
                    .map_err(|e| PluginError::EnvironmentUnavailable(e.to_string()))?;
                (uv, venv)
            }
        };
        let python_path = config.get_venv_python_path();

        let site_packages = resolve_site_package_path(&PathBuf::from(&venv_path))
            .map_err(|e| PluginError::Python(format!("Failed to resolve site-packages: {e}")))?;

        let locator = PackageLocator::new(site_packages).map_err(|e| {
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

    pub(crate) fn refresh_locator(&mut self) -> Result<(), PluginError> {
        self.locator
            .refresh()
            .map_err(|e| PluginError::Locator(format!("Failed to refresh package locator: {e}")))
    }
}
