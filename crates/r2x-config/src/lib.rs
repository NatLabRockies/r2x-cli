pub mod venv_paths;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// =============================================================================
// ConfigError - typed error for configuration operations
// =============================================================================

/// Errors that can occur during configuration operations
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config TOML: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("Failed to serialize config to TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("uv not found: {0}")]
    UvNotFound(String),

    #[error("Failed to create venv: {0}")]
    VenvCreation(String),

    #[error("Unknown config key: '{0}'")]
    UnknownKey(String),

    #[error("Failed to parse config value for '{key}': {message}")]
    InvalidValue { key: String, message: String },

    #[error("Home directory not found")]
    NoHomeDir,

    #[error("Config directory not found")]
    NoConfigDir,
}
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use which::which;

const FALLBACK_PYTHON_VERSION: &str = "3.12";

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uv_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub venv_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r2x_core_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_python: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_stdout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_max_size: Option<u64>,
}

impl Config {
    pub fn path() -> PathBuf {
        // Honor explicit override via R2X_CONFIG for tests / isolated runs.
        // If set and non-empty, use that path immediately.
        if let Ok(env_path) = std::env::var("R2X_CONFIG") {
            let trimmed = env_path.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }

        // Default config file path (platform-appropriate).
        #[cfg(not(target_os = "windows"))]
        let Some(default) =
            dirs::home_dir().map(|h| h.join(".config").join("r2x").join("config.toml"))
        else {
            return PathBuf::from(".config.toml");
        };

        #[cfg(target_os = "windows")]
        let Some(default) = dirs::config_dir().map(|c| c.join("r2x").join("config.toml")) else {
            return PathBuf::from(".config.toml");
        };

        // Look for a pointer file next to the default config, e.g. ~/.config/r2x/.r2x_config_path
        // If present and contains a non-empty path, use that path as the config file location.
        if let Some(parent) = default.parent() {
            let pointer = parent.join(".r2x_config_path");
            if pointer.exists() {
                if let Ok(contents) = std::fs::read_to_string(&pointer) {
                    let trimmed = contents.trim();
                    if !trimmed.is_empty() {
                        return PathBuf::from(trimmed);
                    }
                }
            }
        }

        default
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::path();
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "cache-path" => self.cache_path.clone(),
            "uv-path" => self.uv_path.clone(),
            "python-version" => self.python_version.clone(),
            "venv-path" => self.venv_path.clone(),
            "r2x-core-version" => self.r2x_core_version.clone(),
            "log-python" => self.log_python.map(|v| v.to_string()),
            "no-stdout" => self.no_stdout.map(|v| v.to_string()),
            "log-path" => self.log_path.clone(),
            "log-max-size" => self.log_max_size.map(|v| v.to_string()),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: String) -> Result<(), ConfigError> {
        match key {
            "cache-path" => self.cache_path = Some(value),
            "uv-path" => self.uv_path = Some(value),
            "python-version" => self.python_version = Some(normalize_python_version(&value)?),
            "venv-path" => self.venv_path = Some(value),
            "r2x-core-version" => self.r2x_core_version = Some(value),
            "log-python" => {
                self.log_python =
                    Some(
                        value
                            .parse::<bool>()
                            .map_err(|_| ConfigError::InvalidValue {
                                key: key.to_string(),
                                message: format!("expected 'true' or 'false', got '{}'", value),
                            })?,
                    );
            }
            "no-stdout" => {
                self.no_stdout =
                    Some(
                        value
                            .parse::<bool>()
                            .map_err(|_| ConfigError::InvalidValue {
                                key: key.to_string(),
                                message: format!("expected 'true' or 'false', got '{}'", value),
                            })?,
                    );
            }
            "log-path" => self.log_path = Some(value),
            "log-max-size" => {
                self.log_max_size =
                    Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| ConfigError::InvalidValue {
                                key: key.to_string(),
                                message: format!("expected a positive integer, got '{}'", value),
                            })?,
                    );
            }
            _ => return Err(ConfigError::UnknownKey(key.to_string())),
        }
        Ok(())
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.cache_path.is_none()
            && self.uv_path.is_none()
            && self.python_version.is_none()
            && self.venv_path.is_none()
            && self.r2x_core_version.is_none()
            && self.log_python.is_none()
            && self.no_stdout.is_none()
            && self.log_path.is_none()
            && self.log_max_size.is_none()
    }

    pub fn reset() -> Result<(), ConfigError> {
        let path = Self::path();
        if path.exists() {
            fs::remove_file(&path)?;
        }
        if let Some(parent) = path.parent() {
            let pointer = parent.join(".r2x_config_path");
            if pointer.exists() {
                fs::remove_file(pointer)?;
            }
        }
        Ok(())
    }

    pub fn get_cache_path(&self) -> String {
        self.cache_path.clone().unwrap_or_else(|| {
            #[cfg(not(target_os = "windows"))]
            {
                dirs::home_dir()
                    .map(|h| h.join(".cache").join("r2x"))
                    .and_then(|p| p.to_str().map(String::from))
                    .unwrap_or_else(|| ".cache/r2x".to_string())
            }
            #[cfg(target_os = "windows")]
            {
                dirs::cache_dir()
                    .map(|c| c.join("r2x"))
                    .and_then(|p| p.to_str().map(String::from))
                    .unwrap_or_else(|| "cache\\r2x".to_string())
            }
        })
    }

    pub fn get_venv_path(&self) -> String {
        // If explicitly configured, use it.
        if let Some(ref p) = self.venv_path {
            return p.clone();
        }

        #[cfg(not(target_os = "windows"))]
        {
            let Some(default) =
                dirs::home_dir().map(|h| h.join(".config").join("r2x").join(".venv"))
            else {
                return ".config/r2x/.venv".to_string();
            };

            // If the legacy location exists but the default does not,
            // return the legacy path so that callers who attempt migration
            // (e.g., migrate_legacy_venv) can still find it.
            let legacy = dirs::config_dir()
                .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
                .map(|c| c.join("r2x").join(".venv"));

            if let Some(ref legacy_path) = legacy {
                if legacy_path.exists() && !default.exists() {
                    return legacy_path.to_string_lossy().to_string();
                }
            }

            default.to_string_lossy().to_string()
        }

        #[cfg(target_os = "windows")]
        {
            dirs::config_dir()
                .map(|c| c.join("r2x").join(".venv"))
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "config\\r2x\\.venv".to_string())
        }
    }

    /// Migrate a legacy venv location to the new default.
    ///
    /// The legacy path (e.g. macOS Application Support) is renamed to
    /// `~/.config/r2x/.venv` when the legacy exists and the default does
    /// not. Call this once during startup or config migration.
    #[cfg(not(target_os = "windows"))]
    pub fn migrate_legacy_venv(&self) {
        let Some(default) = dirs::home_dir().map(|h| h.join(".config").join("r2x").join(".venv"))
        else {
            return;
        };

        if default.exists() {
            return;
        }

        let Some(legacy) = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .map(|c| c.join("r2x").join(".venv"))
        else {
            return;
        };

        if !legacy.exists() {
            return;
        }

        if let Some(parent) = default.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(&legacy, &default);
    }

    /// No-op on Windows — the default path is already the canonical config_dir location.
    #[cfg(target_os = "windows")]
    #[allow(clippy::unused_self)]
    pub fn migrate_legacy_venv(&self) {}

    pub fn get_venv_python_path(&self) -> String {
        let venv_path = self.get_venv_path();
        #[cfg(not(target_os = "windows"))]
        {
            format!("{}/bin/python", venv_path)
        }
        #[cfg(target_os = "windows")]
        {
            format!("{}\\Scripts\\python.exe", venv_path)
        }
    }

    pub fn get_r2x_core_package_spec(&self) -> String {
        let version = self.r2x_core_version.as_deref().unwrap_or("0.1.0rc1");
        // If version contains operators (>=, <=, ~=, !=, ==, <, >), use it as-is
        // Otherwise, prefix with == for exact version matching
        if version.contains(">=")
            || version.contains("<=")
            || version.contains("~=")
            || version.contains("!=")
            || version.contains("==")
            || version.contains('>')
            || version.contains('<')
        {
            format!("r2x-core{}", version)
        } else {
            format!("r2x-core=={}", version)
        }
    }

    pub fn ensure_uv_path(&mut self) -> Result<String, ConfigError> {
        // Check if the stored path exists
        if let Some(ref path) = self.uv_path {
            if std::path::Path::new(path).exists() {
                return Ok(path.clone());
            }
            // Path was in config but doesn't exist, clear it
            eprintln!("Stored uv path no longer exists: {}", path);
            self.uv_path = None;
        }

        if let Ok(path) = which("uv") {
            let path_str = path.to_string_lossy().trim().to_string();
            self.uv_path = Some(path_str.clone());
            self.save()?;
            return Ok(path_str);
        }

        // Auto-install uv if not found — prompt for confirmation first
        {
            use std::io::{self, Write};

            print!(
                "uv was not found. Install uv automatically using the official installer? [y/n] "
            );
            let _ = io::stdout().flush();
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_ok() {
                let response = input.trim().to_lowercase();
                if response != "y" && response != "yes" {
                    eprintln!("uv installation skipped. Install it manually: https://docs.astral.sh/uv/getting-started/installation/");
                    return Err(ConfigError::UvNotFound(
                        "uv is required but not installed".to_string(),
                    ));
                }
            } else {
                return Err(ConfigError::UvNotFound(
                    "uv is required but could not read confirmation".to_string(),
                ));
            }
        }

        eprintln!("Installing uv using official installer...\n");

        #[cfg(target_os = "windows")]
        {
            // On Windows, use PowerShell to download and run the installer
            let status = Command::new("powershell")
                .args([
                    "-ExecutionPolicy",
                    "ByPass",
                    "-Command",
                    "irm https://astral.sh/uv/install.ps1 | iex",
                ])
                .status()?;

            if !status.success() {
                return Err(ConfigError::UvNotFound("Failed to install uv. Please install it manually from: https://docs.astral.sh/uv/getting-started/installation/".to_string()));
            }

            eprintln!("\nuv installation completed. Verifying installation...");

            // On Windows, uv is typically installed to %USERPROFILE%\.local\bin or %USERPROFILE%\.cargo\bin
            // Try to find it using where.exe
            if let Ok(output) = Command::new("where.exe").arg("uv").output() {
                if output.status.success() {
                    let path = String::from_utf8(output.stdout)
                        .map_err(|e| {
                            ConfigError::UvNotFound(format!("Failed to parse uv path: {}", e))
                        })?
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !path.is_empty() {
                        eprintln!("Found uv at: {}", path);
                        self.uv_path = Some(path.clone());
                        self.save()?;
                        return Ok(path);
                    }
                }
            }

            return Err(ConfigError::UvNotFound("Failed to locate uv after installation. Please add %USERPROFILE%\\.local\\bin to your PATH and restart your terminal".to_string()));
        }

        #[cfg(not(target_os = "windows"))]
        {
            // On Unix systems, use curl to download and run the installer
            let status = Command::new("sh")
                .arg("-c")
                .arg("curl -LsSf https://astral.sh/uv/install.sh | sh")
                .status()?;

            if !status.success() {
                return Err(ConfigError::UvNotFound("Failed to install uv".to_string()));
            }

            eprintln!("\nuv installation completed. Verifying installation...");

            // Verify the installation
            if let Ok(output) = Command::new("which").arg("uv").output() {
                if output.status.success() {
                    let path = String::from_utf8(output.stdout)
                        .map_err(|e| {
                            ConfigError::UvNotFound(format!("Failed to parse uv path: {}", e))
                        })?
                        .trim()
                        .to_string();
                    eprintln!("Found uv at: {}", path);
                    self.uv_path = Some(path.clone());
                    self.save()?;
                    return Ok(path);
                }
            }

            Err(ConfigError::UvNotFound("Failed to locate uv after installation. Verify that ~/.local/bin or ~/.cargo/bin is in your PATH".to_string()))
        }
    }

    pub fn ensure_cache_path(&mut self) -> Result<String, ConfigError> {
        let cache_path = self.get_cache_path();
        fs::create_dir_all(&cache_path)?;
        Ok(cache_path)
    }

    pub fn ensure_venv_path(&mut self) -> Result<String, ConfigError> {
        use std::process::Command;

        // Attempt one-time migration from legacy venv location
        self.migrate_legacy_venv();

        let venv_path = self.get_venv_path();

        // Check if venv already exists
        if std::path::Path::new(&venv_path).exists() {
            return Ok(venv_path);
        }

        // Ensure uv is installed first (this will auto-install if needed)
        let uv_path = self.ensure_uv_path()?;

        // Use the Python version from config, or the build-selected default.
        let python_version = self.runtime_python_version()?;

        // Create the venv using uv, with patch->ABI fallback for patch requests.
        let mut last_stderr: Option<String> = None;
        for python_query in python_version.query_candidates() {
            let output = Command::new(&uv_path)
                .args(["venv", &venv_path, "--python", python_query])
                .output()?;

            if output.status.success() {
                return Ok(venv_path);
            }

            last_stderr = Some(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }

        let details = last_stderr
            .filter(|stderr| !stderr.is_empty())
            .unwrap_or_else(|| "uv venv command failed".to_string());
        Err(ConfigError::VenvCreation(format!(
            "Failed to create venv for Python {} (ABI {}): {}\nTry: {}",
            python_version.requested(),
            python_version.abi(),
            details,
            python_version.install_hint()
        )))
    }

    pub fn runtime_python_version(&self) -> Result<PythonRuntimeVersion, ConfigError> {
        runtime_python_version(self.python_version.as_deref())
    }
}

pub fn default_python_version() -> &'static str {
    option_env!("R2X_BUILD_PYTHON_VERSION").unwrap_or(FALLBACK_PYTHON_VERSION)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonRuntimeVersion {
    requested: String,
    abi: String,
}

impl PythonRuntimeVersion {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        let requested = normalize_python_version(value)?;
        let abi = python_abi_version(&requested);

        Ok(Self { requested, abi })
    }

    pub fn requested(&self) -> &str {
        &self.requested
    }

    pub fn abi(&self) -> &str {
        &self.abi
    }

    pub fn query_candidates(&self) -> Vec<&str> {
        let mut candidates = vec![self.requested()];
        if self.abi() != self.requested() {
            candidates.push(self.abi());
        }
        candidates
    }

    pub fn install_hint(&self) -> String {
        if self.requested() == self.abi() {
            format!("uv python install {}", self.requested())
        } else {
            format!(
                "uv python install {} || uv python install {}",
                self.requested(),
                self.abi()
            )
        }
    }

    pub fn find_hint(&self) -> String {
        if self.requested() == self.abi() {
            format!("uv python find {}", self.requested())
        } else {
            format!(
                "uv python find {} || uv python find {}",
                self.requested(),
                self.abi()
            )
        }
    }
}

fn runtime_python_version(
    configured_version: Option<&str>,
) -> Result<PythonRuntimeVersion, ConfigError> {
    let requested = match configured_version.map(str::trim) {
        Some(version) if !version.is_empty() => version,
        _ => default_python_version(),
    };

    PythonRuntimeVersion::parse(requested)
}

pub fn normalize_python_version(value: &str) -> Result<String, ConfigError> {
    let version = value.trim();
    if version.is_empty() {
        return Err(ConfigError::InvalidValue {
            key: "python-version".to_string(),
            message: "expected a Python version like '3.12' or '3.12.1'".to_string(),
        });
    }

    let parts = version.split('.').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()))
    {
        return Err(ConfigError::InvalidValue {
            key: "python-version".to_string(),
            message: format!("expected a Python version like '3.12' or '3.12.1', got '{value}'"),
        });
    }

    let major = parts[0]
        .parse::<u16>()
        .map_err(|_| ConfigError::InvalidValue {
            key: "python-version".to_string(),
            message: format!("expected a numeric major version, got '{}'", parts[0]),
        })?;
    let minor = parts[1]
        .parse::<u16>()
        .map_err(|_| ConfigError::InvalidValue {
            key: "python-version".to_string(),
            message: format!("expected a numeric minor version, got '{}'", parts[1]),
        })?;

    if major != 3 || minor < 11 {
        return Err(ConfigError::InvalidValue {
            key: "python-version".to_string(),
            message: format!("r2x supports Python 3.11 or newer, got '{version}'"),
        });
    }

    Ok(version.to_string())
}

fn python_abi_version(version: &str) -> String {
    let mut parts = version.split('.');
    let Some(major) = parts.next() else {
        return version.to_string();
    };
    let Some(minor) = parts.next() else {
        return version.to_string();
    };

    format!("{major}.{minor}")
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::fs;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;

    #[cfg(unix)]
    fn detect_python_version_with_retry(path: &std::path::Path) -> Result<String, String> {
        let path = path.to_string_lossy();
        match r2x_build_support::detect_python_version(&path) {
            Ok(version) => Ok(version),
            Err(error) if error.contains("Text file busy") => {
                thread::sleep(Duration::from_millis(20));
                r2x_build_support::detect_python_version(&path)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(unix)]
    fn write_test_executable(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
        let mut file = fs::File::create(path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);

        let metadata = fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }

    #[cfg(unix)]
    fn detect_requested_python_via_uv_with_retry(
        uv: &std::path::Path,
        requested: &str,
    ) -> Result<String, String> {
        let uv = uv.to_string_lossy();
        match r2x_build_support::detect_requested_python_via_uv(&uv, requested) {
            Ok(version) => Ok(version),
            Err(error) if error.contains("Text file busy") => {
                thread::sleep(Duration::from_millis(20));
                r2x_build_support::detect_requested_python_via_uv(&uv, requested)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(unix)]
    fn ensure_venv_path_with_retry(config: &mut Config) -> Result<String, ConfigError> {
        match config.ensure_venv_path() {
            Ok(path) => Ok(path),
            Err(ConfigError::Io(error))
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy =>
            {
                thread::sleep(Duration::from_millis(20));
                config.ensure_venv_path()
            }
            Err(error) => Err(error),
        }
    }

    #[test]
    fn test_config_new() {
        let config = Config::default();
        assert!(config.is_empty());
    }

    #[test]
    fn test_config_set_get() {
        let mut config = Config::default();
        assert!(config.set("cache-path", "test-value".to_string()).is_ok());
        assert_eq!(config.get("cache-path"), Some("test-value".to_string()));
    }

    #[test]
    fn test_config_multiple_fields() {
        let mut config = Config::default();
        assert!(config.set("cache-path", "/tmp/cache".to_string()).is_ok());
        assert_eq!(config.get("cache-path"), Some("/tmp/cache".to_string()));
        assert!(!config.is_empty());
    }

    #[test]
    fn test_config_unknown_key_returns_error() {
        let mut config = Config::default();
        let result = config.set("unknown-key", "value".to_string());
        assert!(result.is_err());
        if let Err(err) = result {
            let msg = err.to_string();
            assert!(
                msg.contains("unknown-key"),
                "error should mention the key: {msg}"
            );
        }
    }

    #[test]
    fn test_config_default_cache_path() {
        let config = Config::default();
        let cache_path = config.get_cache_path();
        assert!(!cache_path.is_empty());
        assert!(cache_path.contains("r2x"));
    }

    #[test]
    fn test_config_set_get_bool_fields() {
        let mut config = Config::default();
        assert!(config.set("no-stdout", "true".to_string()).is_ok());
        assert!(config.set("log-python", "false".to_string()).is_ok());
        assert_eq!(config.get("no-stdout"), Some("true".to_string()));
        assert_eq!(config.get("log-python"), Some("false".to_string()));
    }

    #[test]
    fn test_config_set_get_log_fields() {
        let mut config = Config::default();
        assert!(config
            .set("log-path", "/tmp/r2x-custom.log".to_string())
            .is_ok());
        assert!(config.set("log-max-size", "1048576".to_string()).is_ok());
        assert_eq!(
            config.get("log-path"),
            Some("/tmp/r2x-custom.log".to_string())
        );
        assert_eq!(config.get("log-max-size"), Some("1048576".to_string()));
    }

    #[test]
    fn test_default_python_version_is_minor_version() {
        let version = default_python_version();
        assert!(
            version.starts_with("3.") && version.split('.').count() == 2,
            "default python version should be a major.minor version, got {version}"
        );
    }

    #[test]
    fn test_python_version_normalization_accepts_minor_and_patch_versions() {
        assert_eq!(
            normalize_python_version("3.13").ok().as_deref(),
            Some("3.13")
        );
        assert_eq!(
            normalize_python_version(" 3.13.1 ").ok().as_deref(),
            Some("3.13.1")
        );
    }

    #[test]
    fn test_python_runtime_version_tracks_requested_and_abi_versions() {
        assert_eq!(
            PythonRuntimeVersion::parse("3.13.1").ok(),
            Some(PythonRuntimeVersion {
                requested: "3.13.1".to_string(),
                abi: "3.13".to_string(),
            })
        );
    }

    #[test]
    fn test_python_runtime_version_query_candidates_include_patch_then_abi() {
        let Ok(patch) = PythonRuntimeVersion::parse("3.13.1") else {
            return;
        };
        assert_eq!(patch.query_candidates(), vec!["3.13.1", "3.13"]);
        assert_eq!(
            patch.install_hint(),
            "uv python install 3.13.1 || uv python install 3.13"
        );
        assert_eq!(
            patch.find_hint(),
            "uv python find 3.13.1 || uv python find 3.13"
        );

        let Ok(abi) = PythonRuntimeVersion::parse("3.13") else {
            return;
        };
        assert_eq!(abi.query_candidates(), vec!["3.13"]);
        assert_eq!(abi.install_hint(), "uv python install 3.13");
        assert_eq!(abi.find_hint(), "uv python find 3.13");
    }

    #[test]
    fn test_config_runtime_python_version_uses_config_or_default() {
        let configured = Config {
            python_version: Some("3.14.2".to_string()),
            ..Config::default()
        };
        assert_eq!(
            configured.runtime_python_version().ok(),
            Some(PythonRuntimeVersion {
                requested: "3.14.2".to_string(),
                abi: "3.14".to_string(),
            })
        );

        let default = Config::default().runtime_python_version();
        assert!(default.is_ok());
        if let Ok(version) = default {
            assert_eq!(version.requested(), default_python_version());
            assert_eq!(version.abi(), default_python_version());
        }
    }

    #[test]
    fn test_python_version_normalization_rejects_invalid_versions() {
        for version in ["", "3", "3.10", "2.7", "3.13-dev", "3.13.1.2"] {
            assert!(
                normalize_python_version(version).is_err(),
                "expected {version:?} to be rejected"
            );
        }
    }

    #[test]
    fn test_config_set_python_version_normalizes_before_storing() {
        let mut config = Config::default();
        assert!(config.set("python-version", " 3.14.1 ".to_string()).is_ok());

        assert_eq!(config.python_version.as_deref(), Some("3.14.1"));
    }

    #[test]
    fn test_config_set_python_version_rejects_unsupported_version() {
        let mut config = Config::default();
        let result = config.set("python-version", "3.10".to_string());

        assert!(result.is_err());
        assert!(config.python_version.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_detects_interpreter_minor_version() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };
        let python = temp_dir.path().join("python");
        assert!(fs::write(&python, "#!/usr/bin/env sh\nprintf '3.13\\n'\n").is_ok());
        let Ok(metadata) = fs::metadata(&python) else {
            return;
        };
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        assert!(fs::set_permissions(&python, permissions).is_ok());

        assert_eq!(
            detect_python_version_with_retry(&python).ok().as_deref(),
            Some("3.13")
        );
    }

    #[test]
    fn test_build_python_version_reports_unusable_interpreter() {
        let result = r2x_build_support::detect_python_version("/definitely/missing/python");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("failed to execute interpreter"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_rejects_unsupported_interpreter() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };
        let python = temp_dir.path().join("python");
        assert!(fs::write(&python, "#!/usr/bin/env sh\nprintf '3.10\\n'\n").is_ok());
        let Ok(metadata) = fs::metadata(&python) else {
            return;
        };
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        assert!(fs::set_permissions(&python, permissions).is_ok());

        let result = detect_python_version_with_retry(&python);

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("requires Python 3.11 or newer"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn test_build_python_version_normalizes_requested_python_abi() {
        assert_eq!(
            r2x_build_support::requested_python_abi_version("R2X_PYTHON_VERSION", "3.13.1")
                .ok()
                .as_deref(),
            Some("3.13")
        );
    }

    #[test]
    fn test_build_python_version_rejects_invalid_requested_python_version() {
        let result =
            r2x_build_support::requested_python_abi_version("R2X_PYTHON_VERSION", "3.13-dev");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("must be a Python version like"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn test_build_python_version_rejects_empty_requested_python_version() {
        let result = r2x_build_support::requested_python_abi_version("R2X_PYTHON_VERSION", "  ");

        assert!(result.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_uses_uv_for_requested_python_version() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };
        let python = temp_dir.path().join("python");
        assert!(fs::write(&python, "#!/usr/bin/env sh\nprintf '3.13\\n'\n").is_ok());
        let uv = temp_dir.path().join("uv");
        assert!(
            fs::write(
                &uv,
                format!(
                    "#!/usr/bin/env sh\nif [ \"$1\" = python ] && [ \"$2\" = find ] && [ \"$3\" = 3.13.1 ]; then\n  printf '{}\\n'\n  exit 0\nfi\nexit 1\n",
                    python.display()
                )
            )
            .is_ok()
        );
        for executable in [&python, &uv] {
            let Ok(metadata) = fs::metadata(executable) else {
                return;
            };
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            assert!(fs::set_permissions(executable, permissions).is_ok());
        }

        assert_eq!(
            detect_requested_python_via_uv_with_retry(&uv, "3.13.1")
                .ok()
                .as_deref(),
            Some("3.13")
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_falls_back_to_requested_abi_when_patch_missing() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };
        let python = temp_dir.path().join("python");
        assert!(fs::write(&python, "#!/usr/bin/env sh\nprintf '3.13\\n'\n").is_ok());
        let uv = temp_dir.path().join("uv");
        assert!(
            fs::write(
                &uv,
                format!(
                    "#!/usr/bin/env sh\nif [ \"$1\" = python ] && [ \"$2\" = find ] && [ \"$3\" = 3.13.1 ]; then\n  exit 1\nfi\nif [ \"$1\" = python ] && [ \"$2\" = find ] && [ \"$3\" = 3.13 ]; then\n  printf '{}\\n'\n  exit 0\nfi\nexit 1\n",
                    python.display()
                )
            )
            .is_ok()
        );
        for executable in [&python, &uv] {
            let Ok(metadata) = fs::metadata(executable) else {
                return;
            };
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            assert!(fs::set_permissions(executable, permissions).is_ok());
        }

        assert_eq!(
            detect_requested_python_via_uv_with_retry(&uv, "3.13.1")
                .ok()
                .as_deref(),
            Some("3.13")
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_rejects_uv_requested_python_mismatch() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };
        let python = temp_dir.path().join("python");
        assert!(fs::write(&python, "#!/usr/bin/env sh\nprintf '3.12\\n'\n").is_ok());
        let uv = temp_dir.path().join("uv");
        assert!(
            fs::write(
                &uv,
                format!(
                    "#!/usr/bin/env sh\nif [ \"$1\" = python ] && [ \"$2\" = find ]; then\n  printf '{}\\n'\n  exit 0\nfi\nexit 1\n",
                    python.display()
                )
            )
            .is_ok()
        );
        for executable in [&python, &uv] {
            let Ok(metadata) = fs::metadata(executable) else {
                return;
            };
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            assert!(fs::set_permissions(executable, permissions).is_ok());
        }

        let result = detect_requested_python_via_uv_with_retry(&uv, "3.13");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("uv python find resolves to Python 3.12"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_reports_missing_requested_uv_python() {
        let result = r2x_build_support::detect_requested_python_via_uv("false", "3.13");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("uv python install 3.13"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_reports_missing_requested_patch_and_abi() {
        let result = r2x_build_support::detect_requested_python_via_uv("false", "3.13.1");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("Requested R2X_PYTHON_VERSION=3.13.1 was not found."),
                "unexpected error: {error}"
            );
            assert!(
                error.contains("uv python install 3.13.1"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_reports_missing_uv_binary_for_requested_python() {
        let result =
            r2x_build_support::detect_requested_python_via_uv("/definitely/missing/uv", "3.13");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("Install uv, then run: uv python install 3.13"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_build_python_version_reports_patch_install_fallback_when_uv_is_missing() {
        let result =
            r2x_build_support::detect_requested_python_via_uv("/definitely/missing/uv", "3.13.1");

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error.contains("uv python install 3.13.1 || uv python install 3.13"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_ensure_venv_path_falls_back_to_abi_when_patch_query_fails() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };

        let uv = temp_dir.path().join("uv");
        let calls_path = temp_dir.path().join("uv-calls.log");
        let venv_path = temp_dir.path().join(".venv");

        assert!(
            write_test_executable(
                &uv,
                &format!(
                    "#!/usr/bin/env sh\necho \"$@\" >> \"{}\"\nif [ \"$1\" != \"venv\" ]; then\n  echo \"unexpected command: $@\" >&2\n  exit 1\nfi\nvenv_path=\"\"\npython_query=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    venv)\n      shift\n      venv_path=\"${{1:-}}\"\n      ;;\n    --python)\n      shift\n      python_query=\"${{1:-}}\"\n      ;;\n  esac\n  shift || break\ndone\ncase \"$python_query\" in\n  3.13.1)\n    echo \"patch unavailable\" >&2\n    exit 1\n    ;;\n  3.13)\n    mkdir -p \"$venv_path\"\n    exit 0\n    ;;\nesac\necho \"unexpected python query: $python_query\" >&2\nexit 1\n",
                    calls_path.display()
                )
            )
            .is_ok()
        );

        let mut config = Config {
            uv_path: Some(uv.to_string_lossy().to_string()),
            python_version: Some("3.13.1".to_string()),
            venv_path: Some(venv_path.to_string_lossy().to_string()),
            ..Config::default()
        };

        let result = ensure_venv_path_with_retry(&mut config);
        let calls = fs::read_to_string(&calls_path).unwrap_or_default();
        assert!(
            matches!(result.as_deref(), Ok(path) if path == venv_path.to_string_lossy()),
            "ensure_venv_path failed: {result:?}\nuv calls:\n{calls}"
        );

        assert!(
            calls.contains("--python 3.13.1"),
            "missing patch query: {calls}"
        );
        assert!(
            calls.contains("--python 3.13"),
            "missing abi fallback query: {calls}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_ensure_venv_path_error_includes_patch_fallback_hint() {
        let Ok(temp_dir) = tempfile::tempdir() else {
            return;
        };

        let uv = temp_dir.path().join("uv");
        assert!(
            write_test_executable(
                &uv,
                "#!/usr/bin/env sh\nif [ \"$1\" = \"venv\" ]; then\n  echo 'no interpreter found' >&2\n  exit 1\nfi\nexit 1\n"
            )
            .is_ok()
        );

        let mut config = Config {
            uv_path: Some(uv.to_string_lossy().to_string()),
            python_version: Some("3.13.1".to_string()),
            venv_path: Some(temp_dir.path().join(".venv").to_string_lossy().to_string()),
            ..Config::default()
        };

        let result = ensure_venv_path_with_retry(&mut config);
        assert!(result.is_err());
        if let Err(error) = result {
            let message = error.to_string();
            assert!(
                message.contains("Failed to create venv for Python 3.13.1 (ABI 3.13)"),
                "unexpected error: {message}"
            );
            assert!(
                message.contains("uv python install 3.13.1 || uv python install 3.13"),
                "unexpected error: {message}"
            );
        }
    }
}
