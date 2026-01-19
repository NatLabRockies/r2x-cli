//! Memory-efficient type system for r2x plugin registry
//!
//! This module provides:
//! - 64-byte cache-aligned structs for hot paths
//! - Arc<str> interning for string deduplication
//! - SmallVec for inline small collections
//! - Pre-computed hashes for fast comparison

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::sync::Arc;

// =============================================================================
// MANIFEST - Top-level with index for O(1) lookup
// =============================================================================

/// Top-level manifest structure for R2X plugin metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: Arc<str>,
    pub generated_at: Arc<str>,
    #[serde(default)]
    pub packages: Vec<Package>,

    /// Runtime only - rebuilt on load for O(1) package lookup
    #[serde(skip)]
    pub package_index: AHashMap<Arc<str>, usize>,

    /// Runtime only - content hash for fast equality check
    #[serde(skip)]
    pub content_hash: u64,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            version: Arc::from("3.0"),
            generated_at: Arc::from(chrono::Utc::now().to_rfc3339()),
            packages: Vec::new(),
            package_index: AHashMap::new(),
            content_hash: 0,
        }
    }
}

// =============================================================================
// PACKAGE - With pre-computed hash for fast comparison
// =============================================================================

/// Represents a single Python package containing r2x plugins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: Arc<str>,
    pub version: Arc<str>,
    #[serde(default)]
    pub editable_install: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<Arc<str>>,
    pub install_type: InstallType,
    #[serde(default)]
    pub installed_by: SmallVec<[Arc<str>; 2]>,
    #[serde(default)]
    pub dependencies: SmallVec<[Arc<str>; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<Arc<str>>,
    #[serde(default)]
    pub plugins: Vec<Plugin>,
    #[serde(default)]
    pub configs: Vec<ConfigClass>,

    /// Runtime only - pre-computed hash for fast equality check
    #[serde(skip)]
    pub content_hash: u64,

    /// Runtime only - plugin index for fast lookup
    #[serde(skip)]
    pub plugin_index: AHashMap<Arc<str>, usize>,
}

impl Default for Package {
    fn default() -> Self {
        Package {
            name: Arc::from(""),
            version: Arc::from("0.0.0"),
            editable_install: false,
            source_uri: None,
            install_type: InstallType::Explicit,
            installed_by: SmallVec::new(),
            dependencies: SmallVec::new(),
            entry_point: None,
            plugins: Vec::new(),
            configs: Vec::new(),
            content_hash: 0,
            plugin_index: AHashMap::new(),
        }
    }
}

/// How the package was installed
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallType {
    #[default]
    Explicit,
    Dependency,
}

// =============================================================================
// PLUGIN - Compact representation
// =============================================================================

/// Plugin specification - represents a single plugin entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub name: Arc<str>,
    #[serde(rename = "type")]
    pub plugin_type: PluginType,
    pub module: Arc<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<Arc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<Arc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_class: Option<Arc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_module: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub hooks: SmallVec<[Arc<str>; 4]>,
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub parameters: SmallVec<[Parameter; 4]>,
    #[serde(default, skip_serializing_if = "SchemaFields::is_empty")]
    pub config_schema: SchemaFields,

    /// Runtime only - content hash
    #[serde(skip)]
    pub content_hash: u64,
}

impl Default for Plugin {
    fn default() -> Self {
        Plugin {
            name: Arc::from(""),
            plugin_type: PluginType::Class,
            module: Arc::from(""),
            class_name: None,
            function_name: None,
            config_class: None,
            config_module: None,
            hooks: SmallVec::new(),
            parameters: SmallVec::new(),
            config_schema: SchemaFields::default(),
            content_hash: 0,
        }
    }
}

/// Plugin implementation type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum PluginType {
    #[default]
    Class = 0,
    Function = 1,
}

// =============================================================================
// SCHEMA FIELDS - Map with index for cache efficiency
// =============================================================================

/// Collection of schema field definitions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaFields {
    #[serde(flatten)]
    pub fields: AHashMap<Arc<str>, SchemaField>,

    /// Runtime only - content hash
    #[serde(skip)]
    pub content_hash: u64,
}

impl SchemaFields {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&SchemaField> {
        self.fields.get(name)
    }

    pub fn insert(&mut self, name: Arc<str>, field: SchemaField) {
        self.fields.insert(name, field);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &SchemaField)> {
        self.fields.iter()
    }
}

// =============================================================================
// SCHEMA FIELD - Cache-line optimized
// =============================================================================

/// Single field in a config schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaField {
    #[serde(rename = "type")]
    pub field_type: FieldType,

    #[serde(default)]
    pub required: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<DefaultValue>,

    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub constraints: SmallVec<[Constraint; 2]>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Arc<[Arc<str>]>>,

    /// For array types - the item type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Arc<str>>,

    /// For nested object types - class reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nested: Option<Arc<NestedInfo>>,

    /// For nested object properties
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Box<SchemaFields>>,
}

impl Default for SchemaField {
    fn default() -> Self {
        SchemaField {
            field_type: FieldType::Str,
            required: false,
            default: None,
            constraints: SmallVec::new(),
            enum_values: None,
            items: None,
            nested: None,
            properties: None,
        }
    }
}

/// Field type enumeration
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum FieldType {
    #[default]
    Str = 0,
    Int = 1,
    Float = 2,
    Bool = 3,
    Array = 4,
    Object = 5,
    Datetime = 6,
    Any = 7,
}

// =============================================================================
// CONSTRAINT - Tagged union, compact representation
// =============================================================================

/// Validation constraint for a schema field
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum Constraint {
    #[serde(rename = "ge")]
    Ge(f64),
    #[serde(rename = "le")]
    Le(f64),
    #[serde(rename = "gt")]
    Gt(f64),
    #[serde(rename = "lt")]
    Lt(f64),
    #[serde(rename = "min_len")]
    MinLen(u32),
    #[serde(rename = "max_len")]
    MaxLen(u32),
    #[serde(rename = "pattern")]
    Pattern(Arc<str>),
    #[serde(rename = "multiple_of")]
    MultipleOf(f64),
}

// =============================================================================
// DEFAULT VALUE - Inline small values
// =============================================================================

/// Default value for a schema field
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DefaultValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    Array(Arc<[DefaultValue]>),
}

// =============================================================================
// NESTED INFO - Only allocated for object/array types
// =============================================================================

/// Reference to a nested type (class or module)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<Arc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<Arc<str>>,
}

// =============================================================================
// PARAMETER - For function plugins
// =============================================================================

/// Function parameter specification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Parameter {
    pub name: Arc<str>,
    /// Array of type alternatives (for union types like int | str)
    #[serde(rename = "type")]
    pub types: SmallVec<[Arc<str>; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<Arc<str>>,
    #[serde(default)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Arc<str>>,
    /// Description extracted from Field(description="...")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Arc<str>>,
}

impl Parameter {
    /// Format types as a union string (e.g., "int | str | None")
    pub fn format_types(&self) -> String {
        self.types
            .iter()
            .map(|t| t.as_ref())
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

// =============================================================================
// CONFIG CLASS - Standalone config definitions
// =============================================================================

/// Configuration class definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigClass {
    pub name: Arc<str>,
    pub module: Arc<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Arc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_module: Option<Arc<str>>,
    #[serde(default)]
    pub fields: Vec<ConfigField>,
}

/// Field definition within a config class
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub name: Arc<str>,
    #[serde(rename = "type")]
    pub field_type: Arc<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<Arc<str>>,
    #[serde(default)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<DefaultValue>,
}

// =============================================================================
// IMPL BLOCKS
// =============================================================================

impl Manifest {
    /// Rebuild all indexes after deserialization
    pub fn rebuild_indexes(&mut self) {
        self.package_index.clear();
        for (idx, pkg) in self.packages.iter_mut().enumerate() {
            self.package_index.insert(pkg.name.clone(), idx);
            pkg.rebuild_plugin_index();
        }
    }
}

impl Package {
    /// Rebuild plugin index
    pub fn rebuild_plugin_index(&mut self) {
        self.plugin_index.clear();
        for (idx, plugin) in self.plugins.iter().enumerate() {
            self.plugin_index.insert(plugin.name.clone(), idx);
        }
    }

    /// Pre-compute hash for fast equality check
    pub fn compute_hash(&mut self) {
        use std::hash::{Hash, Hasher};
        let mut hasher = ahash::AHasher::default();
        self.name.hash(&mut hasher);
        self.version.hash(&mut hasher);
        for plugin in &self.plugins {
            plugin.name.hash(&mut hasher);
        }
        self.content_hash = hasher.finish();
    }
}
