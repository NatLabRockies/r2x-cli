use crate::types::{
    ArgumentSpec, ConfigSpec, ImplementationType, PluginKind, PluginSpec, ResourceSpec, StoreMode,
};

#[derive(Debug, Clone)]
pub struct RuntimeBindings {
    pub entry_module: String,
    pub entry_name: String,
    pub implementation_type: ImplementationType,
    pub plugin_kind: PluginKind,
    pub config: Option<ConfigSpec>,
    pub resources: Option<ResourceSpec>,
    pub call_method: Option<String>,
    pub requires_store: bool,
    pub constructor_args: Vec<ArgumentSpec>,
    pub call_args: Vec<ArgumentSpec>,
}

pub fn build_runtime_bindings(plugin: &PluginSpec) -> RuntimeBindings {
    let (entry_module, entry_name) = parse_entry_point(&plugin.entry);

    let store_required = plugin
        .resources
        .as_ref()
        .and_then(|r| r.store.as_ref())
        .map(|s| s.required || s.modes.contains(&StoreMode::Folder))
        .unwrap_or(false);

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
        resources: plugin.resources.clone(),
        call_method,
        requires_store: store_required,
        constructor_args: plugin.invocation.constructor.clone(),
        call_args: plugin.invocation.call.clone(),
    }
}

fn parse_entry_point(entry: &str) -> (String, String) {
    if let Some(pos) = entry.rfind(':') {
        return (entry[..pos].to_string(), entry[pos + 1..].to_string());
    }
    entry
        .rfind('.')
        .map(|pos| (entry[..pos].to_string(), entry[pos + 1..].to_string()))
        .unwrap_or((String::new(), entry.to_string()))
}

fn default_method_for_kind(kind: &crate::types::PluginKind) -> Option<String> {
    use crate::types::PluginKind::*;
    match kind {
        Parser => Some("build_system".to_string()),
        Exporter => Some("export".to_string()),
        _ => None,
    }
}
