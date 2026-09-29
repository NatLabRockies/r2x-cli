use crate::manifest_lookup::resolve_plugin_ref;
use colored::Colorize;
use r2x_manifest::types::{Manifest, Plugin};
use std::collections::HashMap;

/// Show help for the run command when invoked with no arguments
pub(crate) fn show_run_help() -> Result<(), String> {
    let manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;

    println!();
    println!("{}", "No pipeline or plugin specified.".bold());
    println!();

    // Show installed plugins
    if manifest.is_empty() {
        println!("{}", "No plugins installed.".yellow());
        println!("Install plugins with: r2x install <package>");
        println!();
    } else {
        println!("{}", "Installed plugins:".bold());
        for pkg in &manifest.packages {
            for plugin in &pkg.plugins {
                let plugin_type = format!("{:?}", plugin.plugin_type);
                println!(
                    "  {} {} - from package {}",
                    plugin.name.as_ref().cyan(),
                    format!("({})", plugin_type).dimmed(),
                    pkg.name.as_ref().dimmed()
                );
            }
        }
        println!();
    }

    // Show usage hints
    println!("{}", "Usage:".bold());
    println!("  Run a pipeline:");
    println!("    r2x run <pipeline.yaml> [pipeline-name]");
    println!();
    println!("  Run a plugin directly:");
    println!("    r2x run <plugin-name> [OPTIONS]");
    println!("      (use -i/--input for a durable System, -o/--output to persist one)");
    println!("      (use --pdb for interactive post-mortem debugging; prefer --input FILE)");
    println!("      (use `r2x run plugin <plugin-name>` for the legacy explicit form)");
    println!();
    println!("  Get plugin help:");
    println!("    r2x run <plugin-name> --help");
    println!();
    println!("  Debug a failing pipeline step:");
    println!("    r2x run <pipeline.yaml> <pipeline-name> --pdb");
    println!();
    println!("  List pipelines in YAML:");
    println!("    r2x run <pipeline.yaml> --list");
    println!();
    println!("  Print resolved pipeline config:");
    println!("    r2x run <pipeline.yaml> --print <pipeline-name>");
    println!();

    Ok(())
}

/// Show help for a specific plugin using the direct-run CLI syntax.
pub(crate) fn show_plugin_help(plugin_name: &str) -> Result<(), String> {
    let manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;
    let resolved = resolve_plugin_ref(&manifest, plugin_name).map_err(|e| e.to_string())?;

    print!("{}", render_plugin_help(plugin_name, resolved.plugin));
    Ok(())
}

struct PluginHelpOption {
    key: String,
    value_name: &'static str,
    required: bool,
    description: Option<String>,
}

fn render_plugin_help(plugin_name: &str, plugin: &Plugin) -> String {
    let options = plugin_help_options(plugin);
    let mut output = format!(
        "Run the {plugin_name} plugin\n\nUsage: r2x run {plugin_name} [OPTIONS]\n\nPlugin Options:\n"
    );

    if options.is_empty() {
        output.push_str("      (none)\n");
    }
    for option in &options {
        let required = if option.required { " [required]" } else { "" };
        output.push_str(&format!(
            "      --{} <{}>{required}\n",
            cli_flag_name(&option.key),
            option.value_name
        ));
        if let Some(description) = option.description.as_deref() {
            let description = description.trim();
            if !description.is_empty() {
                output.push_str(&format!("          {description}\n"));
            }
        }
    }

    if options.iter().any(|option| option.key.contains('_')) {
        output.push_str("\nPlugin option names accept kebab-case and snake_case spellings.\n");
    }

    output.push_str("\nGlobal Options:\n");
    for (name, description) in [
        (
            "-q, --quiet...",
            "Decrease verbosity. Repeat to suppress plugin stdout.",
        ),
        (
            "-v, --verbose...",
            "Increase verbosity. Repeat for trace output.",
        ),
        ("--log-python", "Show Python logs on the console."),
        ("--no-stdout", "Disable logging plugin stdout to file."),
        (
            "-i, --input <FILE>",
            "Read plugin input from FILE instead of stdin.",
        ),
        (
            "-o, --output <FILE>",
            "Write plugin output to FILE instead of stdout.",
        ),
        ("--repeat <N>", "Invoke the plugin N times. [default: 1]"),
        ("--benchmark", "Print a benchmark summary."),
        (
            "--pdb",
            "Enter Python post-mortem debugging after a plugin failure.",
        ),
        ("-h, --help", "Display help for this command."),
    ] {
        output.push_str(&format!("  {name}\n          {description}\n"));
    }

    output
}

fn plugin_help_options(plugin: &Plugin) -> Vec<PluginHelpOption> {
    let mut options = Vec::new();
    let mut indexes = HashMap::new();

    for param in &plugin.parameters {
        let key = canonical_option_key(param.name.as_ref());
        let type_name = param.format_types();
        let value_name = option_value_name(&key, &type_name);
        let required = required_param_is_user_supplied(
            plugin,
            &key,
            param.required && param.default.is_none(),
        );
        add_plugin_help_option(
            &mut options,
            &mut indexes,
            key,
            value_name,
            required,
            param.description.as_deref().map(str::to_owned),
        );
    }

    let mut fields: Vec<_> = plugin.config_schema.iter().collect();
    fields.sort_by_key(|(left, _)| *left);
    for (field_name, field) in fields {
        let key = canonical_option_key(field_name.as_ref());
        let type_name = format!("{:?}", field.field_type);
        let value_name = option_value_name(&key, &type_name);
        add_plugin_help_option(
            &mut options,
            &mut indexes,
            key,
            value_name,
            field.required && field.default.is_none(),
            None,
        );
    }

    options
}

fn add_plugin_help_option(
    options: &mut Vec<PluginHelpOption>,
    indexes: &mut HashMap<String, usize>,
    key: String,
    value_name: &'static str,
    required: bool,
    description: Option<String>,
) {
    if let Some(index) = indexes.get(&key).copied() {
        if let Some(option) = options.get_mut(index) {
            option.required |= required;
            if option.description.is_none() {
                option.description = description;
            }
        }
        return;
    }

    indexes.insert(key.clone(), options.len());
    options.push(PluginHelpOption {
        key,
        value_name,
        required,
        description,
    });
}

fn option_value_name(name: &str, type_name: &str) -> &'static str {
    let name = canonical_option_key(name).to_ascii_lowercase();
    if name.contains("directory") || name.ends_with("_dir") {
        return "DIR";
    }
    if name.contains("path") {
        return "PATH";
    }
    if name.ends_with("_year") {
        return "YEAR";
    }
    if name.ends_with("_name") || name == "template" {
        return "NAME";
    }
    if name.ends_with("_config") {
        return "CONFIG";
    }

    let type_name = type_name.to_ascii_lowercase();
    if type_name.contains("directory") {
        "DIR"
    } else if type_name.contains("path") {
        "PATH"
    } else if type_name.contains("bool") {
        "BOOL"
    } else if type_name.contains("int") {
        "INT"
    } else if type_name.contains("float") {
        "NUMBER"
    } else {
        "VALUE"
    }
}

fn required_param_is_user_supplied(
    plugin: &Plugin,
    param_name: &str,
    has_no_default: bool,
) -> bool {
    if !has_no_default {
        return false;
    }
    if param_name == "config" && plugin.config_class.is_some() && plugin.config_module.is_some() {
        return false;
    }
    !matches!(param_name, "store" | "data_store")
}

fn canonical_option_key(key: &str) -> String {
    key.trim_start_matches('-').replace('-', "_")
}

fn cli_flag_name(key: &str) -> String {
    canonical_option_key(key).replace('_', "-")
}
