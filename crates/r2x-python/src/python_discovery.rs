//! Python executable and shared library discovery
//!
//! This module handles runtime discovery of Python installations, preferring
//! uv-managed Python over system installations. It locates both the Python
//! executable and the shared library needed for dynamic loading.

use crate::errors::BridgeError;
use r2x_config::Config;
use r2x_logger as logger;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Minimum supported Python version
pub const MIN_PYTHON_VERSION: (u8, u8) = (3, 11);

/// Discovered Python environment with all necessary paths
#[derive(Debug, Clone)]
pub struct PythonEnvironment {
    /// Path to the Python executable
    pub executable: PathBuf,
    /// Python installation prefix (PYTHONHOME)
    pub prefix: PathBuf,
    /// Path to the shared library (libpython3.X.so/dylib/dll)
    pub lib_path: PathBuf,
    /// Python version as (major, minor)
    pub version: (u8, u8),
}

/// Errors during Python discovery
#[derive(Debug)]
pub enum DiscoveryError {
    /// No Python installation found
    NoPython(String),
    /// Python found but version too old
    VersionTooOld { found: (u8, u8), required: (u8, u8) },
    /// Could not locate shared library
    NoSharedLibrary(String),
    /// IO error during discovery
    Io(std::io::Error),
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryError::NoPython(msg) => write!(f, "No Python found: {}", msg),
            DiscoveryError::VersionTooOld { found, required } => {
                write!(
                    f,
                    "Python {}.{} found, but {}.{} or newer is required",
                    found.0, found.1, required.0, required.1
                )
            }
            DiscoveryError::NoSharedLibrary(msg) => {
                write!(f, "Could not locate Python shared library: {}", msg)
            }
            DiscoveryError::Io(e) => write!(f, "IO error during discovery: {}", e),
        }
    }
}

impl std::error::Error for DiscoveryError {}

impl From<std::io::Error> for DiscoveryError {
    fn from(e: std::io::Error) -> Self {
        DiscoveryError::Io(e)
    }
}

impl From<DiscoveryError> for BridgeError {
    fn from(e: DiscoveryError) -> Self {
        BridgeError::Initialization(e.to_string())
    }
}

impl PythonEnvironment {
    /// Discover a suitable Python environment
    ///
    /// Discovery order:
    /// 1. Check cached path in config (if valid)
    /// 2. Use `uv python find 3.11` to find uv-managed Python
    /// 3. Fallback to `which python3` with version validation
    pub fn discover(config: &Config) -> Result<Self, DiscoveryError> {
        logger::debug("Starting Python discovery");

        // Try uv-managed Python first
        if let Some(uv_path) = &config.uv_path {
            if let Some(env) = Self::discover_uv_python(uv_path)? {
                logger::info(&format!(
                    "Using uv-managed Python {}.{} at {}",
                    env.version.0,
                    env.version.1,
                    env.executable.display()
                ));
                return Ok(env);
            }
        }

        // Fallback to system Python
        if let Some(env) = Self::discover_system_python()? {
            logger::info(&format!(
                "Using system Python {}.{} at {}",
                env.version.0,
                env.version.1,
                env.executable.display()
            ));
            return Ok(env);
        }

        Err(DiscoveryError::NoPython(
            "No suitable Python 3.11+ installation found. Install Python via `uv python install 3.11` or your system package manager.".to_string()
        ))
    }

    /// Try to find Python via uv
    fn discover_uv_python(uv_path: &str) -> Result<Option<Self>, DiscoveryError> {
        logger::debug("Attempting to find Python via uv");

        // Use uv python find to get the Python path
        let output = Command::new(uv_path)
            .args(["python", "find", "3.11"])
            .output()?;

        if !output.status.success() {
            logger::debug("uv python find 3.11 failed, trying to install");
            // Try to install Python 3.11 via uv
            let install_output = Command::new(uv_path)
                .args(["python", "install", "3.11"])
                .output()?;

            if !install_output.status.success() {
                logger::debug("Failed to install Python via uv");
                return Ok(None);
            }

            // Try finding again after install
            let output = Command::new(uv_path)
                .args(["python", "find", "3.11"])
                .output()?;

            if !output.status.success() {
                return Ok(None);
            }

            return Self::parse_python_path_and_probe(&String::from_utf8_lossy(&output.stdout));
        }

        Self::parse_python_path_and_probe(&String::from_utf8_lossy(&output.stdout))
    }

    /// Parse Python path from uv output and probe for details
    fn parse_python_path_and_probe(output: &str) -> Result<Option<Self>, DiscoveryError> {
        let python_path = output.trim();
        if python_path.is_empty() {
            return Ok(None);
        }

        let executable = PathBuf::from(python_path);
        if !executable.exists() {
            logger::debug(&format!(
                "Python path from uv doesn't exist: {}",
                executable.display()
            ));
            return Ok(None);
        }

        Self::probe_python(&executable)
    }

    /// Try to find system Python
    fn discover_system_python() -> Result<Option<Self>, DiscoveryError> {
        logger::debug("Attempting to find system Python");

        // Try python3 first, then python
        for cmd in &["python3", "python"] {
            let output = Command::new("which").arg(cmd).output();

            if let Ok(output) = output {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let executable = PathBuf::from(&path);

                    if let Ok(Some(env)) = Self::probe_python(&executable) {
                        // Verify version meets minimum
                        if env.version >= MIN_PYTHON_VERSION {
                            return Ok(Some(env));
                        } else {
                            logger::debug(&format!(
                                "System Python {}.{} is too old (need {}.{}+)",
                                env.version.0,
                                env.version.1,
                                MIN_PYTHON_VERSION.0,
                                MIN_PYTHON_VERSION.1
                            ));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Probe a Python executable for version and paths
    fn probe_python(executable: &Path) -> Result<Option<Self>, DiscoveryError> {
        logger::debug(&format!("Probing Python at: {}", executable.display()));

        // Run Python to get version and prefix
        let output = Command::new(executable)
            .args([
                "-c",
                "import sys; print(sys.version_info.major); print(sys.version_info.minor); print(sys.base_prefix)",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();

        if lines.len() < 3 {
            return Ok(None);
        }

        let major: u8 = lines[0].parse().unwrap_or(0);
        let minor: u8 = lines[1].parse().unwrap_or(0);
        let prefix = PathBuf::from(lines[2].trim());

        if major < 3 {
            return Ok(None);
        }

        // Find the shared library
        let lib_path = Self::find_shared_library(&prefix, major, minor)?;

        Ok(Some(PythonEnvironment {
            executable: executable.to_path_buf(),
            prefix,
            lib_path,
            version: (major, minor),
        }))
    }

    /// Find the Python shared library given a prefix
    fn find_shared_library(prefix: &Path, major: u8, minor: u8) -> Result<PathBuf, DiscoveryError> {
        let version_str = format!("{}.{}", major, minor);

        // Platform-specific library search
        #[cfg(target_os = "macos")]
        let candidates = vec![
            prefix.join(format!("lib/libpython{}.dylib", version_str)),
            prefix.join(format!(
                "lib/python{}/config-{}-darwin/libpython{}.dylib",
                version_str, version_str, version_str
            )),
            // Framework path for system Python
            PathBuf::from(format!(
                "/Library/Frameworks/Python.framework/Versions/{}/lib/libpython{}.dylib",
                version_str, version_str
            )),
            // Homebrew paths
            PathBuf::from(format!(
                "/opt/homebrew/opt/python@{}/Frameworks/Python.framework/Versions/{}/lib/libpython{}.dylib",
                version_str, version_str, version_str
            )),
            PathBuf::from(format!(
                "/usr/local/opt/python@{}/Frameworks/Python.framework/Versions/{}/lib/libpython{}.dylib",
                version_str, version_str, version_str
            )),
        ];

        #[cfg(target_os = "linux")]
        let candidates = vec![
            prefix.join(format!("lib/libpython{}.so.1.0", version_str)),
            prefix.join(format!("lib/libpython{}.so", version_str)),
            prefix.join(format!("lib64/libpython{}.so.1.0", version_str)),
            prefix.join(format!("lib64/libpython{}.so", version_str)),
            // Common Linux paths
            PathBuf::from(format!("/usr/lib/libpython{}.so.1.0", version_str)),
            PathBuf::from(format!(
                "/usr/lib/x86_64-linux-gnu/libpython{}.so.1.0",
                version_str
            )),
            PathBuf::from(format!(
                "/usr/lib/aarch64-linux-gnu/libpython{}.so.1.0",
                version_str
            )),
        ];

        #[cfg(target_os = "windows")]
        let candidates = vec![
            prefix.join(format!("python{}{}.dll", major, minor)),
            prefix.join(format!("DLLs/python{}{}.dll", major, minor)),
        ];

        for candidate in &candidates {
            logger::debug(&format!("Checking for library at: {}", candidate.display()));
            if candidate.exists() {
                logger::debug(&format!("Found Python library: {}", candidate.display()));
                return Ok(candidate.clone());
            }
        }

        // On some systems, we need to search more dynamically
        #[cfg(unix)]
        {
            let lib_dir = prefix.join("lib");
            if lib_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&lib_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with(&format!("libpython{}", version_str)) {
                                #[cfg(target_os = "macos")]
                                if name.ends_with(".dylib") {
                                    return Ok(path);
                                }
                                #[cfg(target_os = "linux")]
                                if name.contains(".so") {
                                    return Ok(path);
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(DiscoveryError::NoSharedLibrary(format!(
            "Could not find libpython{}.* in {} or standard locations",
            version_str,
            prefix.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_version_constant() {
        assert_eq!(MIN_PYTHON_VERSION, (3, 11));
    }

    #[test]
    fn test_discovery_error_display() {
        let err = DiscoveryError::NoPython("test".to_string());
        assert!(err.to_string().contains("No Python found"));

        let err = DiscoveryError::VersionTooOld {
            found: (3, 10),
            required: (3, 11),
        };
        assert!(err.to_string().contains("3.10"));
        assert!(err.to_string().contains("3.11"));
    }
}
