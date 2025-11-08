//! Manifest building from plugin packages
//!
//! This module handles converting loaded plugin packages into manifest entries
//! that can be stored and queried by the manifest system.

use crate::errors::BridgeError;
use r2x_logger as logger;
use r2x_manifest::{
    CallableMetadata, ConfigMetadata, ParameterMetadata, Plugin, UpgraderMetadata,
};
use std::collections::HashMap;

impl super::Bridge {
    /// Build manifest entries from a plugin package
    ///
    /// This converts the Package JSON (from load_plugin_package) into manifest entries
    /// ready for storage in the manifest.toml file.
    ///
    /// # Arguments
    /// * `package_name` - Name of the package (short form, e.g., "reeds")
    /// * `full_package_name` - Full package name (e.g., "r2x-reeds")
    ///
    /// # Returns
    /// Vector of (key, Plugin) tuples where key is the plugin name
    ///
    /// # Example
    /// ```ignore
    /// let entries = bridge.build_manifest_from_package("reeds", "r2x-reeds")?;
    /// for (key, plugin) in entries {
    ///     manifest.add_plugin(&key, plugin);
    /// }
    /// ```
    pub fn build_manifest_from_package(
        &self,
        package_name: &str,
        full_package_name: &str,
    ) -> Result<Vec<(String, Plugin)>, BridgeError> {
        let build_start = std::time::Instant::now();

        // Load the package via the entry point (using short name for entry point lookup)
        let package_json = self.load_plugin_package(package_name)?;
        logger::debug(&format!(
            "load_plugin_package took: {:?}",
            build_start.elapsed()
        ));

        // Parse the JSON
        let json_start = std::time::Instant::now();
        let package: serde_json::Value = serde_json::from_str(&package_json).map_err(|e| {
            BridgeError::Serialization(format!("Failed to parse package JSON: {}", e))
        })?;
        logger::debug(&format!("JSON parsing took: {:?}", json_start.elapsed()));

        // Debug: Show package name from JSON
        if let Some(pkg_name) = package.get("name").and_then(|n| n.as_str()) {
            logger::debug(&format!(
                "Package name from JSON: '{}', full_package_name param: '{}'",
                pkg_name, full_package_name
            ));
        }

        let mut plugins = Vec::new();

        // Extract plugins array from the package
        if let Some(plugins_array) = package.get("plugins").and_then(|p| p.as_array()) {
            for plugin_obj in plugins_array {
                // Extract core plugin information
                let plugin_name = plugin_obj
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| BridgeError::Serialization("Missing plugin name".to_string()))?
                    .to_string();

                let plugin_type = plugin_obj
                    .get("plugin_type")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());

                // Extract and parse obj metadata
                let obj = if let Some(obj_val) = plugin_obj.get("obj") {
                    let module = obj_val.get("module").and_then(|m| m.as_str()).unwrap_or("");
                    let name = obj_val.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let callable_type = obj_val
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    let return_annotation = obj_val
                        .get("return_annotation")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string());

                    // Parse parameters into HashMap
                    let mut parameters = HashMap::new();
                    if let Some(params_obj) = obj_val.get("parameters") {
                        if let Some(params_map) = params_obj.as_object() {
                            for (param_name, param_val) in params_map {
                                let annotation = param_val
                                    .get("annotation")
                                    .and_then(|a| a.as_str())
                                    .map(|s| s.to_string());
                                let default = param_val.get("default").map(|d| d.to_string());
                                let is_required = param_val
                                    .get("is_required")
                                    .and_then(|r| r.as_bool())
                                    .unwrap_or(false);

                                parameters.insert(
                                    param_name.clone(),
                                    ParameterMetadata {
                                        annotation,
                                        default,
                                        is_required,
                                    },
                                );
                            }
                        }
                    }

                    if !module.is_empty() && !name.is_empty() {
                        Some(CallableMetadata {
                            module: module.to_string(),
                            name: name.to_string(),
                            callable_type: callable_type.to_string(),
                            return_annotation,
                            parameters,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Extract io_type
                let io_type = plugin_obj
                    .get("io_type")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());

                // Extract call_method
                let call_method = plugin_obj
                    .get("call_method")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());

                // Extract requires_store
                let requires_store = plugin_obj.get("requires_store").and_then(|r| r.as_bool());

                // Extract and parse config metadata
                let config = if let Some(config_val) = plugin_obj.get("config") {
                    let module = config_val
                        .get("module")
                        .and_then(|m| m.as_str())
                        .unwrap_or("");
                    let name = config_val
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let return_annotation = config_val
                        .get("return_annotation")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string());

                    // Parse config parameters into HashMap
                    let mut parameters = HashMap::new();
                    if let Some(params_obj) = config_val.get("parameters") {
                        if let Some(params_map) = params_obj.as_object() {
                            for (param_name, param_val) in params_map {
                                let annotation = param_val
                                    .get("annotation")
                                    .and_then(|a| a.as_str())
                                    .map(|s| s.to_string());
                                let default = param_val.get("default").map(|d| d.to_string());
                                let is_required = param_val
                                    .get("is_required")
                                    .and_then(|r| r.as_bool())
                                    .unwrap_or(false);

                                parameters.insert(
                                    param_name.clone(),
                                    ParameterMetadata {
                                        annotation,
                                        default,
                                        is_required,
                                    },
                                );
                            }
                        }
                    }

                    if !module.is_empty() && !name.is_empty() {
                        Some(ConfigMetadata {
                            module: module.to_string(),
                            name: name.to_string(),
                            return_annotation,
                            parameters,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Extract description
                let description = plugin_obj
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());

                // Extract doc
                let doc = plugin_obj
                    .get("doc")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());

                // Extract upgrader-specific fields
                let upgrader = if plugin_obj.get("version_strategy").is_some()
                    || plugin_obj.get("version_reader").is_some()
                    || plugin_obj.get("upgrade_steps").is_some()
                {
                    Some(UpgraderMetadata {
                        version_strategy_json: plugin_obj
                            .get("version_strategy")
                            .map(|v| v.to_string()),
                        version_reader_json: plugin_obj
                            .get("version_reader")
                            .map(|v| v.to_string()),
                        upgrade_steps_json: plugin_obj.get("upgrade_steps").map(|u| u.to_string()),
                    })
                } else {
                    None
                };

                // Create manifest plugin entry with structured metadata
                let manifest_plugin = Plugin {
                    package_name: Some(full_package_name.to_string()),
                    plugin_type: plugin_type.clone(),
                    description,
                    doc,
                    io_type,
                    call_method,
                    requires_store,
                    obj: obj.clone(),
                    config,
                    upgrader,
                    install_type: Some("explicit".to_string()),
        installed_by: Vec::new(),
                };

                // Use clean plugin name as key (e.g., "reeds-parser")
                // The type information is stored in the plugin metadata itself
                plugins.push((plugin_name.clone(), manifest_plugin));
            }
        }

        logger::debug(&format!(
            "Total build_manifest_from_package took: {:?}",
            build_start.elapsed()
        ));

        Ok(plugins)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_manifest_building_placeholder() {
        // Manifest building tests would require actual plugin packages
        // This is a placeholder for integration testing
    }
}
