//! Utility constants and functions for platform-specific Python venv path handling
//!
//! This module provides compile-time constants for directories and files that differ
//! between Windows and Unix-like systems in Python virtual environments.

use super::errors::BridgeError;
use std::fs;
use std::path::PathBuf;

/// The name of the library directory in a Python venv (e.g., "Lib" on Windows, "lib" on Unix)
#[cfg(windows)]
const PYTHON_LIB_DIR: &str = "Lib";
#[cfg(unix)]
const PYTHON_LIB_DIR: &str = "lib";

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
    // Verify the venv_path exists and is a directory.
    if !venv_path.is_dir() {
        return Err(BridgeError::VenvNotFound(venv_path.to_path_buf()));
    }

    #[cfg(windows)]
    {
        let site_packages = venv_path.join(PYTHON_LIB_DIR).join("site-packages");

        // verify site_package_path exists
        if !site_packages.is_dir() {
            return Err(BridgeError::Initialization(format!(
                "unable to locate package directory: {}",
                site_packages.display()
            )));
        }
        Ok(site_packages)
    }

    #[cfg(not(windows))]
    {
        let lib_dir = venv_path.join(PYTHON_LIB_DIR);

        if !lib_dir.is_dir() {
            return Err(BridgeError::Initialization(format!(
                "unable to locate lib directory: {}",
                lib_dir.display()
            )));
        }

        let python_version_dir = fs::read_dir(&lib_dir)
            .map_err(|e| {
                BridgeError::Initialization(format!("Failed to read lib directory: {}", e))
            })?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("python"))
            .ok_or_else(|| {
                BridgeError::Initialization("No python3.X directory found in venv/lib".to_string())
            })?;

        let site_packages = python_version_dir.path().join("site-packages");

        if !site_packages.is_dir() {
            return Err(BridgeError::Initialization(format!(
                "unable to locate package directory: {}",
                site_packages.display()
            )));
        }

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
