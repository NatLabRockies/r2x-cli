use crate::r2x_manifest::{ImplementationType, Manifest};
use crate::GlobalOpts;
use colored::Colorize;
use std::collections::BTreeMap;

pub fn list_plugins(
    opts: &GlobalOpts,
    plugin_filter: Option<String>,
    module_filter: Option<String>,
) -> Result<(), String> {
    let manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;

    let has_plugins = !manifest.is_empty();
    let has_decorators = manifest
        .packages
        .iter()
        .any(|pkg| !pkg.decorator_registrations.is_empty());

    if !has_plugins && !has_decorators {
        println!("There are no current plugins installed.\n");
        println!(
            "To install a plugin, run:\n  {} install <package>",
            "r2x".bold().cyan()
        );
        return Ok(());
    }

    // If a plugin filter is provided, show detailed information
    if let Some(ref plugin_name) = plugin_filter {
        return show_plugin_details(
            &manifest,
            plugin_name,
            module_filter.as_deref(),
            opts.verbose,
        );
    }

    // Otherwise, show the standard list view
    let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in &manifest.packages {
        let mut names: Vec<String> = pkg.plugins.iter().map(|p| p.name.clone()).collect();
        names.sort();
        packages.insert(pkg.name.clone(), names);
    }

    if has_plugins {
        println!("{}", "Plugins:".bold().green());
        for (package_name, plugin_names) in &packages {
            println!(" {}:", package_name.bold().blue());
            for plugin_name in plugin_names {
                println!("    - {}", plugin_name);
            }
            println!();
        }

        println!("{}: {}", "Total plugin packages".bold(), packages.len());
    }

    if has_decorators {
        println!();
        println!("{}", "Decorator Registrations:".bold().green());

        let mut total_decorator_packages = 0;
        for pkg in &manifest.packages {
            if !pkg.decorator_registrations.is_empty() {
                println!(
                    " {} {}:",
                    pkg.name.bold().blue(),
                    format!("({} registrations)", pkg.decorator_registrations.len()).dimmed()
                );

                for reg in &pkg.decorator_registrations {
                    println!(
                        "    @{}.{}() -> {}",
                        reg.decorator_class, reg.decorator_method, reg.function_name
                    );
                    if let Some(source) = &reg.source_file {
                        println!("      {}: {}", "Source".dimmed(), source.dimmed());
                    }
                }
                println!();
                total_decorator_packages += 1;
            }
        }

        println!(
            "{}: {}",
            "Total packages with decorators".bold(),
            total_decorator_packages
        );
    }

    Ok(())
}

fn show_plugin_details(
    manifest: &Manifest,
    plugin_filter: &str,
    module_filter: Option<&str>,
    verbose_level: u8,
) -> Result<(), String> {
    // Find the package containing this plugin
    let package = manifest
        .packages
        .iter()
        .find(|pkg| pkg.name == plugin_filter)
        .ok_or_else(|| format!("Plugin package '{}' not found", plugin_filter))?;

    println!(
        "{} {}",
        "Package:".bold().green(),
        package.name.bold().blue()
    );
    println!();

    // Filter plugins by module name if provided
    let plugins_to_show: Vec<_> = if let Some(module_name) = module_filter {
        package
            .plugins
            .iter()
            .filter(|p| {
                // Match if the plugin name ends with the module filter
                // e.g., "r2x_reeds.break_gens" matches module "break_gens"
                let parts: Vec<&str> = p.name.split('.').collect();
                parts
                    .last()
                    .map(|&last| last == module_name)
                    .unwrap_or(false)
            })
            .collect()
    } else {
        package.plugins.iter().collect()
    };

    if plugins_to_show.is_empty() {
        return Err(format!(
            "No plugins found matching the filter criteria in package '{}'",
            plugin_filter
        ));
    }

    for plugin in plugins_to_show {
        if verbose_level > 0 {
            show_plugin_verbose(plugin);
        } else {
            show_plugin_compact(plugin);
        }
        println!();
    }

    Ok(())
}

fn show_plugin_compact(plugin: &crate::r2x_manifest::PluginSpec) {
    println!("{} [{:?}]", plugin.name.bold().cyan(), plugin.kind);

    if let Some(desc) = &plugin.description {
        println!("  {}: {}", "Description".dimmed(), desc);
    }

    // Collect all parameters (constructor + call)
    let mut all_params = Vec::new();

    // Add constructor parameters if present
    if !plugin.invocation.constructor.is_empty() {
        all_params.extend(plugin.invocation.constructor.iter());
    }

    // Add call/method parameters
    if !plugin.invocation.call.is_empty() {
        all_params.extend(plugin.invocation.call.iter());
    }

    if !all_params.is_empty() {
        println!("  {}:", "Parameters".dimmed());
        for arg in all_params {
            let default_str = arg
                .default
                .as_ref()
                .map(|d| format!(" (default={})", d))
                .unwrap_or_default();

            println!("    {}{}", arg.name, default_str);
        }
    }
}

fn show_plugin_verbose(plugin: &crate::r2x_manifest::PluginSpec) {
    println!("{}", plugin.name.bold().cyan());

    if let Some(desc) = &plugin.description {
        println!("  {}: {}", "Description".dimmed(), desc);
    }

    println!("  {}: {:?}", "Kind".dimmed(), plugin.kind);
    println!("  {}: {}", "Entry".dimmed(), plugin.entry);

    // Show implementation type
    println!(
        "  {}: {:?}",
        "Implementation".dimmed(),
        plugin.invocation.implementation
    );

    // Show constructor parameters if it's a class
    if !plugin.invocation.constructor.is_empty() {
        println!("\n  {}:", "Constructor Parameters".bold().yellow());
        for arg in &plugin.invocation.constructor {
            let req_marker = if arg.required { "*" } else { "" };
            let annotation = arg.annotation.as_deref().unwrap_or("Any");
            let default_str = arg
                .default
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();

            println!(
                "    {}{}: {}{}",
                arg.name, req_marker, annotation, default_str
            );
        }
    }

    // Show call/method parameters
    if !plugin.invocation.call.is_empty() {
        let label = match plugin.invocation.implementation {
            ImplementationType::Function => "Function Parameters",
            ImplementationType::Class => {
                let method_name = plugin.invocation.method.as_deref().unwrap_or("__call__");
                println!(
                    "\n  {} ({}):",
                    "Method Parameters".bold().yellow(),
                    method_name
                );
                ""
            }
        };

        if !label.is_empty() {
            println!("\n  {}:", label.bold().yellow());
        }

        for arg in &plugin.invocation.call {
            let req_marker = if arg.required { "*" } else { "" };
            let annotation = arg.annotation.as_deref().unwrap_or("Any");
            let default_str = arg
                .default
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();

            println!(
                "    {}{}{}{}",
                arg.name,
                req_marker,
                if annotation != "Any" {
                    format!(": {}", annotation)
                } else {
                    String::new()
                },
                default_str
            );
        }
    }

    // Show I/O contract
    if !plugin.io.consumes.is_empty() || !plugin.io.produces.is_empty() {
        println!("\n  {}:", "I/O Contract".bold().yellow());
        if !plugin.io.consumes.is_empty() {
            println!("    {}: {:?}", "Consumes".dimmed(), plugin.io.consumes);
        }
        if !plugin.io.produces.is_empty() {
            println!("    {}: {:?}", "Produces".dimmed(), plugin.io.produces);
        }
    }

    // Show resource requirements
    if let Some(resources) = &plugin.resources {
        println!("\n  {}:", "Resources".bold().yellow());

        if let Some(config) = &resources.config {
            println!("    {}:", "Config".dimmed());
            println!("      Module: {}", config.module);
            println!("      Class: {}", config.name);
            if !config.fields.is_empty() {
                println!("      Fields:");
                for field in &config.fields {
                    let req_marker = if field.required { "*" } else { "" };
                    let annotation = field.annotation.as_deref().unwrap_or("Any");
                    let default_str = field
                        .default
                        .as_ref()
                        .map(|d| format!(" = {}", d))
                        .unwrap_or_default();

                    println!(
                        "        {}{}: {}{}",
                        field.name, req_marker, annotation, default_str
                    );
                }
            }
        }

        if let Some(store) = &resources.store {
            println!("    {}:", "Store".dimmed());
            println!("      Mode: {:?}", store.mode);
            if let Some(path) = &store.path {
                println!("      Path: {}", path);
            }
        }
    }
}
