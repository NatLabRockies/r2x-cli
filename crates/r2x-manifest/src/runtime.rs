//! Runtime bindings for plugin execution
//!
//! This module provides utilities for building runtime bindings that are used
//! when invoking plugins through the Python bridge.

use crate::execution_types::{
    ArgumentSpec, ConfigSpec, ExecConfigField, ImplementationType, PluginKind, PluginSpec,
};
use crate::types::{Plugin, PluginType};

/// Runtime bindings for plugin invocation
#[derive(Debug, Clone)]
pub struct RuntimeBindings {
    pub entry_module: String,
    pub entry_name: String,
    pub implementation_type: ImplementationType,
    pub plugin_kind: PluginKind,
    pub config: Option<ConfigSpec>,
    pub call_method: Option<String>,
    pub requires_store: bool,
    pub entry_parameters: Vec<ArgumentSpec>,
}

/// Build runtime bindings from a plugin specification
pub fn build_runtime_bindings(plugin: &PluginSpec) -> RuntimeBindings {
    let (entry_module, entry_name) = parse_entry_point(&plugin.entry);

    let requires_store = plugin
        .resources
        .as_ref()
        .and_then(|r| r.store.as_ref())
        .is_some();

    let entry_parameters = match plugin.invocation.implementation {
        ImplementationType::Class => plugin.invocation.constructor.clone(),
        ImplementationType::Function => plugin.invocation.call.clone(),
    };

    let call_method = plugin
        .invocation
        .method
        .clone()
        .or_else(|| default_method_for_kind(&plugin.kind));

    RuntimeBindings {
        entry_module,
        entry_name,
        implementation_type: plugin.invocation.implementation.clone(),
        plugin_kind: plugin.kind.clone(),
        config: plugin.resources.as_ref().and_then(|r| r.config.clone()),
        call_method,
        requires_store,
        entry_parameters,
    }
}

fn parse_entry_point(entry: &str) -> (String, String) {
    if let Some(pos) = entry.rfind('.') {
        (entry[..pos].to_string(), entry[pos + 1..].to_string())
    } else {
        (String::new(), entry.to_string())
    }
}

fn default_method_for_kind(kind: &PluginKind) -> Option<String> {
    match kind {
        PluginKind::Parser => Some("build_system".to_string()),
        PluginKind::Exporter => Some("export".to_string()),
        PluginKind::Translation => Some("run".to_string()),
        PluginKind::Upgrader => Some("run".to_string()),
        PluginKind::Modifier => Some("run".to_string()),
        PluginKind::Utility => None,
    }
}

/// Build runtime bindings from a storage Plugin (manifest type)
///
/// This is a convenience function for when you have a Plugin from the manifest
/// but need RuntimeBindings for execution.
pub fn build_runtime_bindings_from_plugin(plugin: &Plugin) -> RuntimeBindings {
    let entry_module = plugin.module.to_string();
    let entry_name = if let Some(ref class_name) = plugin.class_name {
        class_name.to_string()
    } else if let Some(ref function_name) = plugin.function_name {
        function_name.to_string()
    } else {
        // Fallback to plugin name
        plugin.name.to_string()
    };

    let implementation_type = match plugin.plugin_type {
        PluginType::Class => ImplementationType::Class,
        PluginType::Function => ImplementationType::Function,
    };

    // Infer plugin kind from name patterns
    let plugin_kind = infer_plugin_kind(&plugin.name);

    // Build config spec if we have config info
    let config = if let (Some(ref config_class), Some(ref config_module)) =
        (&plugin.config_class, &plugin.config_module)
    {
        // Convert schema fields to exec config fields
        let fields: Vec<ExecConfigField> = plugin
            .config_schema
            .iter()
            .map(|(name, field)| ExecConfigField {
                name: name.to_string(),
                annotation: Some(format!("{:?}", field.field_type)),
                default: field.default.as_ref().map(|d| format!("{:?}", d)),
                required: field.required,
            })
            .collect();

        Some(ConfigSpec {
            module: config_module.to_string(),
            name: config_class.to_string(),
            fields,
        })
    } else {
        None
    };

    let call_method = default_method_for_kind(&plugin_kind);

    // Convert parameters to ArgumentSpec
    let entry_parameters: Vec<ArgumentSpec> = plugin
        .parameters
        .iter()
        .map(|p| ArgumentSpec {
            name: p.name.to_string(),
            annotation: Some(p.format_types()),
            default: p.default.as_ref().map(|d| d.to_string()),
            required: p.required,
        })
        .collect();

    RuntimeBindings {
        entry_module,
        entry_name,
        implementation_type,
        plugin_kind,
        config,
        call_method,
        requires_store: false, // We don't have this info from Plugin
        entry_parameters,
    }
}

fn infer_plugin_kind(name: &str) -> PluginKind {
    let name_lower = name.to_lowercase();
    if name_lower.contains("parser") {
        PluginKind::Parser
    } else if name_lower.contains("exporter") || name_lower.contains("export") {
        PluginKind::Exporter
    } else if name_lower.contains("modifier") {
        PluginKind::Modifier
    } else if name_lower.contains("upgrader") {
        PluginKind::Upgrader
    } else if name_lower.contains("translation") || name_lower.contains("translate") {
        PluginKind::Translation
    } else {
        PluginKind::Utility
    }
}
