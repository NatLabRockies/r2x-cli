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

use super::types::Manifest;

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
    use super::*;
    use crate::types::{
        ArgumentSpec, IOContract, ImplementationType, InvocationSpec, Metadata, Package,
        PluginKind, PluginSpec,
    };
    use tempfile::TempDir;

    #[test]
    fn test_write_and_read_custom_path() {
        let temp_dir = TempDir::new().unwrap();
        let manifest_path = temp_dir.path().join("test_manifest.toml");

        let packages = vec![Package {
            name: "r2x-example".to_string(),
            entry_points_dist_info: "/path/to/entry_points.txt".to_string(),
            editable_install: true,
            pth_file: Some("/path/to/easy-install.pth".to_string()),
            resolved_source_path: Some("/home/dev/r2x-example".to_string()),
            install_type: Some("explicit".to_string()),
            installed_by: Vec::new(),
            dependencies: Vec::new(),
            plugins: vec![PluginSpec {
                name: "example-plugin".to_string(),
                kind: PluginKind::Parser,
                entry: "example_module.ExampleParser".to_string(),
                invocation: InvocationSpec {
                    implementation: ImplementationType::Class,
                    method: None,
                    constructor: vec![ArgumentSpec {
                        name: "name".to_string(),
                        annotation: Some("str".to_string()),
                        default: Some("example-plugin".to_string()),
                        required: false,
                    }],
                    call: vec![],
                },
                io: IOContract {
                    consumes: vec![],
                    produces: vec![],
                },
                resources: None,
                upgrade: None,
                description: None,
                tags: vec![],
            }],
            decorator_registrations: vec![],
        }];

        let manifest = Manifest {
            metadata: Metadata {
                version: "2.0".to_string(),
                generated_at: chrono::Utc::now().to_rfc3339(),
                uv_lock_path: None,
            },
            packages,
        };

        // Write to custom path
        write_to_path(&manifest, &manifest_path).unwrap();

        // Read back from custom path
        let loaded = read_from_path(&manifest_path).unwrap();

        assert_eq!(loaded.packages.len(), 1);
        assert_eq!(loaded.packages[0].name, "r2x-example");
        assert_eq!(loaded.packages[0].editable_install, true);
        assert_eq!(loaded.packages[0].plugins[0].name, "example-plugin");
        assert_eq!(
            loaded.packages[0].plugins[0].invocation.constructor.len(),
            1
        );
    }

    #[test]
    fn test_version_preserved() {
        let temp_dir = TempDir::new().unwrap();
        let manifest_path = temp_dir.path().join("manifest.toml");

        let manifest = Manifest::default();
        write_to_path(&manifest, &manifest_path).unwrap();

        let loaded = read_from_path(&manifest_path).unwrap();
        assert_eq!(loaded.metadata.version, "2.0");
    }
}
