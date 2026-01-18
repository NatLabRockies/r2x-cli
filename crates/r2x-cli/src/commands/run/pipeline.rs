use super::RunError;
use crate::errors::PipelineError;
use crate::logger;
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
        let (_pkg, plugin) = manifest
            .packages
            .iter()
            .find_map(|pkg| {
                pkg.plugins
                    .iter()
                    .find(|p| p.name == *plugin_name)
                    .map(|p| (pkg, p))
            })
            .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

        let bindings = r2x_manifest::build_runtime_bindings(plugin);
        let has_obj = bindings.implementation_type == r2x_manifest::ImplementationType::Class;
        let input_marker = if index > 0 { "← stdin" } else { "" };
        let output_marker = if has_obj { "→ stdout" } else { "" };

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

        let (pkg, plugin) = manifest
            .packages
            .iter()
            .find_map(|pkg| {
                pkg.plugins
                    .iter()
                    .find(|p| p.name == *plugin_name)
                    .map(|p| (pkg, p))
            })
            .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

        let bindings = r2x_manifest::build_runtime_bindings(plugin);

        let yaml_config = if config.config.contains_key(plugin_name) {
            config.get_plugin_config_json(plugin_name)?
        } else {
            "{}".to_string()
        };

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
        logger::debug(&format!("Config: {}", final_config_json));

        // Set current plugin context for logging
        logger::set_current_plugin(Some(plugin_name.to_string()));

        // Reconfigure Python logging with plugin name
        if let Err(e) = Bridge::reconfigure_logging_for_plugin(plugin_name) {
            logger::warn(&format!(
                "Failed to reconfigure Python logging for plugin {}: {}",
                plugin_name, e
            ));
        }

        let invocation_result =
            match bridge.invoke_plugin(&target, &final_config_json, stdin_json, Some(plugin)) {
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

    let Some(target_field) = determine_json_path_field(bindings, plugin_name) else {
        return Ok(Some(raw.to_string()));
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
