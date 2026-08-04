use crate::common::GlobalOpts;
use crate::errors::PipelineError;
use crate::help;
use crate::manifest_lookup::resolve_plugin_ref;
use clap::Parser;
use pipeline::{handle_pipeline_mode, PipelineModeOptions};
use plugin::handle_plugin_command;
use r2x_artifacts::ArtifactError;
use r2x_logger as logger;
use r2x_manifest::errors::ManifestError;
use r2x_manifest::runtime::{PluginRole, RuntimeBindings};
use r2x_manifest::types::{Manifest, PluginType};
use r2x_python::errors::BridgeError;
use r2x_python::plugin_invoker::PluginInvocationTimings;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod pipeline;
mod plugin;

#[derive(Debug)]
pub enum RunError {
    Manifest(ManifestError),
    Bridge(BridgeError),
    Artifact(ArtifactError),
    Pipeline(PipelineError),
    Config(String),
    PluginNotFound(String),
    InvalidArgs(String),
    Verification(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Manifest(e) => write!(f, "Manifest error: {}", e),
            RunError::Bridge(e) => write!(f, "Python bridge error: {}", e),
            RunError::Artifact(e) => write!(f, "Artifact error: {}", e),
            RunError::Pipeline(e) => write!(f, "Pipeline error: {}", e),
            RunError::Config(msg) => write!(f, "Configuration error: {}", msg),
            RunError::PluginNotFound(name) => {
                write!(f, "Plugin '{}' not found in manifest", name)
            }
            RunError::InvalidArgs(msg) => write!(f, "Invalid arguments: {}", msg),
            RunError::Verification(msg) => {
                write!(f, "Package verification error: {}", msg)
            }
        }
    }
}

impl std::error::Error for RunError {}

impl From<ManifestError> for RunError {
    fn from(e: ManifestError) -> Self {
        RunError::Manifest(e)
    }
}

impl From<BridgeError> for RunError {
    fn from(e: BridgeError) -> Self {
        RunError::Bridge(e)
    }
}

impl From<ArtifactError> for RunError {
    fn from(e: ArtifactError) -> Self {
        RunError::Artifact(e)
    }
}

impl From<PipelineError> for RunError {
    fn from(e: PipelineError) -> Self {
        RunError::Pipeline(e)
    }
}

#[derive(Parser, Debug)]
#[command(
    after_help = "Modes:\n  r2x run <plugin-ref> [PLUGIN_OPTIONS...]\n  r2x run <pipeline.yaml> <pipeline-name> [--list|--print|--dry-run]\n\nUse `r2x run plugin <plugin-ref>` to force legacy direct-plugin mode when a pipeline name and plugin name conflict."
)]
pub struct RunCommand {
    #[command(subcommand)]
    command: Option<RunSubcommand>,
    #[arg(
        value_name = "TARGET",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    args: Vec<String>,
}

#[derive(Parser, Debug)]
struct PipelineCommand {
    #[arg(value_name = "YAML_PATH")]
    yaml_path: Option<String>,
    #[arg(value_name = "NAME")]
    pipeline_name: Option<String>,
    #[arg(long)]
    list: bool,
    #[arg(long)]
    print: bool,
    #[arg(short = 'n', long)]
    dry_run: bool,
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<String>,
    /// Save a system pipeline output as an infrasys ZIP archive
    #[arg(long)]
    zip: bool,
}

#[derive(Parser, Debug)]
pub enum RunSubcommand {
    Plugin(PluginCommand),
}

#[derive(Parser, Debug)]
pub struct PluginCommand {
    plugin_name: Option<String>,
    #[arg(long)]
    show_help: bool,
    #[arg(
        short = 'i',
        long,
        value_name = "FILE",
        help = "Read plugin JSON input from FILE instead of stdin"
    )]
    input: Option<PathBuf>,
    #[arg(
        short = 'o',
        long,
        value_name = "FILE",
        help = "Write plugin output to FILE instead of stdout"
    )]
    output: Option<PathBuf>,
    #[arg(
        long,
        value_name = "N",
        default_value = "1",
        help = "Repeat plugin invocation N times"
    )]
    repeat: NonZeroUsize,
    #[arg(
        long,
        help = "Print benchmark summary (also implied when --repeat > 1)"
    )]
    benchmark: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

pub fn handle_run(cmd: RunCommand, opts: GlobalOpts) -> Result<(), RunError> {
    match cmd.command {
        Some(RunSubcommand::Plugin(plugin_cmd)) => handle_plugin_command(plugin_cmd, &opts),
        None => {
            if cmd.args.is_empty() {
                if let Err(error) = help::show_run_help() {
                    logger::error(&error);
                    std::process::exit(1);
                }
                return Ok(());
            }
            if is_direct_plugin_target(&cmd.args) {
                let plugin_cmd = parse_direct_plugin_command(&cmd.args)?;
                handle_plugin_command(plugin_cmd, &opts)
            } else {
                handle_pipeline_command(&cmd.args, &opts)
            }
        }
    }
}

fn is_direct_plugin_target(args: &[String]) -> bool {
    let Some(target) = args.first() else {
        return false;
    };
    if target.starts_with('-') || is_pipeline_path(target) {
        return false;
    }
    if target.contains('.') {
        return true;
    }

    let Ok(manifest) = Manifest::load() else {
        return false;
    };
    resolve_plugin_ref(&manifest, target).is_ok()
}

fn is_pipeline_path(target: &str) -> bool {
    let path = Path::new(target);
    path.exists()
        || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yaml" | "yml")
        )
}

fn parse_direct_plugin_command(args: &[String]) -> Result<PluginCommand, RunError> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("r2x run".to_string());
    argv.extend(args.iter().cloned());
    PluginCommand::try_parse_from(argv).map_err(|error| RunError::InvalidArgs(error.to_string()))
}

fn handle_pipeline_command(args: &[String], opts: &GlobalOpts) -> Result<(), RunError> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("r2x run".to_string());
    argv.extend(args.iter().cloned());
    let pipeline_cmd = PipelineCommand::try_parse_from(argv)
        .map_err(|error| RunError::InvalidArgs(error.to_string()))?;

    let yaml_path = pipeline_cmd
        .yaml_path
        .unwrap_or_else(|| "pipeline.yaml".to_string());
    handle_pipeline_mode(
        yaml_path,
        pipeline_cmd.pipeline_name,
        PipelineModeOptions {
            list: pipeline_cmd.list,
            print: pipeline_cmd.print,
            dry_run: pipeline_cmd.dry_run,
            output: pipeline_cmd.output,
            zip_output: pipeline_cmd.zip,
        },
        opts,
    )
}

pub(super) fn build_call_target(bindings: &RuntimeBindings) -> Result<String, RunError> {
    let target = match bindings.plugin_type {
        PluginType::Class => {
            // Upgrader plugins have their own invoker that already calls .run() internally,
            // so we don't append the call_method to the target string for them.
            if bindings.role == PluginRole::Upgrader {
                format!("{}:{}", bindings.entry_module, bindings.entry_name)
            } else if let Some(call_method) = &bindings.call_method {
                format!(
                    "{}:{}.{}",
                    bindings.entry_module, bindings.entry_name, call_method
                )
            } else {
                format!("{}:{}", bindings.entry_module, bindings.entry_name)
            }
        }
        PluginType::Function => {
            format!("{}:{}", bindings.entry_module, bindings.entry_name)
        }
    };

    Ok(target)
}

fn format_duration(duration: Duration) -> String {
    let total_ms = duration.as_millis();
    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}

fn print_plugin_timing_breakdown(timings: &PluginInvocationTimings) {
    logger::debug(&format!(
        "Plugin python invocation {}",
        format_duration(timings.python_invocation)
    ));
    logger::debug(&format!(
        "Plugin serialization {}",
        format_duration(timings.serialization)
    ));
}
