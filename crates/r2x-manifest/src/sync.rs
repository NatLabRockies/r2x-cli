//! Sync-optimized hot paths for manifest operations
//!
//! This module provides:
//! - SyncEngine for parallel package discovery and incremental updates
//! - StringInterner for string deduplication across packages
//! - Fast diff using pre-computed hashes

use crate::types::{Manifest, Package};
use ahash::AHashMap;
use parking_lot::RwLock;
use rayon::prelude::*;
use std::sync::Arc;

// =============================================================================
// SYNC RESULT
// =============================================================================

/// Result of a sync operation
#[derive(Debug, Default)]
pub struct SyncResult {
    pub packages_inserted: usize,
    pub packages_updated: usize,
    pub packages_unchanged: usize,
    pub total_plugins: usize,
}

// =============================================================================
// CHANGE TRACKING
// =============================================================================

/// Represents a change to be applied to the manifest
#[derive(Debug)]
pub enum Change {
    Insert(Package),
    Update(Package),
    Remove(Arc<str>),
}

// =============================================================================
// SYNC ENGINE
// =============================================================================

/// Engine for syncing packages to the manifest
///
/// Uses parallel discovery and incremental updates with minimal locking
pub struct SyncEngine {
    manifest: Arc<RwLock<Manifest>>,
    interner: StringInterner,
}

impl SyncEngine {
    /// Create a new sync engine with the given manifest
    pub fn new(manifest: Manifest) -> Self {
        SyncEngine {
            manifest: Arc::new(RwLock::new(manifest)),
            interner: StringInterner::new(),
        }
    }

    /// Get a reference to the interner for string deduplication
    pub fn interner(&self) -> &StringInterner {
        &self.interner
    }

    /// Get a read lock on the manifest
    pub fn manifest(&self) -> parking_lot::RwLockReadGuard<'_, Manifest> {
        self.manifest.read()
    }

    /// Get a write lock on the manifest
    pub fn manifest_mut(&self) -> parking_lot::RwLockWriteGuard<'_, Manifest> {
        self.manifest.write()
    }

    /// Take ownership of the manifest
    pub fn into_manifest(self) -> Manifest {
        Arc::try_unwrap(self.manifest)
            .map_or_else(|arc| arc.read().clone(), |lock| lock.into_inner())
    }

    /// Sync packages - HOT PATH
    ///
    /// Takes discovered packages and efficiently updates the manifest
    pub fn sync_packages(&self, discovered: Vec<Package>) -> SyncResult {
        // 1. Compute hashes for all discovered packages in parallel
        let packages_with_hashes: Vec<Package> = discovered
            .into_par_iter()
            .map(|mut pkg| {
                pkg.compute_hash();
                pkg
            })
            .collect();

        // 2. Diff with read lock only
        let changes = {
            let manifest = self.manifest.read();
            Self::compute_changes(&manifest, &packages_with_hashes)
        };

        // 3. Count results
        let mut result = SyncResult::default();
        for change in &changes {
            match change {
                Change::Insert(_) => result.packages_inserted += 1,
                Change::Update(_) => result.packages_updated += 1,
                Change::Remove(_) => {}
            }
        }
        result.packages_unchanged =
            packages_with_hashes.len() - result.packages_inserted - result.packages_updated;

        // 4. Apply with brief write lock
        if !changes.is_empty() {
            let mut manifest = self.manifest.write();
            Self::apply_changes(&mut manifest, changes);
            manifest.rebuild_indexes();
        }

        // 5. Count plugins
        {
            let manifest = self.manifest.read();
            result.total_plugins = manifest.packages.iter().map(|pkg| pkg.plugins.len()).sum();
        }

        result
    }

    /// Fast diff using pre-computed hashes
    fn compute_changes(old: &Manifest, new: &[Package]) -> Vec<Change> {
        new.par_iter()
            .filter_map(|new_pkg| match old.get_package(&new_pkg.name) {
                Some(old_pkg) if old_pkg.content_hash == new_pkg.content_hash => None,
                Some(_) => Some(Change::Update(new_pkg.clone())),
                None => Some(Change::Insert(new_pkg.clone())),
            })
            .collect()
    }

    /// Apply changes to the manifest
    fn apply_changes(manifest: &mut Manifest, changes: Vec<Change>) {
        for change in changes {
            match change {
                Change::Insert(pkg) => {
                    manifest.packages.push(pkg);
                }
                Change::Update(pkg) => {
                    if let Some(idx) = manifest.package_index.get(&pkg.name) {
                        manifest.packages[*idx] = pkg;
                    } else {
                        // Fallback: find by name
                        if let Some(idx) = manifest.packages.iter().position(|p| p.name == pkg.name)
                        {
                            manifest.packages[idx] = pkg;
                        }
                    }
                }
                Change::Remove(name) => {
                    manifest.packages.retain(|p| p.name != name);
                }
            }
        }
    }

    /// Update a single package in the manifest
    pub fn update_package(&self, package: Package) {
        let mut manifest = self.manifest.write();
        if let Some(idx) = manifest
            .packages
            .iter()
            .position(|p| p.name == package.name)
        {
            manifest.packages[idx] = package;
        } else {
            manifest.packages.push(package);
        }
        manifest.rebuild_indexes();
    }

    /// Remove a package from the manifest
    pub fn remove_package(&self, name: &str) -> bool {
        let mut manifest = self.manifest.write();
        let initial_len = manifest.packages.len();
        manifest.packages.retain(|p| p.name.as_ref() != name);
        let removed = manifest.packages.len() < initial_len;
        if removed {
            manifest.rebuild_indexes();
        }
        removed
    }
}

// =============================================================================
// STRING INTERNER
// =============================================================================

/// String interner for deduplication
///
/// Uses hash-based lookup for O(1) average case deduplication
pub struct StringInterner {
    map: RwLock<AHashMap<u64, Arc<str>>>,
}

impl StringInterner {
    /// Create a new empty interner
    pub fn new() -> Self {
        StringInterner {
            map: RwLock::new(AHashMap::new()),
        }
    }

    /// Intern a string, returning a shared reference
    ///
    /// If the string already exists, returns the existing Arc.
    /// Otherwise, creates a new Arc and stores it.
    #[inline]
    pub fn intern(&self, s: &str) -> Arc<str> {
        use std::hash::{Hash, Hasher};

        let mut hasher = ahash::AHasher::default();
        s.hash(&mut hasher);
        let hash = hasher.finish();

        // Fast path: read lock
        {
            let map = self.map.read();
            if let Some(interned) = map.get(&hash) {
                return Arc::clone(interned);
            }
        }

        // Slow path: write lock
        let mut map = self.map.write();
        map.entry(hash).or_insert_with(|| Arc::from(s)).clone()
    }

    /// Intern a string that's already an Arc
    #[inline]
    pub fn intern_arc(&self, s: Arc<str>) -> Arc<str> {
        use std::hash::{Hash, Hasher};

        let mut hasher = ahash::AHasher::default();
        s.hash(&mut hasher);
        let hash = hasher.finish();

        // Fast path: read lock
        {
            let map = self.map.read();
            if let Some(interned) = map.get(&hash) {
                return Arc::clone(interned);
            }
        }

        // Slow path: write lock
        let mut map = self.map.write();
        map.entry(hash).or_insert(s).clone()
    }

    /// Get the number of interned strings
    pub fn len(&self) -> usize {
        self.map.read().len()
    }

    /// Check if the interner is empty
    pub fn is_empty(&self) -> bool {
        self.map.read().is_empty()
    }

    /// Clear all interned strings
    pub fn clear(&self) {
        self.map.write().clear();
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InstallType, Plugin, PluginType};

    fn create_test_package(name: &str) -> Package {
        Package {
            name: Arc::from(name),
            version: Arc::from("1.0.0"),
            editable_install: false,
            source_uri: None,
            install_type: InstallType::Explicit,
            installed_by: smallvec::SmallVec::new(),
            dependencies: smallvec::SmallVec::new(),
            entry_point: None,
            plugins: vec![Plugin {
                name: Arc::from(format!("{}-plugin", name)),
                plugin_type: PluginType::Class,
                module: Arc::from(format!("{}.plugins", name)),
                class_name: Some(Arc::from("TestPlugin")),
                ..Default::default()
            }],
            configs: Vec::new(),
            content_hash: 0,
            plugin_index: ahash::AHashMap::new(),
        }
    }

    #[test]
    fn test_sync_engine_insert() {
        let manifest = Manifest::default();
        let engine = SyncEngine::new(manifest);

        let packages = vec![create_test_package("test-pkg-1")];
        let result = engine.sync_packages(packages);

        assert_eq!(result.packages_inserted, 1);
        assert_eq!(result.packages_updated, 0);
        assert_eq!(result.total_plugins, 1);
    }

    #[test]
    fn test_sync_engine_update() {
        let mut manifest = Manifest::default();
        let mut pkg = create_test_package("test-pkg-1");
        pkg.compute_hash();
        manifest.packages.push(pkg);
        manifest.rebuild_indexes();

        let engine = SyncEngine::new(manifest);

        // Create updated package with different hash
        let mut updated_pkg = create_test_package("test-pkg-1");
        updated_pkg.version = Arc::from("2.0.0");

        let result = engine.sync_packages(vec![updated_pkg]);

        assert_eq!(result.packages_inserted, 0);
        assert_eq!(result.packages_updated, 1);
    }

    #[test]
    fn test_string_interner() {
        let interner = StringInterner::new();

        let s1 = interner.intern("hello");
        let s2 = interner.intern("hello");
        let s3 = interner.intern("world");

        // Same string should return same Arc
        assert!(Arc::ptr_eq(&s1, &s2));

        // Different strings should return different Arcs
        assert!(!Arc::ptr_eq(&s1, &s3));

        // Should have 2 unique strings
        assert_eq!(interner.len(), 2);
    }

    #[test]
    fn test_string_interner_arc() {
        let interner = StringInterner::new();

        let s1 = Arc::from("test");
        let interned1 = interner.intern_arc(s1);
        let interned2 = interner.intern("test");

        assert!(Arc::ptr_eq(&interned1, &interned2));
    }
}
