//! Execution types for plugin invocation
//!
//! These types are used by the runtime execution layer (r2x-python) to invoke plugins.
//! They are separate from the storage types in types.rs which are optimized for
//! manifest storage and synchronization.

use serde::{Deserialize, Serialize};

/// Full plugin specification for runtime execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSpec {
    pub name: String,
    pub kind: PluginKind,
    pub entry: String,
    pub invocation: InvocationSpec,
    pub io: IOContract,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade: Option<UpgradeSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Plugin kind/type enumeration for execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginKind {
    Parser,
    Exporter,
    Modifier,
    Upgrader,
    Utility,
    Translation,
}

/// How to construct and invoke a plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationSpec {
    pub implementation: ImplementationType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constructor: Vec<ArgumentSpec>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub call: Vec<ArgumentSpec>,
}

/// Implementation type for plugins
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImplementationType {
    Class,
    Function,
}

/// Argument specification for constructor or call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub required: bool,
}

/// Input/output contract for a plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IOContract {
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub consumes: Vec<IOSlot>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub produces: Vec<IOSlot>,
}

/// I/O slot type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IOSlot {
    System,
    ConfigFile,
    StoreFolder,
    File,
    Folder,
    Data,
}

/// Resource requirements (config and data store)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<StoreSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ConfigSpec>,
}

/// Data store specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSpec {
    pub mode: StoreMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Data store mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StoreMode {
    Folder,
    Manifest,
    Inline,
}

/// Configuration specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSpec {
    pub module: String,
    pub name: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ExecConfigField>,
}

/// Configuration field specification for execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecConfigField {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub required: bool,
}

/// Upgrade specification for upgrader plugins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_strategy_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_reader_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_steps_json: Option<String>,
}

/// Metadata about the manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub version: String,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uv_lock_path: Option<String>,
}
