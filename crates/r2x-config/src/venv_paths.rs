//! Utility functions for resolving paths in Python virtual environments
//!
//! This module provides platform-specific path resolution for:
//! - site-packages directory
//! - Python executable
//!
//! These utilities are used by multiple crates (r2x-ast, r2x-python) to avoid
//! circular dependencies.

use std::fs;
use std::path::{Path, PathBuf};

/// The name of the library directory in a Python venv
/// "Lib" on Windows, "lib" on Unix
#[cfg(windows)]
pub const PYTHON_LIB_DIR: &str = "Lib";
#[cfg(not(windows))]
pub const PYTHON_LIB_DIR: &str = "lib";

/// The name of the binaries/scripts directory in a Python venv
/// "Scripts" on Windows, "bin" on Unix
#[cfg(windows)]
pub const PYTHON_BIN_DIR: &str = "Scripts";
#[cfg(not(windows))]
pub const PYTHON_BIN_DIR: &str = "bin";

/// Candidate executable names in a venv
#[cfg(not(windows))]
const PYTHON_EXE_CANDIDATES: &[&str] = &["python3", "python"];
#[cfg(windows)]
const PYTHON_EXE_CANDIDATES: &[&str] = &["python.exe", "python3.exe", "python3.12.exe"];

/// Error type for venv path resolution
#[derive(Debug, Clone)]
pub enum VenvPathError {
    /// The venv path does not exist or is not a directory
    VenvNotFound(PathBuf),
    /// Failed to find a required directory or file
    PathResolution(String),
}

impl std::fmt::Display for VenvPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VenvPathError::VenvNotFound(path) => {
                write!(f, "Virtual environment not found: {}", path.display())
            }
            VenvPathError::PathResolution(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for VenvPathError {}

/// Resolve the site-packages path for a Python virtual environment
///
/// # Platform differences
///
/// - **Unix/macOS**: `.venv/lib/python3.X/site-packages`
/// - **Windows**: `.venv/Lib/site-packages`
///
/// # Arguments
///
/// * `venv_path` - Path to the root of the virtual environment
///
/// # Returns
///
/// The path to the site-packages directory, or an error if not found
pub fn resolve_site_packages(venv_path: &Path) -> Result<PathBuf, VenvPathError> {
    if !venv_path.is_dir() {
        return Err(VenvPathError::VenvNotFound(venv_path.to_path_buf()));
    }

    #[cfg(windows)]
    {
        let site_packages = venv_path.join(PYTHON_LIB_DIR).join("site-packages");
        if !site_packages.is_dir() {
            return Err(VenvPathError::PathResolution(format!(
                "site-packages not found: {}",
                site_packages.display()
            )));
        }
        Ok(site_packages)
    }

    #[cfg(not(windows))]
    {
        let lib_dir = venv_path.join(PYTHON_LIB_DIR);
        if !lib_dir.is_dir() {
            return Err(VenvPathError::PathResolution(format!(
                "lib directory not found: {}",
                lib_dir.display()
            )));
        }

        // Find the python version directory (e.g., python3.12)
        let python_version_dir = fs::read_dir(&lib_dir)
            .map_err(|e| VenvPathError::PathResolution(format!("Failed to read lib dir: {}", e)))?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("python"))
            .ok_or_else(|| {
                VenvPathError::PathResolution(
                    "No python3.X directory found in venv/lib".to_string(),
                )
            })?;

        let site_packages = python_version_dir.path().join("site-packages");
        if !site_packages.is_dir() {
            return Err(VenvPathError::PathResolution(format!(
                "site-packages not found: {}",
                site_packages.display()
            )));
        }

        Ok(site_packages)
    }
}

/// Resolve the Python executable path for a virtual environment
///
/// # Platform differences
///
/// - **Unix/macOS**: `.venv/bin/python3` or `.venv/bin/python`
/// - **Windows**: `.venv/Scripts/python.exe`
///
/// # Arguments
///
/// * `venv_path` - Path to the root of the virtual environment
///
/// # Returns
///
/// The path to the Python executable, or an error if not found
pub fn resolve_python_exe(venv_path: &Path) -> Result<PathBuf, VenvPathError> {
    if !venv_path.is_dir() {
        return Err(VenvPathError::VenvNotFound(venv_path.to_path_buf()));
    }

    let bin_dir = venv_path.join(PYTHON_BIN_DIR);
    if !bin_dir.is_dir() {
        return Err(VenvPathError::PathResolution(format!(
            "bin directory not found: {}",
            bin_dir.display()
        )));
    }

    // Try standard executable names first
    for exe in PYTHON_EXE_CANDIDATES {
        let candidate = bin_dir.join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // Fallback: search for any python-like executable
    if let Ok(entries) = fs::read_dir(&bin_dir) {
        if let Some(candidate) = entries.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.contains("python"))
                && p.is_file()
        }) {
            return Ok(candidate);
        }
    }

    Err(VenvPathError::PathResolution(format!(
        "Python executable not found in {}",
        bin_dir.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[cfg(not(windows))]
    fn create_mock_venv_unix(python_version: &str) -> Option<TempDir> {
        let temp_dir = TempDir::new().ok()?;
        let venv_path = temp_dir.path();

        // Create Unix structure: .venv/lib/python3.X/site-packages
        let lib_dir = venv_path.join("lib");
        let python_dir = lib_dir.join(python_version);
        let site_packages = python_dir.join("site-packages");
        fs::create_dir_all(&site_packages).ok()?;

        // Create bin directory with python executable
        let bin_dir = venv_path.join("bin");
        fs::create_dir_all(&bin_dir).ok()?;
        fs::write(bin_dir.join("python3"), "").ok()?;

        Some(temp_dir)
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_site_packages_unix() {
        let Some(temp_venv) = create_mock_venv_unix("python3.12") else {
            return;
        };
        let result = resolve_site_packages(temp_venv.path());
        assert!(result.is_ok(), "Failed to resolve site packages");
        assert!(result.is_ok_and(|p| p.ends_with("lib/python3.12/site-packages")));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_python_exe_unix() {
        let Some(temp_venv) = create_mock_venv_unix("python3.12") else {
            return;
        };
        let result = resolve_python_exe(temp_venv.path());
        assert!(result.is_ok(), "Failed to resolve python exe");
        assert!(result.is_ok_and(|p| p.ends_with("bin/python3")));
    }

    #[test]
    fn test_venv_not_found() {
        let non_existent = PathBuf::from("/tmp/non_existent_venv_12345");
        let result = resolve_site_packages(&non_existent);
        assert!(matches!(result, Err(VenvPathError::VenvNotFound(_))));
    }

    #[test]
    fn test_platform_constants() {
        #[cfg(not(windows))]
        {
            assert_eq!(PYTHON_LIB_DIR, "lib");
            assert_eq!(PYTHON_BIN_DIR, "bin");
        }
        #[cfg(windows)]
        {
            assert_eq!(PYTHON_LIB_DIR, "Lib");
            assert_eq!(PYTHON_BIN_DIR, "Scripts");
        }
    }
}
