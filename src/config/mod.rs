// Configuration management module
// TODO: Implement config file loading (TOML, YAML, JSON)

pub mod workflow;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub python: PythonConfig,
    pub plugins: PluginConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonConfig {
    pub version: String,
    pub uv_version: Option<String>,
    pub venv_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub auto_update: bool,
    pub cache_ttl_hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            python: PythonConfig {
                version: "3.11".to_string(),
                uv_version: None,
                venv_path: None,
            },
            plugins: PluginConfig {
                auto_update: false,
                cache_ttl_hours: 24,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "compact".to_string(),
            },
        }
    }
}
