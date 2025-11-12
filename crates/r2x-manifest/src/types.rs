use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Callable object metadata (parsed from obj JSON)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CallableMetadata {
    /// Python module path (e.g., "r2x_reeds.parser")
    pub module: String,

    /// Callable name (e.g., "ReEDSParser")
    pub name: String,

    /// Callable type: "class" or "function"
    #[serde(rename = "type")]
    pub callable_type: String,

    /// Return annotation (e.g., "None", "System")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_annotation: Option<String>,

    /// Parameters as a map of parameter name to metadata
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub parameters: HashMap<String, ParameterMetadata>,
}

/// Configuration schema metadata (parsed from config JSON)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigMetadata {
    /// Config module path (e.g., "r2x_reeds.config")
    pub module: String,

    /// Config class name (e.g., "ReEDSConfig")
    pub name: String,

    /// Return annotation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_annotation: Option<String>,

    /// Config parameters as a map of parameter name to metadata
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub parameters: HashMap<String, ParameterMetadata>,
}

/// Parameter metadata for a callable or config
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParameterMetadata {
    /// Type annotation (e.g., "str | None", "int", "System")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,

    /// Default value as JSON string (e.g., "null", "true", "5", "\"base\"")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// Whether this parameter is required
    pub is_required: bool,
}

/// Upgrader-specific metadata
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UpgraderMetadata {
    /// Version strategy as JSON (kept as JSON for now due to complexity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_strategy_json: Option<String>,

    /// Version reader as JSON (kept as JSON for now due to complexity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_reader_json: Option<String>,

    /// Upgrade steps as JSON (kept as JSON for now due to complexity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_steps_json: Option<String>,
}

/// Top-level manifest structure for plugin metadata from discovery
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
                version: "1.0".to_string(),
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
    /// Distribution name (e.g., "r2x-reeds")
    pub name: String,
    /// Path to entry_points.txt in dist-info
    pub entry_points_dist_info: String,
    /// Whether this is an editable install (e.g., from `uv pip install -e`)
    #[serde(default)]
    pub editable_install: bool,
    /// Path to .pth file (only set if editable_install=true)
    pub pth_file: Option<String>,
    /// Resolved source path for editable installs
    pub resolved_source_path: Option<String>,
    /// Installation type: "explicit" (user-installed) or "dependency" (auto-installed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_type: Option<String>,
    /// If install_type="dependency", which package(s) required this
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub installed_by: Vec<String>,
    /// R2X packages that this package depends on
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    /// Plugin definitions extracted from register_plugin()
    #[serde(default)]
    pub plugins: Vec<DiscoveryPlugin>,
    /// Decorator-registered functions found in package
    #[serde(default)]
    pub decorator_registrations: Vec<DecoratorRegistration>,
}

/// Runtime-ready plugin metadata derived from discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obj: Option<CallableMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ConfigMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrader: Option<UpgraderMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_type: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub installed_by: Vec<String>,
}

/// Plugin definition extracted from Package.plugins in register_plugin()
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPlugin {
    /// Plugin name as registered (e.g., "reeds-parser")
    pub name: String,
    /// Plugin type/class name (e.g., "ParserPlugin", "UpgraderPlugin")
    pub plugin_type: String,
    /// All constructor arguments from plugin instantiation
    #[serde(default)]
    pub constructor_args: Vec<ConstructorArg>,
    /// Resolved class/function references with full metadata
    #[serde(default)]
    pub resolved_references: Vec<ResolvedReference>,
    /// Decorators associated with this plugin
    #[serde(default)]
    pub decorators: Vec<DecoratorRegistration>,
}

/// Single constructor argument for plugin instantiation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstructorArg {
    /// Parameter name
    pub name: String,
    /// Raw value as string (preserved from source)
    pub value: String,
    /// Type category (string, class_reference, function_reference, enum_value, etc.)
    pub arg_type: String,
}

/// Resolved reference to a class or function with full metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedReference {
    /// Key identifying which constructor_arg this resolves
    pub key: String,
    /// Type of reference: "class" or "function"
    pub ref_type: String,
    /// Python module path (e.g., "r2x_reeds.parser")
    pub module: String,
    /// Name of the class or function
    pub name: String,
    /// Source file path (relative to package root)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// Parameters for this callable
    #[serde(default)]
    pub parameters: Vec<ParameterEntry>,
    /// Return type annotation (for functions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_annotation: Option<String>,
}

/// Parameter entry for resolved references (array-based for TOML compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterEntry {
    /// Parameter name
    pub name: String,
    /// Type annotation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    /// Default value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Whether this parameter is required
    pub is_required: bool,
}

/// Function registration via decorator (e.g., @ReEDSUpgrader.register_step(...))
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoratorRegistration {
    /// Class that provides the decorator (e.g., "ReEDSUpgrader")
    pub decorator_class: String,
    /// Decorator method name (e.g., "register_step")
    pub decorator_method: String,
    /// Function name being decorated
    pub function_name: String,
    /// Module containing the function
    pub function_module: String,
    /// Source file path (relative to package root)
    pub source_file: Option<String>,
    /// Line number in source file
    pub line_number: Option<usize>,
    /// Arguments passed to the decorator
    #[serde(default)]
    pub decorator_args: toml::Table,
    /// Function signature (parameters and return type)
    pub function_signature: Option<FunctionSignature>,
}

/// Complete function signature extracted from source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSignature {
    /// Return type as string (e.g., "None", "dict[str, Any]")
    pub return_type: String,
    /// Function parameters in order
    #[serde(default)]
    pub parameters: Vec<FunctionParameter>,
}

/// Single function parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionParameter {
    /// Parameter name
    pub name: String,
    /// Type annotation as string (preserved as-is from source)
    pub param_type: String,
    /// Default value if any
    pub default: Option<String>,
    /// Whether this is a keyword-only argument
    #[serde(default)]
    pub is_keyword_only: bool,
    /// Whether this is *args or **kwargs
    pub is_var_arg: Option<VarArgType>,
}

/// Type of variable argument (*args or **kwargs)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarArgType {
    Args,
    Kwargs,
}

// Manifest implementation moved to manifest.rs

// Tests moved to manifest.rs
