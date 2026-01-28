use crate::manifest_lookup::ResolvedPlugin;
use crate::pipeline_config::PipelineConfig;
use r2x_manifest::runtime::{build_runtime_bindings, PluginRole};
use std::collections::HashSet;

use crate::commands::run::RunError;

pub(super) fn resolve_plugin_config_json(
    config: &PipelineConfig,
    plugin_ref: &str,
    resolved: &ResolvedPlugin<'_>,
) -> Result<String, RunError> {
    let plugin_name = resolved.plugin.name.as_ref();
    let package_name = resolved.package.name.as_ref();
    let role = build_runtime_bindings(resolved.plugin).role;
    let kind_alias = plugin_role_alias(role);

    for key in config_key_candidates(plugin_ref, package_name, plugin_name, kind_alias) {
        if config.config.contains_key(&key) {
            return config
                .get_plugin_config_json(&key)
                .map_err(RunError::Pipeline);
        }
    }

    Ok("{}".to_string())
}

fn config_key_candidates(
    plugin_ref: &str,
    package_name: &str,
    plugin_name: &str,
    kind_alias: Option<&str>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let mut push = |key: String| {
        if seen.insert(key.clone()) {
            candidates.push(key);
        }
    };

    push(plugin_ref.to_string());

    if let Some((ref_package, ref_name)) = plugin_ref.split_once('.') {
        let ref_name_underscore = ref_name.replace('-', "_");
        if ref_name_underscore != ref_name {
            push(format!("{}.{}", ref_package, ref_name_underscore));
        }

        if ref_package != package_name {
            push(format!("{}.{}", package_name, ref_name));
            if ref_name_underscore != ref_name {
                push(format!("{}.{}", package_name, ref_name_underscore));
            }
        }
    }

    push(plugin_name.to_string());
    let plugin_name_underscore = plugin_name.replace('-', "_");
    if plugin_name_underscore != plugin_name {
        push(plugin_name_underscore.clone());
    }

    push(format!("{}.{}", package_name, plugin_name));
    if plugin_name_underscore != plugin_name {
        push(format!("{}.{}", package_name, plugin_name_underscore));
    }

    if let Some(alias) = kind_alias {
        if let Some((ref_package, _)) = plugin_ref.split_once('.') {
            push(format!("{}.{}", ref_package, alias));
        }
        push(format!("{}.{}", package_name, alias));
    }

    candidates
}

fn plugin_role_alias(role: PluginRole) -> Option<&'static str> {
    match role {
        PluginRole::Parser => Some("parser"),
        PluginRole::Exporter => Some("exporter"),
        PluginRole::Upgrader => Some("upgrader"),
        PluginRole::Modifier => Some("modifier"),
        PluginRole::Translation => Some("translation"),
        PluginRole::Utility => None,
    }
}
