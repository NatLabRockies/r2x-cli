use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;
use colored::Colorize;
use std::collections::BTreeMap;

pub fn list_plugins(_opts: &GlobalOpts) -> Result<(), String> {
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

    let mut packages: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for pkg in &manifest.packages {
        for plugin in &pkg.plugins {
            packages
                .entry(pkg.name.clone())
                .or_default()
                .entry(format!("{:?}", plugin.kind))
                .or_default()
                .push(plugin.name.clone());
        }
    }

    if has_plugins {
        println!("{}", "Plugins:".bold().green());
        for (package_name, types) in &packages {
            let total_plugins: usize = types.values().map(|v| v.len()).sum();
            println!(
                " {} {}:",
                package_name.bold().blue(),
                format!("(total plugins: {})", total_plugins).dimmed()
            );

            for (type_name, plugin_names) in types {
                println!("    {}:", type_name);
                for plugin_name in plugin_names {
                    println!("      - {}", plugin_name);
                }
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
