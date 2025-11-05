use crate::config_manager::Config;
use crate::logger;
use crate::plugin_cache::{CachedPackage, CachedPlugin, PluginMetadataCache};
use crate::plugin_manifest::PluginManifest;
use crate::plugins::AstDiscovery;
use std::path::PathBuf;

/// Options for plugin discovery and registration
pub struct DiscoveryOptions {
    pub package: String,
    pub package_name_full: String,
    pub dependencies: Vec<String>,
    pub package_version: Option<String>,
    pub no_cache: bool,
}

pub fn discover_and_register_entry_points_with_deps(
    _uv_path: &str,
    _python_path: &str,
    opts: DiscoveryOptions,
) -> Result<usize, String> {
    let package = &opts.package;
    let package_name_full = &opts.package_name_full;
    let dependencies = &opts.dependencies;
    let no_cache = opts.no_cache;

    // Get venv path from config for entry_points.txt lookup
    let venv_path = crate::config_manager::Config::load()
        .ok()
        .map(|c| c.get_venv_path());

    logger::debug(&format!("Registering plugins from package: '{}'", package));

    // Extract short name for entry point lookup (e.g., "reeds" from "r2x-reeds")
    let package_short_name = if package_name_full.starts_with("r2x-") {
        package_name_full.trim_start_matches("r2x-")
    } else {
        package_name_full
    };

    logger::debug(&format!(
        "Full package name: {}, short name: {}",
        package_name_full, package_short_name
    ));

    // Quick check: verify entry_points.txt exists before initializing Python bridge
    // This avoids 1.9s+ Python initialization for packages without plugins
    let has_entry_points = check_entry_points_exists(package_name_full);

    if !has_entry_points {
        logger::debug(&format!(
            "No entry_points.txt found for {} - skipping plugin load",
            package_name_full
        ));
        return Ok(0);
    }

    // Load or create manifest early to check cache
    let mut manifest = match PluginManifest::load() {
        Ok(m) => m,
        Err(e) => {
            logger::warn(&format!("Failed to load manifest: {}", e));
            PluginManifest::default()
        }
    };

    // Check if we already have plugins for this package in the manifest
    let existing_plugins: Vec<String> = manifest
        .plugins
        .iter()
        .filter(|(_, plugin)| plugin.package_name.as_deref() == Some(package_name_full))
        .map(|(key, _)| key.clone())
        .collect();

    // Check metadata cache with version-aware lookup
    let plugin_entries = if !existing_plugins.is_empty() {
        logger::debug(&format!(
            "Found {} plugin(s) in active manifest for '{}', reusing",
            existing_plugins.len(),
            package_name_full
        ));
        // Use existing plugins from manifest instead of reloading
        existing_plugins
            .iter()
            .filter_map(|key| manifest.plugins.get(key).map(|p| (key.clone(), p.clone())))
            .collect()
    } else {
        if no_cache {
            logger::debug(&format!(
                "⊘ Cache skipped (--no-cache): Loading '{}@{}' via AST",
                package_name_full, package_version
            ));

            PluginMetadataCache::extract_plugins(cached_package)
        } else {
            logger::debug(&format!(
                "✗ Cache miss: Loading '{}@{}' via AST",
                package_name_full, package_version
            ));
        }

        let package_path = find_package_path(package_name_full).map_err(|e| {
            format!("Failed to locate package '{}': {}", package_name_full, e)
        })?;

        logger::debug(&format!("Found package at: {}", package_path.display()));

        let json = AstDiscovery::discover_plugins(
            &package_path,
            package_name_full,
            venv_path.as_deref(),
            Some(package_version),
        )
        .map_err(|e| {
            format!(
                "Failed to discover plugins for '{}': {}",
                package_short_name, e
            )
        })?;

        parse_plugin_json(&json, package_name_full).map_err(|e| {
            format!("Failed to parse plugin JSON for '{}': {}", package_short_name, e)
        })?
    };

    let mut total_plugins = plugin_entries.len();

    if total_plugins == 0 {
        logger::warn(&format!(
            "No plugins found in package '{}'",
            package_short_name
        ));
        return Ok(0);
    }

    logger::info(&format!(
        "Found {} plugin(s) in package '{}'",
        total_plugins, package_short_name
    ));

    // Register main package plugins with install_type: "explicit"
    for (key, mut plugin) in plugin_entries {
        plugin.install_type = Some("explicit".to_string());
        plugin.package_name = Some(package_name_full.to_string());
        let _ = manifest.add_plugin(key.clone(), plugin);
        logger::debug(&format!("Registered: {}", key));
    }

    // Register r2x plugin dependencies (dependencies already fetched in parent function)

    if !dependencies.is_empty() {
        let total_deps = dependencies.len();
        logger::debug(&format!(
            "Found {} dependencies for '{}', checking for r2x plugins...",
            total_deps, package
        ));

        // Filter to only check r2x packages (pre-filter for performance)
        let start = std::time::Instant::now();
        let r2x_dependencies: Vec<String> = dependencies
            .iter()
            .filter(|dep| looks_like_r2x_plugin(dep))
            .cloned()
            .collect();
        logger::debug(&format!(
            "Filtering dependencies took: {:?}, result: {} r2x plugins from {} total",
            start.elapsed(),
            r2x_dependencies.len(),
            total_deps
        ));

        if r2x_dependencies.is_empty() {
            logger::debug("No r2x plugin dependencies found");
        } else {
            logger::debug(&format!(
                "Processing {} r2x plugin(s)...",
                r2x_dependencies.len()
            ));

            let mut metadata_cache = PluginMetadataCache::load().unwrap_or_else(|e| {
                logger::debug(&format!(
                    "Failed to load metadata cache for dependencies: {}",
                    e
                ));
                PluginMetadataCache::default()
            });

            for dep in r2x_dependencies {
                let dep_start = std::time::Instant::now();

                // Try metadata cache first (dependencies use "unknown" version), unless --no-cache
                let dep_plugin_entries = if !no_cache
                    && metadata_cache.get_package(&dep, "unknown").is_some()
                {
                    let cached_package = metadata_cache.get_package(&dep, "unknown").unwrap();
                    logger::debug(&format!(
                        "✓ Cache hit: Found {} plugin(s) for '{}' in metadata cache",
                        cached_package.plugins.len(),
                        &dep
                    ));

                    Ok(PluginMetadataCache::extract_plugins(cached_package))
                } else {
                    logger::debug(&format!(
                        "✗ Cache miss: Loading '{}' via AST",
                        &dep
                    ));
                    match find_package_path(&dep) {
                        Ok(dep_path) => {
                            match AstDiscovery::discover_plugins(
                                &dep_path,
                                &dep,
                                venv_path.as_deref(),
                                None, // Dependencies don't have version info
                            ) {
                                Ok(json) => {
                                    match parse_plugin_json(&json, &dep) {
                                        Ok(entries) => entries,
                                        Err(e) => {
                                            logger::warn(&format!(
                                                "Failed to parse plugins from dependency '{}': {}",
                                                &dep, e
                                            ));
                                            Vec::new()
                                        }
                                    }
                                }
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
                        Err(e) => Err(e),
                    }
                };

                if dep_plugin_entries.is_empty() {
                    logger::debug(&format!("No plugins found in dependency '{}'", &dep));
                } else {
                    logger::debug(&format!(
                        "Found {} plugin(s) for '{}' in {:?}",
                        dep_plugin_entries.len(),
                        &dep,
                        dep_start.elapsed()
                    ));

                            for (key, mut plugin) in dep_plugin_entries {
                                plugin.install_type = Some("dependency".to_string());
                                plugin.installed_by = Some(package_name_full.to_string());
                                plugin.package_name = Some(dep.clone());
                                let _ = manifest.add_plugin(key.clone(), plugin);
                                total_plugins += 1;
                                logger::debug(&format!("Registered (dependency): {}", key));
                            }
                        }
                    }
                    Err(e) => {
                        logger::debug(&format!(
                            "Dependency '{}' failed to load as r2x plugin (took {:?}): {}",
                            dep,
                            dep_start.elapsed(),
                            e
                        ));
                    }
                }
            }
        }
    }

    // Save the updated manifest with all plugins (explicit + dependencies)
    manifest
        .save()
        .map_err(|e| format!("Failed to save manifest: {}", e))?;

    Ok(total_plugins)
}

/// Quick check: does the package have an entry_points.txt file?
/// This is a fast file system check to avoid Python bridge initialization
fn find_package_path(package_name_full: &str) -> Result<PathBuf, String> {
    let config = crate::config_manager::Config::load()
        .map_err(|e| format!("Failed to load config: {}", e))?;

    let normalized_package_name = package_name_full.replace('-', "_");

    // First, try to find the package via UV's .pth file cache (for editable/local installs)
    if let Ok(uv_cache_path) = try_find_package_via_pth(&normalized_package_name) {
        logger::debug(&format!(
            "Found package '{}' via UV .pth cache at: {}",
            package_name_full,
            uv_cache_path.display()
        ));
        return Ok(uv_cache_path);
    }

    // Fallback: search in site-packages (for normally installed packages)
    let venv_path = PathBuf::from(config.get_venv_path());

    // Find site-packages directory
    // On Windows: venv\Lib\site-packages
    // On Unix: venv/lib/python3.x/site-packages
    let site_packages_path = if cfg!(windows) {
        venv_path.join("Lib").join("site-packages")
    } else {
        let site_packages = venv_path.join("lib");
        let entries = match fs::read_dir(&site_packages) {
            Ok(e) => e,
            Err(_) => return false,
        };

        let python_version_dir = match entries
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("python"))
        {
            Some(d) => d,
            None => return false,
        };
    let package_dir = std::fs::read_dir(&site_packages)
        .map_err(|e| format!("Failed to read site-packages: {}", e))?
        .filter_map(|e| e.ok())
        .find(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name == normalized_package_name || name.starts_with(&format!("{}-", normalized_package_name))
        })
        .ok_or_else(|| format!("Package '{}' not found in site-packages", package_name_full))?;

    Ok(package_dir.path())
}

fn try_find_package_via_pth(normalized_package_name: &str) -> Result<PathBuf, String> {
    // Look for .pth file in UV cache directory
    // Pattern: ~/.cache/uv/archive-v0/<hash>/<package_name>.pth
    let cache_dir = if let Some(home) = dirs::home_dir() {
        home.join(".cache").join("uv").join("archive-v0")
    } else {
        return Err("Could not determine home directory".to_string());
    };

    if !cache_dir.exists() {
        logger::debug(&format!(
            "UV cache directory does not exist: {}",
            cache_dir.display()
        ));
        return Err("UV cache not found".to_string());
    }

    // Search through all hash directories in the UV cache
    let hash_dirs = std::fs::read_dir(&cache_dir)
        .map_err(|e| format!("Failed to read UV cache directory: {}", e))?;

    for hash_entry in hash_dirs {
        let hash_entry = match hash_entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let hash_path = hash_entry.path();
        if !hash_path.is_dir() {
            continue;
        }

        // Look for .pth file matching the package name
        let pth_path = hash_path.join(format!("{}.pth", normalized_package_name));
        if pth_path.exists() {
            // Read the path from the .pth file
            match std::fs::read_to_string(&pth_path) {
                Ok(content) => {
                    let package_path = content.trim();
                    if !package_path.is_empty() {
                        logger::debug(&format!(
                            "Found .pth file for '{}': {}",
                            normalized_package_name, pth_path.display()
                        ));
                        logger::debug(&format!(
                            "Package path from .pth: {}",
                            package_path
                        ));
                        return Ok(PathBuf::from(package_path));
                    }
                }
                Err(e) => {
                    logger::debug(&format!(
                        "Failed to read .pth file at {}: {}",
                        pth_path.display(),
                        e
                    ));
                }
            }
        }
    }

    Err(format!(
        "Package '{}' not found in UV cache",
        normalized_package_name
    ))
}

fn parse_plugin_json(json: &str, package_name_full: &str) -> Result<Vec<(String, crate::plugin_manifest::Plugin)>, String> {
    let package: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse plugin JSON: {}", e))?;

    let mut plugins = Vec::new();

    if let Some(plugins_array) = package.get("plugins").and_then(|p| p.as_array()) {
        for plugin_obj in plugins_array {
            let plugin_name = plugin_obj
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| "Plugin missing 'name' field".to_string())?;

            let plugin_type = plugin_obj
                .get("plugin_type")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            let io_type = plugin_obj
                .get("io_type")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            let call_method = plugin_obj
                .get("call_method")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());

            let obj = plugin_obj.get("obj").cloned();
            let config = plugin_obj.get("config").cloned();
            let requires_store = plugin_obj.get("requires_store").and_then(|r| r.as_bool());

            let plugin = crate::plugin_manifest::Plugin {
                package_name: Some(package_name_full.to_string()),
                package_version: None,
                cached_at: None,
                plugin_type,
                description: None,
                doc: None,
                io_type,
                call_method,
                requires_store,
                obj: obj.and_then(|o| {
                    if o.is_null() {
                        None
                    } else {
                        serde_json::from_value::<crate::plugin_manifest::CallableMetadata>(o).ok()
                    }
                }),
                config: config.and_then(|c| {
                    if c.is_null() {
                        None
                    } else {
                        serde_json::from_value::<crate::plugin_manifest::ConfigMetadata>(c).ok()
                    }
                }),
                upgrader: None,
                install_type: None,
                installed_by: None,
            };

            logger::debug(&format!("Parsed plugin: {}", plugin_name));
            plugins.push((plugin_name.to_string(), plugin));
        }
    }

    false
}

/// Check if a package name looks like it could be an r2x plugin package.
/// This is a fast pre-filter before attempting expensive Python bridge calls.
fn looks_like_r2x_plugin(package_name: &str) -> bool {
    // Only check packages that start with "r2x-" but skip infrastructure packages
    // that are never plugins
    if !package_name.starts_with("r2x-") {
        return false;
    }

    // Skip known infrastructure/dependency packages
    match package_name {
        "r2x-core" => false, // Core infrastructure, not a plugin
        "chronify" => false, // Time series dependency
        "infrasys" => false, // Infrastructure systems dependency
        "plexosdb" => false, // PLEXOS database dependency
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_entry_points_exists() {
        // Test entry points file detection
    }

    #[test]
    fn test_looks_like_r2x_plugin() {
        assert!(looks_like_r2x_plugin("r2x-reeds"));
        assert!(looks_like_r2x_plugin("r2x-plexos"));
        assert!(!looks_like_r2x_plugin("r2x-core"));
        assert!(!looks_like_r2x_plugin("numpy"));
        assert!(!looks_like_r2x_plugin("pandas"));
    }

    #[test]
    fn test_discover_and_register() {
        // Test plugin discovery and registration
    }
}
