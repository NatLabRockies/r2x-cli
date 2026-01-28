use super::RunError;
use crate::errors::PipelineError;
use crate::logger;
use crate::manifest_lookup::{resolve_plugin_ref, PluginRefError};
use crate::package_verification;
use crate::pipeline_config::PipelineConfig;
use crate::python_bridge::Bridge;
use crate::r2x_manifest::{self, Manifest};
use crate::GlobalOpts;
use colored::Colorize;
use std::time::Instant;

mod builder;
mod config;
mod constants;
mod overrides;
mod validation;

use builder::build_plugin_config;
use config::resolve_plugin_config_json;
use overrides::prepare_pipeline_overrides;
use validation::validate_pipeline_configs;

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
            PluginRefError::NotFound(_) => RunError::PluginNotFound(plugin_name.clone()),
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
    for plugin_name in pipeline {
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
            PluginRefError::NotFound(_) => RunError::PluginNotFound(plugin_name.clone()),
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
        logger::set_current_plugin(Some(plugin_name.clone()));

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
            if opts.no_stdout {
                logger::debug("Plugin produced output (suppressed by --no-stdout)");
            } else {
                logger::debug(&format!("Plugin produced output ({} bytes)", result.len()));
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
