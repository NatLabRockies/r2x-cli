use crate::config_manager::Config;
use crate::logger;
use crate::plugin_manifest::PluginManifest;
use crate::GlobalOpts;
use colored::Colorize;

use super::discovery::{discover_and_register_entry_points_with_deps, DiscoveryOptions};
use super::install::get_package_info;

/// Sync the plugin manifest by re-running plugin discovery for all installed packages
pub fn sync_manifest(_opts: &GlobalOpts) -> Result<(), String> {
    logger::debug("Loading manifest for syncing");

    let manifest = PluginManifest::load().map_err(|e| {
        logger::error(&format!("Failed to load manifest: {}", e));
        format!("Failed to load manifest: {}", e)
    })?;

    if manifest.is_empty() {
        logger::warn("No plugins installed. Nothing to sync.");
        return Ok(());
    }

    let (uv_path, venv_path, python_path) = setup_sync_config()?;
    logger::debug(&format!("Using venv: {}", venv_path));

    let total_start = std::time::Instant::now();

    // Get unique packages (avoid duplicates from multiple plugins in same package)
    let mut packages_to_sync: Vec<(String, String)> = Vec::new();
    for (_, plugin) in &manifest.plugins {
        if let (Some(pkg_name), Some(pkg_version)) = (&plugin.package_name, &plugin.package_version)
        {
            if !packages_to_sync.iter().any(|(name, _)| name == pkg_name) {
                packages_to_sync.push((pkg_name.clone(), pkg_version.clone()));
            }
        }
    }

    if packages_to_sync.is_empty() {
        logger::warn("No packages found in manifest to sync.");
        return Ok(());
    }

    let num_packages = packages_to_sync.len();
    logger::step(&format!("Syncing {} package(s)...", num_packages));

    // Re-discover plugins for each package
    for (package_name, package_version) in packages_to_sync {
        logger::spinner_start(&format!("Syncing: {}", package_name));

        let package_name_for_query = package_name.clone();

        // Get dependencies for this package
        let (_, dependencies) =
            match get_package_info(&uv_path, &python_path, &package_name_for_query) {
                Ok((version, deps)) => (version, deps),
                Err(e) => {
                    logger::spinner_error(&format!(
                        "Failed to get package info for {}: {}",
                        package_name, e
                    ));
                    logger::debug(&format!("Skipping package: {}", package_name));
                    (None, Vec::new())
                }
            };

        // Re-discover entry points
        match discover_and_register_entry_points_with_deps(
            &uv_path,
            &python_path,
            DiscoveryOptions {
                package: package_name.to_string(),
                package_name_full: package_name_for_query.to_string(),
                dependencies,
                package_version: Some(package_version.clone()),
                no_cache: false,
            },
        ) {
            Ok(_) => {
                logger::spinner_stop();
                logger::info(&format!("Successfully synced: {}", package_name));
            }
            Err(e) => {
                logger::spinner_error(&format!("Failed to sync {}: {}", package_name, e));
                return Err(format!("Failed to sync package '{}': {}", package_name, e));
            }
        }
    }

    let total_elapsed = total_start.elapsed();
    let elapsed_ms = total_elapsed.as_millis();

    println!(
        "{}",
        format!("Synced {} package(s) in {}ms", num_packages, elapsed_ms)
            .bold()
            .green()
    );

    Ok(())
}

fn setup_sync_config() -> Result<(String, String, String), String> {
    let mut config = Config::load().map_err(|e| {
        logger::error(&format!("Failed to load config: {}", e));
        format!("Failed to load config: {}", e)
    })?;

    config.ensure_uv_path().map_err(|e| {
        logger::error(&format!("Failed to setup uv: {}", e));
        format!("Failed to setup uv: {}", e)
    })?;

    let uv_path = config
        .uv_path
        .as_ref()
        .cloned()
        .ok_or_else(|| "uv path not configured".to_string())?;
    let venv_path = config.get_venv_path();
    let python_path = config.get_venv_python_path();

    Ok((uv_path, venv_path, python_path))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_sync_manifest() {
        // Test manifest syncing
    }
}
