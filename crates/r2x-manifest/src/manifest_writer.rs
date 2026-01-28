//! Manifest writer utilities for custom paths
//!
//! This module provides helper functions for writing/reading manifests
//! to/from custom paths, primarily used in testing scenarios.
//!
//! For normal operations, use the `Manifest` methods in `manifest.rs`
//! which work with the default manifest location.

use anyhow::Result;
use std::fs;
use std::path::Path;
use tracing::{debug, info};

use crate::types::Manifest;

/// Write manifest to a custom path (primarily for testing)
pub fn write_to_path(manifest: &Manifest, output_path: &Path) -> Result<()> {
    debug!("Writing manifest to custom path: {:?}", output_path);

    let toml_string = toml::to_string_pretty(manifest)?;
    fs::write(output_path, &toml_string)?;

    info!("Manifest written successfully to: {:?}", output_path);
    info!("Total packages: {}", manifest.packages.len());

    Ok(())
}

/// Read manifest from a custom path (primarily for testing)
pub fn read_from_path(manifest_path: &Path) -> Result<Manifest> {
    debug!("Reading manifest from custom path: {:?}", manifest_path);

    let content = fs::read_to_string(manifest_path)?;
    let manifest: Manifest = toml::from_str(&content)?;

    info!("Manifest loaded successfully");
    info!("Manifest version: {}", manifest.metadata.version);
    info!("Generated at: {}", manifest.metadata.generated_at);
    info!("Total packages: {}", manifest.packages.len());

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use crate::manifest_writer::*;
    use crate::types::{Manifest, Package, Plugin, PluginType};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn test_write_and_read_custom_path() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let manifest_path = temp_dir.path().join("test_manifest.toml");

        let mut package = Package {
            name: Arc::from("r2x-example"),
            editable_install: true,
            source_uri: Some(Arc::from("/home/dev/r2x-example")),
            ..Default::default()
        };

        package.plugins.push(Plugin {
            name: Arc::from("example-plugin"),
            plugin_type: PluginType::Class,
            module: Arc::from("example_module"),
            class_name: Some(Arc::from("ExampleParser")),
            ..Default::default()
        });

        let mut manifest = Manifest::default();
        manifest.packages.push(package);
        manifest.rebuild_indexes();

        // Write to custom path
        assert!(
            write_to_path(&manifest, &manifest_path).is_ok(),
            "Failed to write manifest"
        );

        // Read back from custom path
        let loaded = read_from_path(&manifest_path);
        assert!(loaded.is_ok(), "Failed to read manifest");
        let loaded = loaded.unwrap_or_default();

        assert_eq!(loaded.packages.len(), 1);
        assert_eq!(loaded.packages[0].name.as_ref(), "r2x-example");
        assert!(loaded.packages[0].editable_install);
        assert_eq!(
            loaded.packages[0].plugins[0].name.as_ref(),
            "example-plugin"
        );
        assert_eq!(loaded.packages[0].plugins[0].plugin_type, PluginType::Class);
    }

    #[test]
    fn test_version_preserved() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let manifest_path = temp_dir.path().join("manifest.toml");

        let manifest = Manifest::default();
        assert!(
            write_to_path(&manifest, &manifest_path).is_ok(),
            "Failed to write manifest"
        );

        let loaded = read_from_path(&manifest_path);
        assert!(loaded.is_ok(), "Failed to read manifest");
        assert!(loaded.is_ok_and(|m| m.metadata.version == "3.0"));
    }
}
