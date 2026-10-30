use crate::uv::UvCommandError;
use r2x_logger as logger;
use thiserror::Error;

use r2x_manifest::errors::ManifestError;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Environment unavailable: {0}")]
    EnvironmentUnavailable(String),

    #[error("Plugin package '{0}' not found")]
    PackageNotFound(String),

    #[error("No plugins found matching the filter in package '{package}'")]
    PluginNotFound {
        package: String,
        candidates: Vec<String>,
    },

    #[error("Manifest error: {0}")]
    Manifest(#[from] ManifestError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

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

impl PluginError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EnvironmentUnavailable(_) => "ENVIRONMENT_UNAVAILABLE",
            Self::PackageNotFound(_) => "PACKAGE_NOT_FOUND",
            Self::PluginNotFound { .. } => "PLUGIN_NOT_FOUND",
            Self::Config(_) => "CONFIGURATION_ERROR",
            Self::Manifest(_) => "MANIFEST_ERROR",
            Self::InvalidArgs(_) | Self::PackageSpec(_) => "INVALID_ARGUMENT",
            Self::Io(_) => "IO_ERROR",
            Self::Json(_) => "JSON_ERROR",
            Self::Discovery(_) | Self::Locator(_) => "DISCOVERY_ERROR",
            Self::Python(_) => "PYTHON_ERROR",
            Self::CommandFailed { .. } | Self::UvCommandFailed { .. } => "COMMAND_FAILED",
        }
    }

    pub fn candidates(&self) -> &[String] {
        match self {
            Self::PluginNotFound { candidates, .. } => candidates,
            _ => &[],
        }
    }
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
