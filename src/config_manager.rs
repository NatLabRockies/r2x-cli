use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use which::which;

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
        let default = dirs::home_dir()
            .expect("Could not determine home directory")
            .join(".config")
            .join("r2x")
            .join("r2x.toml");

        #[cfg(target_os = "windows")]
        let default = dirs::config_dir()
            .expect("Could not determine config directory")
            .join("r2x")
            .join("r2x.toml");

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

    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path();
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
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
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: String) {
        match key {
            "cache-path" => self.cache_path = Some(value),
            "uv-path" => self.uv_path = Some(value),
            "python-version" => self.python_version = Some(value),
            "venv-path" => self.venv_path = Some(value),
            "r2x-core-version" => self.r2x_core_version = Some(value),
            _ => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cache_path.is_none()
            && self.uv_path.is_none()
            && self.python_version.is_none()
            && self.venv_path.is_none()
            && self.r2x_core_version.is_none()
    }

    pub fn values_iter(&self) -> Vec<(&str, String)> {
        let mut values = Vec::new();
        if let Some(ref val) = self.cache_path {
            values.push(("cache-path", val.clone()));
        }
        if let Some(ref val) = self.uv_path {
            values.push(("uv-path", val.clone()));
        }
        if let Some(ref val) = self.python_version {
            values.push(("python-version", val.clone()));
        }
        if let Some(ref val) = self.venv_path {
            values.push(("venv-path", val.clone()));
        }
        if let Some(ref val) = self.r2x_core_version {
            values.push(("r2x-core-version", val.clone()));
        }
        values
    }

    pub fn get_cache_path(&self) -> String {
        self.cache_path.clone().unwrap_or_else(|| {
            #[cfg(not(target_os = "windows"))]
            {
                dirs::home_dir()
                    .expect("Could not determine home directory")
                    .join(".cache")
                    .join("r2x")
                    .to_str()
                    .expect("Invalid path")
                    .to_string()
            }
            #[cfg(target_os = "windows")]
            {
                dirs::cache_dir()
                    .expect("Could not determine cache directory")
                    .join("r2x")
                    .to_str()
                    .expect("Invalid path")
                    .to_string()
            }
        })
    }

    pub fn get_venv_path(&self) -> String {
        // If explicitly configured, use it.
        if let Some(ref p) = self.venv_path {
            return p.clone();
        }

        // Compute platform-default and legacy locations.
        #[cfg(not(target_os = "windows"))]
        {
            // New preferred default: ~/.config/r2x/.venv (hidden folder, avoids spaces on macOS)
            let default = dirs::home_dir()
                .expect("Could not determine home directory")
                .join(".config")
                .join("r2x")
                .join(".venv");

            // Legacy location (may point to macOS 'Application Support' via config_dir)
            let legacy = dirs::config_dir()
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .expect("Could not determine home directory")
                        .join(".config")
                })
                .join("r2x")
                .join(".venv");

            // If a legacy venv exists and the default does not, attempt a best-effort migration
            // by renaming the legacy directory into the default location. If migration fails,
            // prefer returning the legacy path so we don't lose the existing environment.
            if legacy.exists() && !default.exists() {
                if let Some(parent) = default.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::rename(&legacy, &default).is_ok() {
                    return default.to_str().expect("Invalid path").to_string();
                } else {
                    return legacy.to_str().expect("Invalid path").to_string();
                }
            }

            // Otherwise return the default path
            default.to_str().expect("Invalid path").to_string()
        }

        #[cfg(target_os = "windows")]
        {
            // On Windows, use the platform config_dir as before (with .venv hidden folder).
            let path = dirs::config_dir()
                .expect("Could not determine config directory")
                .join("r2x")
                .join(".venv");
            return path.to_str().expect("Invalid path").to_string();
        }
    }

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
            || version.contains(">")
            || version.contains("<")
        {
            format!("r2x-core{}", version)
        } else {
            format!("r2x-core=={}", version)
        }
    }

    pub fn ensure_uv_path(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        // Check if the stored path exists
        if let Some(ref path) = self.uv_path {
            if std::path::Path::new(path).exists() {
                return Ok(path.clone());
            }
            // Path was in config but doesn't exist, clear it
            eprintln!("Stored uv path no longer exists: {}", path);
            self.uv_path = None;
        }

        match which("uv") {
            Ok(path) => {
                let path_str = path.to_string_lossy().trim().to_string();
                self.uv_path = Some(path_str.clone());
                self.save()?;
                return Ok(path_str);
            }
            Err(_) => {
                return Err("uv is not installed. Please install it from: https://docs.astral.sh/uv/getting-started/installation/".into())
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("uv not found. Installing uv using official installer...\n");

            let status = Command::new("sh")
                .arg("-c")
                .arg("curl -LsSf https://astral.sh/uv/install.sh | sh")
                .status()?;

            if !status.success() {
                return Err("Failed to install uv".into());
            }

            eprintln!("\nuv installation completed. Verifying installation...");

            // Verify the installation
            if let Ok(output) = Command::new("which").arg("uv").output() {
                if output.status.success() {
                    let path = String::from_utf8(output.stdout)?.trim().to_string();
                    eprintln!("Found uv at: {}", path);
                    self.uv_path = Some(path.clone());
                    self.save()?;
                    return Ok(path);
                }
            }

            Err("Failed to locate uv after installation. Verify that ~/.local/bin or ~/.cargo/bin is in your PATH".into())
        }
    }

    pub fn ensure_cache_path(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        let cache_path = self.get_cache_path();
        fs::create_dir_all(&cache_path)?;
        Ok(cache_path)
    }

    pub fn ensure_venv_path(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        use std::process::Command;

        let venv_path = self.get_venv_path();

        // Check if venv already exists
        if std::path::Path::new(&venv_path).exists() {
            return Ok(venv_path);
        }

        // Ensure uv is installed first (this will auto-install if needed)
        let uv_path = self.ensure_uv_path()?;

        // Use the Python version from config, or default to 3.12
        let python_version = self.python_version.as_deref().unwrap_or("3.12");

        // Create the venv using uv
        let output = Command::new(&uv_path)
            .args(["venv", &venv_path, "--python", python_version])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to create venv: {}", stderr).into());
        }

        Ok(venv_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = Config::default();
        assert!(config.is_empty());
    }

    #[test]
    fn test_config_set_get() {
        let mut config = Config::default();
        config.set("cache-path", "test-value".to_string());
        assert_eq!(config.get("cache-path"), Some("test-value".to_string()));
    }

    #[test]
    fn test_config_multiple_fields() {
        let mut config = Config::default();
        config.set("cache-path", "/tmp/cache".to_string());
        assert_eq!(config.get("cache-path"), Some("/tmp/cache".to_string()));
        assert!(!config.is_empty());
    }

    #[test]
    fn test_config_unknown_key() {
        let mut config = Config::default();
        config.set("unknown-key", "value".to_string());
        assert_eq!(config.get("unknown-key"), None);
    }

    #[test]
    fn test_config_default_cache_path() {
        let config = Config::default();
        let cache_path = config.get_cache_path();
        assert!(!cache_path.is_empty());
        assert!(cache_path.contains("r2x"));
    }
}
