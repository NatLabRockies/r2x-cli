//! R2X Manifest Management
//!
//! This module handles all manifest types and operations for the r2x plugin system.
//! It provides the core types for managing plugin metadata from discovery and AST analysis.
//!
//! The manifest is stored in TOML format and contains comprehensive metadata about
//! installed plugins, their configurations, and config schemas.
//!
//! # Version 3.0 Format
//!
//! The new format uses memory-efficient types:
//! - `Arc<str>` for string interning
//! - `SmallVec` for inline small collections
//! - Pre-computed hashes for O(1) comparisons
//! - Indexed lookups for O(1) package/plugin access

pub mod errors;
pub mod execution_types;
pub mod manifest;
pub mod package_discovery;
pub mod runtime;
pub mod sync;
pub mod types;

// Re-export core types
pub use manifest::RemovedPackage;
pub use types::{
    ConfigClass, ConfigField, Constraint, DefaultValue, FieldType, InstallType, Manifest,
    NestedInfo, Package, Parameter, Plugin, PluginType, SchemaField, SchemaFields,
};

// Re-export package discovery helpers
pub use package_discovery::{DiscoveredPackage, PackageDiscoverer, PackageLocator};

// Re-export sync utilities
pub use sync::{Change, StringInterner, SyncEngine, SyncResult};

// Re-export error types
pub use errors::ManifestError;

// Re-export execution types (used by r2x-python)
pub use execution_types::{
    ArgumentSpec, ConfigSpec, ExecConfigField, IOContract, IOSlot, ImplementationType,
    InvocationSpec, Metadata, PluginKind, PluginSpec, ResourceSpec, StoreMode, StoreSpec,
    UpgradeSpec,
};

// Re-export runtime utilities
pub use runtime::{build_runtime_bindings, build_runtime_bindings_from_plugin, RuntimeBindings};
