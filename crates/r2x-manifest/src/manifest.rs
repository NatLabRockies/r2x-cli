//! Manifest operations - loading, saving, and business logic
//!
//! This module provides the core operations for managing the r2x plugin manifest,
//! including CRUD operations, dependency tracking, and persistence.

use crate::errors::ManifestError;
use crate::types::{InstallType, Manifest, Package, Plugin};
use smallvec::SmallVec;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

impl Manifest {
    /// Get the default path to the manifest file
    pub fn path() -> PathBuf {
        // On Unix/macOS: use ~/.cache/r2x/manifest.toml
        // On Windows: use AppData/Local/r2x/manifest.toml
        #[cfg(not(target_os = "windows"))]
        {
            dirs::home_dir().map_or_else(
                || PathBuf::from(".cache/r2x/manifest.toml"),
                |h| h.join(".cache").join("r2x").join("manifest.toml"),
            )
        }

        #[cfg(target_os = "windows")]
        {
            dirs::cache_dir().map_or_else(
                || PathBuf::from("cache\\r2x\\manifest.toml"),
                |c| c.join("r2x").join("manifest.toml"),
            )
        }
    }

    /// Load manifest from default location, returning empty manifest if file doesn't exist
    pub fn load() -> Result<Self, ManifestError> {
        let path = Self::path();
        Self::load_from_path(&path)
    }

    /// Load manifest from a specific path
    pub fn load_from_path(path: &Path) -> Result<Self, ManifestError> {
        if !path.exists() {
            return Ok(Manifest::default());
        }

        let content = std::fs::read_to_string(path)?;
        let mut manifest: Manifest = toml::from_str(&content)?;
        manifest.rebuild_indexes();
        Ok(manifest)
    }

    /// Save manifest to default location with atomic write
    pub fn save(&self) -> Result<(), ManifestError> {
        let path = Self::path();
        self.save_to_path(&path)
    }

    /// Save manifest to a specific path with atomic write
    pub fn save_to_path(&self, path: &Path) -> Result<(), ManifestError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Serialize
        let content = toml::to_string_pretty(self)?;

        // Atomic write: write to temp file then rename
        let temp_path = path.with_extension("toml.tmp");
        {
            let file = std::fs::File::create(&temp_path)?;
            let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file);
            writer.write_all(content.as_bytes())?;
            writer.flush()?;
        }

        // Atomic rename
        std::fs::rename(&temp_path, path)?;
        Ok(())
    }

    /// Clear all packages and save
    pub fn clear(&mut self) -> Result<(), ManifestError> {
        self.packages.clear();
        self.package_index.clear();
        self.save()
    }

    /// O(1) package lookup by name
    #[inline]
    pub fn get_package(&self, name: &str) -> Option<&Package> {
        self.package_index.get(name).map(|&idx| &self.packages[idx])
    }

    /// O(1) mutable package lookup by name
    #[inline]
    pub fn get_package_mut(&mut self, name: &str) -> Option<&mut Package> {
        self.package_index
            .get(name)
            .copied()
            .map(move |idx| &mut self.packages[idx])
    }

    /// Find or create a package in the manifest
    pub fn get_or_create_package(&mut self, name: &str) -> &mut Package {
        if !self.package_index.contains_key(name) {
            let name_arc: Arc<str> = Arc::from(name);
            let idx = self.packages.len();
            self.packages.push(Package {
                name: name_arc.clone(),
                version: Arc::from("0.0.0"),
                editable_install: false,
                source_uri: None,
                install_type: InstallType::Explicit,
                installed_by: SmallVec::new(),
                dependencies: SmallVec::new(),
                entry_point: None,
                plugins: Vec::new(),
                configs: Vec::new(),
                content_hash: 0,
                plugin_index: ahash::AHashMap::new(),
            });
            self.package_index.insert(name_arc, idx);
        }

        // Safety: we just inserted the package if it didn't exist, so this lookup will succeed
        let idx = self.package_index.get(name).copied().unwrap_or(0);
        &mut self.packages[idx]
    }

    /// Remove a package from the manifest
    pub fn remove_package(&mut self, name: &str) -> bool {
        if let Some(&idx) = self.package_index.get(name) {
            self.packages.remove(idx);
            self.rebuild_indexes();
            true
        } else {
            false
        }
    }

    /// Remove all plugins belonging to a package from the manifest
    pub fn remove_plugins_by_package(&mut self, package_name: &str) -> usize {
        if let Some(pkg) = self.get_package_mut(package_name) {
            let count = pkg.plugins.len();
            pkg.plugins.clear();
            pkg.plugin_index.clear();
            count
        } else {
            0
        }
    }

    /// List all plugins (compatibility method) - returns (plugin_name, package_name) tuples
    pub fn list_plugins(&self) -> Vec<(String, String)> {
        self.packages
            .iter()
            .flat_map(|pkg| {
                pkg.plugins
                    .iter()
                    .map(move |plugin| (plugin.name.to_string(), pkg.name.to_string()))
            })
            .collect()
    }

    /// Check if manifest has no packages
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// List all plugins across all packages (compatibility helper)
    pub fn list_all_plugins(&self) -> Vec<(String, String)> {
        self.list_plugins()
    }

    /// Count total plugins across all packages
    pub fn total_plugin_count(&self) -> usize {
        self.packages.iter().map(|pkg| pkg.plugins.len()).sum()
    }

    /// Mark a package as explicitly installed
    pub fn mark_explicit(&mut self, package_name: &str) {
        if let Some(pkg) = self.get_package_mut(package_name) {
            pkg.install_type = InstallType::Explicit;
        }
    }

    /// Mark a package as a dependency of another package
    pub fn mark_dependency(&mut self, package_name: &str, installed_by: &str) {
        if let Some(pkg) = self.get_package_mut(package_name) {
            pkg.install_type = InstallType::Dependency;
            let installed_by_arc = Arc::from(installed_by);
            if !pkg.installed_by.iter().any(|s| s.as_ref() == installed_by) {
                pkg.installed_by.push(installed_by_arc);
            }
        }
    }

    /// Record that a package depends on another package
    pub fn add_dependency(&mut self, package_name: &str, dependency: &str) {
        if let Some(pkg) = self.get_package_mut(package_name) {
            let dep_arc = Arc::from(dependency);
            if !pkg.dependencies.iter().any(|s| s.as_ref() == dependency) {
                pkg.dependencies.push(dep_arc);
            }
        }
    }

    /// Remove a package and its dependencies if no other packages depend on them
    /// Returns list of packages removed
    pub fn remove_package_with_deps(&mut self, package_name: &str) -> Vec<String> {
        let mut removed = Vec::new();

        // Find the package and its dependencies
        let dependencies: Vec<Arc<str>> = if let Some(pkg) = self.get_package(package_name) {
            pkg.dependencies.iter().cloned().collect()
        } else {
            return removed;
        };

        // Remove the main package
        if self.remove_package(package_name) {
            removed.push(package_name.to_string());
        }

        // Check each dependency
        for dep in dependencies {
            // Remove this package from the dependency's installed_by list
            if let Some(dep_pkg) = self.get_package_mut(&dep) {
                dep_pkg
                    .installed_by
                    .retain(|pkg| pkg.as_ref() != package_name);

                // If no other packages depend on it, remove it
                if dep_pkg.installed_by.is_empty()
                    && dep_pkg.install_type == InstallType::Dependency
                {
                    let dep_name = dep.to_string();
                    if self.remove_package(&dep_name) {
                        removed.push(dep_name);
                    }
                }
            }
        }

        removed
    }

    /// Check if a package can be safely removed (has no dependents)
    pub fn can_remove_package(&self, package_name: &str) -> bool {
        // Check if any other package depends on this one
        !self.packages.iter().any(|pkg| {
            pkg.dependencies
                .iter()
                .any(|dep| dep.as_ref() == package_name)
        })
    }

    /// Get all packages that depend on the given package
    pub fn get_dependents(&self, package_name: &str) -> Vec<String> {
        self.packages
            .iter()
            .filter(|pkg| {
                pkg.dependencies
                    .iter()
                    .any(|dep| dep.as_ref() == package_name)
            })
            .map(|pkg| pkg.name.to_string())
            .collect()
    }

    /// Serialize this Manifest to a JSON string
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(&self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Return the manifest JSON for CLI/UI consumers
    pub fn get_manifest_json() -> String {
        match Manifest::load() {
            Ok(manifest) => manifest.to_json_string(),
            Err(_) => "{}".to_string(),
        }
    }
}

impl Package {
    /// Get a plugin by name with O(1) lookup
    #[inline]
    pub fn get_plugin(&self, name: &str) -> Option<&Plugin> {
        self.plugin_index.get(name).map(|&idx| &self.plugins[idx])
    }

    /// Get a mutable plugin by name
    #[inline]
    pub fn get_plugin_mut(&mut self, name: &str) -> Option<&mut Plugin> {
        self.plugin_index
            .get(name)
            .copied()
            .map(move |idx| &mut self.plugins[idx])
    }

    /// Add a plugin to the package
    pub fn add_plugin(&mut self, plugin: Plugin) {
        let name = plugin.name.clone();
        let idx = self.plugins.len();
        self.plugins.push(plugin);
        self.plugin_index.insert(name, idx);
    }

    /// Remove a plugin from the package
    pub fn remove_plugin(&mut self, name: &str) -> bool {
        if let Some(&idx) = self.plugin_index.get(name) {
            self.plugins.remove(idx);
            self.rebuild_plugin_index();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PluginType;

    #[test]
    fn test_manifest_default() {
        let manifest = Manifest::default();
        assert!(manifest.is_empty());
        assert_eq!(manifest.version.as_ref(), "3.0");
    }

    #[test]
    fn test_get_or_create_package() {
        let mut manifest = Manifest::default();

        let pkg = manifest.get_or_create_package("r2x-test");
        pkg.install_type = InstallType::Explicit;

        assert_eq!(manifest.packages.len(), 1);
        assert_eq!(manifest.packages[0].name.as_ref(), "r2x-test");

        // Verify O(1) lookup works
        assert!(manifest.get_package("r2x-test").is_some());
    }

    #[test]
    fn test_remove_package() {
        let mut manifest = Manifest::default();
        manifest.get_or_create_package("r2x-test");

        assert_eq!(manifest.packages.len(), 1);
        assert!(manifest.remove_package("r2x-test"));
        assert_eq!(manifest.packages.len(), 0);
        assert!(manifest.get_package("r2x-test").is_none());
    }

    #[test]
    fn test_dependency_tracking() {
        let mut manifest = Manifest::default();

        // Create main package
        manifest.get_or_create_package("r2x-main");
        manifest.mark_explicit("r2x-main");

        // Create dependency
        manifest.get_or_create_package("r2x-dep");
        manifest.mark_dependency("r2x-dep", "r2x-main");
        manifest.add_dependency("r2x-main", "r2x-dep");

        // Verify structure - using assert! for test failure semantics
        assert!(
            manifest
                .get_package("r2x-main")
                .is_some_and(|main_pkg| main_pkg.install_type == InstallType::Explicit
                    && main_pkg.dependencies.len() == 1
                    && main_pkg.dependencies[0].as_ref() == "r2x-dep"),
            "Expected r2x-main package with correct dependencies"
        );

        assert!(
            manifest.get_package("r2x-dep").is_some_and(|dep_pkg| dep_pkg.install_type
                == InstallType::Dependency
                && dep_pkg.installed_by.len() == 1
                && dep_pkg.installed_by[0].as_ref() == "r2x-main"),
            "Expected r2x-dep as dependency of r2x-main"
        );
    }

    #[test]
    fn test_remove_with_deps() {
        let mut manifest = Manifest::default();

        // Setup: main package with dependency
        manifest.get_or_create_package("r2x-main");
        manifest.mark_explicit("r2x-main");
        manifest.get_or_create_package("r2x-dep");
        manifest.mark_dependency("r2x-dep", "r2x-main");
        manifest.add_dependency("r2x-main", "r2x-dep");

        // Remove main package
        let removed = manifest.remove_package_with_deps("r2x-main");

        // Both should be removed
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"r2x-main".to_string()));
        assert!(removed.contains(&"r2x-dep".to_string()));
        assert!(manifest.is_empty());
    }

    #[test]
    fn test_shared_dependency_not_removed() {
        let mut manifest = Manifest::default();

        // Setup: two packages sharing a dependency
        manifest.get_or_create_package("r2x-main1");
        manifest.mark_explicit("r2x-main1");
        manifest.get_or_create_package("r2x-main2");
        manifest.mark_explicit("r2x-main2");
        manifest.get_or_create_package("r2x-shared");
        manifest.mark_dependency("r2x-shared", "r2x-main1");
        manifest.mark_dependency("r2x-shared", "r2x-main2");
        manifest.add_dependency("r2x-main1", "r2x-shared");
        manifest.add_dependency("r2x-main2", "r2x-shared");

        // Remove only main1
        let removed = manifest.remove_package_with_deps("r2x-main1");

        // Only main1 should be removed, shared is still used by main2
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "r2x-main1");
        assert_eq!(manifest.packages.len(), 2);

        assert!(
            manifest.get_package("r2x-shared").is_some_and(|shared| shared.installed_by.len() == 1
                && shared.installed_by[0].as_ref() == "r2x-main2"),
            "Expected r2x-shared with r2x-main2 as installer"
        );
    }

    #[test]
    fn test_clear_manifest() {
        let mut manifest = Manifest::default();
        manifest.get_or_create_package("r2x-test");

        assert!(!manifest.is_empty());
        manifest.packages.clear();
        manifest.package_index.clear();
        assert!(manifest.is_empty());
    }

    #[test]
    fn test_plugin_operations() {
        let mut pkg = Package {
            name: Arc::from("test-package"),
            ..Default::default()
        };

        let plugin = Plugin {
            name: Arc::from("test-plugin"),
            plugin_type: PluginType::Class,
            module: Arc::from("test.module"),
            class_name: Some(Arc::from("TestClass")),
            ..Default::default()
        };

        pkg.add_plugin(plugin);

        // Test O(1) lookup
        assert!(pkg.get_plugin("test-plugin").is_some());
        assert!(pkg.get_plugin("nonexistent").is_none());

        // Test removal
        assert!(pkg.remove_plugin("test-plugin"));
        assert!(pkg.get_plugin("test-plugin").is_none());
    }
}
