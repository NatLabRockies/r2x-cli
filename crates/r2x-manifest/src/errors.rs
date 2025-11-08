use std::io;
use thiserror::Error;

/// Errors that can occur during plugin manifest operations
#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Failed to parse manifest: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Failed to serialize manifest: {0}")]
    Serialize(#[from] toml::ser::Error),

    #[error("Invalid plugin: {0}")]
    InvalidPlugin(String),
}
