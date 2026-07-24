//! Plugin discovery orchestration
//!
//! Manages the discovery and registration of plugins from packages,
//! handling caching, dependencies, and manifest updates.

use crate::plugins::error::PluginError;
use r2x_ast::AstDiscovery;
use r2x_logger as logger;
use r2x_manifest::package_discovery::PackageLocator;
use r2x_manifest::types::{InstallType, Manifest, Plugin};
use std::path::PathBuf;
use std::sync::Arc;

/// Options for plugin discovery and registration
pub(crate) struct DiscoveryOptions {
    pub(crate) package: String,
    pub(crate) package_name_full: String,
    pub(crate) dependencies: Vec<String>,
    pub(crate) package_version: Option<String>,
    pub(crate) no_cache: bool,
    pub(crate) editable: bool,
    /// Local filesystem path for editable installs (used for AST discovery)
    pub(crate) source_path: Option<String>,
    /// Display URI stored in manifest (git URL or local path)
    pub(crate) source_uri: Option<String>,
}

/// Discover and register plugins from a package and its dependencies
pub(crate) fn discover_and_register_entry_points_with_deps(
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

    // Discover or use cached plugins
    let discovered_plugins: Vec<Plugin> =
        if should_use_cached_plugins(manifest, package_name_full, package_version, no_cache) {
            // Use cached plugins
            manifest
                .get_package(package_name_full)
                .map(|pkg| pkg.plugins.clone())
                .unwrap_or_default()
        } else {
            // Discover from source
            let package_path =
                resolve_package_path(locator, package_name_full, opts.source_path.as_deref())?;

            logger::debug(&format!(
                "Found package path for '{}': {}",
                package_name_full,
                package_path.display()
            ));

            let dist_info = locator.find_dist_info_path(package_name_full);
            AstDiscovery::discover_plugins(
                &package_path,
                package_name_full,
                venv_path,
                Some(package_version),
                dist_info.as_deref(),
            )
            .map_err(|e| {
                PluginError::Discovery(format!(
                    "Failed to discover plugins for '{}': {}",
                    package, e
                ))
            })?
        };

    for plugin in &discovered_plugins {
        logger::debug(&format!(
            "Discovered plugin '{}' of type {:?}",
            plugin.name, plugin.plugin_type
        ));
    }

    let mut total_plugins = discovered_plugins.len();

    if total_plugins > 0 {
        logger::debug(&format!(
            "Registered {} plugin(s) from package '{}'",
            total_plugins, package
        ));
    }

    let package_source =
        locator.detect_package_source(package_name_full, opts.source_path.as_deref());

    // Only save to manifest if the package has plugins (skip empty wrapper packages)
    if total_plugins > 0 {
        let pkg = manifest.get_or_create_package(package_name_full);
        pkg.plugins = discovered_plugins;
        pkg.version = Arc::from(package_version);
        pkg.install_type = InstallType::Explicit;
        pkg.source_kind = package_source;
        pkg.editable_install = opts.editable;
        pkg.source_uri = opts.source_uri.as_deref().map(Arc::from);
        manifest.mark_explicit(package_name_full);
    }

    // Filter dependencies that have r2x plugin entry points
    let r2x_dependencies: Vec<String> = dependencies
        .iter()
        .filter(|dep| locator.has_plugin_entry_points(dep))
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
                    let dep_dist_info = locator.find_dist_info_path(&dep);
                    match AstDiscovery::discover_plugins(
                        &dep_path,
                        &dep,
                        venv_path,
                        None,
                        dep_dist_info.as_deref(),
                    ) {
                        Ok(ast_plugins) => ast_plugins,
                        Err(e) => {
                            logger::warn(&format!(
                                "Failed to discover plugins from dependency '{}': {}",
                                dep, e
                            ));
                            Vec::new()
                        }
                    }
                }
                Err(e) => {
                    logger::warn(&format!(
                        "Failed to locate dependency package '{}': {}",
                        dep, e
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
            dep_pkg.source_uri = None;
            dep_pkg.source_kind = locator.detect_package_source(&dep, None);
        }
        manifest.mark_dependency(&dep, package_name_full);
        total_plugins += dep_count;
    }

    if total_plugins == 0 {
        logger::warn(&format!("No plugins found in package '{}'", package));
        return Ok(0);
    }

    // Save the updated manifest with all plugins (explicit + dependencies)
    manifest.save()?;

    Ok(total_plugins)
}

fn should_use_cached_plugins(
    manifest: &Manifest,
    package_name_full: &str,
    package_version: &str,
    no_cache: bool,
) -> bool {
    if no_cache || package_version.is_empty() || package_version == "unknown" {
        return false;
    }

    manifest
        .get_package(package_name_full)
        .is_some_and(|pkg| !pkg.plugins.is_empty() && pkg.version.as_ref() == package_version)
}

fn resolve_package_path(
    locator: &PackageLocator,
    package_name_full: &str,
    source_path: Option<&str>,
) -> Result<PathBuf, PluginError> {
    if let Some(source_path) = source_path {
        let candidate = PathBuf::from(source_path);
        if candidate.exists() {
            let resolved = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            logger::debug(&format!(
                "Using source path for '{}': {}",
                package_name_full,
                resolved.display()
            ));
            return Ok(resolved);
        }

        logger::warn(&format!(
            "Source path for '{}' does not exist: {}. Falling back to site-packages.",
            package_name_full, source_path
        ));
    }

    locator.find_package_path(package_name_full).map_err(|e| {
        PluginError::Locator(format!(
            "Failed to locate package '{}': {}",
            package_name_full, e
        ))
    })
}

#[cfg(test)]
mod tests {
    use crate::plugins::discovery::*;
    use r2x_manifest::types::{Manifest, Plugin, PluginType};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn manifest_with_cached_plugin(version: &str) -> Manifest {
        let mut manifest = Manifest::default();
        let package = manifest.get_or_create_package("r2x-reeds");
        package.version = Arc::from(version);
        package.plugins.push(Plugin {
            name: Arc::from("add-pcm-defaults"),
            plugin_type: PluginType::Function,
            module: Arc::from("stale.module"),
            function_name: Some(Arc::from("add_pcm_defaults")),
            ..Plugin::default()
        });
        manifest
    }

    #[test]
    fn should_use_cached_plugins_only_for_matching_versions() {
        let manifest = manifest_with_cached_plugin("0.5.0");

        assert!(should_use_cached_plugins(
            &manifest,
            "r2x-reeds",
            "0.5.0",
            false
        ));
        assert!(!should_use_cached_plugins(
            &manifest,
            "r2x-reeds",
            "0.6.0",
            false
        ));
        assert!(!should_use_cached_plugins(
            &manifest,
            "r2x-reeds",
            "0.5.0",
            true
        ));
        assert!(!should_use_cached_plugins(
            &manifest,
            "r2x-reeds",
            "unknown",
            false
        ));
        assert!(!should_use_cached_plugins(
            &manifest,
            "r2x-reeds",
            "",
            false
        ));
    }

    #[test]
    fn test_resolve_package_path_prefers_source_path() {
        let site_packages = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let locator = match PackageLocator::new(site_packages.path().to_path_buf()) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        let source_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create source dir: {err}"
                );
                return;
            }
        };
        let resolved = match resolve_package_path(&locator, "r2x-reeds", source_dir.path().to_str())
        {
            Ok(resolved) => resolved,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to resolve package path: {err}"
                );
                return;
            }
        };

        let expected = match source_dir.path().canonicalize() {
            Ok(path) => path,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to canonicalize path: {err}"
                );
                return;
            }
        };
        assert_eq!(resolved, expected);
    }
}
