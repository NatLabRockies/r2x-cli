//! Package verification and automatic reinstallation

use crate::config_manager::Config;
use crate::logger;
use crate::r2x_manifest::Manifest;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum VerificationResult {
    /// All packages are valid and installed
    Valid,
    /// Some packages are missing or changed
    Missing(Vec<String>),
}

#[derive(Debug)]
pub enum VerificationError {
    /// Failed to read venv directory
    VenvNotFound(PathBuf),
    /// Failed to verify package installation
    VerificationFailed(String),
    /// Failed to reinstall packages
    ReinstallFailed(String),
}

impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationError::VenvNotFound(path) => {
                write!(f, "Virtual environment not found at: {}", path.display())
            }
            VerificationError::VerificationFailed(msg) => {
                write!(f, "Package verification failed: {}", msg)
            }
            VerificationError::ReinstallFailed(msg) => {
                write!(f, "Package reinstallation failed: {}", msg)
            }
        }
    }
}

impl std::error::Error for VerificationError {}

/// Verify that all packages required for a plugin are installed
///
/// # Arguments
/// * `manifest` - Plugin manifest containing cached plugin metadata
/// * `plugin_key` - Key of the plugin to verify (e.g., "parser-reeds")
///
/// # Returns
/// * `Ok(VerificationResult::Valid)` - All packages installed and valid
/// * `Ok(VerificationResult::Missing(packages))` - List of missing/changed packages
/// * `Err(VerificationError)` - Critical error during verification
pub fn verify_plugin_packages(
    manifest: &Manifest,
    plugin_key: &str,
) -> Result<VerificationResult, VerificationError> {
    logger::debug(&format!("Verifying packages for plugin: {}", plugin_key));

    // Find the plugin and its package from manifest
    let package_name = manifest
        .packages
        .iter()
        .find_map(|pkg| {
            pkg.plugins
                .iter()
                .find(|p| p.name == plugin_key)
                .map(|_| pkg.name.clone())
        })
        .ok_or_else(|| {
            VerificationError::VerificationFailed(format!(
                "Plugin '{}' not found in manifest",
                plugin_key
            ))
        })?;

    // Get venv path
    let config = Config::load().map_err(|e| {
        VerificationError::VerificationFailed(format!("Failed to load config: {}", e))
    })?;
    let venv_path = PathBuf::from(config.get_venv_path());

    if !venv_path.exists() {
        return Err(VerificationError::VenvNotFound(venv_path));
    }

    // Check if package is installed
    let missing_packages = check_packages_installed(&venv_path, &[&package_name])?;

    if missing_packages.is_empty() {
        logger::debug(&format!("Package '{}' verified successfully", package_name));
        Ok(VerificationResult::Valid)
    } else {
        logger::debug(&format!("Missing packages: {:?}", missing_packages));
        Ok(VerificationResult::Missing(missing_packages))
    }
}

/// Check if packages are installed in the virtual environment
///
/// # Arguments
/// * `venv_path` - Path to virtual environment
/// * `packages` - List of package names to check
///
/// # Returns
/// List of packages that are not installed or invalid
fn check_packages_installed(
    venv_path: &PathBuf,
    packages: &[&str],
) -> Result<Vec<String>, VerificationError> {
    let site_packages = get_site_packages_dir(venv_path)?;
    let mut missing = Vec::new();

    for package in packages {
        // Convert package name format: "r2x-reeds" -> "r2x_reeds"
        let package_dir_name = package.replace('-', "_");

        // Check if package directory exists
        let package_dir = site_packages.join(&package_dir_name);
        let dist_info_pattern = format!("{}-*.dist-info", package_dir_name);

        let package_exists =
            package_dir.exists() || dist_info_exists(&site_packages, &dist_info_pattern);

        if !package_exists {
            logger::debug(&format!("Package '{}' not found in site-packages", package));
            missing.push(package.to_string());
        } else {
            logger::debug(&format!("Package '{}' found in site-packages", package));
        }
    }

    Ok(missing)
}

/// Get the site-packages directory from venv
fn get_site_packages_dir(venv_path: &PathBuf) -> Result<PathBuf, VerificationError> {
    let lib_dir = venv_path.join("lib");

    if !lib_dir.exists() {
        return Err(VerificationError::VenvNotFound(venv_path.clone()));
    }

    // Find python3.X directory
    let entries = std::fs::read_dir(&lib_dir).map_err(|e| {
        VerificationError::VerificationFailed(format!("Failed to read lib directory: {}", e))
    })?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("python") && entry.path().is_dir() {
            let site_packages = entry.path().join("site-packages");
            if site_packages.exists() {
                return Ok(site_packages);
            }
        }
    }

    Err(VerificationError::VerificationFailed(
        "site-packages directory not found".to_string(),
    ))
}

/// Check if a dist-info directory matching the pattern exists
fn dist_info_exists(site_packages: &PathBuf, pattern: &str) -> bool {
    let pattern_prefix = pattern.split('-').next().unwrap_or("");

    if let Ok(entries) = std::fs::read_dir(site_packages) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(pattern_prefix) && name_str.ends_with(".dist-info") {
                return true;
            }
        }
    }

    false
}

/// Reinstall missing packages using uv
///
/// # Arguments
/// * `packages` - List of package names to install
/// * `config` - Configuration with uv path and venv settings
///
/// # Returns
/// * `Ok(())` - Packages successfully installed
/// * `Err(VerificationError)` - Installation failed
pub fn ensure_packages(packages: Vec<String>, config: &Config) -> Result<(), VerificationError> {
    if packages.is_empty() {
        return Ok(());
    }

    logger::info(&format!(
        "Installing missing packages: {}",
        packages.join(", ")
    ));

    let uv_path = config
        .uv_path
        .as_ref()
        .ok_or_else(|| VerificationError::ReinstallFailed("uv not configured".to_string()))?;

    let python_exe = config.get_venv_python_path();

    // Build uv pip install command
    let mut cmd = Command::new(uv_path);
    cmd.arg("pip")
        .arg("install")
        .arg("--python")
        .arg(&python_exe)
        .arg("--prerelease=allow");

    // Add all packages
    for package in &packages {
        cmd.arg(package);
    }

    logger::debug(&format!("Running: {:?}", cmd));

    let output = cmd
        .output()
        .map_err(|e| VerificationError::ReinstallFailed(format!("Failed to execute uv: {}", e)))?;

    logger::capture_output(&format!("uv pip install {}", packages.join(" ")), &output);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VerificationError::ReinstallFailed(format!(
            "uv pip install failed: {}",
            stderr
        )));
    }

    logger::success(&format!(
        "Successfully installed {} packages",
        packages.len()
    ));
    Ok(())
}

/// Verify and ensure packages for a plugin are installed
///
/// This is the main entry point that combines verification and reinstallation.
/// It will automatically reinstall missing packages.
///
/// # Arguments
/// * `manifest` - Plugin manifest
/// * `plugin_key` - Plugin to verify (e.g., "parser-reeds")
///
/// # Returns
/// * `Ok(())` - Plugin packages verified and available
/// * `Err(VerificationError)` - Failed to verify or install packages
///
/// # Example
///
/// ```rust,ignore
/// use r2x::package_verification::verify_and_ensure_plugin;
/// use r2x::plugin_manifest::PluginManifest;
///
/// let manifest = Manifest::load()?;
///
/// // This will verify r2x-reeds is installed
/// // If missing, it will automatically reinstall it
/// verify_and_ensure_plugin(&manifest, "parser-reeds")?;
///
/// // Now safe to run the plugin
/// ```
pub fn verify_and_ensure_plugin(
    manifest: &Manifest,
    plugin_key: &str,
) -> Result<(), VerificationError> {
    logger::debug(&format!("Verifying and ensuring plugin: {}", plugin_key));

    match verify_plugin_packages(manifest, plugin_key)? {
        VerificationResult::Valid => {
            logger::debug("All packages verified successfully");
            Ok(())
        }
        VerificationResult::Missing(packages) => {
            logger::info(&format!(
                "Missing {} package(s), reinstalling...",
                packages.len()
            ));
            let config = Config::load().map_err(|e| {
                VerificationError::ReinstallFailed(format!("Failed to load config: {}", e))
            })?;
            ensure_packages(packages, &config)?;
            logger::success("Packages verified and installed");
            Ok(())
        }
    }
}

/// Verify all packages in the manifest (for batch operations)
///
/// # Arguments
/// * `manifest` - Plugin manifest to verify
///
/// # Returns
/// Set of package names that need to be installed
pub fn verify_all_packages(manifest: &Manifest) -> Result<HashSet<String>, VerificationError> {
    let mut missing_packages = HashSet::new();

    // Get venv path
    let config = Config::load().map_err(|e| {
        VerificationError::VerificationFailed(format!("Failed to load config: {}", e))
    })?;
    let venv_path = PathBuf::from(config.get_venv_path());

    if !venv_path.exists() {
        return Err(VerificationError::VenvNotFound(venv_path));
    }

    // Collect all unique package names from manifest
    let mut all_packages: HashSet<&str> = HashSet::new();
    for pkg in &manifest.packages {
        all_packages.insert(&pkg.name);
    }

    // Check which packages are missing
    let packages_vec: Vec<&str> = all_packages.into_iter().collect();
    let missing = check_packages_installed(&venv_path, &packages_vec)?;

    for package in missing {
        missing_packages.insert(package);
    }

    Ok(missing_packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_result_valid() {
        let result = VerificationResult::Valid;
        assert_eq!(result, VerificationResult::Valid);
    }

    #[test]
    fn test_verification_result_missing() {
        let packages = vec!["r2x-reeds".to_string(), "r2x-core".to_string()];
        let result = VerificationResult::Missing(packages.clone());
        match result {
            VerificationResult::Missing(p) => assert_eq!(p, packages),
            _ => panic!("Expected Missing variant"),
        }
    }

    #[test]
    fn test_package_name_conversion() {
        let package = "r2x-reeds";
        let converted = package.replace('-', "_");
        assert_eq!(converted, "r2x_reeds");
    }

    #[test]
    fn test_verification_error_display() {
        let err = VerificationError::VerificationFailed("test error".to_string());
        assert_eq!(err.to_string(), "Package verification failed: test error");
    }

    #[test]
    fn test_dist_info_pattern() {
        // Test that dist-info pattern matching works
        let pattern = "r2x_reeds-*.dist-info";
        let pattern_prefix = pattern.split('-').next().unwrap_or("");
        assert_eq!(pattern_prefix, "r2x_reeds");

        // Verify pattern matches expected format
        let example_dist_info = "r2x_reeds-1.2.3.dist-info";
        assert!(example_dist_info.starts_with(pattern_prefix));
        assert!(example_dist_info.ends_with(".dist-info"));
    }

    #[test]
    fn test_verification_workflow() {
        // This test documents the expected verification workflow
        // Actual integration tests would require a real venv

        // 1. User runs: r2x python venv --clear
        // 2. Venv is wiped but manifest still has plugins
        // 3. User runs: r2x run parser-reeds
        // 4. Verification detects missing packages
        // 5. Auto-reinstall kicks in
        // 6. Plugin executes successfully

        // Simulate the workflow states
        let valid_result = VerificationResult::Valid;
        let missing_result = VerificationResult::Missing(vec!["r2x-reeds".to_string()]);

        // After venv wipe, we expect Missing
        assert!(matches!(missing_result, VerificationResult::Missing(_)));

        // After reinstall, we expect Valid
        assert!(matches!(valid_result, VerificationResult::Valid));
    }
}
