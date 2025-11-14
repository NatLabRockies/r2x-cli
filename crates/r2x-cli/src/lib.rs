//! R2X library - expose modules for testing
//!
//! This library exposes core modules needed for testing and integration.

pub mod commands;
pub mod common;
pub mod errors;
pub mod help;
pub mod package_verification;
pub mod pipeline_config;
pub mod plugin_manifest;
pub mod plugins;

// Re-export dedicated crates so internal modules can continue using the previous paths.
pub use r2x_ast;
pub use r2x_config as config_manager;
pub use r2x_logger as logger;
pub use r2x_manifest;
pub use r2x_python as python_bridge;

// Re-export common types for convenience
pub use common::GlobalOpts;
pub use errors::PipelineError;
pub use python_bridge::errors::BridgeError;
pub use r2x_manifest::errors::ManifestError;

// Re-export manifest types from new module for convenience
pub use r2x_manifest::{
    DecoratorRegistration, FunctionParameter, FunctionSignature,
    Manifest, Metadata, Package, VarArgType,
};
