use crate::commands::run::{build_call_target, PluginCommand, RunError};
use crate::common::GlobalOpts;
use crate::help::show_plugin_help;
use crate::manifest_lookup::resolve_plugin_ref;
use crate::package_verification;
use colored::Colorize;
use r2x_logger as logger;
use r2x_manifest::runtime::build_runtime_bindings;
use r2x_manifest::types::Manifest;
use r2x_manifest::types::Plugin;
use r2x_python::plugin_invoker::{PluginInput, PluginInvocationOutput, PluginInvocationResult};
use r2x_python::python_bridge::Bridge;
use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
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
                    cmd.input,
                    cmd.output,
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
    println!("No plugin name provided.\n");
    println!(
        "  Use {} to list installed plugins, then:",
        "r2x list".bold()
    );
    println!("  r2x run <plugin-name> [args...]");
    println!("  r2x run plugin <plugin-name> --show-help");
    println!();
    Ok(())
}

fn run_plugin(
    plugin_name: &str,
    args: &[String],
    opts: &GlobalOpts,
    input_file: Option<PathBuf>,
    output_file: Option<PathBuf>,
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

    let argument_spec = PluginArgumentSpec::from_plugin(plugin_name, plugin);
    let config_map = parse_plugin_args(args, &argument_spec)?;
    let config_json = serde_json::to_string(&config_map)
        .map_err(|e| RunError::Config(format!("Failed to serialize config: {}", e)))?;

    let runtime_bindings = build_runtime_bindings(plugin);
    let target = build_call_target(&runtime_bindings)?;

    let input = read_plugin_input(input_file.as_deref())?;
    let bridge = Bridge::get()?;
    logger::debug(&format!("Invoking plugin with target: {}", target));

    let mut total_elapsed = Duration::ZERO;
    let mut total_python_invocation = Duration::ZERO;
    let mut total_serialization = Duration::ZERO;
    let mut timing_samples: usize = 0;
    let mut result = PluginInvocationOutput::Empty;
    let mut timings = None;

    for iteration in 0..repeat {
        logger::set_current_plugin(Some(plugin_name.to_string()));
        let start = Instant::now();
        let invocation_input = input.as_ref().map(PluginInputSource::as_bridge_input);
        let invocation_output = if iteration + 1 == repeat {
            output_file.as_deref()
        } else {
            None
        };
        let invocation_result = bridge.invoke_plugin_direct(
            &target,
            &config_json,
            invocation_input,
            invocation_output,
            Some(plugin),
        );
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
    if let Some(output_path) = output_file {
        match result {
            PluginInvocationOutput::Persisted => {
                logger::success(&format!(
                    "Plugin output saved to: {}",
                    output_path.display()
                ));
            }
            PluginInvocationOutput::Json(output) => {
                logger::step(&format!(
                    "Writing plugin output to: {}",
                    output_path.display()
                ));
                write_json_output(&output_path, &output)?;
                logger::success(&format!(
                    "Plugin output saved to: {}",
                    output_path.display()
                ));
            }
            PluginInvocationOutput::Empty => {
                return Err(RunError::InvalidArgs(format!(
                    "plugin '{}' produced no JSON output; use its plugin-specific output option instead",
                    plugin_name
                )));
            }
        }
    } else if let PluginInvocationOutput::Json(output) = result {
        if !output.is_empty() && output != "null" {
            if opts.suppress_stdout() || no_stdout {
                logger::debug("Plugin output suppressed");
            } else {
                println!("{}", output);
            }
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

enum PluginInputSource {
    Stdin(String),
    File(PathBuf),
}

impl PluginInputSource {
    fn as_bridge_input(&self) -> PluginInput<'_> {
        match self {
            Self::Stdin(payload) => PluginInput::Json(payload),
            Self::File(path) => PluginInput::File(path),
        }
    }
}

fn write_json_output(output_path: &Path, output: &str) -> Result<(), RunError> {
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| RunError::Pipeline(crate::errors::PipelineError::Io(error)))?;
    }
    std::fs::write(output_path, output.as_bytes())
        .map_err(|error| RunError::Pipeline(crate::errors::PipelineError::Io(error)))
}

fn read_plugin_input(input_file: Option<&Path>) -> Result<Option<PluginInputSource>, RunError> {
    if input_file.is_some_and(|path| path == Path::new("-")) {
        return read_piped_stdin().map(|input| input.map(PluginInputSource::Stdin));
    }

    let stdin = read_piped_stdin()?;
    if let (Some(_), Some(_)) = (input_file, stdin.as_ref()) {
        return Err(RunError::InvalidArgs(
            "--input FILE cannot be combined with JSON supplied on stdin".to_string(),
        ));
    }

    if let Some(path) = input_file {
        return Ok(Some(PluginInputSource::File(path.to_path_buf())));
    }

    Ok(stdin.map(PluginInputSource::Stdin))
}

fn read_piped_stdin() -> Result<Option<String>, RunError> {
    let mut stdin = io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }

    let mut input = String::new();
    stdin.read_to_string(&mut input).map_err(|error| {
        RunError::Config(format!("Failed to read plugin input from stdin: {error}"))
    })?;

    if input.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

#[derive(Debug)]
struct PluginArgumentSpec {
    plugin_name: String,
    known_keys: BTreeSet<String>,
    required_keys: BTreeSet<String>,
}

impl PluginArgumentSpec {
    fn from_plugin(plugin_name: &str, plugin: &Plugin) -> Self {
        let mut known_keys = BTreeSet::new();
        let mut required_keys = BTreeSet::new();

        let config_is_constructed = plugin.config_class.is_some() && plugin.config_module.is_some();
        for param in &plugin.parameters {
            let key = canonical_key(param.name.as_ref());
            known_keys.insert(key.clone());
            if is_store_like_param(&key) {
                known_keys.extend([
                    "path".to_string(),
                    "store_path".to_string(),
                    "store".to_string(),
                ]);
            }
            if param.required
                && param.default.is_none()
                && !(config_is_constructed && key == "config")
                && !is_store_like_param(&key)
            {
                required_keys.insert(key);
            }
        }

        for (field_name, field) in plugin.config_schema.iter() {
            let key = canonical_key(field_name.as_ref());
            known_keys.insert(key.clone());
            if field.required && field.default.is_none() {
                required_keys.insert(key);
            }
        }

        Self {
            plugin_name: plugin_name.to_string(),
            known_keys,
            required_keys,
        }
    }

    #[cfg(test)]
    fn for_test(plugin_name: &str, known_keys: &[&str], required_keys: &[&str]) -> Self {
        Self {
            plugin_name: plugin_name.to_string(),
            known_keys: known_keys.iter().map(|key| canonical_key(key)).collect(),
            required_keys: required_keys.iter().map(|key| canonical_key(key)).collect(),
        }
    }

    fn validate_flag_key(&self, original: &str, key: &str) -> Result<(), RunError> {
        if self.known_keys.is_empty() || self.known_keys.contains(key) {
            return Ok(());
        }

        let mut message = format!("unknown option '--{}'", original);
        if let Some(suggestion) = self.suggest_flag(original) {
            message.push_str(&format!("\n\nDid you mean '--{}'?", suggestion));
        }
        message.push_str(&format!("\n\nTry:\n  {}", self.example_command()));
        Err(RunError::InvalidArgs(message))
    }

    fn validate_required_keys(&self, config: &serde_json::Value) -> Result<(), RunError> {
        if self.required_keys.is_empty() {
            return Ok(());
        }

        let missing: Vec<String> = self
            .required_keys
            .iter()
            .filter(|key| config.get(key.as_str()).is_none())
            .map(|key| format!("--{}", cli_flag_name(key)))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }

        Err(RunError::InvalidArgs(format!(
            "missing required plugin option(s): {}\n\nTry:\n  {}",
            missing.join(", "),
            self.example_command()
        )))
    }

    fn suggest_flag(&self, original: &str) -> Option<String> {
        let requested = canonical_key(original);
        self.known_keys
            .iter()
            .filter_map(|key| {
                let distance = edit_distance(&requested, key);
                if distance <= 3 {
                    Some((distance, cli_flag_name(key)))
                } else {
                    None
                }
            })
            .min_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
            .map(|(_, flag)| flag)
    }

    fn example_command(&self) -> String {
        let mut command = format!("r2x run {}", self.plugin_name);
        if self.required_keys.is_empty() {
            if let Some(key) = self.known_keys.iter().next() {
                command.push_str(&format!(" --{} <value>", cli_flag_name(key)));
            } else {
                command.push_str(" [OPTIONS]");
            }
            return command;
        }

        for key in &self.required_keys {
            command.push_str(&format!(" --{} <value>", cli_flag_name(key)));
        }
        command
    }
}

fn parse_plugin_args(
    args: &[String],
    argument_spec: &PluginArgumentSpec,
) -> Result<serde_json::Value, RunError> {
    let mut config = serde_json::json!({});
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if arg == "--set" {
            let Some(next_arg) = args.get(index + 1) else {
                return Err(RunError::InvalidArgs(
                    "--set requires a key=value argument".to_string(),
                ));
            };
            insert_key_value_arg(next_arg, &mut config, argument_spec, true)?;
            index += 2;
            continue;
        }

        if let Some(set_arg) = arg.strip_prefix("--set=") {
            insert_key_value_arg(set_arg, &mut config, argument_spec, true)?;
            index += 1;
            continue;
        }

        if let Some(flag_arg) = arg.strip_prefix("--") {
            parse_flag_arg(flag_arg, args, &mut index, &mut config, argument_spec)?;
            continue;
        }

        if arg.contains('=') {
            insert_key_value_arg(arg, &mut config, argument_spec, false)?;
            index += 1;
            continue;
        }

        return Err(RunError::InvalidArgs(format!(
            "unexpected positional argument '{}'. Use --<option> <value> or key=value",
            arg
        )));
    }

    argument_spec.validate_required_keys(&config)?;
    Ok(config)
}

fn parse_flag_arg(
    flag_arg: &str,
    args: &[String],
    index: &mut usize,
    config: &mut serde_json::Value,
    argument_spec: &PluginArgumentSpec,
) -> Result<(), RunError> {
    if flag_arg.is_empty() {
        return Err(RunError::InvalidArgs(
            "empty option '--' is not supported".to_string(),
        ));
    }

    let (raw_key, value_str) = if let Some(eq_pos) = flag_arg.find('=') {
        (&flag_arg[..eq_pos], &flag_arg[eq_pos + 1..])
    } else {
        let Some(next_arg) = args.get(*index + 1) else {
            return Err(RunError::InvalidArgs(format!(
                "option '--{}' requires a value",
                flag_arg
            )));
        };
        if next_arg.starts_with("--") {
            return Err(RunError::InvalidArgs(format!(
                "option '--{}' requires a value",
                flag_arg
            )));
        }
        (flag_arg, next_arg.as_str())
    };

    let key = canonical_key(raw_key);
    argument_spec.validate_flag_key(raw_key, &key)?;
    config[key] = parse_json_value(value_str)?;
    *index += if flag_arg.contains('=') { 1 } else { 2 };
    Ok(())
}

fn insert_key_value_arg(
    arg: &str,
    config: &mut serde_json::Value,
    argument_spec: &PluginArgumentSpec,
    validate_key: bool,
) -> Result<(), RunError> {
    let Some(eq_pos) = arg.find('=') else {
        return Err(RunError::InvalidArgs(format!(
            "Invalid argument format: '{}'. Expected key=value",
            arg
        )));
    };

    let raw_key = &arg[..eq_pos];
    if raw_key.is_empty() {
        return Err(RunError::InvalidArgs(format!(
            "Invalid argument format: '{}'. Expected key=value",
            arg
        )));
    }

    let key = canonical_key(raw_key);
    if validate_key {
        argument_spec.validate_flag_key(raw_key, &key)?;
    }
    config[key] = parse_json_value(&arg[eq_pos + 1..])?;
    Ok(())
}

fn canonical_key(key: &str) -> String {
    key.trim_start_matches('-').replace('-', "_")
}

fn cli_flag_name(key: &str) -> String {
    key.replace('_', "-")
}

fn is_store_like_param(key: &str) -> bool {
    matches!(key, "store" | "data_store")
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
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

#[cfg(test)]
mod tests {
    use crate::commands::run::plugin::{parse_plugin_args, PluginArgumentSpec};

    fn spec() -> PluginArgumentSpec {
        PluginArgumentSpec::for_test(
            "r2x-reeds.reeds-parser",
            &["path", "weather_year", "solve_year", "enabled", "years"],
            &["path", "weather_year", "solve_year"],
        )
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_plugin_flags_and_key_value_forms_to_same_config(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let expected = serde_json::json!({
            "path": "/runs/reeds",
            "weather_year": 2012,
            "solve_year": 2025,
        });

        for candidate in [
            args(&[
                "--path",
                "/runs/reeds",
                "--weather-year",
                "2012",
                "--solve-year",
                "2025",
            ]),
            args(&[
                "--path=/runs/reeds",
                "--weather-year=2012",
                "--solve-year=2025",
            ]),
            args(&[
                "--path",
                "/runs/reeds",
                "--weather_year",
                "2012",
                "--solve_year",
                "2025",
            ]),
            args(&["path=/runs/reeds", "weather_year=2012", "solve_year=2025"]),
            args(&[
                "--set",
                "path=/runs/reeds",
                "--set",
                "weather_year=2012",
                "--set",
                "solve_year=2025",
            ]),
        ] {
            let parsed = parse_plugin_args(&candidate, &spec())?;
            assert_eq!(parsed, expected);
        }
        Ok(())
    }

    #[test]
    fn preserves_jsonish_value_parsing() -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_plugin_args(
            &args(&[
                "path=/runs/reeds",
                "weather_year=2012",
                "solve_year=2025",
                "enabled=true",
                "years=[2012,2025]",
            ]),
            &spec(),
        )?;

        assert_eq!(parsed["weather_year"], serde_json::json!(2012));
        assert_eq!(parsed["enabled"], serde_json::json!(true));
        assert_eq!(parsed["years"], serde_json::json!([2012, 2025]));
        Ok(())
    }

    #[test]
    fn reports_unknown_flag_with_suggestion() -> Result<(), Box<dyn std::error::Error>> {
        let err = match parse_plugin_args(
            &args(&[
                "--path",
                "/runs/reeds",
                "--weathear_year",
                "2012",
                "--solve-year",
                "2025",
            ]),
            &spec(),
        ) {
            Ok(_) => return Err("typo should be rejected".into()),
            Err(err) => err,
        };
        let message = err.to_string();

        assert!(message.contains("unknown option '--weathear_year'"));
        assert!(message.contains("Did you mean '--weather-year'?"));
        assert!(message.contains("Try:"));
        Ok(())
    }

    #[test]
    fn reports_missing_required_options() -> Result<(), Box<dyn std::error::Error>> {
        let err = match parse_plugin_args(&args(&["--weather-year", "2012"]), &spec()) {
            Ok(_) => return Err("missing required args should be rejected".into()),
            Err(err) => err,
        };
        let message = err.to_string();

        assert!(message.contains("missing required plugin option(s)"));
        assert!(message.contains("--path"));
        assert!(message.contains("--solve-year"));
        Ok(())
    }
}
