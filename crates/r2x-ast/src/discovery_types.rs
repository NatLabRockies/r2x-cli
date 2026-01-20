//! Local types for AST-based plugin discovery
//!
//! These types are used internally by the r2x-ast crate for discovery
//! and are converted to the new manifest types when stored.

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
    /// Check if this entry point refers to a manifest-based registration
    /// (e.g., register_plugin function or manifest object)
    pub fn is_manifest_based(&self) -> bool {
        let symbol_lower = self.symbol.to_lowercase();
        symbol_lower == "manifest"
            || symbol_lower == "register_plugin"
            || symbol_lower.ends_with("_manifest")
            || symbol_lower.ends_with("_plugin")
    }

    /// Check if the symbol likely refers to a class (starts with uppercase)
    pub fn is_class(&self) -> bool {
        self.symbol
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }

    /// Infer plugin kind from the section name
    pub fn infer_kind(&self) -> PluginKind {
        // Extract the suffix after "r2x." if present
        let section_suffix = self.section.strip_prefix("r2x.").unwrap_or(&self.section);

        match section_suffix {
            "transforms" | "modifiers" => PluginKind::Modifier,
            "parsers" => PluginKind::Parser,
            "exporters" => PluginKind::Exporter,
            "upgraders" => PluginKind::Upgrader,
            "utilities" => PluginKind::Utility,
            "translations" => PluginKind::Translation,
            // For "r2x_plugin" section, we need to infer from the symbol name
            _ if self.section == "r2x_plugin" => self.infer_kind_from_symbol(),
            _ => PluginKind::Utility,
        }
    }

    /// Infer plugin kind from the symbol name when section doesn't specify
    fn infer_kind_from_symbol(&self) -> PluginKind {
        let lower = self.symbol.to_lowercase();
        if lower.contains("parser") {
            PluginKind::Parser
        } else if lower.contains("export") {
            PluginKind::Exporter
        } else if lower.contains("upgrade") {
            PluginKind::Upgrader
        } else if lower.contains("modif") || lower.contains("transform") {
            PluginKind::Modifier
        } else if lower.contains("translat") {
            PluginKind::Translation
        } else {
            // Default to Parser for main r2x_plugin entries (most common case)
            PluginKind::Parser
        }
    }

    /// Get the full qualified entry point (module:symbol)
    pub fn full_entry(&self) -> String {
        format!("{}:{}", self.module, self.symbol)
    }
}

/// Discovered plugin specification from AST analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPlugin {
    pub name: String,
    pub kind: PluginKind,
    pub entry: String,
    pub invocation: InvocationSpec,
    pub io: IOContract,
    pub resources: Option<ResourceSpec>,
    pub upgrade: Option<UpgradeSpec>,
    pub description: Option<String>,
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
    Translation,
}

/// How to construct and invoke a plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationSpec {
    pub implementation: ImplementationType,
    pub method: Option<String>,
    #[serde(default)]
    pub constructor: Vec<ArgumentSpec>,
    #[serde(default)]
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
    pub annotation: Option<String>,
    pub default: Option<String>,
    pub required: bool,
}

/// Input/output contract for a plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IOContract {
    #[serde(default)]
    pub consumes: Vec<IOSlot>,
    #[serde(default)]
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
    pub store: Option<StoreSpec>,
    pub config: Option<ConfigSpec>,
}

/// Data store specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSpec {
    pub mode: StoreMode,
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
    pub fields: Vec<ConfigField>,
}

/// Configuration field specification
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

/// Upgrade specification for upgrader plugins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeSpec {
    pub version_strategy_json: Option<String>,
    pub version_reader_json: Option<String>,
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

// =============================================================================
// CONVERSION TO NEW MANIFEST TYPES
// =============================================================================

impl DiscoveredPlugin {
    /// Convert to the new manifest Plugin type
    pub fn to_manifest_plugin(&self) -> r2x_manifest::Plugin {
        use crate::schema_extractor::parse_union_types_from_annotation;
        use r2x_manifest::{Plugin, PluginType};
        use smallvec::SmallVec;
        use std::sync::Arc;

        let plugin_type = match self.invocation.implementation {
            ImplementationType::Class => PluginType::Class,
            ImplementationType::Function => PluginType::Function,
        };

        let (module, class_name, function_name) = if let Some(idx) = self.entry.rfind('.') {
            let module = Arc::from(&self.entry[..idx]);
            let symbol = &self.entry[idx + 1..];
            match plugin_type {
                PluginType::Class => (module, Some(Arc::from(symbol)), None),
                PluginType::Function => (module, None, Some(Arc::from(symbol))),
            }
        } else {
            (Arc::from(""), None, None)
        };

        // Runtime-injected params that shouldn't appear in user-facing config
        const RUNTIME_PARAMS: &[&str] = &[
            "self", "system", "config", "store", "stdin", "ctx", "context",
        ];

        let config_spec = self.resources.as_ref().and_then(|r| r.config.as_ref());

        let (config_class, config_module) = config_spec
            .map(|c| {
                (
                    Some(Arc::from(c.name.as_str())),
                    Some(Arc::from(c.module.as_str())),
                )
            })
            .unwrap_or((None, None));

        let config_fields = config_spec.map(|c| c.fields.as_slice()).unwrap_or(&[]);

        let mut parameters: SmallVec<[r2x_manifest::Parameter; 4]> = config_fields
            .iter()
            .map(|field| r2x_manifest::Parameter {
                name: Arc::from(field.name.as_str()),
                types: field.types.iter().map(|t| Arc::from(t.as_str())).collect(),
                module: None,
                required: field.required,
                default: field.default.as_ref().map(|d| Arc::from(d.as_str())),
                description: field.description.as_ref().map(|d| Arc::from(d.as_str())),
            })
            .collect();

        let existing_names: std::collections::HashSet<String> =
            parameters.iter().map(|p| p.name.to_string()).collect();

        for arg in self.invocation.call.iter() {
            let name = arg.name.as_str();
            if RUNTIME_PARAMS.contains(&name) || existing_names.contains(name) {
                continue;
            }
            let types: SmallVec<[Arc<str>; 2]> = arg
                .annotation
                .as_deref()
                .map(|ann| {
                    parse_union_types_from_annotation(ann)
                        .into_iter()
                        .map(|t| Arc::from(t.as_str()))
                        .collect()
                })
                .unwrap_or_else(|| SmallVec::from_elem(Arc::from("Any"), 1));

            parameters.push(r2x_manifest::Parameter {
                name: Arc::from(arg.name.as_str()),
                types,
                module: None,
                required: arg.required,
                default: arg.default.as_ref().map(|d| Arc::from(d.as_str())),
                description: None,
            });
        }

        Plugin {
            name: Arc::from(self.name.as_str()),
            plugin_type,
            module,
            class_name,
            function_name,
            config_class,
            config_module,
            hooks: SmallVec::new(),
            parameters,
            config_schema: Default::default(),
            content_hash: 0,
        }
    }
}
