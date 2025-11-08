use super::{runtime_bindings_from_disc, RunError};
use crate::errors::PipelineError;
use crate::logger;
use crate::package_verification;
use crate::pipeline_config::PipelineConfig;
use crate::python_bridge::Bridge;
use crate::r2x_manifest::{self, Manifest};
use colored::Colorize;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub(super) fn handle_pipeline_mode(
    yaml_path: String,
    pipeline_name: Option<String>,
    list: bool,
    print: bool,
    dry_run: bool,
    output: Option<String>,
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
            run_pipeline(&config, &name, output.as_deref())?;
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
        let (_pkg, disc_plugin) = manifest
            .packages
            .iter()
            .find_map(|pkg| {
                pkg.plugins
                    .iter()
                    .find(|p| p.name == *plugin_name)
                    .map(|p| (pkg, p))
            })
            .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

        let bindings = runtime_bindings_from_disc(disc_plugin)?;
        let has_obj = bindings.callable.callable_type == "class";
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

        let (pkg, disc_plugin) = manifest
            .packages
            .iter()
            .find_map(|pkg| {
                pkg.plugins
                    .iter()
                    .find(|p| p.name == *plugin_name)
                    .map(|p| (pkg, p))
            })
            .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

        let bindings = runtime_bindings_from_disc(disc_plugin)?;

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

        let final_config_json = build_plugin_config(
            &bindings,
            &pkg.name,
            &yaml_config,
            resolved_output_folder.as_deref(),
            current_store_path.as_deref(),
        )?;

        let uses_stdin = matches!(bindings.io_type.as_deref(), Some("stdin") | Some("both"));
        let stdin_json = if uses_stdin {
            current_stdin.as_deref()
        } else {
            None
        };

        let target = super::build_call_target(&bindings)?;
        let bridge = Bridge::get()?;
        logger::debug(&format!("Invoking: {}", target));
        logger::debug(&format!("Config: {}", final_config_json));

        let result = match bridge.invoke_plugin(
            &target,
            &final_config_json,
            stdin_json,
            Some(disc_plugin),
        ) {
            Ok(result) => {
                let elapsed = step_start.elapsed();
                logger::spinner_success(&format!(
                    "{} [{}/{}] ({})",
                    plugin_name,
                    step_num,
                    total_steps,
                    format_duration(elapsed)
                ));
                result
            }
            Err(e) => {
                let elapsed = step_start.elapsed();
                logger::spinner_error(&format!(
                    "{} [{}/{}] ({})",
                    plugin_name,
                    step_num,
                    total_steps,
                    format_duration(elapsed)
                ));
                return Err(RunError::Bridge(e));
            }
        };

        let produces_stdout = matches!(bindings.io_type.as_deref(), Some("stdout") | Some("both"));
        if produces_stdout && !result.is_empty() && result != "null" {
            logger::debug(&format!("Plugin produced output ({} bytes)", result.len()));
            current_stdin = Some(result);
        } else {
            logger::debug("Plugin produced no output or output not used");
        }
    }

    eprintln!(
        "{}",
        format!("Finished in: {}", format_duration(pipeline_start.elapsed()))
            .green()
            .bold()
    );

    if let Some(final_output) = current_stdin {
        if let Some(output_path) = output_file {
            logger::step(&format!("Writing output to: {}", output_path));
            std::fs::write(output_path, final_output.as_bytes())
                .map_err(|e| RunError::Pipeline(PipelineError::Io(e)))?;
            logger::success(&format!("Output saved to: {}", output_path));
        } else {
            println!("{}", final_output);
        }
    }

    Ok(())
}

fn format_duration(duration: Duration) -> String {
    let total_ms = duration.as_millis();
    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}

fn build_plugin_config(
    bindings: &r2x_manifest::runtime::RuntimeBindings,
    package_name: &str,
    yaml_config_json: &str,
    output_folder: Option<&str>,
    inherited_store_path: Option<&str>,
) -> Result<String, RunError> {
    let yaml_config: serde_json::Value = serde_json::from_str(yaml_config_json)
        .map_err(|e| RunError::Config(format!("Failed to parse YAML config: {}", e)))?;

    let mut final_config = serde_json::Map::new();
    let obj = &bindings.callable;
    if obj.callable_type == "class" {
        let mut config_class_params = serde_json::Map::new();
        let mut constructor_params = serde_json::Map::new();
        let config_param_names: HashSet<String> = bindings
            .config
            .as_ref()
            .map(|config_meta| config_meta.parameters.keys().cloned().collect())
            .unwrap_or_default();

        if let serde_json::Value::Object(ref yaml_map) = yaml_config {
            for (key, value) in yaml_map {
                if key == "store" {
                    continue;
                } else if config_param_names.contains(key) {
                    config_class_params.insert(key.clone(), value.clone());
                } else if obj.parameters.contains_key(key) {
                    constructor_params.insert(key.clone(), value.clone());
                } else {
                    config_class_params.insert(key.clone(), value.clone());
                }
            }
        }

        if !config_class_params.is_empty() && obj.parameters.contains_key("config") {
            final_config.insert(
                "config".to_string(),
                serde_json::Value::Object(config_class_params),
            );
        }

        final_config.extend(constructor_params);

        if obj.parameters.contains_key("path")
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

        if obj.parameters.contains_key("data_store") {
            let store_value = if let serde_json::Value::Object(ref yaml_map) = yaml_config {
                match yaml_map.get("store") {
                    Some(value) => value.clone(),
                    None => {
                        if let Some(explicit_path) = yaml_map.get("store_path").cloned() {
                            explicit_path
                        } else if let Some(inherited) = inherited_store_path {
                            serde_json::Value::String(inherited.to_string())
                        } else {
                            fallback_store_value(package_name, output_folder)?
                        }
                    }
                }
            } else {
                if let Some(inherited) = inherited_store_path {
                    serde_json::Value::String(inherited.to_string())
                } else {
                    fallback_store_value(package_name, output_folder)?
                }
            };

            final_config.insert("data_store".to_string(), store_value);
        }
    } else if let serde_json::Value::Object(ref yaml_map) = yaml_config {
        final_config.extend(yaml_map.clone());
    }

    serde_json::to_string(&serde_json::Value::Object(final_config))
        .map_err(|e| RunError::Config(format!("Failed to serialize final config: {}", e)))
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
