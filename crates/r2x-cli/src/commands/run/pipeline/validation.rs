use crate::manifest_lookup::resolve_plugin_ref;
use crate::pipeline_config::PipelineConfig;
use r2x_manifest::runtime::build_runtime_bindings;
use r2x_manifest::types::Manifest;
use std::collections::HashSet;

use crate::commands::run::pipeline::config::resolve_plugin_config_json;
use crate::commands::run::pipeline::constants::AUTO_PROVIDED_PARAMS;
use crate::commands::run::RunError;

pub(super) fn validate_pipeline_configs(
    config: &PipelineConfig,
    pipeline: &[String],
    manifest: &Manifest,
) -> Result<(), RunError> {
    let mut errors: Vec<String> = Vec::new();

    for plugin_name in pipeline {
        let resolved = match resolve_plugin_ref(manifest, plugin_name) {
            Ok(r) => r,
            Err(_) => continue, // Skip validation for unresolved plugins (will fail later anyway)
        };

        let plugin = resolved.plugin;
        let bindings = build_runtime_bindings(plugin);

        // Get user-provided config from YAML
        let yaml_config = match resolve_plugin_config_json(config, plugin_name, &resolved) {
            Ok(c) => c,
            Err(_) => "{}".to_string(),
        };

        let provided_keys: HashSet<String> =
            match serde_json::from_str::<serde_json::Value>(&yaml_config) {
                Ok(serde_json::Value::Object(map)) => map.keys().cloned().collect(),
                _ => HashSet::new(),
            };

        // Check parameters for required ones
        for param in &bindings.parameters {
            let param_name = param.name.as_ref();
            if param.required
                && param.default.is_none()
                && !provided_keys.contains(param_name)
                && !is_auto_provided_param(param_name)
            {
                errors.push(format!(
                    "{}: missing required parameter '{}'",
                    plugin_name, param_name
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(RunError::Config(format!(
            "Pipeline config validation failed:\n  - {}",
            errors.join("\n  - ")
        )))
    }
}

fn is_auto_provided_param(name: &str) -> bool {
    AUTO_PROVIDED_PARAMS.contains(&name)
}

#[cfg(test)]
mod tests {
    use crate::commands::run::pipeline::validation::is_auto_provided_param;

    #[test]
    fn is_auto_provided_param_recognizes_store() {
        assert!(is_auto_provided_param("store"));
        assert!(is_auto_provided_param("data_store"));
        assert!(is_auto_provided_param("stdin"));
        assert!(is_auto_provided_param("system"));
        assert!(is_auto_provided_param("path"));
        assert!(is_auto_provided_param("folder_path"));
        assert!(is_auto_provided_param("config"));
    }

    #[test]
    fn is_auto_provided_param_rejects_user_params() {
        assert!(!is_auto_provided_param("json_path"));
        assert!(!is_auto_provided_param("output_path"));
        assert!(!is_auto_provided_param("system_base_power"));
        assert!(!is_auto_provided_param("model_year"));
    }
}
