use crate::errors::PipelineError;
use r2x_config::Config;
use r2x_logger as logger;
use r2x_manifest::runtime::RuntimeBindings;
use r2x_python::plugin_invoker::ArtifactBundle;
use std::path::{Path, PathBuf};
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

pub(super) fn prepare_pipeline_artifact_overrides(
    pipeline_input: Option<&ArtifactBundle>,
    bindings: &RuntimeBindings,
    plugin_name: &str,
) -> Option<String> {
    let input = pipeline_input?;
    let target_field = determine_json_path_field(bindings, plugin_name)?;
    let mut overrides = serde_json::Map::new();
    overrides.insert(
        target_field.to_string(),
        serde_json::Value::String(input.entrypoint_path().to_string_lossy().into_owned()),
    );
    Some(serde_json::Value::Object(overrides).to_string())
}

fn determine_json_path_field(
    bindings: &RuntimeBindings,
    plugin_name: &str,
) -> Option<&'static str> {
    // Check plugin parameters for json_path-like fields
    for field in JSON_PATH_FIELDS {
        if bindings
            .parameters
            .iter()
            .any(|p| p.name.as_ref() == *field)
        {
            return Some(*field);
        }
    }

    if bindings.role == r2x_manifest::runtime::PluginRole::Upgrader {
        return Some("path");
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
    persist_pipeline_system_json_at(
        payload,
        &dir,
        &std::env::current_dir().map_err(|e| {
            RunError::Config(format!("Failed to determine current directory: {}", e))
        })?,
    )
}

fn persist_pipeline_system_json_at(
    payload: &str,
    destination_dir: &Path,
    sidecar_parent_dir: &Path,
) -> Result<String, RunError> {
    std::fs::create_dir_all(destination_dir)
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
    let path = destination_dir.join(filename);

    // Inline pipeline payloads only contain JSON. If a System references a
    // relative or absolute time-series sidecar, copy a self-contained bundle
    // next to the persisted JSON and make the reference relative to it.
    // Otherwise the next plugin can deserialize the JSON but cannot resolve
    // its attached time series.
    let mut persisted = serde_json::from_str::<serde_json::Value>(payload).ok();
    if let Some(ref mut value) = persisted {
        if let Some(time_series) = time_series_metadata_mut(value) {
            if let Some(directory) = time_series.get("directory").and_then(|v| v.as_str()) {
                let source = PathBuf::from(directory);
                let source = if source.is_absolute() {
                    source
                } else {
                    sidecar_parent_dir.join(source)
                };
                if source.is_dir() {
                    let sidecar_name = format!(
                        "{}_time_series",
                        path.file_stem().unwrap().to_string_lossy()
                    );
                    let destination = destination_dir.join(&sidecar_name);
                    copy_sidecar_directory(&source, &destination)?;
                    time_series.insert(
                        "directory".to_string(),
                        serde_json::Value::String(sidecar_name),
                    );
                }
            }
        }
    }

    let output = persisted
        .as_ref()
        .map(serde_json::Value::to_string)
        .unwrap_or_else(|| payload.to_string());
    std::fs::write(&path, output)
        .map_err(PipelineError::Io)
        .map_err(RunError::Pipeline)?;
    Ok(path.to_string_lossy().to_string())
}

fn time_series_metadata_mut(
    value: &mut serde_json::Value,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    let object = value.as_object_mut()?;
    if object.contains_key("time_series") {
        return object
            .get_mut("time_series")
            .and_then(serde_json::Value::as_object_mut);
    }

    if object.contains_key("system") {
        return object.get_mut("system").and_then(time_series_metadata_mut);
    }
    object.get_mut("data").and_then(time_series_metadata_mut)
}

fn copy_sidecar_directory(source: &Path, destination: &Path) -> Result<(), RunError> {
    std::fs::create_dir_all(destination)
        .map_err(PipelineError::Io)
        .map_err(RunError::Pipeline)?;
    for entry in std::fs::read_dir(source)
        .map_err(PipelineError::Io)
        .map_err(RunError::Pipeline)?
    {
        let entry = entry
            .map_err(PipelineError::Io)
            .map_err(RunError::Pipeline)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path)
            .map_err(PipelineError::Io)
            .map_err(RunError::Pipeline)?;
        if metadata.file_type().is_symlink() {
            return Err(RunError::Config(format!(
                "Time-series sidecar contains unsupported symlink: {}",
                source_path.display()
            )));
        }
        if metadata.is_dir() {
            copy_sidecar_directory(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            std::fs::copy(&source_path, &destination_path)
                .map_err(PipelineError::Io)
                .map_err(RunError::Pipeline)?;
        } else {
            return Err(RunError::Config(format!(
                "Time-series sidecar contains unsupported filesystem entry: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

fn rand_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::persist_pipeline_system_json_at;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn persists_inline_system_with_relative_time_series_sidecar() {
        let source = tempdir().unwrap();
        let sidecar = source.path().join("system_time_series");
        fs::create_dir(&sidecar).unwrap();
        fs::write(sidecar.join("time_series_metadata.db"), b"metadata").unwrap();

        let destination = tempdir().unwrap();
        let payload = serde_json::json!({
            "components": [],
            "time_series": { "directory": "system_time_series" }
        })
        .to_string();

        let persisted =
            persist_pipeline_system_json_at(&payload, destination.path(), source.path()).unwrap();
        let persisted_path = std::path::PathBuf::from(persisted);
        let persisted_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&persisted_path).unwrap()).unwrap();
        let directory = persisted_json["time_series"]["directory"].as_str().unwrap();

        assert!(!std::path::Path::new(directory).is_absolute());
        assert_eq!(
            fs::read(
                persisted_path
                    .parent()
                    .unwrap()
                    .join(directory)
                    .join("time_series_metadata.db")
            )
            .unwrap(),
            b"metadata"
        );
    }
}
