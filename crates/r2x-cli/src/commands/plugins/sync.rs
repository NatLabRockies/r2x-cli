use super::PluginContext;
use crate::logger;
use crate::plugins::AstDiscovery;
use crate::plugins::PluginError;
use colored::Colorize;
use std::path::PathBuf;

/// Fast sync that re-discovers plugins using AST parsing
///
/// Optimized for speed:
/// 1. Uses source_uri from manifest directly (no directory scanning)
/// 2. Loads config/manifest only once
/// 3. Pure Rust AST parsing via ast-grep
pub fn sync_manifest(ctx: &mut PluginContext) -> Result<(), PluginError> {
    let total_start = std::time::Instant::now();
    let venv_path = &ctx.venv_path;
    let manifest = &mut ctx.manifest;

    if manifest.is_empty() {
        logger::warn("No plugins installed. Nothing to sync.");
        return Ok(());
    }

    // Collect package info from manifest
    let packages_to_sync: Vec<_> = manifest
        .packages
        .iter()
        .filter_map(|pkg| {
            // source_uri is required for sync - it tells us where the package source is
            pkg.source_uri.as_ref().map(|uri| {
                (
                    pkg.name.to_string(),
                    pkg.version.to_string(),
                    pkg.editable_install,
                    uri.to_string(),
                )
            })
        })
        .collect();

    if packages_to_sync.is_empty() {
        logger::warn("No packages with source_uri found. Nothing to sync.");
        return Ok(());
    }

    let num_packages = packages_to_sync.len();
    logger::step(&format!("Syncing {} package(s)...", num_packages));

    let mut total_plugins = 0;

    for (package_name, version, editable, source_uri) in packages_to_sync {
        let package_path = PathBuf::from(&source_uri);

        // Re-discover plugins using AST (fast, pure Rust)
        let (ast_plugins, _decorators) = match AstDiscovery::discover_plugins(
            &package_path,
            &package_name,
            Some(venv_path),
            Some(&version),
        ) {
            Ok(result) => result,
            Err(e) => {
                logger::warn(&format!(
                    "Failed to discover plugins for '{}': {}",
                    package_name, e
                ));
                continue;
            }
        };

        let plugin_count = ast_plugins.len();
        if plugin_count == 0 {
            logger::debug(&format!("No plugins found in package '{}'", package_name));
            continue;
        }

        // Convert and update manifest
        let plugins: Vec<_> = ast_plugins
            .into_iter()
            .map(|p| p.to_manifest_plugin())
            .collect();

        {
            let pkg = manifest.get_or_create_package(&package_name);
            pkg.plugins = plugins;
            pkg.editable_install = editable;
            pkg.source_uri = Some(std::sync::Arc::from(source_uri));
        }

        total_plugins += plugin_count;
        logger::debug(&format!(
            "Synced {} plugin(s) from '{}'",
            plugin_count, package_name
        ));
    }

    // Save manifest once at the end
    manifest.save()?;

    let elapsed_ms = total_start.elapsed().as_millis();
    println!(
        "{}",
        format!(
            "Synced {} package(s), {} plugin(s) in {}ms",
            num_packages, total_plugins, elapsed_ms
        )
        .dimmed()
    );

    Ok(())
}
