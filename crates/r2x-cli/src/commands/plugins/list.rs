use crate::config_manager::Config;
use crate::plugins::get_package_info;
use crate::r2x_manifest::{Manifest, Plugin};
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

    if !has_plugins {
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
        let mut names: Vec<String> = pkg.plugins.iter().map(|p| p.name.to_string()).collect();
        names.sort();
        packages.insert(pkg.name.to_string(), names);
    }

    if has_plugins {
        println!("{}", "Plugins:".bold().green());

        // Get package version info
        let config = Config::load().ok();
        let python_path = config.as_ref().map(|c| c.get_venv_python_path());
        let uv_path = config
            .as_ref()
            .and_then(|c| c.uv_path.as_deref())
            .unwrap_or("uv");

        for (package_name, plugin_names) in &packages {
            // Get package metadata
            let pkg = manifest
                .packages
                .iter()
                .find(|p| p.name.as_ref() == package_name);
            let is_editable = pkg.map(|p| p.editable_install).unwrap_or(false);

            // Get version info
            let version_info = if let Some(ref py_path) = python_path {
                get_package_info(uv_path, py_path, package_name)
                    .ok()
                    .and_then(|(v, _)| v)
            } else {
                None
            };

            // Build package header with version and editable status
            let mut package_header = format!(" {}:", package_name.bold().blue());
            if let Some(version) = version_info {
                package_header.push_str(&format!(" {}", format!("v{}", version).dimmed()));
            }
            if is_editable {
                if let Some(source_uri) = pkg.and_then(|p| p.source_uri.as_ref()) {
                    package_header.push_str(&format!(" {}", format!("({})", source_uri).dimmed()));
                } else {
                    package_header.push_str(&format!(" {}", "[editable]".yellow()));
                }
            }
            println!("{}", package_header);

            for plugin_name in plugin_names {
                println!("    - {}", plugin_name);
            }
            println!();
        }

        println!("{}: {}", "Total plugin packages".bold(), packages.len());
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
        .find(|pkg| pkg.name.as_ref() == plugin_filter)
        .ok_or_else(|| format!("Plugin package '{}' not found", plugin_filter))?;

    // Build package header with version and editable info
    let config = Config::load().ok();
    let python_path = config.as_ref().map(|c| c.get_venv_python_path());
    let uv_path = config
        .as_ref()
        .and_then(|c| c.uv_path.as_deref())
        .unwrap_or("uv");

    let version_info = if let Some(ref py_path) = python_path {
        get_package_info(uv_path, py_path, package.name.as_ref())
            .ok()
            .and_then(|(v, _)| v)
    } else {
        None
    };

    print!(
        "{} {}",
        "Package:".bold().green(),
        package.name.as_ref().bold().blue()
    );
    if let Some(version) = version_info {
        print!(" {}", format!("v{}", version).dimmed());
    }
    if package.editable_install {
        if let Some(ref source_uri) = package.source_uri {
            print!(" {}", format!("({})", source_uri).dimmed());
        } else {
            print!(" {}", "[editable]".yellow());
        }
    }
    println!();
    println!();

    // Filter plugins by module name if provided
    let plugins_to_show: Vec<_> = if let Some(module_name) = module_filter {
        package
            .plugins
            .iter()
            .filter(|p| {
                // Match if the plugin name ends with the module filter
                // e.g., "r2x_reeds.break_gens" matches module "break_gens"
                let name_str = p.name.as_ref();
                let parts: Vec<&str> = name_str.split('.').collect();
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

fn show_plugin_compact(plugin: &Plugin) {
    println!(
        "{} [{:?}]",
        plugin.name.as_ref().bold().cyan(),
        plugin.plugin_type
    );

    // Show module info
    println!("  {}: {}", "Module".dimmed(), plugin.module);

    // Show class or function name
    if let Some(ref class_name) = plugin.class_name {
        println!("  {}: {}", "Class".dimmed(), class_name);
    }
    if let Some(ref function_name) = plugin.function_name {
        println!("  {}: {}", "Function".dimmed(), function_name);
    }

    // Show config if available
    if let Some(ref config_class) = plugin.config_class {
        println!("  {}: {}", "Config".dimmed(), config_class);
    }

    // Show arguments if available
    if !plugin.parameters.is_empty() {
        println!("  {}:", "Arguments".dimmed());
        for param in &plugin.parameters {
            let req_marker = if param.required { "*" } else { " " };
            let default_str = param
                .default
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            println!(
                "    {}{}: {}{}",
                req_marker,
                param.name,
                param.format_types(),
                default_str
            );

            if let Some(ref desc) = param.description {
                println!("      {}", desc.dimmed());
            }
        }
    }
}

fn show_plugin_verbose(plugin: &Plugin) {
    println!("{}", plugin.name.as_ref().bold().cyan());

    println!("  {}: {:?}", "Type".dimmed(), plugin.plugin_type);
    println!("  {}: {}", "Module".dimmed(), plugin.module);

    // Show class or function name
    if let Some(ref class_name) = plugin.class_name {
        println!("  {}: {}", "Class".dimmed(), class_name);
    }
    if let Some(ref function_name) = plugin.function_name {
        println!("  {}: {}", "Function".dimmed(), function_name);
    }

    // Show config info
    if let Some(ref config_class) = plugin.config_class {
        print!("  {}: {}", "Config Class".dimmed(), config_class);
        if let Some(ref config_module) = plugin.config_module {
            print!(" ({})", config_module);
        }
        println!();
    }

    // Show hooks
    if !plugin.hooks.is_empty() {
        println!("  {}:", "Hooks".dimmed());
        for hook in &plugin.hooks {
            println!("    - {}", hook);
        }
    }

    // Show arguments
    if !plugin.parameters.is_empty() {
        println!("  {}:", "Arguments".dimmed());
        for param in &plugin.parameters {
            let req_marker = if param.required { "*" } else { " " };
            let module_str = param
                .module
                .as_ref()
                .map(|m| format!(" ({})", m))
                .unwrap_or_default();
            let default_str = param
                .default
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            println!(
                "    {}{}: {}{}{}",
                req_marker,
                param.name,
                param.format_types(),
                module_str,
                default_str
            );

            if let Some(ref desc) = param.description {
                println!("      {}", desc.dimmed());
            }
        }
    }

    // Show config schema if available
    if !plugin.config_schema.is_empty() {
        println!("  {}:", "Config Schema".dimmed());
        for (field_name, field) in plugin.config_schema.iter() {
            let req_marker = if field.required { "*" } else { "" };
            println!("    {}{}: {:?}", field_name, req_marker, field.field_type);
        }
    }
}
