//! R2X Manifest Management
//!
//! This module handles all manifest types and operations for the r2x plugin system.
//! It provides the core types for managing plugin metadata from discovery and AST analysis.
//!
//! The manifest is stored in TOML format and contains comprehensive metadata about
//! installed plugins, their configurations, and decorator registrations.

pub mod errors;
pub mod manifest;
pub mod manifest_writer;
pub mod package_discovery;
pub mod runtime;
pub mod types;

pub use runtime::{build_runtime_bindings, RuntimeBindings};
pub use types::{
    ArgumentSource, ArgumentSpec, ConfigField, ConfigSpec, DecoratorRegistration, FunctionParameter,
    FunctionSignature, IOContract, IOSlot, IOSlotKind, ImplementationType, InvocationSpec,
    Manifest, Metadata, Package, PluginKind, PluginSpec, ResourceSpec, StoreMode, StoreSpec,
    UpgradeSpec, UpgradeStepSpec, UpgradeType, VarArgType,
};

pub use errors::ManifestError;

// Re-export manifest writer utilities for custom paths (testing)
pub use manifest_writer::{read_from_path, write_to_path};

// Re-export package discovery for convenience
pub use package_discovery::{parse_entry_points, DiscoveredPackage, PackageDiscoverer};
