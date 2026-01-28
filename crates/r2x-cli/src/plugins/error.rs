use thiserror::Error;

use crate::r2x_manifest::errors::ManifestError;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Manifest error: {0}")]
    Manifest(#[from] ManifestError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Package spec error: {0}")]
    PackageSpec(String),

    #[error("Discovery error: {0}")]
    Discovery(String),

    #[error("Package locator error: {0}")]
    Locator(String),

    #[error("Python error: {0}")]
    Python(String),

    #[error("Command failed: {command} (exit {status:?})")]
    CommandFailed {
        command: String,
        status: Option<i32>,
    },

    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),
}
