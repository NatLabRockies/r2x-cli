use serde::{Deserialize, Serialize};

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
}

/// Plugin kind/type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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
    pub fields: Vec<ConfigField>,
}

/// Configuration field specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
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
