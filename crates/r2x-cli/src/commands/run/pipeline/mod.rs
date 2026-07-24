use crate::commands::run::RunError;
use crate::common::GlobalOpts;
use crate::errors::PipelineError;
use crate::manifest_lookup::{resolve_plugin_ref, PluginRefError};
use crate::package_verification;
use crate::pipeline_config::PipelineConfig;
use colored::Colorize;
use r2x_artifacts::pipeline_artifact::{write_bundle_output, PipelineArtifactWorkspace};
use r2x_logger as logger;
use r2x_manifest::runtime::build_runtime_bindings;
use r2x_manifest::types::Manifest;
use r2x_python::plugin_invoker::{
    ArtifactBundle, ArtifactOutputKind, PluginArtifactInvocationResult, PluginInvocationResult,
    PluginInvocationTimings,
};
use r2x_python::python_bridge::Bridge;
use std::path::Path;
use std::time::Instant;

mod builder;
mod config;
mod constants;
mod overrides;
mod validation;

use builder::build_plugin_config;
use config::resolve_plugin_config_json;
use overrides::{prepare_pipeline_artifact_overrides, prepare_pipeline_overrides};
use validation::validate_pipeline_configs;

enum PipelinePayload {
    Inline(String),
    Artifact(ArtifactBundle),
}

struct PipelineExecution {
    _workspace: PipelineArtifactWorkspace,
    final_payload: Option<PipelinePayload>,
}

enum PipelineInvocation {
    Inline(PluginInvocationResult),
    Artifact(PluginArtifactInvocationResult),
}

impl PipelineInvocation {
    fn timings(&self) -> Option<&PluginInvocationTimings> {
        match self {
            Self::Inline(result) => result.timings.as_ref(),
            Self::Artifact(result) => result.timings.as_ref(),
        }
    }
}

pub(super) struct PipelineModeOptions {
    pub list: bool,
    pub print: bool,
    pub dry_run: bool,
    pub output: Option<String>,
    pub zip_output: bool,
}

pub(super) fn handle_pipeline_mode(
    yaml_path: String,
    pipeline_name: Option<String>,
    options: PipelineModeOptions,
    opts: &GlobalOpts,
) -> Result<(), RunError> {
    if options.zip_output && (options.list || options.print || options.dry_run) {
        return Err(RunError::InvalidArgs(
            "--zip can only be used while executing a pipeline".to_string(),
        ));
    }

    let config = PipelineConfig::load(&yaml_path)?;

    if options.list {
        list_pipelines(&config);
    } else if options.print {
        if let Some(name) = pipeline_name {
            print_pipeline_config(&config, &name)?;
        } else {
            return Err(RunError::InvalidArgs(
                "Pipeline name required with --print. Use --list to see available pipelines."
                    .to_string(),
            ));
        }
    } else if let Some(name) = pipeline_name {
        if options.dry_run {
            show_pipeline_flow(&config, &name)?;
        } else {
            run_pipeline(
                &config,
                &name,
                options.output.as_deref(),
                options.zip_output,
                opts,
            )?;
        }
    } else {
        return Err(RunError::InvalidArgs(
            "Pipeline name required. Use --list to see available pipelines.".to_string(),
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
            PluginRefError::Ambiguous { .. } => RunError::Config(err.to_string()),
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
    zip_output: bool,
    opts: &GlobalOpts,
) -> Result<(), RunError> {
    let execution = execute_pipeline(config, pipeline_name, opts)?;
    write_pipeline_output(
        execution.final_payload.as_ref(),
        output_file,
        zip_output,
        opts,
    )
}

fn execute_pipeline(
    config: &PipelineConfig,
    pipeline_name: &str,
    opts: &GlobalOpts,
) -> Result<PipelineExecution, RunError> {
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

    let artifact_workspace = PipelineArtifactWorkspace::create()?;
    let mut current_payload = None;

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
            PluginRefError::Ambiguous { .. } => RunError::Config(err.to_string()),
        })?;
        let pkg = resolved.package;
        let plugin = resolved.plugin;

        let bindings = build_runtime_bindings(plugin);

        let yaml_config = resolve_plugin_config_json(config, plugin_name, &resolved)?;

        if let Ok(serde_json::Value::Object(map)) =
            serde_json::from_str::<serde_json::Value>(&yaml_config)
        {
            if let Some(store_path) = map.get("store_path").and_then(|value| value.as_str()) {
                current_store_path = Some(store_path.to_string());
            }
        }

        let pipeline_overrides = match current_payload.as_ref() {
            Some(PipelinePayload::Inline(input)) => {
                prepare_pipeline_overrides(Some(input), &bindings, plugin_name)?
            }
            Some(PipelinePayload::Artifact(input)) => {
                prepare_pipeline_artifact_overrides(Some(input), &bindings, plugin_name)
            }
            None => None,
        };

        let final_config_json = build_plugin_config(
            &bindings,
            &pkg.name,
            &yaml_config,
            resolved_output_folder.as_deref(),
            current_store_path.as_deref(),
            pipeline_overrides.as_deref(),
        )?;

        let target = crate::commands::run::build_call_target(&bindings)?;
        let bridge = Bridge::get()?;
        logger::debug(&format!("Invoking: {}", target));

        // Set current plugin context for logging
        logger::set_current_plugin(Some(plugin_name.clone()));

        let output_artifact = artifact_workspace.step_bundle(idx)?;
        let upgraded_artifact = match current_payload.as_ref() {
            Some(PipelinePayload::Artifact(input))
                if bindings.role == r2x_manifest::runtime::PluginRole::Upgrader =>
            {
                Some(input.clone())
            }
            _ => None,
        };
        let invocation = match current_payload.as_ref() {
            Some(PipelinePayload::Inline(input)) => bridge
                .invoke_plugin_with_bindings(
                    &target,
                    &final_config_json,
                    Some(input),
                    Some(&bindings),
                )
                .map(PipelineInvocation::Inline),
            Some(PipelinePayload::Artifact(_))
                if bindings.role == r2x_manifest::runtime::PluginRole::Upgrader =>
            {
                bridge
                    .invoke_plugin_with_bindings(&target, &final_config_json, None, Some(&bindings))
                    .map(PipelineInvocation::Inline)
            }
            Some(PipelinePayload::Artifact(input)) => bridge
                .invoke_plugin_with_artifact_bindings(
                    &target,
                    &final_config_json,
                    Some(input),
                    &output_artifact,
                    Some(&bindings),
                )
                .map(PipelineInvocation::Artifact),
            None if bindings.role == r2x_manifest::runtime::PluginRole::Upgrader => bridge
                .invoke_plugin_with_bindings(&target, &final_config_json, None, Some(&bindings))
                .map(PipelineInvocation::Inline),
            None => bridge
                .invoke_plugin_with_artifact_bindings(
                    &target,
                    &final_config_json,
                    None,
                    &output_artifact,
                    Some(&bindings),
                )
                .map(PipelineInvocation::Artifact),
        };

        let invocation_result = match invocation {
            Ok(invocation_result) => {
                let elapsed = step_start.elapsed();
                logger::spinner_success(&format!(
                    "{} [{}/{}] ({})",
                    plugin_name,
                    step_num,
                    total_steps,
                    crate::commands::run::format_duration(elapsed)
                ));
                if logger::get_verbosity() > 0 {
                    if let Some(timings) = invocation_result.timings() {
                        crate::commands::run::print_plugin_timing_breakdown(timings);
                    }
                }
                invocation_result
            }
            Err(e) => {
                let elapsed = step_start.elapsed();
                logger::spinner_error(&format!(
                    "{} [{}/{}] ({})",
                    plugin_name,
                    step_num,
                    total_steps,
                    crate::commands::run::format_duration(elapsed)
                ));
                // Clear plugin context before returning error
                logger::set_current_plugin(None);
                return Err(RunError::Bridge(e));
            }
        };

        // Clear plugin context after execution
        logger::set_current_plugin(None);

        let no_stdout = opts.no_stdout || logger::get_no_stdout();
        match (invocation_result, upgraded_artifact) {
            (PipelineInvocation::Inline(result), Some(artifact)) => {
                if !result.output.is_empty() && result.output != "null" {
                    std::fs::write(artifact.entrypoint_path(), result.output.as_bytes())
                        .map_err(PipelineError::Io)?;
                }
                logger::debug("Upgrader preserved the current artifact bundle");
                current_payload = Some(PipelinePayload::Artifact(artifact));
            }
            (PipelineInvocation::Inline(result), None)
                if !result.output.is_empty() && result.output != "null" =>
            {
                if no_stdout {
                    logger::debug("Plugin produced output (suppressed by --no-stdout)");
                } else {
                    logger::debug(&format!(
                        "Plugin produced output ({} bytes)",
                        result.output.len()
                    ));
                }
                current_payload = Some(PipelinePayload::Inline(result.output));
            }
            (PipelineInvocation::Artifact(result), _)
                if result.output_kind != ArtifactOutputKind::Empty =>
            {
                logger::debug(&format!(
                    "Plugin produced {:?} artifact at {}",
                    result.output_kind,
                    output_artifact.entrypoint_path().display()
                ));
                current_payload = Some(PipelinePayload::Artifact(output_artifact));
            }
            (PipelineInvocation::Inline(_), None) | (PipelineInvocation::Artifact(_), _) => {
                logger::debug("Plugin produced no output or output not used");
            }
        }
    }

    eprintln!(
        "{}",
        format!(
            "Finished in: {}",
            crate::commands::run::format_duration(pipeline_start.elapsed())
        )
        .green()
        .bold()
    );

    Ok(PipelineExecution {
        _workspace: artifact_workspace,
        final_payload: current_payload,
    })
}

fn write_pipeline_output(
    final_output: Option<&PipelinePayload>,
    output_file: Option<&str>,
    zip_output: bool,
    opts: &GlobalOpts,
) -> Result<(), RunError> {
    if zip_output && output_file.is_none() {
        return Err(RunError::InvalidArgs(
            "--zip requires --output <FILE>.zip".to_string(),
        ));
    }
    if zip_output && final_output.is_none() {
        return Err(RunError::InvalidArgs(
            "--zip requested, but the pipeline produced no output".to_string(),
        ));
    }

    if let Some(final_output) = final_output {
        let no_stdout = opts.no_stdout || logger::get_no_stdout();
        match final_output {
            PipelinePayload::Inline(final_output) => {
                if zip_output {
                    return Err(RunError::InvalidArgs(
                        "--zip is only supported for System pipeline outputs".to_string(),
                    ));
                }
                if let Some(output_path) = output_file {
                    logger::step(&format!("Writing output to: {}", output_path));
                    std::fs::write(output_path, final_output.as_bytes())
                        .map_err(|e| RunError::Pipeline(PipelineError::Io(e)))?;
                    logger::success(&format!("Output saved to: {}", output_path));
                } else if opts.suppress_stdout() || no_stdout {
                    logger::debug("Pipeline output suppressed");
                } else {
                    println!("{}", final_output);
                }
            }
            PipelinePayload::Artifact(final_output) => {
                if let Some(output_path) = output_file {
                    logger::step(&format!("Writing output bundle to: {}", output_path));
                    write_bundle_output(
                        final_output,
                        Some(Path::new(output_path)),
                        zip_output,
                        opts.suppress_stdout() || no_stdout,
                    )?;
                    if zip_output {
                        logger::success(&format!("System ZIP archive saved to: {}", output_path));
                    } else {
                        logger::success(&format!("Output bundle saved to: {}", output_path));
                    }
                } else {
                    write_bundle_output(
                        final_output,
                        None,
                        false,
                        opts.suppress_stdout() || no_stdout,
                    )?;
                }
            }
        }
    }

    Ok(())
}
