use crate::logger;
use crate::r2x_manifest::{runtime::build_runtime_bindings, Manifest};
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
                let plugin_type = &plugin.plugin_type;
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

    let (_pkg, disc_plugin) = manifest
        .packages
        .iter()
        .find_map(|pkg| {
            pkg.plugins
                .iter()
                .find(|p| p.name == plugin_name)
                .map(|p| (pkg, p))
        })
        .ok_or_else(|| format!("Plugin '{}' not found in manifest", plugin_name))?;

    let bindings = build_runtime_bindings(disc_plugin)
        .map_err(|e| format!("Failed to load plugin '{}': {}", plugin_name, e))?;

    logger::step(&format!("Plugin: {}", plugin_name));

    println!("\nType: {}", disc_plugin.plugin_type);

    if let Some(io_type) = &bindings.io_type {
        println!("I/O: {}", io_type);
    }

    // Check if plugin requires data store
    let needs_store = check_needs_datastore(&bindings);

    if needs_store {
        println!("\nRequires data store: yes");
        println!("\nData Store Arguments:");
        println!("  --store-path <PATH>       Path to store directory (required)");
        println!("  --store-name <NAME>       Name of the store (optional)");
    }

    // Show callable parameters
    let obj = &bindings.callable;
    println!("\nCallable: {}.{}", obj.module, obj.name);
    if let Some(call_method) = &bindings.call_method {
        println!("Method: {}", call_method);
    }

    if !obj.parameters.is_empty() {
        println!("\nCallable Parameters:");
        for (name, param) in &obj.parameters {
            let annotation = param.annotation.as_deref().unwrap_or("Any");
            let required = if param.is_required {
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
                name, annotation, required, default
            );
        }
    }

    // Show config parameters
    if let Some(config) = &bindings.config {
        println!("\nConfiguration Class: {}.{}", config.module, config.name);
        if !config.parameters.is_empty() {
            println!("\nConfiguration Parameters:");
            for (name, param) in &config.parameters {
                let annotation = param.annotation.as_deref().unwrap_or("Any");
                let required = if param.is_required {
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
                    name, annotation, required, default
                );
            }
        }
    }

    println!("\nUsage:");
    println!("  r2x run --plugin {} [OPTIONS]", plugin_name);
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

/// Check if a plugin requires a DataStore
fn check_needs_datastore(bindings: &r2x_manifest::runtime::RuntimeBindings) -> bool {
    if bindings.callable.parameters.contains_key("data_store") {
        return true;
    }

    bindings.requires_store.unwrap_or(false)
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
