use crate::types::{
    CallableMetadata, ConfigMetadata, ConstructorArg, DiscoveryPlugin, ParameterEntry,
    ParameterMetadata, ResolvedReference,
};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RuntimeBindings {
    pub callable: CallableMetadata,
    pub config: Option<ConfigMetadata>,
    pub call_method: Option<String>,
    pub io_type: Option<String>,
    pub requires_store: Option<bool>,
}

pub fn build_runtime_bindings(plugin: &DiscoveryPlugin) -> Result<RuntimeBindings, String> {
    let callable_arg = find_arg(plugin, "obj")
        .ok_or_else(|| format!("Plugin '{}' missing 'obj' argument", plugin.name))?;
    let callable_ref = find_reference(plugin, &callable_arg.value).ok_or_else(|| {
        format!(
            "Plugin '{}' missing resolved reference for '{}'",
            plugin.name, callable_arg.value
        )
    })?;

    let callable = CallableMetadata {
        module: callable_ref.module.clone(),
        name: callable_ref.name.clone(),
        callable_type: callable_ref.ref_type.clone(),
        return_annotation: callable_ref.return_annotation.clone(),
        parameters: build_parameter_map(&callable_ref.parameters),
    };

    let config = find_arg(plugin, "config")
        .and_then(|arg| find_reference(plugin, &arg.value))
        .map(|reference| ConfigMetadata {
            module: reference.module.clone(),
            name: reference.name.clone(),
            return_annotation: reference.return_annotation.clone(),
            parameters: build_parameter_map(&reference.parameters),
        });

    let call_method = find_arg(plugin, "call_method").map(|arg| arg.value.clone());
    let io_type = find_arg(plugin, "io_type").map(|arg| arg.value.clone());
    let requires_store = find_arg(plugin, "requires_store").and_then(|arg| parse_bool(&arg.value));

    Ok(RuntimeBindings {
        callable,
        config,
        call_method,
        io_type,
        requires_store,
    })
}

fn find_arg<'a>(plugin: &'a DiscoveryPlugin, name: &str) -> Option<&'a ConstructorArg> {
    plugin.constructor_args.iter().find(|arg| arg.name == name)
}

fn find_reference<'a>(plugin: &'a DiscoveryPlugin, key: &str) -> Option<&'a ResolvedReference> {
    plugin
        .resolved_references
        .iter()
        .find(|reference| reference.key == key)
}

fn build_parameter_map(entries: &[ParameterEntry]) -> HashMap<String, ParameterMetadata> {
    let mut map = HashMap::new();
    for entry in entries {
        map.insert(
            entry.name.clone(),
            ParameterMetadata {
                annotation: entry.annotation.clone(),
                default: entry.default.clone(),
                is_required: entry.is_required,
            },
        );
    }
    map
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
