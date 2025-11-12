use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

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
pub struct PackageDiscoverer {
    /// Site-packages directory
    site_packages: PathBuf,
}

impl PackageDiscoverer {
    /// Create a new discovery instance for the given site-packages path
    pub fn new(site_packages: PathBuf) -> Result<Self> {
        debug!("Initializing package discovery for: {:?}", site_packages);

        if !site_packages.exists() {
            return Err(anyhow!(
                "Site-packages directory not found: {:?}",
                site_packages
            ));
        }

        Ok(PackageDiscoverer { site_packages })
    }

    /// Discover all r2x-* packages in site-packages
    pub fn discover_packages(&self) -> Result<Vec<DiscoveredPackage>> {
        debug!("Discovering r2x packages in: {:?}", self.site_packages);

        let mut packages = Vec::new();
        let entries = fs::read_dir(&self.site_packages)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            // Look for dist-info directories
            if file_name_str.ends_with(".dist-info") {
                if let Ok(pkg) = self.process_dist_info(&path, &file_name_str) {
                    debug!("Discovered package: {}", pkg.name);
                    packages.push(pkg);
                }
            }
        }

        info!("Found {} r2x packages", packages.len());
        Ok(packages)
    }

    /// Process a single .dist-info directory
    fn process_dist_info(
        &self,
        dist_info_path: &Path,
        dist_info_name: &str,
    ) -> Result<DiscoveredPackage> {
        // Extract package name from dist-info (e.g., r2x_reeds-1.2.3.dist-info -> r2x-reeds)
        let package_name = dist_info_name
            .strip_suffix(".dist-info")
            .ok_or_else(|| anyhow!("Invalid dist-info name: {}", dist_info_name))?
            .split('-')
            .next()
            .ok_or_else(|| anyhow!("Cannot extract package name from: {}", dist_info_name))?
            .replace('_', "-");

        // Only process r2x-* packages except the shared runtime
        if !package_name.starts_with("r2x-") || package_name == "r2x-core" {
            return Err(anyhow!("Package is not an r2x plugin: {}", package_name));
        }

        debug!("Processing dist-info for: {}", package_name);

        // Check for entry_points.txt with r2x_plugin entry point
        let entry_points_file = dist_info_path.join("entry_points.txt");
        if !entry_points_file.exists() {
            return Err(anyhow!(
                "No entry_points.txt found in: {:?}",
                dist_info_path
            ));
        }

        // Verify it has r2x_plugin entry point
        let entry_points_content = fs::read_to_string(&entry_points_file)?;
        if !entry_points_content.contains("[r2x_plugin]") {
            return Err(anyhow!(
                "No [r2x_plugin] entry point found in: {}",
                package_name
            ));
        }

        // Get package location (parent directory of dist-info)
        let location = dist_info_path
            .parent()
            .ok_or_else(|| anyhow!("Cannot get parent of dist-info"))?
            .to_path_buf();

        // Check if it's an editable install
        let (is_editable, pth_file, resolved_source_path) =
            self.check_editable_install(&package_name, &location)?;

        Ok(DiscoveredPackage {
            name: package_name,
            is_explicit: true, // TODO: Read from installed.json to distinguish
            location,
            entry_points_file,
            is_editable,
            pth_file,
            resolved_source_path,
        })
    }

    /// Check if package is an editable install and resolve source path
    #[allow(dead_code)]
    fn check_editable_install(
        &self,
        package_name: &str,
        _location: &Path,
    ) -> Result<(bool, Option<PathBuf>, Option<PathBuf>)> {
        // Look for .pth file in site-packages
        let pth_pattern = format!("__{}-*__.pth", package_name.replace('-', "_"));
        debug!("Looking for editable install marker: {}", pth_pattern);

        for entry in fs::read_dir(&self.site_packages)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if file_name_str.ends_with(".pth")
                && file_name_str.contains(&package_name.replace('-', "_"))
            {
                // Try to read the .pth file and resolve the actual source path
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(resolved_path) = self.resolve_pth_path(&content) {
                        debug!(
                            "Found editable install for {} at: {:?}",
                            package_name, resolved_path
                        );
                        return Ok((true, Some(path), Some(resolved_path)));
                    }
                }
            }
        }

        Ok((false, None, None))
    }

    /// Parse .pth file content and resolve the actual source path
    #[allow(dead_code)]
    fn resolve_pth_path(&self, content: &str) -> Result<PathBuf> {
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
            "No valid r2x_plugin entry point found in: {:?}",
            entry_points_path
        ));
    }

    debug!("Parsed entry point: {}:{}", module, function);
    Ok((module, function))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entry_points() {
        let content = r#"[r2x_plugin]
reeds = r2x_reeds.plugins:register_plugin

[other]
something = some.module:function
"#;

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_entry_points.txt");
        fs::write(&temp_file, content).unwrap();

        let result = parse_entry_points(&temp_file).unwrap();
        assert_eq!(result.0, "r2x_reeds.plugins");
        assert_eq!(result.1, "register_plugin");

        let _ = fs::remove_file(&temp_file);
    }

    #[test]
    fn test_parse_entry_points_multiple_entries() {
        let content = r#"[r2x_plugin]
reeds = r2x_reeds.plugins:register_plugin
other = other.module:func
"#;

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_entry_points2.txt");
        fs::write(&temp_file, content).unwrap();

        let result = parse_entry_points(&temp_file).unwrap();
        // Should get the first one
        assert_eq!(result.0, "r2x_reeds.plugins");
        assert_eq!(result.1, "register_plugin");

        let _ = fs::remove_file(&temp_file);
    }
}
