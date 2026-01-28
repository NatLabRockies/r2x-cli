//! DEPRECATED: This module is for backward compatibility only.
//! All types and functionality have been moved to the r2x_manifest module.
//! This file provides limited type aliases pointing to the new implementation.

use r2x_manifest::{errors as manifest_errors, execution_types, runtime, types};

pub type PluginManifest = types::Manifest;
pub type Manifest = types::Manifest;
pub type Package = types::Package;
pub type Plugin = types::Plugin;
pub type PluginType = types::PluginType;
pub type PluginSpec = execution_types::PluginSpec;
pub type PluginKind = execution_types::PluginKind;
pub type ImplementationType = execution_types::ImplementationType;
pub type ConfigSpec = execution_types::ConfigSpec;
pub type ExecConfigField = execution_types::ExecConfigField;
pub type RuntimeBindings = runtime::RuntimeBindings;
pub type ManifestError = manifest_errors::ManifestError;
