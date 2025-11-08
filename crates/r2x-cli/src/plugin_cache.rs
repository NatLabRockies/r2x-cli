use crate::config_manager::Config;
use crate::logger;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Top-level package containing plugins (matches Python Package from RUST_PYTHON_INTEROP.md)
///
/// This structure represents the serialized plugin package as defined in the
/// Python serialization spec, enabling type-safe Rust/Python interop.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CachedPackage {
    /// Package name (e.g., "r2x-reeds", "r2x-plexos")
    pub name: String,

    /// List of plugins in this package
    pub plugins: Vec<CachedPlugin>,

    /// Package metadata (version, author, etc.) as JSON for flexibility
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CachedPlugin {
    /// Plugin name (display name, not type-prefixed)
    pub name: String,

    /// Callable object metadata (class or function)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obj: Option<CallableMetadata>,

    /// Plugin type: "parser", "exporter", "sysmod", "upgrader"
    #[serde(rename = "plugin_type")]
    pub plugin_type: String,

    /// Configuration class metadata (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,

    /// Method to call on the callable object (e.g., "build_system", "export")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_method: Option<String>,
}

/// Callable object metadata (matches Python Callable from RUST_PYTHON_INTEROP.md)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CallableMetadata {
    /// Python module path (e.g., "r2x_reeds.parser")
    pub module: String,

    /// Callable name (e.g., "ReEDSParser" or "parse_function")
    pub name: String,

    /// Callable type: "class" or "function"
    #[serde(rename = "type")]
    pub callable_type: String,

    /// Return type annotation (e.g., "System", "None")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_annotation: Option<String>,

    /// Parameter metadata keyed by parameter name
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub parameters: HashMap<String, ParameterMetadata>,
}

/// Parameter metadata for callable or config
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParameterMetadata {
    /// Type annotation as string (e.g., "str | None", "int", "System")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,

    /// Default value as JSON (can be any JSON type: null, number, string, object, array)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// Whether this parameter is required
    pub is_required: bool,
}

impl CachedPackage {
    /// Create a new cached package
    pub fn new(name: String) -> Self {
        CachedPackage {
            name,
            plugins: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Add a plugin to the package
    pub fn add_plugin(&mut self, plugin: CachedPlugin) {
        logger::debug(&format!("Added {} to cache", &plugin.name));
        self.plugins.push(plugin);
    }

    /// Get a plugin by name
    pub fn get_plugin(&self, name: &str) -> Option<&CachedPlugin> {
        self.plugins.iter().find(|p| p.name == name)
    }
}

/// Cache entry for a specific package version
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CacheEntry {
    /// Package version
    pub version: String,

    /// Cached package containing plugins and metadata
    pub package: CachedPackage,

    /// Timestamp when cached (RFC 3339)
    pub cached_at: String,
}

/// Plugin metadata cache
///
/// Stores plugin packages keyed by package name and version. Each entry is versioned
/// to enable cache invalidation when package versions change.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginMetadataCache {
    #[serde(default)]
    pub entries: HashMap<String, CacheEntry>,
}

impl PluginMetadataCache {
    /// Get the path to the metadata cache file
    pub fn cache_file_path() -> Result<PathBuf, String> {
        let config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
        let cache_path = config.get_cache_path();
        Ok(PathBuf::from(cache_path).join("plugin_metadata.toml"))
    }

    /// Load cache from disk, returning empty cache if file doesn't exist
    pub fn load() -> Result<Self, String> {
        let path = Self::cache_file_path()?;

        if !path.exists() {
            return Ok(PluginMetadataCache::default());
        }

        let content =
            fs::read_to_string(&path).map_err(|e| format!("Failed to read cache file: {}", e))?;

        let cache: PluginMetadataCache =
            toml::from_str(&content).map_err(|e| format!("Failed to parse cache: {}", e))?;

        Ok(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::cache_file_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize cache: {}", e))?;

        fs::write(&path, content).map_err(|e| format!("Failed to write cache file: {}", e))?;

        Ok(())
    }

    /// Get cached package by name and version (NEW API)
    ///
    /// Returns the cached package if found and version matches.
    /// Returns None if version mismatch (cache miss) or package not found.
    pub fn get_package(&self, package_name: &str, version: &str) -> Option<&CachedPackage> {
        self.entries.get(package_name).and_then(|entry| {
            if entry.version == version {
                Some(&entry.package)
            } else {
                None
            }
        })
    }

    /// Store package in cache (NEW API)
    pub fn set_package(
        &mut self,
        package_name: String,
        version: String,
        package: CachedPackage,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();

        self.entries.insert(
            package_name,
            CacheEntry {
                version,
                package,
                cached_at: now,
            },
        );

        Ok(())
    }

    /// Extract plugins from cached package as manifest plugins (CONVERSION HELPER)
    ///
    /// Converts CachedPlugin objects to plugin_manifest::Plugin objects.
    /// Used during cache hit to convert typed cache data back to manifest format.
    pub fn extract_plugins(
        cached_package: &CachedPackage,
    ) -> Vec<(String, crate::plugin_manifest::Plugin)> {
        cached_package
            .plugins
            .iter()
            .map(|cached_plugin| {
                let plugin = Self::convert_cached_to_manifest(cached_plugin);
                (cached_plugin.name.clone(), plugin)
            })
            .collect()
    }

    /// Convert CachedPlugin to manifest Plugin (INTERNAL CONVERSION)
    ///
    /// Translates from the cache representation to the manifest representation.
    /// The caller is responsible for setting package_name and install_type.
    fn convert_cached_to_manifest(cached: &CachedPlugin) -> crate::plugin_manifest::Plugin {
        crate::plugin_manifest::Plugin {
            package_name: None, // Set by caller
            plugin_type: Some(cached.plugin_type.clone()),
            description: None,
            doc: None,
            io_type: None,
            call_method: cached.call_method.clone(),
            requires_store: None,
            obj: cached.obj.as_ref().map(|obj| {
                let parameters = obj
                    .parameters
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            crate::plugin_manifest::ParameterMetadata {
                                annotation: v.annotation.clone(),
                                default: v
                                    .default
                                    .as_ref()
                                    .map(|d| serde_json::to_string(d).unwrap_or_default()),
                                is_required: v.is_required,
                            },
                        )
                    })
                    .collect();

                crate::plugin_manifest::CallableMetadata {
                    module: obj.module.clone(),
                    name: obj.name.clone(),
                    callable_type: obj.callable_type.clone(),
                    return_annotation: obj.return_annotation.clone(),
                    parameters,
                }
            }),
            config: cached.config.as_ref().and_then(|config_json| {
                // Try to deserialize config JSON to ConfigMetadata
                serde_json::from_value::<crate::plugin_manifest::ConfigMetadata>(
                    config_json.clone(),
                )
                .ok()
            }),
            upgrader: None,
            install_type: None,
            installed_by: Vec::new(),
        }
    }

    /// Clear the entire cache
    pub fn clear() -> Result<(), String> {
        let path = Self::cache_file_path()?;

        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("Failed to delete cache file: {}", e))?;
        }

        Ok(())
    }

    /// Check if cache has an entry for a package
    pub fn has_entry(&self, package_name: &str) -> bool {
        self.entries.contains_key(package_name)
    }

    /// Remove a package entry from cache (but keep cache file)
    pub fn remove_entry(&mut self, package_name: &str) -> bool {
        self.entries.remove(package_name).is_some()
    }

    /// Get cache statistics for debugging
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            total_entries: self.entries.len(),
            packages: self.entries.keys().cloned().collect(),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub packages: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_new() {
        let cache = PluginMetadataCache::default();
        assert_eq!(cache.entries.len(), 0);
    }

    #[test]
    fn test_cached_package_creation() {
        let mut package = CachedPackage::new("r2x-test".to_string());
        assert_eq!(package.name, "r2x-test");
        assert_eq!(package.plugins.len(), 0);

        let plugin = CachedPlugin {
            name: "test-plugin".to_string(),
            obj: None,
            plugin_type: "parser".to_string(),
            config: None,
            call_method: Some("build".to_string()),
        };

        package.add_plugin(plugin.clone());
        assert_eq!(package.plugins.len(), 1);
        assert!(package.get_plugin("test-plugin").is_some());
    }

    #[test]
    fn test_cached_package_serialization() {
        let mut package = CachedPackage::new("r2x-test".to_string());
        package
            .metadata
            .insert("version".to_string(), serde_json::json!("1.0.0"));

        let plugin = CachedPlugin {
            name: "test-parser".to_string(),
            obj: Some(CallableMetadata {
                module: "test_module".to_string(),
                name: "TestParser".to_string(),
                callable_type: "class".to_string(),
                return_annotation: Some("System".to_string()),
                parameters: HashMap::new(),
            }),
            plugin_type: "parser".to_string(),
            config: None,
            call_method: Some("build_system".to_string()),
        };

        package.add_plugin(plugin);

        // Serialize to JSON and back
        let json = serde_json::to_string(&package).expect("Failed to serialize");
        let restored: CachedPackage = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(restored.name, package.name);
        assert_eq!(restored.plugins.len(), package.plugins.len());
    }

    #[test]
    fn test_cache_set_get_package() {
        let mut cache = PluginMetadataCache::default();
        let package = CachedPackage::new("r2x-test".to_string());

        cache
            .set_package("r2x-test".to_string(), "1.0.0".to_string(), package)
            .unwrap();

        let retrieved = cache.get_package("r2x-test", "1.0.0");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "r2x-test");
    }

    #[test]
    fn test_cache_version_mismatch() {
        let mut cache = PluginMetadataCache::default();
        let package = CachedPackage::new("r2x-test".to_string());

        cache
            .set_package("r2x-test".to_string(), "1.0.0".to_string(), package)
            .unwrap();

        // Different version should not match
        let retrieved = cache.get_package("r2x-test", "2.0.0");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_extract_plugins() {
        let mut package = CachedPackage::new("r2x-test".to_string());

        let plugin = CachedPlugin {
            name: "test-parser".to_string(),
            obj: Some(CallableMetadata {
                module: "test.parser".to_string(),
                name: "Parser".to_string(),
                callable_type: "class".to_string(),
                return_annotation: None,
                parameters: HashMap::new(),
            }),
            plugin_type: "parser".to_string(),
            config: None,
            call_method: Some("build_system".to_string()),
        };

        package.add_plugin(plugin);

        let plugins = PluginMetadataCache::extract_plugins(&package);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].0, "test-parser");
    }

    #[test]
    fn test_cache_remove_entry() {
        let mut cache = PluginMetadataCache::default();
        let package = CachedPackage::new("r2x-test".to_string());

        cache
            .set_package("r2x-test".to_string(), "1.0.0".to_string(), package)
            .unwrap();

        assert!(cache.has_entry("r2x-test"));
        assert!(cache.remove_entry("r2x-test"));
        assert!(!cache.has_entry("r2x-test"));
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = PluginMetadataCache::default();
        let package = CachedPackage::new("r2x-test".to_string());

        cache
            .set_package("r2x-test".to_string(), "1.0.0".to_string(), package)
            .unwrap();

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 1);
        assert!(stats.packages.contains(&"r2x-test".to_string()));
    }

    #[test]
    fn test_cache_serialization_round_trip() {
        let mut cache = PluginMetadataCache::default();

        let mut package = CachedPackage::new("r2x-test".to_string());
        package
            .metadata
            .insert("version".to_string(), serde_json::json!("0.1.0"));

        let plugin = CachedPlugin {
            name: "test-parser".to_string(),
            obj: Some(CallableMetadata {
                module: "test.parser".to_string(),
                name: "TestParser".to_string(),
                callable_type: "class".to_string(),
                return_annotation: None,
                parameters: HashMap::new(),
            }),
            plugin_type: "parser".to_string(),
            config: None,
            call_method: Some("build_system".to_string()),
        };

        package.add_plugin(plugin);

        cache
            .set_package("r2x-test".to_string(), "0.1.0".to_string(), package)
            .unwrap();

        // Simulate serialization round-trip
        let toml_str = toml::to_string_pretty(&cache).expect("Failed to serialize to TOML");
        let restored: PluginMetadataCache =
            toml::from_str(&toml_str).expect("Failed to deserialize from TOML");

        let pkg = restored.get_package("r2x-test", "0.1.0");
        assert!(pkg.is_some());
        assert_eq!(pkg.unwrap().plugins.len(), 1);
    }
}
