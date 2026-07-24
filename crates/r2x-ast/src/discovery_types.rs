//! Local types for AST-based plugin discovery
//!
//! These types are used during AST parsing and discovery. They are distinct
//! from the manifest types and keep AST-specific details out of the manifest.

use r2x_manifest::types::SchemaFields;
use serde::{Deserialize, Serialize};

/// Entry point information parsed from entry_points.txt
///
/// Represents a single entry point from any r2x-related section
/// in the package's entry_points.txt file.
#[derive(Debug, Clone)]
pub struct EntryPointInfo {
    /// Entry point name (e.g., "reeds", "add-pcm-defaults")
    pub(crate) name: String,
    /// Module path (e.g., "r2x_reeds", "r2x_reeds.sysmod.pcm_defaults")
    pub(crate) module: String,
    /// Symbol name (e.g., "ReEDSParser", "add_pcm_defaults")
    pub(crate) symbol: String,
    /// Section name (e.g., "r2x_plugin", "r2x.transforms")
    pub(crate) section: String,
}

impl EntryPointInfo {
    /// Check if the symbol likely refers to a class (starts with uppercase)
    pub(crate) fn is_class(&self) -> bool {
        self.symbol.chars().next().is_some_and(|c| c.is_uppercase())
    }
}

/// Configuration specification (AST-specific version with ConfigField)
///
/// This version supports union types via `types: Vec<String>` which is
/// needed during AST discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSpec {
    pub(crate) module: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) fields: Vec<ConfigField>,
    /// Schema with nested type information for the config class.
    /// This includes recursively extracted properties for nested object types.
    #[serde(default, skip_serializing_if = "SchemaFields::is_empty")]
    pub(crate) config_schema: SchemaFields,
}

/// Configuration field specification (AST-specific)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub(crate) name: String,
    /// Array of type alternatives (for union types like int | str)
    pub(crate) types: Vec<String>,
    pub(crate) default: Option<String>,
    pub(crate) required: bool,
    /// Description extracted from Field(description="...")
    pub(crate) description: Option<String>,
}
