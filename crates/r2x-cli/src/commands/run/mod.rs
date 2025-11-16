use crate::errors::{BridgeError, ManifestError, PipelineError};
use crate::logger;
use crate::r2x_manifest;
use crate::GlobalOpts;
use clap::Parser;
use pipeline::handle_pipeline_mode;
use plugin::handle_plugin_command;
use r2x_manifest::{runtime::RuntimeBindings, PluginKind};
use r2x_python::plugin_invoker::PluginInvocationTimings;
use std::time::Duration;

mod pipeline;
mod plugin;

#[derive(Debug)]
pub enum RunError {
    Manifest(ManifestError),
    Bridge(BridgeError),
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

impl From<PipelineError> for RunError {
    fn from(e: PipelineError) -> Self {
        RunError::Pipeline(e)
    }
}

#[derive(Parser, Debug)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: Option<RunSubcommand>,
    #[arg(value_name = "YAML_PATH")]
    pub yaml_path: Option<String>,
    #[arg(value_name = "NAME")]
    pub pipeline_name: Option<String>,
    #[arg(long)]
    pub list: bool,
    #[arg(long)]
    pub print: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<String>,
}

#[derive(Parser, Debug)]
pub enum RunSubcommand {
    Plugin(PluginCommand),
}

#[derive(Parser, Debug)]
pub struct PluginCommand {
    pub plugin_name: Option<String>,
    #[arg(long)]
    pub show_help: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

pub fn handle_run(cmd: RunCommand, opts: GlobalOpts) -> Result<(), RunError> {
    match cmd.command {
        Some(RunSubcommand::Plugin(plugin_cmd)) => handle_plugin_command(plugin_cmd, &opts),
        None => {
            let yaml_path = cmd.yaml_path.unwrap_or_else(|| "pipeline.yaml".to_string());
            handle_pipeline_mode(
                yaml_path,
                cmd.pipeline_name,
                cmd.list,
                cmd.print,
                cmd.dry_run,
                cmd.output,
                &opts,
            )
        }
    }
}

pub(super) fn build_call_target(bindings: &RuntimeBindings) -> Result<String, RunError> {
    let target = match bindings.implementation_type {
        r2x_manifest::ImplementationType::Class => {
            if bindings.plugin_kind == PluginKind::Upgrader {
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
        r2x_manifest::ImplementationType::Function => {
            format!("{}:{}", bindings.entry_module, bindings.entry_name)
        }
    };

    Ok(target)
}

pub(super) fn format_duration(duration: Duration) -> String {
    let total_ms = duration.as_millis();
    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}

pub(super) fn print_plugin_timing_breakdown(timings: &PluginInvocationTimings) {
    logger::debug(&format!(
        "Plugin python invocation {}",
        format_duration(timings.python_invocation)
    ));
    logger::debug(&format!(
        "Plugin serialization {}",
        format_duration(timings.serialization)
    ));
}
