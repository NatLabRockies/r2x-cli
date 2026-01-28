//! DEPRECATED: This module is for backward compatibility only.
//! All types and functionality have been moved to the r2x_manifest module.
//! This file provides limited type aliases pointing to the new implementation.

use r2x_manifest::{errors as manifest_errors, runtime, types};

pub type PluginManifest = types::Manifest;
pub type Manifest = types::Manifest;
pub type Package = types::Package;
pub type Plugin = types::Plugin;
pub type PluginType = types::PluginType;
pub type PluginRole = runtime::PluginRole;
pub type RuntimeBindings = runtime::RuntimeBindings;
pub type RuntimeConfig = runtime::RuntimeConfig;
pub type ManifestError = manifest_errors::ManifestError;
