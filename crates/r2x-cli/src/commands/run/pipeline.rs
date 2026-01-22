use super::RunError;
use crate::errors::PipelineError;
use crate::logger;
use crate::manifest_lookup::{resolve_plugin_ref, PluginRefError, ResolvedPlugin};
use crate::package_verification;
use crate::pipeline_config::PipelineConfig;
use crate::python_bridge::Bridge;
use crate::r2x_manifest::{self, Manifest};
use crate::GlobalOpts;
use colored::Colorize;
use r2x_config::Config;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(super) fn handle_pipeline_mode(
    yaml_path: String,
    pipeline_name: Option<String>,
    list: bool,
    print: bool,
    dry_run: bool,
    output: Option<String>,
    opts: &GlobalOpts,
) -> Result<(), RunError> {
    let config = PipelineConfig::load(&yaml_path)?;

    if list {
        list_pipelines(&config);
    } else if print {
        if let Some(name) = pipeline_name {
            print_pipeline_config(&config, &name)?;
        } else {
            return Err(RunError::InvalidArgs(
                "Pipeline name required with --print".to_string(),
            ));
        }
    } else if let Some(name) = pipeline_name {
        if dry_run {
            show_pipeline_flow(&config, &name)?;
        } else {
            run_pipeline(&config, &name, output.as_deref(), opts)?;
        }
    } else {
        return Err(RunError::InvalidArgs(
            "Pipeline name required for execution".to_string(),
        ));
    }

    Ok(())
}

fn list_pipelines(config: &PipelineConfig) {
    let pipelines = config.list_pipelines();

    if pipelines.is_empty() {
        logger::warn("No pipelines found in YAML file");
        return;
    }

    logger::step("Available Pipelines:");
    for name in pipelines {
        if let Some(steps) = config.get_pipeline(&name) {
            println!("  {} ({} steps)", name, steps.len());
            for step in steps {
                println!("    - {}", step);
            }
        }
    }
}

fn print_pipeline_config(config: &PipelineConfig, pipeline_name: &str) -> Result<(), RunError> {
    let output = config.print_pipeline_config(pipeline_name)?;
    println!("{}", output);
    Ok(())
}

fn show_pipeline_flow(config: &PipelineConfig, pipeline_name: &str) -> Result<(), RunError> {
    let pipeline = config
        .get_pipeline(pipeline_name)
        .ok_or_else(|| PipelineError::PipelineNotFound(pipeline_name.to_string()))?;

    let manifest = Manifest::load()?;

    logger::success(&format!("Pipeline: {}", pipeline_name));
    println!("\nPipeline flow (--dry-run):");

    for (index, plugin_name) in pipeline.iter().enumerate() {
        let resolved = resolve_plugin_ref(&manifest, plugin_name).map_err(|err| match err {
            PluginRefError::NotFound(_) => RunError::PluginNotFound(plugin_name.to_string()),
            _ => RunError::Config(err.to_string()),
        })?;
        let plugin = resolved.plugin;

        // Check if it's a class-based plugin
        let is_class = plugin.class_name.is_some();
        let input_marker = if index > 0 { "← stdin" } else { "" };
        let output_marker = if is_class { "→ stdout" } else { "" };

        print!("  {}", plugin_name);
        if !input_marker.is_empty() {
            print!("  {}", input_marker.dimmed());
        }
        if !output_marker.is_empty() {
            print!("  {}", output_marker.dimmed());
        }
        println!();
    }

    println!(
        "\n{}  No actual execution. Use without --dry-run to run the pipeline.",
        "✔".green()
    );

    Ok(())
}

fn run_pipeline(
    config: &PipelineConfig,
    pipeline_name: &str,
    output_file: Option<&str>,
    opts: &GlobalOpts,
) -> Result<(), RunError> {
    let pipeline = config
        .get_pipeline(pipeline_name)
        .ok_or_else(|| PipelineError::PipelineNotFound(pipeline_name.to_string()))?;

    let manifest = Manifest::load()?;
    let total_steps = pipeline.len();

    logger::debug("Verifying packages for pipeline...");
    for plugin_name in pipeline.iter() {
        package_verification::verify_and_ensure_plugin(&manifest, plugin_name)
            .map_err(|e| RunError::Verification(e.to_string()))?;
    }
    logger::debug("All pipeline packages verified");

    // Validate all plugin configs upfront before running anything
    logger::debug("Validating pipeline configs...");
    validate_pipeline_configs(config, pipeline, &manifest)?;
    logger::debug("All pipeline configs validated");

    let pipeline_start = Instant::now();
    eprintln!("{}", format!("Running: {}", pipeline_name).cyan().bold());

    // Show log file location to user
    if let Some(log_path) = logger::get_log_path() {
        eprintln!("{}", format!("  Log file: {}", log_path.display()).dimmed());
    }

    let mut current_stdin: Option<String> = None;

    let resolved_output_folder = if let Some(folder) = &config.output_folder {
        Some(
            config
                .substitute_string(folder)
                .map_err(RunError::Pipeline)?,
        )
    } else {
        None
    };

    let mut current_store_path: Option<String> = None;

    for (idx, plugin_name) in pipeline.iter().enumerate() {
        let step_num = idx + 1;
        logger::spinner_start(&format!("  {} [{}/{}]", plugin_name, step_num, total_steps));
        let step_start = Instant::now();

        let resolved = resolve_plugin_ref(&manifest, plugin_name).map_err(|err| match err {
            PluginRefError::NotFound(_) => RunError::PluginNotFound(plugin_name.to_string()),
            _ => RunError::Config(err.to_string()),
        })?;
        let pkg = resolved.package;
        let plugin = resolved.plugin;

        let bindings = r2x_manifest::build_runtime_bindings_from_plugin(plugin);

        let yaml_config = resolve_plugin_config_json(config, plugin_name, &resolved)?;

        if let Ok(serde_json::Value::Object(map)) =
            serde_json::from_str::<serde_json::Value>(&yaml_config)
        {
            if let Some(store_path) = map.get("store_path").and_then(|value| value.as_str()) {
                current_store_path = Some(store_path.to_string());
            }
        }

        let pipeline_input = current_stdin.as_deref();
        let stdin_json = pipeline_input;

        let pipeline_overrides =
            prepare_pipeline_overrides(pipeline_input, &bindings, plugin_name)?;

        let final_config_json = build_plugin_config(
            &bindings,
            &pkg.name,
            &yaml_config,
            resolved_output_folder.as_deref(),
            current_store_path.as_deref(),
            pipeline_overrides.as_deref(),
        )?;

        let target = super::build_call_target(&bindings)?;
        let bridge = Bridge::get()?;
        logger::debug(&format!("Invoking: {}", target));

        // Set current plugin context for logging
        logger::set_current_plugin(Some(plugin_name.to_string()));

        // Reconfigure Python logging with plugin name
        if let Err(e) = Bridge::reconfigure_logging_for_plugin(plugin_name) {
            logger::warn(&format!(
                "Failed to reconfigure Python logging for plugin {}: {}",
                plugin_name, e
            ));
        }

        let invocation_result = match bridge.invoke_plugin_with_bindings(
            &target,
            &final_config_json,
            stdin_json,
            Some(&bindings),
        ) {
            Ok(inv_result) => {
                let elapsed = step_start.elapsed();
                logger::spinner_success(&format!(
                    "{} [{}/{}] ({})",
                    plugin_name,
                    step_num,
                    total_steps,
                    super::format_duration(elapsed)
                ));
                if logger::get_verbosity() > 0 {
                    if let Some(timings) = &inv_result.timings {
                        super::print_plugin_timing_breakdown(timings);
                    }
                }
                inv_result
            }
            Err(e) => {
                let elapsed = step_start.elapsed();
                logger::spinner_error(&format!(
                    "{} [{}/{}] ({})",
                    plugin_name,
                    step_num,
                    total_steps,
                    super::format_duration(elapsed)
                ));
                // Clear plugin context before returning error
                logger::set_current_plugin(None);
                return Err(RunError::Bridge(e));
            }
        };

        // Clear plugin context after execution
        logger::set_current_plugin(None);

        let result = invocation_result.output;

        if !result.is_empty() && result != "null" {
            if !opts.no_stdout {
                logger::debug(&format!("Plugin produced output ({} bytes)", result.len()));
            } else {
                logger::debug("Plugin produced output (suppressed by --no-stdout)");
            }
            current_stdin = Some(result);
        } else {
            logger::debug("Plugin produced no output or output not used");
        }
    }

    eprintln!(
        "{}",
        format!(
            "Finished in: {}",
            super::format_duration(pipeline_start.elapsed())
        )
        .green()
        .bold()
    );

    if let Some(final_output) = current_stdin {
        if let Some(output_path) = output_file {
            logger::step(&format!("Writing output to: {}", output_path));
            std::fs::write(output_path, final_output.as_bytes())
                .map_err(|e| RunError::Pipeline(PipelineError::Io(e)))?;
            logger::success(&format!("Output saved to: {}", output_path));
        } else if opts.suppress_stdout() || opts.no_stdout {
            logger::debug("Pipeline output suppressed");
        } else {
            println!("{}", final_output);
        }
    }

    Ok(())
}

fn resolve_plugin_config_json(
    config: &PipelineConfig,
    plugin_ref: &str,
    resolved: &ResolvedPlugin<'_>,
) -> Result<String, RunError> {
    let plugin_name = resolved.plugin.name.as_ref();
    let package_name = resolved.package.name.as_ref();
    let kind = r2x_manifest::build_runtime_bindings_from_plugin(resolved.plugin).plugin_kind;
    let kind_alias = plugin_kind_alias(kind);

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

fn plugin_kind_alias(kind: r2x_manifest::PluginKind) -> Option<&'static str> {
    match kind {
        r2x_manifest::PluginKind::Parser => Some("parser"),
        r2x_manifest::PluginKind::Exporter => Some("exporter"),
        r2x_manifest::PluginKind::Upgrader => Some("upgrader"),
        r2x_manifest::PluginKind::Modifier => Some("modifier"),
        r2x_manifest::PluginKind::Translation => Some("translation"),
        r2x_manifest::PluginKind::Utility => None,
    }
}

/// Parameters that are automatically provided by the pipeline runtime,
/// so they don't need to be specified in YAML config.
fn is_auto_provided_param(name: &str) -> bool {
    matches!(
        name,
        "store" | "data_store" | "stdin" | "system" | "path" | "folder_path" | "config"
    )
}

/// Validate that all required config fields are present for all plugins in the pipeline.
/// This runs BEFORE any plugin execution, so we fail fast on missing config.
fn validate_pipeline_configs(
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
        let bindings = r2x_manifest::build_runtime_bindings_from_plugin(plugin);

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

fn prepare_pipeline_overrides(
    pipeline_input: Option<&str>,
    bindings: &r2x_manifest::runtime::RuntimeBindings,
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
    bindings: &r2x_manifest::runtime::RuntimeBindings,
    plugin_name: &str,
) -> Option<&'static str> {
    if let Some(config) = &bindings.config {
        if config.fields.iter().any(|f| f.name == "json_path") {
            return Some("json_path");
        }
        if config.fields.iter().any(|f| f.name == "path") {
            return Some("path");
        }
    }

    if bindings
        .entry_parameters
        .iter()
        .any(|p| p.name == "json_path")
    {
        return Some("json_path");
    }
    if bindings.entry_parameters.iter().any(|p| p.name == "path") {
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
    fs::create_dir_all(&dir)
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
    fs::write(&path, payload)
        .map_err(PipelineError::Io)
        .map_err(RunError::Pipeline)?;
    Ok(path.to_string_lossy().to_string())
}

fn rand_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn build_plugin_config(
    bindings: &r2x_manifest::runtime::RuntimeBindings,
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
    if bindings.implementation_type == r2x_manifest::ImplementationType::Class {
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
                if let Some(path_value) = yaml_map
                    .get("path")
                    .or_else(|| yaml_map.get("store_path"))
                    .cloned()
                {
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
                yaml_map
                    .get("path")
                    .or_else(|| yaml_map.get("store"))
                    .or_else(|| yaml_map.get("store_path"))
                    .cloned()
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
                yaml_map
                    .get("folder_path")
                    .or_else(|| yaml_map.get("store_path"))
                    .or_else(|| yaml_map.get("path"))
                    .cloned()
            } else {
                None
            };

            let folder_value = explicit_folder
                .or_else(|| {
                    store_value_for_folder
                        .as_ref()
                        .and_then(|value| match value {
                            serde_json::Value::String(s) => {
                                Some(serde_json::Value::String(s.clone()))
                            }
                            _ => None,
                        })
                })
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
    let output_folder = output_folder.unwrap_or("/tmp/r2x-output");
    let store_path = format!("{}/store", output_folder);
    std::fs::create_dir_all(&store_path)
        .map_err(|e| RunError::Config(format!("Failed to create store directory: {}", e)))?;

    Ok(serde_json::Value::String(store_path))
}

#[cfg(test)]
mod tests {
    use super::*;
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
