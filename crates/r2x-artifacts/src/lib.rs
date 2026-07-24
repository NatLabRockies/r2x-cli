//! Artifact storage and transport for r2x pipeline systems.

use r2x_python::errors::BridgeError;
use thiserror::Error;

pub mod artifact_handoff;
pub mod pipeline_artifact;

/// Errors raised while materializing or transferring an artifact.
#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("artifact configuration error: {0}")]
    Config(String),

    #[error("artifact I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Python bridge error: {0}")]
    Bridge(#[from] BridgeError),

    #[error("artifact serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
