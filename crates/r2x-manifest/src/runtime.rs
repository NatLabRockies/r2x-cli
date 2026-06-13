//! Runtime bindings for plugin execution
//!
//! This module provides utilities for building runtime bindings that are used
//! when invoking plugins through the Python bridge.

use crate::types::{Parameter, Plugin, PluginType};

/// Coarse-grained plugin role inferred from the manifest name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginRole {
    Parser,
    Exporter,
    Modifier,
    Upgrader,
    Translation,
    Utility,
}

/// Minimal config metadata needed to instantiate the config class.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub module: String,
    pub name: String,
}

/// Runtime bindings for plugin invocation.
#[derive(Debug, Clone)]
pub struct RuntimeBindings {
    pub entry_module: String,
    pub entry_name: String,
    pub plugin_type: PluginType,
    pub role: PluginRole,
    pub call_method: Option<String>,
    pub config: Option<RuntimeConfig>,
    pub parameters: Vec<Parameter>,
    pub requires_store: bool,
}

/// Build runtime bindings from a manifest plugin.
pub fn build_runtime_bindings(plugin: &Plugin) -> RuntimeBindings {
    let (entry_module, entry_name) = parse_entry_point(plugin);
    let role = infer_plugin_role(&plugin.name);
    let call_method = default_method_for_role(role, plugin.plugin_type);
    let config = match (&plugin.config_class, &plugin.config_module) {
        (Some(class_name), Some(module)) => Some(RuntimeConfig {
            name: class_name.to_string(),
            module: module.to_string(),
        }),
        _ => None,
    };

    let requires_store = plugin.parameters.iter().any(|param| {
        matches!(
            param.name.as_ref(),
            "store" | "data_store" | "store_path" | "path"
        ) || param
            .types
            .iter()
            .any(|ty| ty.as_ref().contains("DataStore"))
    });

    RuntimeBindings {
        entry_module,
        entry_name,
        plugin_type: plugin.plugin_type,
        role,
        call_method,
        config,
        parameters: plugin.parameters.to_vec(),
        requires_store,
    }
}

fn parse_entry_point(plugin: &Plugin) -> (String, String) {
    let entry_module = plugin.module.to_string();
    let entry_name = if let Some(ref class_name) = plugin.class_name {
        class_name.to_string()
    } else if let Some(ref function_name) = plugin.function_name {
        function_name.to_string()
    } else {
        plugin.name.to_string()
    };

    (entry_module, entry_name)
}

/// Infer plugin role from name patterns.
pub fn infer_plugin_role(name: &str) -> PluginRole {
    let name_lower = name.to_lowercase();
    if name_lower.contains("parser") {
        PluginRole::Parser
    } else if name_lower.contains("exporter") || name_lower.contains("export") {
        PluginRole::Exporter
    } else if name_lower.contains("upgrade") {
        PluginRole::Upgrader
    } else if name_lower.contains("modifier") || name_lower.contains("transform") {
        PluginRole::Modifier
    } else if name_lower.contains("translation") || name_lower.contains("translate") {
        PluginRole::Translation
    } else {
        PluginRole::Utility
    }
}

fn default_method_for_role(role: PluginRole, plugin_type: PluginType) -> Option<String> {
    if matches!(plugin_type, PluginType::Function) {
        return None;
    }

    match role {
        PluginRole::Parser => Some("build_system".to_string()),
        PluginRole::Exporter => Some("export".to_string()),
        PluginRole::Translation | PluginRole::Modifier | PluginRole::Upgrader => {
            Some("run".to_string())
        }
        PluginRole::Utility => None,
    }
}
