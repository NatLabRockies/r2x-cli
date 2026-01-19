//! Package path resolution for installed packages
//!
//! Handles locating installed packages in virtual environments,
//! including support for UV editable installs via .pth files.

use r2x_python::resolve_site_package_path;
use std::path::PathBuf;

/// Find the path to an installed package (loads config internally)
pub fn find_package_path(package_name_full: &str) -> Result<PathBuf, String> {
    let config = crate::config_manager::Config::load()
        .map_err(|e| format!("Failed to load config: {}", e))?;

    find_package_path_with_venv(package_name_full, &config.get_venv_path())
}

/// Find the path to an installed package using a pre-loaded venv path
///
/// This is more efficient when resolving multiple packages since it avoids
/// reloading the config for each package.
pub fn find_package_path_with_venv(
    package_name_full: &str,
    venv_path: &str,
) -> Result<PathBuf, String> {
    let normalized_package_name = package_name_full.replace('-', "_");

    // First, try to find the package via UV's .pth file cache (for editable/local installs)
    if let Ok(uv_cache_path) = try_find_package_via_pth(&normalized_package_name) {
        return Ok(uv_cache_path);
    }

    // Fallback: search in site-packages (for normally installed packages)
    let venv_path = PathBuf::from(venv_path);

    let site_packages = resolve_site_package_path(&venv_path)
        .map_err(|e| format!("failed to resolve path to python packages: {}", e))?;

    let package_dir = std::fs::read_dir(&site_packages)
        .map_err(|e| format!("Failed to read site-packages: {}", e))?
        .filter_map(|e| e.ok())
        .find(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name == normalized_package_name
                || name.starts_with(&format!("{}-", normalized_package_name))
        })
        .ok_or_else(|| format!("Package '{}' not found in site-packages", package_name_full))?;

    Ok(package_dir.path())
}

/// Find package path via UV's .pth file cache (for editable/local installs)
fn try_find_package_via_pth(normalized_package_name: &str) -> Result<PathBuf, String> {
    // Look for .pth file in UV cache directory
    // Pattern: ~/.cache/uv/archive-v0/<hash>/<package_name>.pth
    let cache_dir = if let Some(home) = dirs::home_dir() {
        home.join(".cache").join("uv").join("archive-v0")
    } else {
        return Err("Could not determine home directory".to_string());
    };

    if !cache_dir.exists() {
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
        // UV creates files like: __editable__.{package_name}-{version}.pth
        let pth_entries = match std::fs::read_dir(&hash_path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for pth_entry in pth_entries {
            let pth_entry = match pth_entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let pth_file_name = pth_entry.file_name().to_string_lossy().to_string();

            // Match both patterns:
            // 1. {package_name}.pth (older pattern)
            // 2. __editable__.{package_name}-{version}.pth (UV editable installs)
            let matches = pth_file_name == format!("{}.pth", normalized_package_name)
                || (pth_file_name.starts_with("__editable__.")
                    && pth_file_name.contains(&format!("{}-", normalized_package_name))
                    && pth_file_name.ends_with(".pth"));

            if matches {
                // Read the path from the .pth file
                match std::fs::read_to_string(pth_entry.path()) {
                    Ok(content) => {
                        let package_path = content.trim();
                        if !package_path.is_empty() {
                            return Ok(PathBuf::from(package_path));
                        }
                    }
                    Err(_) => continue,
                }
            }
        }
    }

    Err(format!(
        "Package '{}' not found in UV cache",
        normalized_package_name
    ))
}
