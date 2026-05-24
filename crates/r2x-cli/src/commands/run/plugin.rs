use crate::commands::run::{build_call_target, PluginCommand, RunError};
use crate::common::GlobalOpts;
use crate::help::show_plugin_help;
use crate::manifest_lookup::resolve_plugin_ref;
use crate::package_verification;
use colored::Colorize;
use r2x_logger as logger;
use r2x_manifest::runtime::build_runtime_bindings;
use r2x_manifest::types::Manifest;
use r2x_python::plugin_invoker::PluginInvocationResult;
use r2x_python::python_bridge::Bridge;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub(super) fn handle_plugin_command(cmd: PluginCommand, opts: &GlobalOpts) -> Result<(), RunError> {
    match cmd.plugin_name {
        Some(plugin_name) => {
            if cmd.show_help {
                show_plugin_help(&plugin_name)
                    .map_err(|e| RunError::Config(format!("Help error: {}", e)))?;
            } else {
                run_plugin(
                    &plugin_name,
                    &cmd.args,
                    opts,
                    cmd.repeat.get(),
                    cmd.benchmark,
                )?;
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
        let mut names: Vec<String> = pkg.plugins.iter().map(|p| p.name.to_string()).collect();
        names.sort();
        packages.insert(pkg.name.to_string(), names);
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

fn run_plugin(
    plugin_name: &str,
    args: &[String],
    opts: &GlobalOpts,
    repeat: usize,
    benchmark: bool,
) -> Result<(), RunError> {
    logger::step(&format!("Running plugin: {}", plugin_name));
    logger::debug(&format!("Received args: {:?}", args));

    let manifest = Manifest::load()?;
    let resolved = match resolve_plugin_ref(&manifest, plugin_name) {
        Ok(resolved) => resolved,
        Err(err) => {
            return Err(match err {
                crate::manifest_lookup::PluginRefError::NotFound(_) => {
                    RunError::PluginNotFound(plugin_name.to_string())
                }
                crate::manifest_lookup::PluginRefError::Ambiguous { .. } => {
                    RunError::Config(err.to_string())
                }
            })
        }
    };
    let plugin = resolved.plugin;

    package_verification::verify_and_ensure_plugin(&manifest, plugin_name)
        .map_err(|e| RunError::Verification(e.to_string()))?;

    let config_map = parse_plugin_args(args)?;
    let config_json = serde_json::to_string(&config_map)
        .map_err(|e| RunError::Config(format!("Failed to serialize config: {}", e)))?;

    let runtime_bindings = build_runtime_bindings(plugin);
    let target = build_call_target(&runtime_bindings)?;

    let bridge = Bridge::get()?;
    logger::debug(&format!("Invoking plugin with target: {}", target));

    let mut total_elapsed = Duration::ZERO;
    let mut total_python_invocation = Duration::ZERO;
    let mut total_serialization = Duration::ZERO;
    let mut timing_samples: usize = 0;
    let mut result = String::new();
    let mut timings = None;

    for iteration in 0..repeat {
        logger::set_current_plugin(Some(plugin_name.to_string()));
        let start = Instant::now();
        let invocation_result = bridge.invoke_plugin(&target, &config_json, None, Some(plugin));
        let elapsed = start.elapsed();
        total_elapsed += elapsed;
        // Clear plugin context after each execution attempt
        logger::set_current_plugin(None);

        let PluginInvocationResult {
            output,
            timings: invocation_timings,
        } = invocation_result?;

        if let Some(invocation_timings) = invocation_timings.as_ref() {
            total_python_invocation += invocation_timings.python_invocation;
            total_serialization += invocation_timings.serialization;
            timing_samples += 1;
        }

        if iteration + 1 == repeat {
            result = output;
            timings = invocation_timings;
        }
    }

    let per_run_seconds = total_elapsed.as_secs_f64() / repeat as f64;
    let duration_msg = format!(
        "(total {}, avg {})",
        crate::commands::run::format_duration(total_elapsed).dimmed(),
        crate::commands::run::format_duration(Duration::from_secs_f64(per_run_seconds)).dimmed()
    );

    let no_stdout = opts.no_stdout || logger::get_no_stdout();
    if !result.is_empty() && result != "null" {
        if opts.suppress_stdout() || no_stdout {
            logger::debug("Plugin output suppressed");
        } else {
            println!("{}", result);
        }
    }

    if logger::get_verbosity() > 0 {
        logger::success(&format!(
            "{} execution completed {}{}",
            plugin_name,
            duration_msg,
            if repeat > 1 {
                format!(" [{} runs]", repeat)
            } else {
                String::new()
            }
        ));

        if let Some(timings) = timings {
            crate::commands::run::print_plugin_timing_breakdown(&timings);
        }
    }

    if benchmark || repeat > 1 {
        let avg_total = Duration::from_secs_f64(per_run_seconds);
        eprintln!(
            "Benchmark {}: runs={} total={} avg={}",
            plugin_name,
            repeat,
            crate::commands::run::format_duration(total_elapsed),
            crate::commands::run::format_duration(avg_total)
        );
        if timing_samples > 0 {
            let avg_python = Duration::from_secs_f64(
                total_python_invocation.as_secs_f64() / timing_samples as f64,
            );
            let avg_ser =
                Duration::from_secs_f64(total_serialization.as_secs_f64() / timing_samples as f64);
            eprintln!(
                "Benchmark {} breakdown: python={} serialization={} (samples={})",
                plugin_name,
                crate::commands::run::format_duration(avg_python),
                crate::commands::run::format_duration(avg_ser),
                timing_samples
            );
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
