use r2x_manifest::execution_types::ImplementationType;
use r2x_manifest::runtime::RuntimeBindings;
use std::collections::HashSet;

use crate::commands::run::pipeline::constants::{
    DEFAULT_OUTPUT_ROOT, FOLDER_FIELD_KEYS, PATH_FALLBACK_KEYS, STORE_FIELD_KEYS,
};
use crate::commands::run::RunError;

pub(super) fn build_plugin_config(
    bindings: &RuntimeBindings,
    package_name: &str,
    yaml_config_json: &str,
    output_folder: Option<&str>,
    inherited_store_path: Option<&str>,
    stdin_overrides: Option<&str>,
) -> Result<String, RunError> {
    let mut yaml_config: serde_json::Value = serde_json::from_str(yaml_config_json)
        .map_err(|e| RunError::Config(format!("Failed to parse YAML config: {}", e)))?;

    if let Some(overrides) = stdin_overrides {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(overrides) {
            merge_config_values(&mut yaml_config, value);
        }
    }

    let mut final_config = serde_json::Map::new();
    let mut store_value_for_folder: Option<serde_json::Value> = None;
    if bindings.implementation_type == ImplementationType::Class {
        let mut config_class_params = serde_json::Map::new();
        let mut constructor_params = serde_json::Map::new();
        let config_param_names: HashSet<String> = bindings
            .config
            .as_ref()
            .map(|config_meta| config_meta.fields.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default();

        if let serde_json::Value::Object(ref yaml_map) = yaml_config {
            for (key, value) in yaml_map {
                if config_param_names.contains(key) {
                    config_class_params.insert(key.clone(), value.clone());
                } else if bindings.entry_parameters.iter().any(|p| p.name == *key) {
                    constructor_params.insert(key.clone(), value.clone());
                } else {
                    config_class_params.insert(key.clone(), value.clone());
                }
            }
        }

        // For Class-based plugins (parsers, exporters), pass config fields flat.
        // The Python bridge (kwargs.rs) will instantiate the PluginConfig class
        // from these flat fields and pass the instance as the `config` parameter.
        // We do NOT wrap under a `config` key here - that would create double nesting.
        final_config.extend(config_class_params);
        final_config.extend(constructor_params);

        if bindings.entry_parameters.iter().any(|p| p.name == "path")
            && !final_config.contains_key("path")
            && matches!(yaml_config, serde_json::Value::Object(_))
        {
            if let serde_json::Value::Object(ref yaml_map) = yaml_config {
                if let Some(path_value) = pick_value(yaml_map, PATH_FALLBACK_KEYS) {
                    final_config.insert("path".to_string(), path_value);
                }
            }
        }

        // Check if plugin requires a DataStore instance.
        // If so, create it from the `path` config value.
        let needs_store =
            bindings.requires_store || bindings.entry_parameters.iter().any(|p| p.name == "store");

        if needs_store {
            // Use `path` as primary source for store, with fallbacks
            let store_value = if let serde_json::Value::Object(ref yaml_map) = yaml_config {
                pick_value(yaml_map, STORE_FIELD_KEYS)
                    .or_else(|| {
                        inherited_store_path.map(|p| serde_json::Value::String(p.to_string()))
                    })
                    .map_or_else(|| fallback_store_value(package_name, output_folder), Ok)?
            } else if let Some(inherited) = inherited_store_path {
                serde_json::Value::String(inherited.to_string())
            } else {
                fallback_store_value(package_name, output_folder)?
            };

            store_value_for_folder = Some(store_value.clone());
            final_config.insert("store".to_string(), store_value);
        }

        if bindings
            .entry_parameters
            .iter()
            .any(|p| p.name == "folder_path")
            && !final_config.contains_key("folder_path")
        {
            let explicit_folder = if let serde_json::Value::Object(ref yaml_map) = yaml_config {
                pick_value(yaml_map, FOLDER_FIELD_KEYS)
            } else {
                None
            };

            let folder_value = explicit_folder
                .or_else(|| store_value_for_folder.as_ref().and_then(value_string_clone))
                .or_else(|| {
                    inherited_store_path.map(|path| serde_json::Value::String(path.to_string()))
                });

            if let Some(value) = folder_value {
                final_config.insert("folder_path".to_string(), value);
            }
        }
    } else if let serde_json::Value::Object(ref yaml_map) = yaml_config {
        final_config.extend(yaml_map.clone());
    }

    serde_json::to_string(&serde_json::Value::Object(final_config))
        .map_err(|e| RunError::Config(format!("Failed to serialize final config: {}", e)))
}

fn pick_value(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<serde_json::Value> {
    keys.iter().find_map(|key| map.get(*key).cloned())
}

fn value_string_clone(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::String(s) => Some(serde_json::Value::String(s.clone())),
        _ => None,
    }
}

fn merge_config_values(target: &mut serde_json::Value, overrides: serde_json::Value) {
    match (target, overrides) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(override_map)) => {
            for (key, value) in override_map {
                match target_map.get_mut(&key) {
                    Some(existing) => merge_config_values(existing, value),
                    None => {
                        target_map.insert(key, value);
                    }
                }
            }
        }
        (target_value, override_value) => {
            *target_value = override_value;
        }
    }
}

fn fallback_store_value(
    _package_name: &str,
    output_folder: Option<&str>,
) -> Result<serde_json::Value, RunError> {
    let output_folder = output_folder.unwrap_or(DEFAULT_OUTPUT_ROOT);
    let store_path = format!("{}/store", output_folder);
    std::fs::create_dir_all(&store_path)
        .map_err(|e| RunError::Config(format!("Failed to create store directory: {}", e)))?;

    Ok(serde_json::Value::String(store_path))
}

#[cfg(test)]
mod tests {
    use crate::commands::run::pipeline::builder::merge_config_values;
    use serde_json::json;

    #[test]
    fn merge_config_values_replaces_existing() {
        let mut target = json!({
            "system_base_power": 100,
            "system_name": "TestSystem"
        });
        let overrides = json!({
            "system_base_power": 200,
            "new_field": "value"
        });

        merge_config_values(&mut target, overrides);

        assert_eq!(target["system_base_power"], json!(200));
        assert_eq!(target["system_name"], json!("TestSystem"));
        assert_eq!(target["new_field"], json!("value"));
    }

    #[test]
    fn merge_config_values_adds_new_fields() {
        let mut target = json!({
            "system_name": "TestSystem"
        });
        let overrides = json!({
            "optional_field": "new_value",
            "another": 42
        });

        merge_config_values(&mut target, overrides);

        assert_eq!(target["system_name"], json!("TestSystem"));
        assert_eq!(target["optional_field"], json!("new_value"));
        assert_eq!(target["another"], json!(42));
    }

    #[test]
    fn merge_config_values_nested_merge() {
        let mut target = json!({
            "config": {
                "base_power": 100,
                "name": "Test"
            }
        });
        let overrides = json!({
            "config": {
                "base_power": 200,
                "extra": "new"
            }
        });

        merge_config_values(&mut target, overrides);

        assert_eq!(target["config"]["base_power"], json!(200));
        assert_eq!(target["config"]["name"], json!("Test"));
        assert_eq!(target["config"]["extra"], json!("new"));
    }
}
