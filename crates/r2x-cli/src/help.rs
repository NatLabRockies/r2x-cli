use crate::logger;
use crate::r2x_manifest::Manifest;
use colored::Colorize;

/// Show help for the run command when invoked with no arguments
pub fn show_run_help() -> Result<(), String> {
    let manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;

    println!();
    println!("{}", "No pipeline or plugin specified.".bold());
    println!();

    // Show installed plugins
    if !manifest.is_empty() {
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
    } else {
        println!("{}", "No plugins installed.".yellow());
        println!("Install plugins with: r2x install <package>");
        println!();
    }

    // Show usage hints
    println!("{}", "Usage:".bold());
    println!("  Run a pipeline:");
    println!("    r2x run <pipeline.yaml> [pipeline-name]");
    println!();
    println!("  Run a plugin directly:");
    println!("    r2x run plugin <plugin-name> [OPTIONS]");
    println!("      (use -q for quiet logs, -q -q to suppress plugin stdout)");
    println!();
    println!("  Get plugin help:");
    println!("    r2x run plugin <plugin-name> --show-help");
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
pub fn show_plugin_help(plugin_name: &str) -> Result<(), String> {
    let manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;

    let (_pkg, plugin) = manifest
        .packages
        .iter()
        .find_map(|pkg| {
            pkg.plugins
                .iter()
                .find(|p| p.name.as_ref() == plugin_name)
                .map(|p| (pkg, p))
        })
        .ok_or_else(|| format!("Plugin '{}' not found in manifest", plugin_name))?;

    logger::step(&format!("Plugin: {}", plugin_name));

    println!("\nType: {:?}", plugin.plugin_type);
    println!("Module: {}", plugin.module);

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

    // Show parameters
    if !plugin.parameters.is_empty() {
        println!("\nParameters:");
        for param in &plugin.parameters {
            let module_str = param.module.as_ref()
                .map(|m| format!(" ({})", m))
                .unwrap_or_default();
            println!("  --{:<20} {}{}", param.name, param.format_types(), module_str);
            if let Some(ref desc) = param.description {
                println!("      {}", desc);
            }
        }
    }

    // Show config schema
    if !plugin.config_schema.is_empty() {
        println!("\nConfiguration Schema:");
        for (field_name, field) in plugin.config_schema.iter() {
            let req_marker = if field.required { " (required)" } else { "" };
            println!("  --{:<20} {:?}{}", field_name, field.field_type, req_marker);
        }
    }

    println!("\nUsage:");
    println!("  r2x run plugin {} [OPTIONS]", plugin_name);
    println!("    (add -q to silence logs, -q -q to hide stdout)");
    println!("\nExamples:");
    println!("  r2x run --plugin {} --show-help", plugin_name);
    println!("  r2x run --plugin {} <args>", plugin_name);

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_show_run_help() {
        // Test run help display
    }

    #[test]
    fn test_show_plugin_help() {
        // Test plugin help display
    }
}
