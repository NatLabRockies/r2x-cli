//! Utility constants and functions for platform-specific Python venv path handling
//!
//! This module provides compile-time constants for directories and files that differ
//! between Windows and Unix-like systems in Python virtual environments.

/// The name of the library directory in a Python venv (e.g., "Lib" on Windows, "lib" on Unix)
#[cfg(windows)]
pub const PYTHON_LIB_DIR: &str = "Lib";
#[cfg(not(windows))]
pub const PYTHON_LIB_DIR: &str = "lib";

/// The name of the binaries/scripts directory in a Python venv (e.g., "Scripts" on Windows, "bin" on Unix)
#[cfg(windows)]
pub const PYTHON_BIN_DIR: &str = "Scripts";
#[cfg(not(windows))]
pub const PYTHON_BIN_DIR: &str = "bin";

/// The name of the Python executable in a venv (e.g., "python.exe" on Windows, "python" on Unix)
#[cfg(windows)]
pub const PYTHON_EXE: &str = "python.exe";
#[cfg(not(windows))]
pub const PYTHON_EXE: &str = "python";

/// The subdirectory name for site-packages within the lib directory
pub const SITE_PACKAGES: &str = "site-packages";
