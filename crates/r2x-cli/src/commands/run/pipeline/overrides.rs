use crate::errors::PipelineError;
use r2x_config::Config;
use r2x_logger as logger;
use r2x_manifest::runtime::RuntimeBindings;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::run::pipeline::constants::JSON_PATH_FIELDS;
use crate::commands::run::RunError;

pub(super) fn prepare_pipeline_overrides(
    pipeline_input: Option<&str>,
    bindings: &RuntimeBindings,
    plugin_name: &str,
) -> Result<Option<String>, RunError> {
    let Some(raw) = pipeline_input else {
        return Ok(None);
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // If the plugin doesn't have a json_path/path field, don't merge anything into config.
    // The system JSON will be passed separately via stdin and deserialized by the Python bridge.
    // Merging system JSON into config would pollute config fields (e.g., system_base_power: null
    // from the system would overwrite system_base_power: 100 from YAML config).
    let Some(target_field) = determine_json_path_field(bindings, plugin_name) else {
        return Ok(None);
    };

    let parsed = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => value,
        Err(_) => return Ok(Some(raw.to_string())),
    };

    match parsed {
        serde_json::Value::Object(map) => {
            if map.contains_key(target_field) || !looks_like_system_payload(&map) {
                Ok(Some(raw.to_string()))
            } else {
                let persisted = persist_pipeline_system_json(raw)?;
                logger::debug(&format!(
                    "Persisted upstream stdout for '{}' to {}",
                    plugin_name, persisted
                ));
                let mut override_map = serde_json::Map::new();
                override_map.insert(
                    target_field.to_string(),
                    serde_json::Value::String(persisted),
                );
                Ok(Some(serde_json::Value::Object(override_map).to_string()))
            }
        }
        _ => Ok(Some(raw.to_string())),
    }
}

fn determine_json_path_field(
    bindings: &RuntimeBindings,
    plugin_name: &str,
) -> Option<&'static str> {
    if let Some(config) = &bindings.config {
        for field in JSON_PATH_FIELDS {
            if config.fields.iter().any(|f| f.name == *field) {
                return Some(*field);
            }
        }
    }

    for field in JSON_PATH_FIELDS {
        if bindings.entry_parameters.iter().any(|p| p.name == *field) {
            return Some(*field);
        }
    }

    if plugin_name.contains("parser") {
        return Some("json_path");
    }

    None
}

fn looks_like_system_payload(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    if map.contains_key("components") || map.contains_key("system") {
        return true;
    }
    if let Some(data_obj) = map.get("data").and_then(|v| v.as_object()) {
        return data_obj.contains_key("components")
            || data_obj.contains_key("system_information")
            || data_obj.contains_key("system");
    }
    false
}

fn persist_pipeline_system_json(payload: &str) -> Result<String, RunError> {
    let mut config = Config::load().map_err(|e| RunError::Config(e.to_string()))?;
    let cache_root = config
        .ensure_cache_path()
        .map_err(|e| RunError::Config(e.to_string()))?;
    let dir = PathBuf::from(cache_root).join("pipeline-systems");
    std::fs::create_dir_all(&dir)
        .map_err(PipelineError::Io)
        .map_err(RunError::Pipeline)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| RunError::Config(format!("System clock error: {}", e)))?
        .as_millis();
    let filename = format!(
        "system_{}_{}_{}.json",
        timestamp,
        std::process::id(),
        rand_suffix()
    );
    let path = dir.join(filename);
    std::fs::write(&path, payload)
        .map_err(PipelineError::Io)
        .map_err(RunError::Pipeline)?;
    Ok(path.to_string_lossy().to_string())
}

fn rand_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}
