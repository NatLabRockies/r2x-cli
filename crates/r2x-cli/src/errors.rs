//! Centralized error types for the r2x project
//!
//! This module defines all error types used across the project,
//! providing a unified error handling interface.

use std::io;
use thiserror::Error;

/// Errors that can occur during pipeline configuration operations
#[derive(Error, Debug)]
pub enum PipelineError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Failed to parse pipeline YAML: {0}")]
    Parse(#[from] serde_yaml::Error),

    #[error("Variable '{0}' not found in variables section")]
    VariableNotFound(String),

    #[error("Pipeline '{0}' not found in YAML")]
    PipelineNotFound(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

#[cfg(test)]
mod tests {
    use crate::errors::*;

    #[test]
    fn test_pipeline_error_display() {
        let err = PipelineError::PipelineNotFound("test-pipeline".to_string());
        assert_eq!(
            err.to_string(),
            "Pipeline 'test-pipeline' not found in YAML"
        );
    }
}
