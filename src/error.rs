use thiserror::Error;

#[derive(Error, Debug)]
pub enum R2xError {
    #[error("Python initialization failed: {0}")]
    PythonInit(String),

    #[error("Plugin '{0}' not found")]
    PluginNotFound(String),

    #[error("UV not found and download failed: {0}")]
    UvNotFound(String),

    #[error("UV download failed: {0}")]
    UvDownload(String),

    #[error("Python installation failed: {0}")]
    PythonInstall(String),

    #[error("Virtual environment error: {0}")]
    VenvError(String),

    #[error("Schema parsing error: {0}")]
    SchemaParse(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("No cache directory found")]
    NoCacheDir,

    #[error("Unsupported platform")]
    UnsupportedPlatform,

    #[error("{0}")]
    Other(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Upgrade failed: {0}")]
    UpgradeError(String),

    #[error("Subprocess execution failed: {0}")]
    SubprocessError(String),
}

pub type Result<T> = std::result::Result<T, R2xError>;
