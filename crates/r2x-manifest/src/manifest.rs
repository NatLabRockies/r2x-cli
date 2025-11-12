//! Manifest operations - loading, saving, and business logic
//!
//! This module provides the core operations for managing the r2x plugin manifest,
//! including CRUD operations, dependency tracking, and persistence.

use super::types::{Manifest, Metadata, Package};
use crate::errors::ManifestError;
use std::path::PathBuf;

impl Manifest {
    /// Get the default path to the manifest file
    pub fn path() -> PathBuf {
        // On Unix/macOS: use ~/.cache/r2x/manifest.toml
        // On Windows: use AppData/Local/r2x/manifest.toml
        #[cfg(not(target_os = "windows"))]
        {
            dirs::home_dir()
                .expect("Could not determine home directory")
                .join(".cache")
                .join("r2x")
                .join("manifest.toml")
        }

        #[cfg(target_os = "windows")]
        {
            dirs::cache_dir()
                .expect("Could not determine cache directory")
                .join("r2x")
                .join("manifest.toml")
        }
    }

    /// Load manifest from default location, returning empty manifest if file doesn't exist
    pub fn load() -> Result<Self, ManifestError> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Manifest {
                metadata: Metadata {
                    version: "1.0".to_string(),
                    generated_at: chrono::Utc::now().to_rfc3339(),
                    uv_lock_path: None,
                },
                packages: Vec::new(),
            });
        }

        let content = std::fs::read_to_string(&path)?;
        let manifest: Manifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    /// Save manifest to default location
    pub fn save(&self) -> Result<(), ManifestError> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Clear all packages and save
    pub fn clear(&mut self) -> Result<(), ManifestError> {
        self.packages.clear();
        self.save()
    }

    /// Find or create a package in the manifest
    pub fn get_or_create_package(&mut self, name: &str) -> &mut Package {
        if !self.packages.iter().any(|p| p.name == name) {
            self.packages.push(Package {
                name: name.to_string(),
                entry_points_dist_info: String::new(),
                editable_install: false,
                pth_file: None,
                resolved_source_path: None,
                install_type: None,
                installed_by: Vec::new(),
                dependencies: Vec::new(),
                plugins: Vec::new(),
                decorator_registrations: Vec::new(),
            });
        }
        self.packages.iter_mut().find(|p| p.name == name).unwrap()
    }

    /// Remove a package from the manifest
    pub fn remove_package(&mut self, name: &str) -> bool {
        let initial_len = self.packages.len();
        self.packages.retain(|p| p.name != name);
        self.packages.len() < initial_len
    }

    /// Remove all plugins belonging to a package from the manifest
    pub fn remove_plugins_by_package(&mut self, package_name: &str) -> usize {
        let mut count = 0;
        for pkg in &mut self.packages {
            if pkg.name == package_name {
                count = pkg.plugins.len();
                pkg.plugins.clear();
            }
        }
        count
    }

    /// Remove decorator registrations for a package
    pub fn remove_decorator_registrations(&mut self, package_name: &str) -> bool {
        for pkg in &mut self.packages {
            if pkg.name == package_name {
                let had_regs = !pkg.decorator_registrations.is_empty();
                pkg.decorator_registrations.clear();
                return had_regs;
            }
        }
        false
    }

    /// List all plugins (compatibility method) - returns (plugin_name, package_name) tuples
    pub fn list_plugins(&self) -> Vec<(String, String)> {
        self.packages
            .iter()
            .flat_map(|pkg| {
                pkg.plugins
                    .iter()
                    .map(move |plugin| (plugin.name.clone(), pkg.name.clone()))
            })
            .collect()
    }

    /// Check if manifest has no packages
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// List all plugins across all packages (compatibility helper)
    pub fn list_all_plugins(&self) -> Vec<(String, String)> {
        self.packages
            .iter()
            .flat_map(|pkg| {
                pkg.plugins
                    .iter()
                    .map(move |plugin| (plugin.name.clone(), pkg.name.clone()))
            })
            .collect()
    }

    /// Count total plugins across all packages
    pub fn total_plugin_count(&self) -> usize {
        self.packages.iter().map(|pkg| pkg.plugins.len()).sum()
    }

    /// Mark a package as explicitly installed
    pub fn mark_explicit(&mut self, package_name: &str) {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == package_name) {
            pkg.install_type = Some("explicit".to_string());
        }
    }

    /// Mark a package as a dependency of another package
    pub fn mark_dependency(&mut self, package_name: &str, installed_by: &str) {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == package_name) {
            pkg.install_type = Some("dependency".to_string());
            if !pkg.installed_by.contains(&installed_by.to_string()) {
                pkg.installed_by.push(installed_by.to_string());
            }
        }
    }

    /// Record that a package depends on another package
    pub fn add_dependency(&mut self, package_name: &str, dependency: &str) {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == package_name) {
            if !pkg.dependencies.contains(&dependency.to_string()) {
                pkg.dependencies.push(dependency.to_string());
            }
        }
    }

    /// Remove a package and its dependencies if no other packages depend on them
    /// Returns list of packages removed
    pub fn remove_package_with_deps(&mut self, package_name: &str) -> Vec<String> {
        let mut removed = Vec::new();

        // Find the package and its dependencies
        let dependencies = if let Some(pkg) = self.packages.iter().find(|p| p.name == package_name)
        {
            pkg.dependencies.clone()
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
            if let Some(dep_pkg) = self.packages.iter_mut().find(|p| p.name == dep) {
                dep_pkg.installed_by.retain(|pkg| pkg != package_name);

                // If no other packages depend on it, remove it
                if dep_pkg.installed_by.is_empty()
                    && dep_pkg.install_type.as_deref() == Some("dependency")
                {
                    if self.remove_package(&dep) {
                        removed.push(dep);
                    }
                }
            }
        }

        removed
    }

    /// Check if a package can be safely removed (has no dependents)
    pub fn can_remove_package(&self, package_name: &str) -> bool {
        // Check if any other package depends on this one
        !self
            .packages
            .iter()
            .any(|pkg| pkg.dependencies.contains(&package_name.to_string()))
    }

    /// Get all packages that depend on the given package
    pub fn get_dependents(&self, package_name: &str) -> Vec<String> {
        self.packages
            .iter()
            .filter(|pkg| pkg.dependencies.contains(&package_name.to_string()))
            .map(|pkg| pkg.name.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_default() {
        let manifest = Manifest::default();
        assert!(manifest.is_empty());
        assert_eq!(manifest.metadata.version, "1.0");
    }

    #[test]
    fn test_get_or_create_package() {
        let mut manifest = Manifest::default();

        let pkg = manifest.get_or_create_package("r2x-test");
        pkg.install_type = Some("explicit".to_string());

        assert_eq!(manifest.packages.len(), 1);
        assert_eq!(manifest.packages[0].name, "r2x-test");
    }

    #[test]
    fn test_remove_package() {
        let mut manifest = Manifest::default();
        manifest.get_or_create_package("r2x-test");

        assert_eq!(manifest.packages.len(), 1);
        assert!(manifest.remove_package("r2x-test"));
        assert_eq!(manifest.packages.len(), 0);
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

        // Verify structure
        let main_pkg = manifest
            .packages
            .iter()
            .find(|p| p.name == "r2x-main")
            .unwrap();
        assert_eq!(main_pkg.install_type, Some("explicit".to_string()));
        assert_eq!(main_pkg.dependencies, vec!["r2x-dep"]);

        let dep_pkg = manifest
            .packages
            .iter()
            .find(|p| p.name == "r2x-dep")
            .unwrap();
        assert_eq!(dep_pkg.install_type, Some("dependency".to_string()));
        assert_eq!(dep_pkg.installed_by, vec!["r2x-main"]);
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

        let shared = manifest
            .packages
            .iter()
            .find(|p| p.name == "r2x-shared")
            .unwrap();
        assert_eq!(shared.installed_by, vec!["r2x-main2"]);
    }

    #[test]
    fn test_clear_manifest() {
        let mut manifest = Manifest::default();
        manifest.get_or_create_package("r2x-test");

        assert!(!manifest.is_empty());
        manifest.packages.clear();
        assert!(manifest.is_empty());
    }
}
