use super::setup_config;
use crate::logger;
use crate::plugins::{
    discovery::{discover_and_register_entry_points_with_deps, DiscoveryOptions},
    install::get_package_info,
};
use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;
use colored::Colorize;

pub fn sync_manifest(_opts: &GlobalOpts) -> Result<(), String> {
    logger::debug("Loading manifest for syncing");

    let manifest = Manifest::load().map_err(|e| {
        logger::error(&format!("Failed to load manifest: {}", e));
        format!("Failed to load manifest: {}", e)
    })?;

    if manifest.is_empty() {
        logger::warn("No plugins installed. Nothing to sync.");
        return Ok(());
    }

    let (uv_path, _venv_path, python_path) = setup_config()?;
    let total_start = std::time::Instant::now();

    let packages_to_sync: Vec<String> = manifest
        .packages
        .iter()
        .map(|pkg| pkg.name.clone())
        .collect();

    if packages_to_sync.is_empty() {
        logger::warn("No packages found in manifest to sync.");
        return Ok(());
    }

    let num_packages = packages_to_sync.len();
    logger::step(&format!("Syncing {} package(s)...", num_packages));

    for package_name in packages_to_sync {
        logger::spinner_start(&format!("Syncing: {}", package_name));

        let (package_version, dependencies) =
            match get_package_info(&uv_path, &python_path, &package_name) {
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

        match discover_and_register_entry_points_with_deps(
            &uv_path,
            &python_path,
            DiscoveryOptions {
                package: package_name.to_string(),
                package_name_full: package_name.to_string(),
                dependencies,
                package_version,
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

    let elapsed_ms = total_start.elapsed().as_millis();
    println!(
        "{}",
        format!("Synced {} package(s) in {}ms", num_packages, elapsed_ms).dimmed()
    );

    Ok(())
}
