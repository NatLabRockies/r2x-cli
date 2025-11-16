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
                let plugin_type = format!("{:?}", plugin.kind);
                println!(
                    "  {} {} - from package {}",
                    plugin.name.cyan(),
                    format!("({})", plugin_type).dimmed(),
                    pkg.name.dimmed()
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
                .find(|p| p.name == plugin_name)
                .map(|p| (pkg, p))
        })
        .ok_or_else(|| format!("Plugin '{}' not found in manifest", plugin_name))?;

    let bindings = r2x_manifest::build_runtime_bindings(plugin);

    logger::step(&format!("Plugin: {}", plugin_name));

    println!("\nType: {:?}", plugin.kind);

    let needs_store = bindings.requires_store;

    if needs_store {
        println!("\nRequires data store: yes");
        println!("\nData Store Arguments:");
        println!("  --store-path <PATH>       Path to store directory (required)");
        println!("  --store-name <NAME>       Name of the store (optional)");
    }

    println!(
        "\nCallable: {}.{}",
        bindings.entry_module, bindings.entry_name
    );
    if let Some(call_method) = &bindings.call_method {
        println!("Method: {}", call_method);
    }

    if !bindings.entry_parameters.is_empty() {
        println!("\nCallable Parameters:");
        for param in &bindings.entry_parameters {
            let annotation = param.annotation.as_deref().unwrap_or("Any");
            let required = if param.required {
                "required"
            } else {
                "optional"
            };
            let default = param
                .default
                .as_deref()
                .map(|d| format!(" (default: {})", d))
                .unwrap_or_default();
            println!(
                "  --{:<20} {:<15} {}{}",
                param.name, annotation, required, default
            );
        }
    }

    // Show config parameters
    if let Some(config) = &bindings.config {
        println!("\nConfiguration Class: {}.{}", config.module, config.name);
        if !config.fields.is_empty() {
            println!("\nConfiguration Parameters:");
            for field in &config.fields {
                let annotation = field.annotation.as_deref().unwrap_or("Any");
                let required = if field.required {
                    "required"
                } else {
                    "optional"
                };
                let default = field
                    .default
                    .as_deref()
                    .map(|d| format!(" (default: {})", d))
                    .unwrap_or_default();
                println!(
                    "  --{:<20} {:<15} {}{}",
                    field.name, annotation, required, default
                );
            }
        }
    }

    println!("\nUsage:");
    println!("  r2x run plugin {} [OPTIONS]", plugin_name);
    println!("    (add -q to silence logs, -q -q to hide stdout)");
    println!("\nExamples:");
    println!("  r2x run --plugin {} --show-help", plugin_name);

    if needs_store {
        println!(
            "  r2x run --plugin {} --store-path /path/to/store <other args>",
            plugin_name
        );
    } else {
        println!("  r2x run --plugin {} <args>", plugin_name);
    }

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
