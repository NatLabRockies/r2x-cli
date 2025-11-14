//! Plugin discovery orchestration
//!
//! Manages the discovery and registration of plugins from packages,
//! handling caching, dependencies, and manifest updates.

use crate::logger;
use crate::plugins::{find_package_path, utils, AstDiscovery};
use crate::r2x_manifest::Manifest;

/// Options for plugin discovery and registration
pub struct DiscoveryOptions {
    pub package: String,
    pub package_name_full: String,
    pub dependencies: Vec<String>,
    pub package_version: Option<String>,
    pub no_cache: bool,
}

/// Discover and register plugins from a package and its dependencies
pub fn discover_and_register_entry_points_with_deps(
    _uv_path: &str,
    _python_path: &str,
    opts: DiscoveryOptions,
) -> Result<usize, String> {
    let package = &opts.package;
    let package_name_full = &opts.package_name_full;
    let dependencies = &opts.dependencies;
    let no_cache = opts.no_cache;
    let package_version = opts.package_version.as_deref().unwrap_or("unknown");

    // Get venv path from config for entry_points.txt lookup
    let venv_path = crate::config_manager::Config::load()
        .ok()
        .map(|c| c.get_venv_path());

    // Load manifest
    let mut manifest = match Manifest::load() {
        Ok(m) => m,
        Err(e) => {
            logger::warn(&format!("Failed to load manifest: {}", e));
            Manifest {
                metadata: crate::r2x_manifest::Metadata {
                    version: "1.0".to_string(),
                    generated_at: chrono::Utc::now().to_rfc3339(),
                    uv_lock_path: None,
                },
                packages: Vec::new(),
            }
        }
    };

    // Check if we already have this package in the manifest
    let has_package_cached = manifest
        .packages
        .iter()
        .any(|p| p.name == *package_name_full);

    let (discovered_plugins, decorator_regs) = if has_package_cached && !no_cache {
        if let Some(pkg) = manifest
            .packages
            .iter()
            .find(|p| p.name == *package_name_full)
        {
            (pkg.plugins.clone(), pkg.decorator_registrations.clone())
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        let package_path = find_package_path(package_name_full)
            .map_err(|e| format!("Failed to locate package '{}': {}", package_name_full, e))?;

        AstDiscovery::discover_plugins(
            &package_path,
            package_name_full,
            venv_path.as_deref(),
            Some(package_version),
        )
        .map_err(|e| format!("Failed to discover plugins for '{}': {}", package, e))?
    };

    for plugin in &discovered_plugins {
        logger::debug(&format!(
            "Discovered plugin '{}' of kind {:?}",
            plugin.name,
            plugin.kind
        ));
    }

    let mut total_plugins = discovered_plugins.len();

    if total_plugins == 0 {
        logger::warn(&format!("No plugins found in package '{}'", package));
        return Ok(0);
    }

    logger::info(&format!(
        "Found {} plugin(s) in package '{}'",
        total_plugins, package
    ));

    {
        let pkg = manifest.get_or_create_package(package_name_full);
        pkg.entry_points_dist_info = String::new();
        pkg.plugins = discovered_plugins.clone();
        pkg.decorator_registrations = decorator_regs.clone();
    }
    manifest.mark_explicit(package_name_full);

    let r2x_dependencies: Vec<String> = dependencies
        .iter()
        .filter(|dep| utils::looks_like_r2x_plugin(dep))
        .cloned()
        .collect();

    {
        let pkg = manifest.get_or_create_package(package_name_full);
        pkg.dependencies = r2x_dependencies.clone();
    }

    for dep in r2x_dependencies {
        manifest.add_dependency(package_name_full, &dep);

        let has_dep_cached = manifest.packages.iter().any(|p| p.name == dep);
        let (dep_plugins, dep_decorators) = if has_dep_cached && !no_cache {
            if let Some(pkg) = manifest.packages.iter().find(|p| p.name == dep) {
                (pkg.plugins.clone(), pkg.decorator_registrations.clone())
            } else {
                (Vec::new(), Vec::new())
            }
        } else {
            match find_package_path(&dep) {
                Ok(dep_path) => match AstDiscovery::discover_plugins(
                    &dep_path,
                    &dep,
                    venv_path.as_deref(),
                    None,
                ) {
                    Ok(result) => result,
                    Err(e) => {
                        logger::warn(&format!(
                            "Failed to discover plugins from dependency '{}': {}",
                            &dep, e
                        ));
                        (Vec::new(), Vec::new())
                    }
                },
                Err(e) => {
                    logger::warn(&format!(
                        "Failed to locate dependency package '{}': {}",
                        &dep, e
                    ));
                    (Vec::new(), Vec::new())
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
            dep_pkg.decorator_registrations = dep_decorators;
        }
        manifest.mark_dependency(&dep, package_name_full);
        total_plugins += dep_count;
    }

    // Save the updated manifest with all plugins (explicit + dependencies)
    manifest
        .save()
        .map_err(|e| format!("Failed to save manifest: {}", e))?;

    Ok(total_plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_r2x_plugin() {
        assert!(utils::looks_like_r2x_plugin("r2x-reeds"));
        assert!(utils::looks_like_r2x_plugin("r2x-plexos"));
        assert!(!utils::looks_like_r2x_plugin("r2x-core"));
        assert!(!utils::looks_like_r2x_plugin("numpy"));
    }
}
