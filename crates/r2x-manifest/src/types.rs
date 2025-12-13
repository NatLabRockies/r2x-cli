use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Top-level manifest structure for R2X plugin metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub metadata: Metadata,
    #[serde(default)]
    pub packages: Vec<Package>,
}

/// Manifest metadata - version and generation info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    /// Schema version for future compatibility
    pub version: String,
    /// ISO 8601 timestamp when manifest was generated
    pub generated_at: String,
    /// Optional path to UV's lock file for version/dependency info
    pub uv_lock_path: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            metadata: Metadata {
                version: "2.0".to_string(),
                generated_at: chrono::Utc::now().to_rfc3339(),
                uv_lock_path: None,
            },
            packages: Vec::new(),
        }
    }
}

/// Represents a single Python package containing r2x plugins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub entry_points_dist_info: String,
    #[serde(default)]
    pub editable_install: bool,
    pub pth_file: Option<String>,
    pub resolved_source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_type: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub installed_by: Vec<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<PluginSpec>,
    #[serde(default)]
    pub decorator_registrations: Vec<DecoratorRegistration>,
}

/// Complete plugin specification matching R2X-core's PluginSpec
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
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, JsonValue>,
}

/// Plugin kind/type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Parser,
    Exporter,
    Modifier,
    Upgrader,
    Utility,
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
#[serde(rename_all = "lowercase")]
pub enum ImplementationType {
    Class,
    Function,
}

/// Source for an argument value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArgumentSource {
    System,
    Store,
    #[serde(rename = "store_manifest")]
    StoreManifest,
    #[serde(rename = "store_inline")]
    StoreInline,
    Config,
    #[serde(rename = "config_path")]
    ConfigPath,
    Path,
    Stdin,
    Context,
    Literal,
    Custom,
}

/// Argument specification for constructor or call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentSpec {
    pub name: String,
    pub source: ArgumentSource,
    #[serde(default)]
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// I/O slot type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IOSlot {
    pub kind: IOSlotKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// I/O slot kinds
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IOSlotKind {
    System,
    #[serde(rename = "store_folder")]
    StoreFolder,
    #[serde(rename = "store_manifest")]
    StoreManifest,
    #[serde(rename = "store_inline")]
    StoreInline,
    #[serde(rename = "config_file")]
    ConfigFile,
    #[serde(rename = "config_inline")]
    ConfigInline,
    File,
    Folder,
    Stdin,
    Stdout,
    Artifact,
    Void,
}

/// Resource requirements (config and data store)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<StoreSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ConfigSpec>,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, JsonValue>,
}

/// Data store specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSpec {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<StoreMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_mapping_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Configuration field specification (legacy)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub required: bool,
}

/// Function registration via decorator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoratorRegistration {
    pub decorator_class: String,
    pub decorator_method: String,
    pub function_name: String,
    pub function_module: String,
    pub source_file: Option<String>,
    pub line_number: Option<usize>,
    #[serde(default)]
    pub decorator_args: toml::Table,
    pub function_signature: Option<FunctionSignature>,
}

/// Complete function signature extracted from source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSignature {
    pub return_type: String,
    #[serde(default)]
    pub parameters: Vec<FunctionParameter>,
}

/// Single function parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionParameter {
    pub name: String,
    pub param_type: String,
    pub default: Option<String>,
    #[serde(default)]
    pub is_keyword_only: bool,
    pub is_var_arg: Option<VarArgType>,
}

/// Type of variable argument (*args or **kwargs)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarArgType {
    Args,
    Kwargs,
}

/// Upgrade step specification aligned with r2x-core
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeStepSpec {
    pub name: String,
    pub entry: String,
    pub upgrade_type: UpgradeType,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub consumes: Vec<IOSlot>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub produces: Vec<IOSlot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, JsonValue>,
}

/// Upgrade type (FILE or SYSTEM)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum UpgradeType {
    File,
    System,
}

/// Upgrade specification for upgrader plugins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeSpec {
    pub strategy: String,
    pub reader: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<UpgradeStepSpec>,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, JsonValue>,
}
