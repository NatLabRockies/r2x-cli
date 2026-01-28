//! Local types for AST-based plugin discovery
//!
//! These types are used during AST parsing and discovery. They are distinct
//! from the manifest types and keep AST-specific details out of the manifest.

use serde::{Deserialize, Serialize};

/// Entry point information parsed from entry_points.txt
///
/// Represents a single entry point from any r2x-related section
/// in the package's entry_points.txt file.
#[derive(Debug, Clone)]
pub struct EntryPointInfo {
    /// Entry point name (e.g., "reeds", "add-pcm-defaults")
    pub name: String,
    /// Module path (e.g., "r2x_reeds", "r2x_reeds.sysmod.pcm_defaults")
    pub module: String,
    /// Symbol name (e.g., "ReEDSParser", "add_pcm_defaults")
    pub symbol: String,
    /// Section name (e.g., "r2x_plugin", "r2x.transforms")
    pub section: String,
}

impl EntryPointInfo {
    /// Check if the symbol likely refers to a class (starts with uppercase)
    pub fn is_class(&self) -> bool {
        self.symbol.chars().next().is_some_and(|c| c.is_uppercase())
    }

    /// Get the full qualified entry point (module:symbol)
    pub fn full_entry(&self) -> String {
        format!("{}:{}", self.module, self.symbol)
    }
}

/// Configuration specification (AST-specific version with ConfigField)
///
/// This version supports union types via `types: Vec<String>` which is
/// needed during AST discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSpec {
    pub module: String,
    pub name: String,
    #[serde(default)]
    pub fields: Vec<ConfigField>,
}

/// Configuration field specification (AST-specific)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub name: String,
    /// Array of type alternatives (for union types like int | str)
    pub types: Vec<String>,
    pub default: Option<String>,
    pub required: bool,
    /// Description extracted from Field(description="...")
    pub description: Option<String>,
}
