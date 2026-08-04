use crate::manifest_lookup::resolve_plugin_ref;
use colored::Colorize;
use r2x_logger as logger;
use r2x_manifest::types::Manifest;
use std::collections::BTreeSet;

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
    println!("      (use `r2x run plugin <plugin-name>` for the legacy explicit form)");
    println!();
    println!("  Get plugin help:");
    println!("    r2x run <plugin-name> --show-help");
    println!();
    println!("  List pipelines in YAML:");
    println!("    r2x run <pipeline.yaml> --list");
    println!();
    println!("  Print resolved pipeline config:");
    println!("    r2x run <pipeline.yaml> --print <pipeline-name>");
    println!();

    Ok(())
}

/// Show detailed help for a specific plugin
pub(crate) fn show_plugin_help(plugin_name: &str) -> Result<(), String> {
    let manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;

    let resolved = resolve_plugin_ref(&manifest, plugin_name).map_err(|e| e.to_string())?;
    let plugin = resolved.plugin;

    logger::step(&format!("Plugin: {}", plugin_name));

    println!("\nType: {:?}", plugin.plugin_type);
    println!("Module: {}", plugin.module);

    // Show description if available
    if let Some(ref desc) = plugin.description {
        println!("Description: {}", desc);
    }

    // Show class or function name
    if let Some(ref class_name) = plugin.class_name {
        println!("Class: {}", class_name);
    }
    if let Some(ref function_name) = plugin.function_name {
        println!("Function: {}", function_name);
    }

    // Show config if available
    if let Some(ref config_class) = plugin.config_class {
        print!("\nConfiguration Class: {}", config_class);
        if let Some(ref config_module) = plugin.config_module {
            print!(" ({})", config_module);
        }
        println!();
    }

    let required_options = required_plugin_options(plugin);
    let usage_options = if required_options.is_empty() {
        " [OPTIONS]".to_string()
    } else {
        format_option_usage(&required_options)
    };
    let example_options = if required_options.is_empty() {
        plugin_option_names(plugin)
            .iter()
            .next()
            .map(|name| format!(" --{} <value>", name))
            .unwrap_or_default()
    } else {
        format_option_usage(&required_options)
    };

    println!("\nUsage:");
    println!("  r2x run {}{}", plugin_name, usage_options);
    println!("    (add -i <FILE> to load a System or -o <FILE> to persist one)");

    // Show parameters
    if !plugin.parameters.is_empty() {
        println!("\nPlugin options:");
        for param in &plugin.parameters {
            let module_str = param
                .module
                .as_ref()
                .map(|m| format!(" ({})", m))
                .unwrap_or_default();
            let req_marker = if required_param_is_user_supplied(
                plugin,
                param.name.as_ref(),
                param.required && param.default.is_none(),
            ) {
                " (required)"
            } else {
                ""
            };
            println!(
                "  --{:<20} {}{}{}",
                cli_flag_name(param.name.as_ref()),
                param.format_types(),
                module_str,
                req_marker
            );
            if param.name.contains('_') {
                println!("      Alias: --{}", param.name);
            }
            if let Some(ref desc) = param.description {
                println!("      {}", desc);
            }
        }
    }

    // Show config schema
    if !plugin.config_schema.is_empty() {
        println!("\nConfiguration options:");
        for (field_name, field) in plugin.config_schema.iter() {
            let req_marker = if field.required && field.default.is_none() {
                " (required)"
            } else {
                ""
            };
            println!(
                "  --{:<20} {:?}{}",
                cli_flag_name(field_name.as_ref()),
                field.field_type,
                req_marker
            );
            if field_name.contains('_') {
                println!("      Alias: --{}", field_name);
            }
        }
    }

    println!("\nCompatibility:");
    println!("  key=value arguments are also supported:");
    println!("    {}", compatibility_example(plugin));
    println!("  --set key=value is accepted as an explicit key/value form.");

    println!("\nExamples:");
    println!("  r2x run {} --show-help", plugin_name);
    println!("  r2x run {}{}", plugin_name, example_options);

    Ok(())
}

fn required_plugin_options(plugin: &r2x_manifest::types::Plugin) -> BTreeSet<String> {
    let mut options = BTreeSet::new();

    for param in &plugin.parameters {
        if required_param_is_user_supplied(
            plugin,
            param.name.as_ref(),
            param.required && param.default.is_none(),
        ) {
            options.insert(cli_flag_name(param.name.as_ref()));
        }
    }

    for (field_name, field) in plugin.config_schema.iter() {
        if field.required && field.default.is_none() {
            options.insert(cli_flag_name(field_name.as_ref()));
        }
    }

    options
}

fn plugin_option_names(plugin: &r2x_manifest::types::Plugin) -> BTreeSet<String> {
    let mut options = BTreeSet::new();
    for param in &plugin.parameters {
        options.insert(cli_flag_name(param.name.as_ref()));
    }
    for (field_name, _) in plugin.config_schema.iter() {
        options.insert(cli_flag_name(field_name.as_ref()));
    }
    options
}

fn compatibility_example(plugin: &r2x_manifest::types::Plugin) -> String {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for param in &plugin.parameters {
        keys.insert(param.name.to_string());
    }
    for (field_name, _) in plugin.config_schema.iter() {
        keys.insert(field_name.to_string());
    }

    if keys.is_empty() {
        return "key=value".to_string();
    }

    keys.iter()
        .map(|key| format!("{}=<value>", key))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_option_usage(options: &BTreeSet<String>) -> String {
    options.iter().fold(String::new(), |mut usage, name| {
        usage.push_str(" --");
        usage.push_str(name);
        usage.push_str(" <value>");
        usage
    })
}

fn required_param_is_user_supplied(
    plugin: &r2x_manifest::types::Plugin,
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

fn cli_flag_name(key: &str) -> String {
    key.replace('_', "-")
}
