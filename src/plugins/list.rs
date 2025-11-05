use crate::plugin_manifest::PluginManifest;
use crate::GlobalOpts;
use colored::*;
use std::collections::BTreeMap;

pub fn list_plugins(_opts: &GlobalOpts) -> Result<(), String> {
    let manifest = PluginManifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;
    if manifest.is_empty() {
        println!("There are no current plugins installed.");
        println!();
        println!("To install a plugin, run:");
        println!("  {} install <package>", "r2x".bold().cyan());
        return Ok(());
    }

    // Group plugins by package name and then by callable type (class/function)
    let plugins = manifest.list_plugins();
    let mut packages: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();

    for (name, plugin) in &plugins {
        let package_name = plugin
            .package_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let callable_type = if let Some(obj) = &plugin.obj {
            obj.callable_type.clone()
        } else {
            plugin
                .plugin_type
                .clone()
                .unwrap_or_else(|| "other".to_string())
        };

        packages
            .entry(package_name)
            .or_default()
            .entry(callable_type)
            .or_default()
            .push(name.to_string());
    }

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

    println!("{}: {}", "Total installed packages".bold(), packages.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_list_empty_manifest() {
        // Test handling of empty manifest
    }

    #[test]
    fn test_list_formatting() {
        // Test plugin grouping and display
    }
}
