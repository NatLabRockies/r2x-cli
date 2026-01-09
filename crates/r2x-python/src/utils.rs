//! Utility constants and functions for platform-specific Python venv path handling
//!
//! This module provides compile-time constants for directories and files that differ
//! between Windows and Unix-like systems in Python virtual environments.

use super::errors::BridgeError;
use r2x_logger as logger;
use std::fs;
use std::path::PathBuf;

/// The name of the library directory in a Python venv (e.g., "Lib" on Windows, "lib" on Unix)
#[cfg(windows)]
pub const PYTHON_LIB_DIR: &str = "Lib";
#[cfg(unix)]
pub const PYTHON_LIB_DIR: &str = "lib";

/// The name of the binaries/scripts directory in a Python venv (e.g., "Scripts" on Windows, "bin" on Unix)
#[cfg(windows)]
const PYTHON_BIN_DIR: &str = "Scripts";
#[cfg(unix)]
const PYTHON_BIN_DIR: &str = "bin";

/// Candidate executable names in a venv
#[cfg(unix)]
const PYTHON_EXE_CANDIDATES: &[&str] = &["python3", "python"];
#[cfg(windows)]
const PYTHON_EXE_CANDIDATES: &[&str] = &["python.exe", "python3.exe", "python3.12.exe"];

// Site Packages differences.
//
// MacOS
// .venv/lib/python {version}/site-packages
//
// Windows
// .venv/Lib/site-packages

pub fn resolve_site_package_path(venv_path: &PathBuf) -> Result<PathBuf, BridgeError> {
    logger::debug(&format!(
        "Resolving site-packages path for venv: {}",
        venv_path.display()
    ));

    // Verify the venv_path exists and is a directory.
    if !venv_path.is_dir() {
        logger::debug(&format!(
            "Venv path does not exist or is not a directory: {}",
            venv_path.display()
        ));
        return Err(BridgeError::VenvNotFound(venv_path.to_path_buf()));
    }

    #[cfg(windows)]
    {
        let site_packages = venv_path.join(PYTHON_LIB_DIR).join("site-packages");
        logger::debug(&format!(
            "Windows: Looking for site-packages at: {}",
            site_packages.display()
        ));

        // verify site_package_path exists
        if !site_packages.is_dir() {
            logger::debug(&format!(
                "Windows: site-packages directory not found at: {}",
                site_packages.display()
            ));
            return Err(BridgeError::Initialization(format!(
                "unable to locate package directory: {}",
                site_packages.display()
            )));
        }
        logger::debug(&format!(
            "Windows: Successfully resolved site-packages: {}",
            site_packages.display()
        ));
        Ok(site_packages)
    }

    #[cfg(not(windows))]
    {
        let lib_dir = venv_path.join(PYTHON_LIB_DIR);
        logger::debug(&format!(
            "Unix: Looking for lib directory at: {}",
            lib_dir.display()
        ));

        if !lib_dir.is_dir() {
            logger::debug(&format!(
                "Unix: lib directory not found at: {}",
                lib_dir.display()
            ));
            return Err(BridgeError::Initialization(format!(
                "unable to locate lib directory: {}",
                lib_dir.display()
            )));
        }

        let python_version_dir = fs::read_dir(&lib_dir)
            .map_err(|e| {
                logger::debug(&format!("Unix: Failed to read lib directory: {}", e));
                BridgeError::Initialization(format!("Failed to read lib directory: {}", e))
            })?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("python"))
            .ok_or_else(|| {
                logger::debug("Unix: No python3.X directory found in venv/lib");
                BridgeError::Initialization("No python3.X directory found in venv/lib".to_string())
            })?;

        logger::debug(&format!(
            "Unix: Found python version directory: {}",
            python_version_dir.path().display()
        ));

        let site_packages = python_version_dir.path().join("site-packages");
        logger::debug(&format!(
            "Unix: Looking for site-packages at: {}",
            site_packages.display()
        ));

        if !site_packages.is_dir() {
            logger::debug(&format!(
                "Unix: site-packages directory not found at: {}",
                site_packages.display()
            ));
            return Err(BridgeError::Initialization(format!(
                "unable to locate package directory: {}",
                site_packages.display()
            )));
        }

        logger::debug(&format!(
            "Unix: Successfully resolved site-packages: {}",
            site_packages.display()
        ));
        Ok(site_packages)
    }
}

pub fn resolve_python_path(venv_path: &PathBuf) -> Result<PathBuf, BridgeError> {
    // validate venv path is a valid directory
    if !venv_path.is_dir() {
        return Err(BridgeError::VenvNotFound(venv_path.to_path_buf()));
    }

    let bin_dir = venv_path.join(PYTHON_BIN_DIR);
    if !bin_dir.is_dir() {
        return Err(BridgeError::Initialization(format!(
            "Python bin directory missing: {}",
            bin_dir.display()
        )));
    }

    for exe in PYTHON_EXE_CANDIDATES {
        let candidate = bin_dir.join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    if let Ok(entries) = fs::read_dir(&bin_dir) {
        if let Some(candidate) = entries.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|name| name.contains("python"))
                .unwrap_or(false)
                && p.is_file()
        }) {
            return Ok(candidate);
        }
    }

    Err(BridgeError::Initialization(format!(
        "Path to python binary is not valid in {}",
        venv_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a mock venv structure for testing
    #[allow(dead_code)]
    fn create_mock_venv_unix(python_version: &str) -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path();

        // Create Unix structure: .venv/lib/python3.X/site-packages
        let lib_dir = venv_path.join("lib");
        let python_dir = lib_dir.join(python_version);
        let site_packages = python_dir.join("site-packages");
        fs::create_dir_all(&site_packages).unwrap();

        // Create bin directory with python executable
        let bin_dir = venv_path.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("python3"), "").unwrap();

        temp_dir
    }

    /// Helper to create a mock Windows venv structure for testing
    #[allow(dead_code)]
    fn create_mock_venv_windows() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path();

        // Create Windows structure: .venv/Lib/site-packages
        let lib_dir = venv_path.join("Lib");
        let site_packages = lib_dir.join("site-packages");
        fs::create_dir_all(&site_packages).unwrap();

        // Create Scripts directory with python executable
        let scripts_dir = venv_path.join("Scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(scripts_dir.join("python.exe"), "").unwrap();

        temp_dir
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_site_package_path_unix() {
        let temp_venv = create_mock_venv_unix("python3.12");
        let venv_path = temp_venv.path().to_path_buf();

        let result = resolve_site_package_path(&venv_path);
        assert!(result.is_ok());

        let site_packages = result.unwrap();
        assert!(site_packages.ends_with("lib/python3.12/site-packages"));
        assert!(site_packages.exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_site_package_path_unix_different_version() {
        let temp_venv = create_mock_venv_unix("python3.11");
        let venv_path = temp_venv.path().to_path_buf();

        let result = resolve_site_package_path(&venv_path);
        assert!(result.is_ok());

        let site_packages = result.unwrap();
        assert!(site_packages.ends_with("lib/python3.11/site-packages"));
    }

    #[test]
    #[cfg(windows)]
    fn test_resolve_site_package_path_windows() {
        let temp_venv = create_mock_venv_windows();
        let venv_path = temp_venv.path().to_path_buf();

        let result = resolve_site_package_path(&venv_path);
        assert!(result.is_ok());

        let site_packages = result.unwrap();
        assert!(site_packages.ends_with("Lib\\site-packages"));
        assert!(site_packages.exists());
    }

    #[test]
    fn test_resolve_site_package_path_venv_not_found() {
        let non_existent_path = PathBuf::from("/tmp/non_existent_venv_12345");

        let result = resolve_site_package_path(&non_existent_path);
        assert!(result.is_err());

        match result {
            Err(BridgeError::VenvNotFound(path)) => {
                assert_eq!(path, non_existent_path);
            }
            _ => panic!("Expected VenvNotFound error"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_site_package_path_missing_python_dir() {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path();

        // Create lib dir but no python3.X subdirectory
        let lib_dir = venv_path.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();

        let result = resolve_site_package_path(&venv_path.to_path_buf());
        assert!(result.is_err());

        match result {
            Err(BridgeError::Initialization(msg)) => {
                assert!(msg.contains("No python3.X directory found"));
            }
            _ => panic!("Expected Initialization error"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_python_path_unix() {
        let temp_venv = create_mock_venv_unix("python3.12");
        let venv_path = temp_venv.path().to_path_buf();

        let result = resolve_python_path(&venv_path);
        assert!(result.is_ok());

        let python_path = result.unwrap();
        assert!(python_path.ends_with("bin/python3"));
    }

    #[test]
    #[cfg(windows)]
    fn test_resolve_python_path_windows() {
        let temp_venv = create_mock_venv_windows();
        let venv_path = temp_venv.path().to_path_buf();

        let result = resolve_python_path(&venv_path);
        assert!(result.is_ok());

        let python_path = result.unwrap();
        assert!(python_path.ends_with("Scripts\\python.exe"));
    }

    #[test]
    fn test_python_lib_dir_constant() {
        // Test that the compile-time constant is correct for the platform
        #[cfg(unix)]
        assert_eq!(PYTHON_LIB_DIR, "lib");

        #[cfg(windows)]
        assert_eq!(PYTHON_LIB_DIR, "Lib");
    }

    #[test]
    fn test_python_bin_dir_constant() {
        // Test that the compile-time constant is correct for the platform
        #[cfg(unix)]
        assert_eq!(PYTHON_BIN_DIR, "bin");

        #[cfg(windows)]
        assert_eq!(PYTHON_BIN_DIR, "Scripts");
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_site_package_path_with_multiple_python_versions() {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path();

        // Create lib dir with multiple python versions
        let lib_dir = venv_path.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();

        // Create python3.11
        let python_311 = lib_dir.join("python3.11");
        let site_packages_311 = python_311.join("site-packages");
        fs::create_dir_all(&site_packages_311).unwrap();

        // Create python3.12 (should find the first one)
        let python_312 = lib_dir.join("python3.12");
        let site_packages_312 = python_312.join("site-packages");
        fs::create_dir_all(&site_packages_312).unwrap();

        let result = resolve_site_package_path(&venv_path.to_path_buf());
        assert!(result.is_ok());

        let site_packages = result.unwrap();
        // Should find one of them (implementation finds first match)
        assert!(site_packages.to_string_lossy().contains("python3.1"));
        assert!(site_packages.ends_with("site-packages"));
    }
}
