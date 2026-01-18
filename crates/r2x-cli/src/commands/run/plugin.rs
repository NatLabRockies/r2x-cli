use super::{PluginCommand, RunError};
use crate::help::show_plugin_help;
use crate::logger;
use crate::package_verification;
use crate::python_bridge::Bridge;
use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;
use colored::Colorize;
use r2x_python::plugin_invoker::PluginInvocationResult;
use std::collections::BTreeMap;
use std::time::Instant;

pub(super) fn handle_plugin_command(cmd: PluginCommand, opts: &GlobalOpts) -> Result<(), RunError> {
    match cmd.plugin_name {
        Some(plugin_name) => {
            if cmd.show_help {
                show_plugin_help(&plugin_name)
                    .map_err(|e| RunError::Config(format!("Help error: {}", e)))?;
            } else {
                run_plugin(&plugin_name, &cmd.args, opts)?;
            }
        }
        None => {
            list_available_plugins()?;
        }
    }
    Ok(())
}

fn list_available_plugins() -> Result<(), RunError> {
    let manifest = Manifest::load()?;

    if manifest.is_empty() {
        println!("No plugins installed.\n");
        println!("To install a plugin, run:\n  r2x install <package>");
        return Ok(());
    }

    println!("Available plugins:\n");
    let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in &manifest.packages {
        let mut names: Vec<String> = pkg.plugins.iter().map(|p| p.name.clone()).collect();
        names.sort();
        packages.insert(pkg.name.clone(), names);
    }

    for (idx, (package_name, plugin_names)) in packages.iter().enumerate() {
        if idx > 0 {
            println!();
        }
        println!("{}:", package_name.bold());
        for plugin_name in plugin_names {
            println!("  - {}", plugin_name);
        }
    }

    println!("Run a plugin with:\n  r2x run plugin <plugin-name> [args...]\n");
    println!("Show plugin help:\n  r2x run plugin <plugin-name> --show-help");

    Ok(())
}

fn run_plugin(plugin_name: &str, args: &[String], opts: &GlobalOpts) -> Result<(), RunError> {
    logger::step(&format!("Running plugin: {}", plugin_name));
    logger::debug(&format!("Received args: {:?}", args));

    let manifest = Manifest::load()?;
    let (_pkg, plugin) = manifest
        .packages
        .iter()
        .find_map(|pkg| {
            pkg.plugins
                .iter()
                .find(|p| p.name == plugin_name)
                .map(|p| (pkg, p))
        })
        .ok_or_else(|| RunError::PluginNotFound(plugin_name.to_string()))?;

    let bindings = r2x_manifest::build_runtime_bindings(plugin);

    package_verification::verify_and_ensure_plugin(&manifest, plugin_name)
        .map_err(|e| RunError::Verification(e.to_string()))?;

    let config_map = parse_plugin_args(args)?;
    let config_json = serde_json::to_string(&config_map)
        .map_err(|e| RunError::Config(format!("Failed to serialize config: {}", e)))?;

    let target = super::build_call_target(&bindings)?;

    let bridge = Bridge::get()?;
    logger::debug(&format!("Invoking plugin with target: {}", target));
    logger::debug(&format!("Config: {}", config_json));

    // Set current plugin context for logging
    logger::set_current_plugin(Some(plugin_name.to_string()));

    // Reconfigure Python logging with plugin name
    if let Err(e) = Bridge::reconfigure_logging_for_plugin(plugin_name) {
        logger::warn(&format!(
            "Failed to reconfigure Python logging for plugin {}: {}",
            plugin_name, e
        ));
    }

    let start = Instant::now();
    let invocation_result = bridge.invoke_plugin(&target, &config_json, None, Some(plugin))?;
    let PluginInvocationResult {
        output: result,
        timings,
    } = invocation_result;
    let elapsed = start.elapsed();
    let duration_msg = format!("({})", super::format_duration(elapsed).dimmed());

    // Clear plugin context after execution
    logger::set_current_plugin(None);

    if !result.is_empty() && result != "null" {
        if opts.suppress_stdout() || opts.no_stdout {
            logger::debug("Plugin output suppressed");
        } else {
            println!("{}", result);
        }
    }

    if logger::get_verbosity() > 0 {
        logger::success(&format!(
            "{} execution completed {}",
            plugin_name, duration_msg
        ));

        if let Some(timings) = timings {
            super::print_plugin_timing_breakdown(&timings);
        }
    }

    Ok(())
}

fn parse_plugin_args(args: &[String]) -> Result<serde_json::Value, RunError> {
    let mut config = serde_json::json!({});

    for arg in args {
        if let Some(eq_pos) = arg.find('=') {
            let key = &arg[..eq_pos];
            let value_str = &arg[eq_pos + 1..];
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
    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(value_str) {
        return Ok(json_val);
    }

    match value_str.to_lowercase().as_str() {
        "true" => return Ok(serde_json::json!(true)),
        "false" => return Ok(serde_json::json!(false)),
        _ => {}
    }

    if let Ok(num) = value_str.parse::<i64>() {
        return Ok(serde_json::json!(num));
    }

    if let Ok(num) = value_str.parse::<f64>() {
        return Ok(serde_json::json!(num));
    }

    Ok(serde_json::json!(value_str))
}
