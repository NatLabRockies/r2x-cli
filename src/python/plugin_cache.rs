//! Plugin list caching to improve performance

use crate::python::plugin::PluginRegistry;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tracing::debug;

const CACHE_FILENAME: &str = "plugin_list.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedPluginList {
    pub plugins: PluginRegistry,
    pub timestamp: SystemTime,
}

impl CachedPluginList {
    pub fn new(plugins: PluginRegistry) -> Self {
        Self {
            plugins,
            timestamp: SystemTime::now(),
        }
    }

    pub fn age(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.timestamp)
            .unwrap_or(Duration::from_secs(u64::MAX))
    }

    pub fn is_expired(&self, ttl_hours: u64) -> bool {
        self.age() > Duration::from_secs(ttl_hours * 3600)
    }
}

fn get_cache_path() -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir()
        .ok_or(crate::R2xError::NoCacheDir)?
        .join("r2x");

    Ok(cache_dir.join(CACHE_FILENAME))
}

/// Load cached plugin list if it exists and is not expired
pub fn load_cached_plugins() -> Result<Option<CachedPluginList>> {
    let cache_path = get_cache_path()?;

    if !cache_path.exists() {
        return Ok(None);
    }

    let cache_data = std::fs::read_to_string(&cache_path)?;
    let cached: CachedPluginList = serde_json::from_str(&cache_data)
        .map_err(|e| crate::R2xError::ConfigError(format!("Failed to parse cache: {}", e)))?;

    // Load config to get TTL
    let config_path = dirs::cache_dir()
        .ok_or(crate::R2xError::NoCacheDir)?
        .join("r2x")
        .join("config.toml");

    let ttl_hours = if config_path.exists() {
        let config_str = std::fs::read_to_string(&config_path)?;
        let config: crate::config::Config = toml::from_str(&config_str)
            .map_err(|e| crate::R2xError::ConfigError(format!("Failed to parse config: {}", e)))?;
        config.plugins.cache_ttl_hours
    } else {
        24 // Default TTL
    };

    if cached.is_expired(ttl_hours) {
        debug!(
            "Cache expired (age: {}h, ttl: {}h)",
            cached.age().as_secs() / 3600,
            ttl_hours
        );
        return Ok(None);
    }

    Ok(Some(cached))
}

/// Save plugin list to cache
pub fn save_cached_plugins(plugins: &PluginRegistry) -> Result<()> {
    let cache_path = get_cache_path()?;

    // Ensure cache directory exists
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let cached = CachedPluginList::new(plugins.clone());
    let cache_data = serde_json::to_string_pretty(&cached)
        .map_err(|e| crate::R2xError::ConfigError(format!("Failed to serialize cache: {}", e)))?;
    std::fs::write(&cache_path, cache_data)?;

    debug!("Saved plugin list to cache: {:?}", cache_path);

    Ok(())
}

/// Invalidate the cache (delete the cache file)
pub fn invalidate_cache() -> Result<()> {
    let cache_path = get_cache_path()?;

    if cache_path.exists() {
        std::fs::remove_file(&cache_path)?;
        debug!("Invalidated plugin cache: {:?}", cache_path);
    }

    Ok(())
}
