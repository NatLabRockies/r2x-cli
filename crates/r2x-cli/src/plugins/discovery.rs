//! Plugin discovery orchestration
//!
//! Manages the discovery and registration of plugins from packages,
//! handling caching, dependencies, and manifest updates.

use crate::plugins::error::PluginError;
use crate::plugins::utils;
use r2x_ast::AstDiscovery;
use r2x_logger as logger;
use r2x_manifest::package_discovery::PackageLocator;
use r2x_manifest::types::{InstallType, Manifest, Plugin};
use std::sync::Arc;

/// Options for plugin discovery and registration
pub struct DiscoveryOptions {
    pub package: String,
    pub package_name_full: String,
    pub dependencies: Vec<String>,
    pub package_version: Option<String>,
    pub no_cache: bool,
    pub editable: bool,
    pub source_path: Option<String>,
}

/// Discover and register plugins from a package and its dependencies
pub fn discover_and_register_entry_points_with_deps(
    locator: &PackageLocator,
    venv_path: Option<&str>,
    manifest: &mut Manifest,
    opts: DiscoveryOptions,
) -> Result<usize, PluginError> {
    let package = &opts.package;
    let package_name_full = &opts.package_name_full;
    let dependencies = &opts.dependencies;
    let no_cache = opts.no_cache;
    let package_version = opts.package_version.as_deref().unwrap_or("unknown");

    // Check if we already have this package in the manifest with plugins
    let has_cached_plugins = manifest
        .get_package(package_name_full)
        .is_some_and(|pkg| !pkg.plugins.is_empty());

    // Discover or use cached plugins
    let discovered_plugins: Vec<Plugin> = if has_cached_plugins && !no_cache {
        // Use cached plugins
        manifest
            .get_package(package_name_full)
            .map(|pkg| pkg.plugins.clone())
            .unwrap_or_default()
    } else {
        // Discover from source
        let package_path = locator.find_package_path(package_name_full).map_err(|e| {
            PluginError::Locator(format!(
                "Failed to locate package '{}': {}",
                package_name_full, e
            ))
        })?;

        logger::debug(&format!(
            "Found package path for '{}': {}",
            package_name_full,
            package_path.display()
        ));

        let (ast_plugins, _decorator_regs) = AstDiscovery::discover_plugins(
            &package_path,
            package_name_full,
            venv_path,
            Some(package_version),
        )
        .map_err(|e| {
            PluginError::Discovery(format!(
                "Failed to discover plugins for '{}': {}",
                package, e
            ))
        })?;

        // ast_plugins are already in manifest Plugin format
        ast_plugins
    };

    for plugin in &discovered_plugins {
        logger::debug(&format!(
            "Discovered plugin '{}' of type {:?}",
            plugin.name, plugin.plugin_type
        ));
    }

    let mut total_plugins = discovered_plugins.len();

    if total_plugins == 0 {
        logger::warn(&format!("No plugins found in package '{}'", package));
        return Ok(0);
    }

    logger::debug(&format!(
        "Registered {} plugin(s) from package '{}'",
        total_plugins, package
    ));

    // Update package in manifest
    {
        let pkg = manifest.get_or_create_package(package_name_full);
        pkg.plugins = discovered_plugins;
        pkg.version = Arc::from(package_version);
        pkg.install_type = InstallType::Explicit;

        if opts.editable {
            pkg.editable_install = true;
            pkg.source_uri = opts.source_path.map(Arc::from);
        }
    }
    manifest.mark_explicit(package_name_full);

    // Filter r2x dependencies
    let r2x_dependencies: Vec<String> = dependencies
        .iter()
        .filter(|dep| utils::looks_like_r2x_plugin(dep))
        .cloned()
        .collect();

    // Set dependencies on the main package
    {
        let pkg = manifest.get_or_create_package(package_name_full);
        pkg.dependencies = r2x_dependencies
            .iter()
            .map(|s| Arc::from(s.as_str()))
            .collect();
    }

    // Process each r2x dependency
    for dep in r2x_dependencies {
        manifest.add_dependency(package_name_full, &dep);

        let has_dep_cached = manifest
            .get_package(&dep)
            .is_some_and(|pkg| !pkg.plugins.is_empty());

        let dep_plugins: Vec<Plugin> = if has_dep_cached && !no_cache {
            manifest
                .get_package(&dep)
                .map(|pkg| pkg.plugins.clone())
                .unwrap_or_default()
        } else {
            match locator.find_package_path(&dep) {
                Ok(dep_path) => {
                    match AstDiscovery::discover_plugins(&dep_path, &dep, venv_path, None) {
                        Ok((ast_plugins, _decorators)) => ast_plugins,
                        Err(e) => {
                            logger::warn(&format!(
                                "Failed to discover plugins from dependency '{}': {}",
                                &dep, e
                            ));
                            Vec::new()
                        }
                    }
                }
                Err(e) => {
                    logger::warn(&format!(
                        "Failed to locate dependency package '{}': {}",
                        &dep, e
                    ));
                    Vec::new()
                }
            }
        };

        if dep_plugins.is_empty() {
            continue;
        }

        let dep_count = dep_plugins.len();
        {
            let dep_pkg = manifest.get_or_create_package(&dep);
            dep_pkg.plugins = dep_plugins;
        }
        manifest.mark_dependency(&dep, package_name_full);
        total_plugins += dep_count;
    }

    // Save the updated manifest with all plugins (explicit + dependencies)
    manifest.save()?;

    Ok(total_plugins)
}

#[cfg(test)]
mod tests {
    use crate::plugins::discovery::*;

    #[test]
    fn test_looks_like_r2x_plugin() {
        assert!(utils::looks_like_r2x_plugin("r2x-reeds"));
        assert!(utils::looks_like_r2x_plugin("r2x-plexos"));
        assert!(!utils::looks_like_r2x_plugin("r2x-core"));
        assert!(!utils::looks_like_r2x_plugin("numpy"));
    }
}
