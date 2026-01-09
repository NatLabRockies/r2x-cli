use crate::types::{ArgumentSpec, ConfigSpec, ImplementationType, PluginKind, PluginSpec};

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

fn default_method_for_kind(kind: &crate::types::PluginKind) -> Option<String> {
    use crate::types::PluginKind::*;
    match kind {
        Parser => Some("build_system".to_string()),
        Exporter => Some("export".to_string()),
        Translation => Some("run".to_string()),
        _ => None,
    }
}
