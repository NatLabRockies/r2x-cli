use crate::manifest_lookup::resolve_plugin_ref;
use crate::pipeline_config::PipelineConfig;
use crate::r2x_manifest::Manifest;
use std::collections::HashSet;

use super::super::RunError;
use super::config::resolve_plugin_config_json;
use super::constants::AUTO_PROVIDED_PARAMS;

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
        let bindings = crate::r2x_manifest::build_runtime_bindings_from_plugin(plugin);

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

        // Check config fields for required ones
        if let Some(ref config_spec) = bindings.config {
            for field in &config_spec.fields {
                if field.required
                    && field.default.is_none()
                    && !provided_keys.contains(&field.name)
                    && !is_auto_provided_param(&field.name)
                {
                    errors.push(format!(
                        "{}: missing required config field '{}'",
                        plugin_name, field.name
                    ));
                }
            }
        }

        // Check entry parameters for required ones
        for param in &bindings.entry_parameters {
            if param.required
                && param.default.is_none()
                && !provided_keys.contains(&param.name)
                && !is_auto_provided_param(&param.name)
            {
                errors.push(format!(
                    "{}: missing required parameter '{}'",
                    plugin_name, param.name
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
    AUTO_PROVIDED_PARAMS.iter().any(|param| *param == name)
}

#[cfg(test)]
mod tests {
    use super::is_auto_provided_param;

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
