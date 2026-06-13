//! Utility adapters for Python virtual environment path handling.
//!
//! The actual platform-specific resolver lives in `r2x-config` so the CLI,
//! manifest/discovery code, and PyO3 bridge do not drift apart on Python
//! version or platform assumptions.

use crate::errors::BridgeError;
use r2x_config::venv_paths::{resolve_python_exe, resolve_site_packages, VenvPathError};
use std::path::{Path, PathBuf};

pub const PYTHON_LIB_DIR: &str = r2x_config::venv_paths::PYTHON_LIB_DIR;
pub const PYTHON_BIN_DIR: &str = r2x_config::venv_paths::PYTHON_BIN_DIR;

pub fn resolve_site_package_path(venv_path: &Path) -> Result<PathBuf, BridgeError> {
    resolve_site_packages(venv_path).map_err(venv_path_error_to_bridge_error)
}

pub fn resolve_python_path(venv_path: &Path) -> Result<PathBuf, BridgeError> {
    resolve_python_exe(venv_path).map_err(venv_path_error_to_bridge_error)
}

fn venv_path_error_to_bridge_error(error: VenvPathError) -> BridgeError {
    match error {
        VenvPathError::VenvNotFound(path) => BridgeError::VenvNotFound(path),
        VenvPathError::PathResolution(message) => BridgeError::Initialization(message),
    }
}

#[cfg(test)]
mod tests {
    use crate::errors::BridgeError;
    use crate::utils::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[cfg(not(windows))]
    fn create_mock_venv_unix(python_version: &str) -> Option<TempDir> {
        let temp_dir = TempDir::new().ok()?;
        let venv_path = temp_dir.path();

        let site_packages = venv_path
            .join(PYTHON_LIB_DIR)
            .join(python_version)
            .join("site-packages");
        fs::create_dir_all(&site_packages).ok()?;

        let bin_dir = venv_path.join(PYTHON_BIN_DIR);
        fs::create_dir_all(&bin_dir).ok()?;
        fs::write(bin_dir.join("python3"), "").ok()?;

        Some(temp_dir)
    }

    #[cfg(windows)]
    fn create_mock_venv_windows(python_exe: &str) -> Option<TempDir> {
        let temp_dir = TempDir::new().ok()?;
        let venv_path = temp_dir.path();

        let site_packages = venv_path.join(PYTHON_LIB_DIR).join("site-packages");
        fs::create_dir_all(&site_packages).ok()?;

        let scripts_dir = venv_path.join(PYTHON_BIN_DIR);
        fs::create_dir_all(&scripts_dir).ok()?;
        fs::write(scripts_dir.join(python_exe), "").ok()?;

        Some(temp_dir)
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_site_package_path_unix() {
        let Some(temp_venv) = create_mock_venv_unix("python3.12") else {
            return;
        };

        let result = resolve_site_package_path(temp_venv.path());
        assert!(result.is_ok_and(|sp| sp.ends_with("lib/python3.12/site-packages")));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_site_package_path_unix_different_version() {
        let Some(temp_venv) = create_mock_venv_unix("python3.11") else {
            return;
        };

        let result = resolve_site_package_path(temp_venv.path());
        assert!(result.is_ok_and(|sp| sp.ends_with("lib/python3.11/site-packages")));
    }

    #[test]
    #[cfg(windows)]
    fn test_resolve_site_package_path_windows() {
        let Some(temp_venv) = create_mock_venv_windows("python.exe") else {
            return;
        };

        let result = resolve_site_package_path(temp_venv.path());
        assert!(result.is_ok_and(|sp| sp.ends_with("Lib\\site-packages")));
    }

    #[test]
    fn test_resolve_site_package_path_venv_not_found() {
        let non_existent_path = PathBuf::from("/tmp/non_existent_venv_12345");

        let result = resolve_site_package_path(&non_existent_path);
        assert!(matches!(
            result,
            Err(BridgeError::VenvNotFound(path)) if path == non_existent_path
        ));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_site_package_path_missing_python_dir() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let lib_dir = temp_dir.path().join(PYTHON_LIB_DIR);
        if fs::create_dir_all(&lib_dir).is_err() {
            return;
        }

        let result = resolve_site_package_path(temp_dir.path());
        assert!(result.is_err_and(|e| {
            matches!(e, BridgeError::Initialization(msg) if msg.contains("No python3.X directory found"))
        }));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_python_path_unix() {
        let Some(temp_venv) = create_mock_venv_unix("python3.12") else {
            return;
        };

        let result = resolve_python_path(temp_venv.path());
        assert!(result.is_ok_and(|pp| pp.ends_with("bin/python3")));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_python_path_unix_versioned_fallback() {
        let Some(temp_venv) = create_mock_venv_unix("python3.13") else {
            return;
        };
        let bin_dir = temp_venv.path().join(PYTHON_BIN_DIR);
        if fs::remove_file(bin_dir.join("python3")).is_err() {
            return;
        }
        if fs::write(bin_dir.join("python3.13"), "").is_err() {
            return;
        }

        let result = resolve_python_path(temp_venv.path());
        assert!(result.is_ok_and(|pp| pp.ends_with("bin/python3.13")));
    }

    #[test]
    #[cfg(windows)]
    fn test_resolve_python_path_windows() {
        let Some(temp_venv) = create_mock_venv_windows("python.exe") else {
            return;
        };

        let result = resolve_python_path(temp_venv.path());
        assert!(result.is_ok_and(|pp| pp.ends_with("Scripts\\python.exe")));
    }

    #[test]
    #[cfg(windows)]
    fn test_resolve_python_path_windows_versioned_fallback() {
        let Some(temp_venv) = create_mock_venv_windows("python3.13.exe") else {
            return;
        };

        let result = resolve_python_path(temp_venv.path());
        assert!(result.is_ok_and(|pp| pp.ends_with("Scripts\\python3.13.exe")));
    }

    #[test]
    fn test_python_lib_dir_constant() {
        #[cfg(not(windows))]
        assert_eq!(PYTHON_LIB_DIR, "lib");

        #[cfg(windows)]
        assert_eq!(PYTHON_LIB_DIR, "Lib");
    }

    #[test]
    fn test_python_bin_dir_constant() {
        #[cfg(not(windows))]
        assert_eq!(PYTHON_BIN_DIR, "bin");

        #[cfg(windows)]
        assert_eq!(PYTHON_BIN_DIR, "Scripts");
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_site_package_path_with_multiple_python_versions() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let lib_dir = temp_dir.path().join(PYTHON_LIB_DIR);
        if fs::create_dir_all(&lib_dir).is_err() {
            return;
        }

        for python_version in ["python3.11", "python3.12"] {
            let site_packages = lib_dir.join(python_version).join("site-packages");
            if fs::create_dir_all(&site_packages).is_err() {
                return;
            }
        }

        let result = resolve_site_package_path(temp_dir.path());
        assert!(result.is_ok_and(
            |sp| sp.to_string_lossy().contains("python3.1") && sp.ends_with("site-packages")
        ));
    }
}
