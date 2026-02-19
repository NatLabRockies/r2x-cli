use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Resolve installed package paths from site-packages (optionally using UV cache).
#[derive(Debug, Clone)]
pub struct PackageLocator {
    site_packages: PathBuf,
    uv_cache_dir: Option<PathBuf>,
    /// Cached directory entries: filename -> full path
    dir_entries: HashMap<String, PathBuf>,
}

impl PackageLocator {
    /// Create a new locator for the given site-packages root.
    pub fn new(site_packages: PathBuf, uv_cache_dir: Option<PathBuf>) -> Result<Self> {
        debug!("Initializing package locator for: {:?}", site_packages);

        if !site_packages.exists() {
            return Err(anyhow!(
                "Site-packages directory not found: {}",
                site_packages.display()
            ));
        }

        // Read directory once and cache all entries
        let mut dir_entries = HashMap::new();
        let entries = fs::read_dir(&site_packages)?;
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            dir_entries.insert(filename, entry.path());
        }

        Ok(PackageLocator {
            site_packages,
            uv_cache_dir,
            dir_entries,
        })
    }

    /// Return the site-packages root used by this locator.
    pub fn site_packages(&self) -> &Path {
        &self.site_packages
    }

    /// Return an iterator over the cached directory entries (filename -> path).
    pub fn dir_entries(&self) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.dir_entries.iter()
    }

    /// Find the `.dist-info` directory for a given package name.
    ///
    /// Looks for a directory matching `{normalized_name}-*.dist-info` pattern.
    pub fn find_dist_info_path(&self, package_name: &str) -> Option<PathBuf> {
        let normalized = package_name.replace('-', "_");
        let prefix = format!("{}-", normalized);

        for (filename, path) in &self.dir_entries {
            if filename.starts_with(&prefix) && filename.ends_with(".dist-info") {
                return Some(path.clone());
            }
        }
        None
    }

    /// Find the `entry_points.txt` file for a given package.
    ///
    /// Returns the path if it exists inside the package's dist-info directory.
    pub fn find_entry_points_txt(&self, package_name: &str) -> Option<PathBuf> {
        let dist_info = self.find_dist_info_path(package_name)?;
        let entry_points = dist_info.join("entry_points.txt");
        if entry_points.exists() {
            Some(entry_points)
        } else {
            None
        }
    }

    /// Check if a package has r2x plugin entry points.
    ///
    /// Returns true if the package's `entry_points.txt` contains `[r2x_plugin]`
    /// or any `[r2x.*]` section.
    pub fn has_plugin_entry_points(&self, package_name: &str) -> bool {
        let Some(entry_points_path) = self.find_entry_points_txt(package_name) else {
            return false;
        };

        let Ok(content) = fs::read_to_string(&entry_points_path) else {
            return false;
        };

        // Check for [r2x_plugin] or [r2x.*] sections
        for line in content.lines() {
            let line = line.trim();
            if line == "[r2x_plugin]" {
                return true;
            }
            // Match [r2x.something] pattern
            if line.starts_with("[r2x.") && line.ends_with(']') {
                return true;
            }
        }
        false
    }

    /// Locate a package root suitable for AST discovery.
    pub fn find_package_path(&self, package_name_full: &str) -> Result<PathBuf> {
        let normalized = package_name_full.replace('-', "_");

        if let Some(path) = self.find_package_path_via_pth(&normalized) {
            return Ok(path);
        }

        let direct = self.site_packages.join(&normalized);
        if direct.is_dir() {
            return Ok(direct);
        }

        let mut dist_info_path: Option<PathBuf> = None;
        let dist_prefix = format!("{}-", normalized);

        // Use cached directory entries instead of read_dir
        for (name_str, path) in &self.dir_entries {
            if name_str == &normalized {
                return Ok(path.clone());
            }
            if name_str.starts_with(&dist_prefix) && name_str.ends_with(".dist-info") {
                dist_info_path = Some(path.clone());
            }
        }

        if let Some(dist_info) = dist_info_path {
            if let Some(resolved) = self.resolve_from_dist_info(&dist_info) {
                return Ok(resolved);
            }
            debug!(
                "Found dist-info for '{}' but could not resolve top-level module",
                package_name_full
            );
            return Ok(self.site_packages.clone());
        }

        Err(anyhow!(
            "Package '{}' not found in site-packages: {}",
            package_name_full,
            self.site_packages.display()
        ))
    }

    fn find_package_path_via_pth(&self, normalized_package_name: &str) -> Option<PathBuf> {
        let cache_dir = self.uv_cache_dir.as_ref()?;
        if !cache_dir.exists() {
            return None;
        }

        let hash_dirs = fs::read_dir(cache_dir).ok()?;
        for hash_entry in hash_dirs {
            let hash_entry = match hash_entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            let hash_path = hash_entry.path();
            if !hash_path.is_dir() {
                continue;
            }

            let pth_entries = match fs::read_dir(&hash_path) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for pth_entry in pth_entries {
                let pth_entry = match pth_entry {
                    Ok(entry) => entry,
                    Err(_) => continue,
                };

                let pth_file_name = pth_entry.file_name().to_string_lossy().to_string();
                let matches = pth_file_name == format!("{}.pth", normalized_package_name)
                    || (pth_file_name.starts_with("__editable__.")
                        && pth_file_name.contains(&format!("{}-", normalized_package_name))
                        && pth_file_name.ends_with(".pth"));

                if !matches {
                    continue;
                }

                if let Ok(content) = fs::read_to_string(pth_entry.path()) {
                    for line in content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        let candidate = PathBuf::from(line);
                        if candidate.exists() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }

        None
    }

    fn resolve_from_dist_info(&self, dist_info_path: &Path) -> Option<PathBuf> {
        let top_level = dist_info_path.join("top_level.txt");
        let content = fs::read_to_string(&top_level).ok()?;
        for line in content.lines() {
            let module = line.trim();
            if module.is_empty() {
                continue;
            }
            let module_dir = self.site_packages.join(module);
            if module_dir.is_dir() {
                return Some(module_dir);
            }
            let module_file = self.site_packages.join(format!("{}.py", module));
            if module_file.is_file() {
                return Some(self.site_packages.clone());
            }
        }
        None
    }
}

/// Information about a discovered r2x package
#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    /// Distribution name (e.g., "r2x-reeds")
    pub name: String,
    /// Whether this was explicitly installed or a dependency
    pub is_explicit: bool,
    /// Root directory of the package
    pub location: PathBuf,
    /// Path to entry_points.txt in dist-info
    pub entry_points_file: PathBuf,
    /// Whether this is an editable install
    pub is_editable: bool,
    /// .pth file path if editable
    pub pth_file: Option<PathBuf>,
    /// Resolved source path if editable
    pub resolved_source_path: Option<PathBuf>,
}

/// Discovers r2x packages in a Python environment
pub struct PackageDiscoverer<'a> {
    /// Reference to the package locator with cached directory entries
    locator: &'a PackageLocator,
}

impl<'a> PackageDiscoverer<'a> {
    /// Create a new discovery instance using the given package locator
    pub fn new(locator: &'a PackageLocator) -> Self {
        debug!(
            "Initializing package discovery for: {:?}",
            locator.site_packages()
        );
        PackageDiscoverer { locator }
    }

    /// Discover all packages with r2x plugin entry points in site-packages
    pub fn discover_packages(&self) -> Vec<DiscoveredPackage> {
        debug!(
            "Discovering r2x packages in: {:?}",
            self.locator.site_packages()
        );

        let mut packages = Vec::new();

        for (file_name, path) in self.locator.dir_entries() {
            // Look for dist-info directories
            if file_name.ends_with(".dist-info") {
                if let Some(pkg) = self.process_dist_info(path, file_name) {
                    debug!("Discovered package: {}", pkg.name);
                    packages.push(pkg);
                }
            }
        }

        info!("Found {} r2x packages", packages.len());
        packages
    }

    /// Process a single .dist-info directory.
    /// Returns Some(package) if this package declares r2x plugin entry points, None otherwise.
    fn process_dist_info(
        &self,
        dist_info_path: &Path,
        dist_info_name: &str,
    ) -> Option<DiscoveredPackage> {
        // Extract package name from dist-info (e.g., r2x_reeds-1.2.3.dist-info -> r2x-reeds)
        let package_name = dist_info_name
            .strip_suffix(".dist-info")?
            .split('-')
            .next()?
            .replace('_', "-");

        // Skip r2x-core (the shared runtime, not a plugin)
        if package_name == "r2x-core" {
            return None;
        }

        // Check for entry_points.txt with r2x plugin entry points
        let entry_points_file = dist_info_path.join("entry_points.txt");
        let entry_points_content = fs::read_to_string(&entry_points_file).ok()?;

        // Filter: must have [r2x_plugin] or [r2x.*] section
        if !Self::has_r2x_entry_points(&entry_points_content) {
            return None;
        }

        debug!("Processing dist-info for: {}", package_name);

        // Get package location (parent directory of dist-info)
        let location = dist_info_path.parent()?.to_path_buf();

        // Check if it's an editable install
        let (is_editable, pth_file, resolved_source_path) =
            self.check_editable_install(&package_name);

        Some(DiscoveredPackage {
            name: package_name,
            is_explicit: true, // TODO: Read from installed.json to distinguish
            location,
            entry_points_file,
            is_editable,
            pth_file,
            resolved_source_path,
        })
    }

    /// Check if entry_points.txt content contains r2x plugin entry points.
    /// Matches [r2x_plugin] or any [r2x.*] section (for transform plugins etc).
    fn has_r2x_entry_points(content: &str) -> bool {
        for line in content.lines() {
            let line = line.trim();
            if line == "[r2x_plugin]" {
                return true;
            }
            // Match [r2x.something] pattern
            if line.starts_with("[r2x.") && line.ends_with(']') {
                return true;
            }
        }
        false
    }

    /// Check if package is an editable install and resolve source path
    fn check_editable_install(
        &self,
        package_name: &str,
    ) -> (bool, Option<PathBuf>, Option<PathBuf>) {
        let normalized_name = package_name.replace('-', "_");
        debug!(
            "Looking for editable install marker for: {}",
            normalized_name
        );

        for (file_name, path) in self.locator.dir_entries() {
            if file_name.ends_with(".pth") && file_name.contains(&normalized_name) {
                // Try to read the .pth file and resolve the actual source path
                if let Ok(content) = fs::read_to_string(path) {
                    if let Ok(resolved_path) = Self::resolve_pth_path(&content) {
                        debug!(
                            "Found editable install for {} at: {:?}",
                            package_name, resolved_path
                        );
                        return (true, Some(path.clone()), Some(resolved_path));
                    }
                }
            }
        }

        (false, None, None)
    }

    /// Parse .pth file content and resolve the actual source path
    fn resolve_pth_path(content: &str) -> Result<PathBuf> {
        // .pth files can contain multiple lines, typically with import statements
        // For editable installs, usually just contains a path
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Try to parse as a direct path
            let path = PathBuf::from(line);
            if path.exists() && path.is_dir() {
                return Ok(path);
            }
        }

        Err(anyhow!("Cannot resolve path from .pth content"))
    }
}

/// Parse entry_points.txt and extract r2x_plugin entry point
pub fn parse_entry_points(entry_points_path: &Path) -> Result<(String, String)> {
    let content = fs::read_to_string(entry_points_path)?;
    let mut in_r2x_section = false;
    let mut module = String::new();
    let mut function = String::new();

    for line in content.lines() {
        let line = line.trim();

        if line == "[r2x_plugin]" {
            in_r2x_section = true;
            continue;
        }

        if in_r2x_section {
            if line.starts_with('[') {
                // Entered another section
                break;
            }

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse "key = module:function" format
            if let Some(eq_idx) = line.find('=') {
                let value = line[eq_idx + 1..].trim();
                if let Some(colon_idx) = value.find(':') {
                    module = value[..colon_idx].trim().to_string();
                    function = value[colon_idx + 1..].trim().to_string();
                    break;
                }
            }
        }
    }

    if module.is_empty() || function.is_empty() {
        return Err(anyhow!(
            "No valid r2x_plugin entry point found in: {}",
            entry_points_path.display()
        ));
    }

    debug!("Parsed entry point: {}:{}", module, function);
    Ok((module, function))
}

#[cfg(test)]
mod tests {
    use crate::package_discovery::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_entry_points() {
        let content = r"[r2x_plugin]
reeds = r2x_reeds.plugins:register_plugin

[other]
something = some.module:function
";

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_entry_points.txt");
        let Ok(()) = fs::write(&temp_file, content) else {
            return;
        };

        let result = parse_entry_points(&temp_file);
        assert!(result.is_ok(), "Failed to parse entry points");
        assert!(result.is_ok_and(|r| r.0 == "r2x_reeds.plugins" && r.1 == "register_plugin"));

        let _ = fs::remove_file(&temp_file);
    }

    #[test]
    fn test_parse_entry_points_multiple_entries() {
        let content = r"[r2x_plugin]
reeds = r2x_reeds.plugins:register_plugin
other = other.module:func
";

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_entry_points2.txt");
        let Ok(()) = fs::write(&temp_file, content) else {
            return;
        };

        let result = parse_entry_points(&temp_file);
        assert!(result.is_ok(), "Failed to parse entry points");
        // Should get the first one
        assert!(result.is_ok_and(|r| r.0 == "r2x_reeds.plugins" && r.1 == "register_plugin"));

        let _ = fs::remove_file(&temp_file);
    }

    #[test]
    fn test_dir_entries_cache_populated() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create some dist-info directories
        if let Err(err) = fs::create_dir(site_packages.join("r2x_reeds-1.0.0.dist-info")) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }
        if let Err(err) = fs::create_dir(site_packages.join("r2x_sienna-2.1.0.dist-info")) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }
        if let Err(err) = fs::create_dir(site_packages.join("some_other_package-0.1.0.dist-info")) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        // Verify the cache contains all the entries
        let entries: Vec<&String> = locator.dir_entries().map(|(name, _)| name).collect();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&&"r2x_reeds-1.0.0.dist-info".to_string()));
        assert!(entries.contains(&&"r2x_sienna-2.1.0.dist-info".to_string()));
        assert!(entries.contains(&&"some_other_package-0.1.0.dist-info".to_string()));
    }

    #[test]
    fn test_find_dist_info_path() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create dist-info directories
        let reeds_dist_info = site_packages.join("r2x_reeds-1.0.0.dist-info");
        if let Err(err) = fs::create_dir(&reeds_dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create reeds dist-info: {err}"
            );
            return;
        }

        let sienna_dist_info = site_packages.join("r2x_sienna-2.1.0.dist-info");
        if let Err(err) = fs::create_dir(&sienna_dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create sienna dist-info: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        // Test finding by package name (with hyphen, should normalize to underscore)
        let found = locator.find_dist_info_path("r2x-reeds");
        assert!(found.is_some());
        assert!(
            found.as_ref().is_some_and(|p| *p == reeds_dist_info),
            "Expected reeds_dist_info path"
        );

        // Test finding with underscore
        let found = locator.find_dist_info_path("r2x_sienna");
        assert!(found.is_some());
        assert!(
            found.as_ref().is_some_and(|p| *p == sienna_dist_info),
            "Expected sienna_dist_info path"
        );

        // Test non-existent package
        let not_found = locator.find_dist_info_path("nonexistent-package");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_has_plugin_entry_points_true() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create dist-info with [r2x_plugin] section
        let dist_info = site_packages.join("r2x_reeds-1.0.0.dist-info");
        if let Err(err) = fs::create_dir(&dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }

        let entry_points_content = r"[console_scripts]
some-cli = r2x_reeds.cli:main

[r2x_plugin]
reeds = r2x_reeds.plugins:register_plugin
";
        if let Err(err) = fs::write(dist_info.join("entry_points.txt"), entry_points_content) {
            assert!(
                err.to_string().is_empty(),
                "Failed to write entry_points.txt: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        assert!(locator.has_plugin_entry_points("r2x-reeds"));
    }

    #[test]
    fn test_has_plugin_entry_points_false() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create dist-info WITHOUT any r2x sections
        let dist_info = site_packages.join("some_package-1.0.0.dist-info");
        if let Err(err) = fs::create_dir(&dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }

        let entry_points_content = r"[console_scripts]
some-cli = some_package.cli:main

[other_section]
foo = bar.baz:qux
";
        if let Err(err) = fs::write(dist_info.join("entry_points.txt"), entry_points_content) {
            assert!(
                err.to_string().is_empty(),
                "Failed to write entry_points.txt: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        assert!(!locator.has_plugin_entry_points("some-package"));
    }

    #[test]
    fn test_has_plugin_entry_points_transforms() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create dist-info with [r2x.transforms] section (not [r2x_plugin])
        let dist_info = site_packages.join("r2x_transforms-1.0.0.dist-info");
        if let Err(err) = fs::create_dir(&dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }

        let entry_points_content = r"[console_scripts]
transform-cli = r2x_transforms.cli:main

[r2x.transforms]
my_transform = r2x_transforms.transform:MyTransform
";
        if let Err(err) = fs::write(dist_info.join("entry_points.txt"), entry_points_content) {
            assert!(
                err.to_string().is_empty(),
                "Failed to write entry_points.txt: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        // Should return true because [r2x.transforms] matches the [r2x.*] pattern
        assert!(locator.has_plugin_entry_points("r2x-transforms"));
    }

    #[test]
    fn test_has_plugin_entry_points_no_entry_points_file() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create dist-info WITHOUT entry_points.txt
        let dist_info = site_packages.join("no_entry_points-1.0.0.dist-info");
        if let Err(err) = fs::create_dir(&dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        // Should return false when entry_points.txt doesn't exist
        assert!(!locator.has_plugin_entry_points("no-entry-points"));
    }

    #[test]
    fn test_find_entry_points_txt() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = temp_dir.path();

        // Create dist-info with entry_points.txt
        let dist_info = site_packages.join("test_package-1.0.0.dist-info");
        if let Err(err) = fs::create_dir(&dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info: {err}"
            );
            return;
        }

        let entry_points_path = dist_info.join("entry_points.txt");
        if let Err(err) = fs::write(&entry_points_path, "[r2x_plugin]\ntest = test:register") {
            assert!(
                err.to_string().is_empty(),
                "Failed to write entry_points.txt: {err}"
            );
            return;
        }

        let locator = match PackageLocator::new(site_packages.to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create locator: {err}"
                );
                return;
            }
        };

        let found = locator.find_entry_points_txt("test-package");
        assert!(found.is_some());
        assert!(
            found.as_ref().is_some_and(|p| *p == entry_points_path),
            "Expected entry_points_path"
        );

        // Test non-existent package
        let not_found = locator.find_entry_points_txt("nonexistent");
        assert!(not_found.is_none());
    }
}
