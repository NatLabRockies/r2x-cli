//! Package path resolution for installed packages
//!
//! Handles locating installed packages in virtual environments,
//! including support for UV editable installs via .pth files.

use std::path::PathBuf;

/// Find the path to an installed package
pub fn find_package_path(package_name_full: &str) -> Result<PathBuf, String> {
    let config = crate::config_manager::Config::load()
        .map_err(|e| format!("Failed to load config: {}", e))?;

    let normalized_package_name = package_name_full.replace('-', "_");

    // First, try to find the package via UV's .pth file cache (for editable/local installs)
    if let Ok(uv_cache_path) = try_find_package_via_pth(&normalized_package_name) {
        return Ok(uv_cache_path);
    }

    // Fallback: search in site-packages (for normally installed packages)
    let venv_path = PathBuf::from(config.get_venv_path());
    let lib_dir = venv_path.join("lib");

    let python_version_dir = std::fs::read_dir(&lib_dir)
        .map_err(|e| format!("Failed to read lib directory: {}", e))?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("python"))
        .ok_or_else(|| "No python directory found in venv".to_string())?;

    let site_packages = python_version_dir.path().join("site-packages");

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
        let pth_path = hash_path.join(format!("{}.pth", normalized_package_name));
        if pth_path.exists() {
            // Read the path from the .pth file
            match std::fs::read_to_string(&pth_path) {
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

    Err(format!(
        "Package '{}' not found in UV cache",
        normalized_package_name
    ))
}
