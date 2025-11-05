use crate::errors::{BridgeError, ManifestError, PipelineError};
use crate::help::{show_plugin_help, show_run_help};
use crate::logger;
use crate::package_verification;
use crate::pipeline_config::PipelineConfig;
use crate::plugin_manifest::PluginManifest;
use crate::python_bridge::Bridge;
use crate::GlobalOpts;
use clap::Parser;
use colored::Colorize;
use std::time::Instant;

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
            RunError::PluginNotFound(name) => write!(f, "Plugin '{}' not found in manifest", name),
            RunError::InvalidArgs(msg) => write!(f, "Invalid arguments: {}", msg),
            RunError::Verification(msg) => write!(f, "Package verification error: {}", msg),
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

/// Run pipelines or plugins
#[derive(Parser, Debug)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: Option<RunSubcommand>,

    /// Path to pipeline YAML file (used when no subcommand)
    #[arg(value_name = "YAML_PATH")]
    pub yaml_path: Option<String>,

    /// Pipeline name to execute (used when no subcommand)
    #[arg(value_name = "NAME")]
    pub pipeline_name: Option<String>,

    /// List available pipelines (used when no subcommand)
    #[arg(long)]
    pub list: bool,

    /// Print resolved pipeline configuration (used when no subcommand)
    #[arg(long)]
    pub print: bool,

    /// Show pipeline flow without executing (display which plugins produce/consume stdout)
    #[arg(long)]
    pub dry_run: bool,

    /// Output file for final pipeline stdout (used when no subcommand)
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<String>,
}

#[derive(Parser, Debug)]
pub enum RunSubcommand {
    /// Run a plugin directly
    Plugin(PluginCommand),
}

#[derive(Parser, Debug)]
pub struct PluginCommand {
    /// Plugin name to run (optional - if not provided, lists available plugins)
    pub plugin_name: Option<String>,

    /// Show help for the plugin
    #[arg(long)]
    pub show_help: bool,

    /// Plugin arguments as key=value pairs
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

pub fn handle_run(cmd: RunCommand, _opts: GlobalOpts) -> Result<(), RunError> {
    match cmd.command {
        Some(RunSubcommand::Plugin(plugin_cmd)) => handle_plugin_command(plugin_cmd),
        None => {
            // No subcommand - run pipeline mode
            if let Some(yaml_path) = cmd.yaml_path {
                handle_pipeline_mode(
                    yaml_path,
                    cmd.pipeline_name,
                    cmd.list,
                    cmd.print,
                    cmd.dry_run,
                    cmd.output,
                )
            } else {
                // No subcommand and no yaml_path - show help
                show_run_help().map_err(|e| RunError::Config(format!("Help error: {}", e)))?;
                Ok(())
            }
        }
    }
}

fn handle_pipeline_mode(
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

fn handle_plugin_command(cmd: PluginCommand) -> Result<(), RunError> {
    match cmd.plugin_name {
        Some(plugin_name) => {
            if cmd.show_help {
                show_plugin_help(&plugin_name)
                    .map_err(|e| RunError::Config(format!("Help error: {}", e)))?;
            } else {
                run_plugin(&plugin_name, &cmd.args)?;
            }
        }
        None => {
            // No plugin name provided - list available plugins
            list_available_plugins()?;
        }
    }
    Ok(())
}

fn list_available_plugins() -> Result<(), RunError> {
    let manifest = PluginManifest::load()?;

    if manifest.is_empty() {
        println!("No plugins installed.");
        println!();
        println!("To install a plugin, run:");
        println!("  r2x install <package>");
        return Ok(());
    }

    println!("Available plugins:");
    println!();

    // Group plugins by package name and type
    use std::collections::BTreeMap;
    let plugins = manifest.list_plugins();
    let mut packages: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();

    for (name, plugin) in &plugins {
        let package_name = plugin
            .package_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let plugin_type = if let Some(obj) = &plugin.obj {
            obj.callable_type.clone()
        } else {
            plugin
                .plugin_type
                .clone()
                .unwrap_or_else(|| "other".to_string())
        };

        packages
            .entry(package_name)
            .or_default()
            .entry(plugin_type)
            .or_default()
            .push(name.to_string());
    }

    for (package_name, types) in &packages {
        println!("{}:", package_name.bold());
        for (type_name, plugin_names) in types {
            println!("  {}:", type_name);
            for plugin_name in plugin_names {
                println!("    - {}", plugin_name);
            }
        }
        println!();
    }

    println!("Run a plugin with:");
    println!("  r2x run plugin <plugin-name> [args...]");
    println!();
    println!("Show plugin help:");
    println!("  r2x run plugin <plugin-name> --show-help");

    Ok(())
}

fn run_plugin(plugin_name: &str, args: &[String]) -> Result<(), RunError> {
    logger::step(&format!("Running plugin: {}", plugin_name));
    logger::debug(&format!("Received args: {:?}", args));

    let manifest = PluginManifest::load()?;
    let plugin = manifest
        .plugins
        .get(plugin_name)
        .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

    // Verify packages are installed before running
    logger::debug("Verifying packages...");
    package_verification::verify_and_ensure_plugin(&manifest, plugin_name)
        .map_err(|e| RunError::Verification(e.to_string()))?;
    logger::debug("Package verification complete");

    // Parse arguments into config
    let config_map = parse_plugin_args(args)?;
    logger::debug(&format!("Parsed config_map: {:?}", config_map));
    let config_json = serde_json::to_string(&config_map)
        .map_err(|e| RunError::Config(format!("Failed to serialize config: {}", e)))?;

    // Build call target
    let target = build_call_target(plugin)?;

    // Initialize bridge and invoke plugin
    let bridge = Bridge::get()?;
    logger::debug(&format!("Invoking plugin with target: {}", target));
    logger::debug(&format!("Config: {}", config_json));

    let result = bridge.invoke_plugin(&target, &config_json, None, Some(plugin))?;

    // Output result
    if !result.is_empty() && result != "null" {
        println!("{}", result);
        logger::success("Plugin execution completed");
    } else {
        logger::success("Plugin execution completed (no output)");
    }

    Ok(())
}

fn parse_plugin_args(args: &[String]) -> Result<serde_json::Value, RunError> {
    let mut config = serde_json::json!({});

    for arg in args {
        if let Some(eq_pos) = arg.find('=') {
            let key = &arg[..eq_pos];
            let value_str = &arg[eq_pos + 1..];

            // Convert hyphens to underscores for Python compatibility
            let python_key = key.replace('-', "_");

            let value = parse_json_value(value_str)?;
            config[python_key] = value;
        } else {
            return Err(RunError::InvalidArgs(format!(
                "Invalid argument format: '{}'. Expected key=value",
                arg
            )));
        }
    }

    Ok(config)
}

fn parse_json_value(value_str: &str) -> Result<serde_json::Value, RunError> {
    // Try to parse as JSON first
    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(value_str) {
        return Ok(json_val);
    }

    // Try parsing as boolean
    match value_str.to_lowercase().as_str() {
        "true" => return Ok(serde_json::json!(true)),
        "false" => return Ok(serde_json::json!(false)),
        _ => {}
    }

    // Try parsing as number
    if let Ok(num) = value_str.parse::<i64>() {
        return Ok(serde_json::json!(num));
    }

    if let Ok(num) = value_str.parse::<f64>() {
        return Ok(serde_json::json!(num));
    }

    // Default to string
    Ok(serde_json::json!(value_str))
}

fn build_call_target(plugin: &crate::plugin_manifest::Plugin) -> Result<String, RunError> {
    let obj = plugin
        .obj
        .as_ref()
        .ok_or_else(|| RunError::Config("Plugin missing callable metadata".to_string()))?;

    let target = if obj.callable_type == "class" {
        let call_method = plugin
            .call_method
            .as_ref()
            .ok_or_else(|| RunError::Config("Class plugin missing call_method".to_string()))?;
        format!("{}:{}.{}", obj.module, obj.name, call_method)
    } else {
        format!("{}:{}", obj.module, obj.name)
    };

    Ok(target)
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

    let manifest = PluginManifest::load()?;

    logger::success(&format!("Pipeline: {}", pipeline_name));
    println!();
    println!("Pipeline flow (--dry-run):");

    for (index, plugin_name) in pipeline.iter().enumerate() {
        let plugin = manifest
            .plugins
            .get(plugin_name)
            .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

        // Determine if plugin reads from stdin/stdout
        let has_obj = plugin.obj.is_some();
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

    println!();
    println!("{}  No actual execution. Use without --dry-run to run the pipeline.",
        "✔".green());

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

    let manifest = PluginManifest::load()?;
    let total_steps = pipeline.len();

    // Verify all packages in pipeline before starting
    logger::debug("Verifying packages for pipeline...");
    for plugin_name in pipeline.iter() {
        package_verification::verify_and_ensure_plugin(&manifest, plugin_name)
            .map_err(|e| RunError::Verification(e.to_string()))?;
    }
    logger::debug("All pipeline packages verified");

    let pipeline_start = Instant::now();

    eprintln!("{}", format!("Running: {}", pipeline_name).cyan().bold());

    // Track stdin/stdout chain
    let mut current_stdin: Option<String> = None;

    for (idx, plugin_name) in pipeline.iter().enumerate() {
        let step_num = idx + 1;

        // Start spinner for this step
        logger::spinner_start(&format!("  {} [{}/{}]", plugin_name, step_num, total_steps));

        let step_start = Instant::now();

        let plugin = manifest
            .plugins
            .get(plugin_name)
            .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

        // Get plugin config from YAML
        let yaml_config = if config.config.contains_key(plugin_name) {
            config.get_plugin_config_json(plugin_name)?
        } else {
            "{}".to_string()
        };

        // Build proper config structure
        let final_config_json = build_plugin_config(plugin, &yaml_config, &config.output_folder)?;

        // Determine if plugin uses stdin
        let uses_stdin = matches!(plugin.io_type.as_deref(), Some("stdin") | Some("both"));

        let stdin_json = if uses_stdin {
            current_stdin.as_deref()
        } else {
            None
        };

        let target = build_call_target(plugin)?;

        // Invoke plugin
        let bridge = Bridge::get()?;
        logger::debug(&format!("Invoking: {}", target));
        logger::debug(&format!("Config: {}", final_config_json));

        let result =
            match bridge.invoke_plugin(&target, &final_config_json, stdin_json, Some(plugin)) {
                Ok(result) => {
                    let elapsed = step_start.elapsed();
                    let elapsed_str = format_duration(elapsed);
                    logger::spinner_success(&format!(
                        "{} [{}/{}] ({})",
                        plugin_name, step_num, total_steps, elapsed_str
                    ));
                    result
                }
                Err(e) => {
                    let elapsed = step_start.elapsed();
                    let elapsed_str = format_duration(elapsed);
                    logger::spinner_error(&format!(
                        "{} [{}/{}] ({})",
                        plugin_name, step_num, total_steps, elapsed_str
                    ));
                    return Err(RunError::Bridge(e));
                }
            };

        // Determine if plugin produces stdout
        let produces_stdout = matches!(plugin.io_type.as_deref(), Some("stdout") | Some("both"));

        if produces_stdout && !result.is_empty() && result != "null" {
            logger::debug(&format!("Plugin produced output ({} bytes)", result.len()));
            current_stdin = Some(result);
        } else {
            logger::debug("Plugin produced no output or output not used");
        }
    }

    let total_elapsed = pipeline_start.elapsed();
    let total_elapsed_str = format_duration(total_elapsed);

    eprintln!(
        "{}",
        format!("Finished in: {}", total_elapsed_str).green().bold()
    );

    // Handle final stdout output
    if let Some(final_output) = current_stdin {
        if let Some(output_path) = output_file {
            // Save to file
            logger::step(&format!("Writing output to: {}", output_path));
            std::fs::write(output_path, final_output.as_bytes())
                .map_err(|e| RunError::Pipeline(PipelineError::Io(e)))?;
            logger::success(&format!("Output saved to: {}", output_path));
        } else {
            // Print to stdout
            println!("{}", final_output);
        }
    }

    Ok(())
}

fn format_duration(duration: std::time::Duration) -> String {
    let total_ms = duration.as_millis();
    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else {
        let secs = duration.as_secs_f64();
        format!("{:.2}s", secs)
    }
}

fn build_plugin_config(
    plugin: &crate::plugin_manifest::Plugin,
    yaml_config_json: &str,
    output_folder: &Option<String>,
) -> Result<String, RunError> {
    let yaml_config: serde_json::Value = serde_json::from_str(yaml_config_json)
        .map_err(|e| RunError::Config(format!("Failed to parse YAML config: {}", e)))?;

    let mut final_config = serde_json::Map::new();

    // Check if this is a class with a config parameter
    if let Some(obj) = &plugin.obj {
        if obj.callable_type == "class" {
            let mut config_class_params = serde_json::Map::new();
            let mut constructor_params = serde_json::Map::new();

            // Get config class parameter names
            let config_param_names: std::collections::HashSet<String> =
                if let Some(config_meta) = &plugin.config {
                    config_meta.parameters.keys().cloned().collect()
                } else {
                    std::collections::HashSet::new()
                };

            // Separate YAML params into config class vs constructor
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

            // If we have config class params, nest them under 'config' key
            if !config_class_params.is_empty() && obj.parameters.contains_key("config") {
                final_config.insert(
                    "config".to_string(),
                    serde_json::Value::Object(config_class_params),
                );
            }

            // Add constructor params at top level
            final_config.extend(constructor_params);

            // Handle data_store if required
            if obj.parameters.contains_key("data_store") {
                let store_value = if let serde_json::Value::Object(ref yaml_map) = yaml_config {
                    if let Some(store) = yaml_map.get("store") {
                        store.clone()
                    } else {
                        let output_folder = output_folder.as_deref().unwrap_or("/tmp/r2x-output");
                        let store_path = format!("{}/store", output_folder);
                        std::fs::create_dir_all(&store_path).map_err(|e| {
                            RunError::Config(format!("Failed to create store directory: {}", e))
                        })?;

                        serde_json::json!({
                            "path": store_path,
                            "name": plugin.package_name.as_deref().unwrap_or("default"),
                        })
                    }
                } else {
                    let output_folder = output_folder.as_deref().unwrap_or("/tmp/r2x-output");
                    let store_path = format!("{}/store", output_folder);
                    std::fs::create_dir_all(&store_path).map_err(|e| {
                        RunError::Config(format!("Failed to create store directory: {}", e))
                    })?;

                    serde_json::json!({
                        "path": store_path,
                        "name": plugin.package_name.as_deref().unwrap_or("default"),
                    })
                };

                final_config.insert("data_store".to_string(), store_value);
            }
        } else if let serde_json::Value::Object(ref yaml_map) = yaml_config {
            final_config.extend(yaml_map.clone());
        }
    } else if let serde_json::Value::Object(ref yaml_map) = yaml_config {
        final_config.extend(yaml_map.clone());
    }

    serde_json::to_string(&serde_json::Value::Object(final_config))
        .map_err(|e| RunError::Config(format!("Failed to serialize final config: {}", e)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_run_command() {
        // Integration tests for run command
    }
}
