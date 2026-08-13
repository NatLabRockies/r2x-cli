use crate::uv::UvCommandError;
use r2x_logger as logger;
use thiserror::Error;

use r2x_manifest::errors::ManifestError;

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

    #[error(
        "uv command failed during {phase} for '{target}' after {elapsed_ms}ms (exit {status:?}): {reason}. Command: {command}. See log: {log_path}"
    )]
    UvCommandFailed {
        phase: String,
        target: String,
        command: String,
        status: Option<i32>,
        elapsed_ms: u128,
        reason: String,
        log_path: String,
    },

    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),
}

impl From<UvCommandError> for PluginError {
    fn from(error: UvCommandError) -> Self {
        Self::UvCommandFailed {
            phase: error.phase,
            target: error.target,
            command: error.command,
            status: error.status,
            elapsed_ms: error.elapsed.as_millis(),
            reason: error.reason,
            log_path: if error.log_path.is_empty() {
                logger::get_log_path_string()
            } else {
                error.log_path
            },
        }
    }
}
