//! Plugin package loading from entry points
//!
//! This module handles discovering and loading plugin packages from Python
//! entry points, with both fast (direct import) and slow (importlib.metadata) paths.

use crate::errors::BridgeError;
use r2x_config::Config;
use r2x_logger as logger;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::path::PathBuf;
use std::time::SystemTime;

impl super::Bridge {
    /// Load plugin package metadata from the entry point
    ///
    /// Each plugin package exposes an r2x_plugin entry point that returns a Package object.
    /// This method:
    /// 1. Discovers the r2x_plugin entry point for a package name
    /// 2. Calls the entry point to get a Package object
    /// 3. Serializes Package to JSON via model_dump_json()
    /// 4. Returns the JSON string for Rust deserialization
    ///
    /// # Arguments
    /// * `package_name` - Name of the package (e.g., "reeds" for r2x-reeds)
    ///
    /// # Returns
    /// JSON string containing the Package object structure
    pub fn load_plugin_package(&self, package_name: &str) -> Result<String, BridgeError> {
        let load_start = std::time::Instant::now();

        // Convert package name format: "reeds" -> "r2x_reeds"
        // Normalize hyphens to underscores for Python package names (PEP 503)
        let normalized_package_name = package_name.replace('-', "_");
        let full_package_name = format!("r2x_{}", normalized_package_name);

        logger::debug(&format!(
            "Attempting fast path for package: {} (full name: {})",
            package_name, full_package_name
        ));

        // Try the fast path first
        if let Some(result) =
            Self::load_plugin_package_fast(&normalized_package_name, &full_package_name)
        {
            logger::debug(&format!(
                "Fast path succeeded (took: {:?})",
                load_start.elapsed()
            ));
            return result;
        }

        logger::debug("Fast path failed, falling back to slow path");

        // Fall back to slow path using importlib.metadata
        let slow_path_start = std::time::Instant::now();
        let result = Python::attach(|py| {
            // Import importlib.metadata
            let metadata = PyModule::import(py, "importlib.metadata").map_err(|e| {
                BridgeError::Import("importlib.metadata".to_string(), format!("{}", e))
            })?;

            // Get entry_points function
            let entry_points_func = metadata.getattr("entry_points")?;
            let eps = entry_points_func.call0()?;

            // Select r2x_plugin group
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("group", "r2x_plugin")?;
            let plugin_eps = eps.call_method("select", (), Some(&kwargs))?;

            // Find entry point matching package name (normalized with underscores)
            let mut found_ep = None;
            let iterator = plugin_eps.try_iter()?;
            for ep_result in iterator {
                let ep = ep_result?;
                let name = ep.getattr("name")?.extract::<String>()?;
                if name == normalized_package_name {
                    found_ep = Some(ep);
                    break;
                }
            }

            let ep = match found_ep {
                Some(e) => e,
                None => {
                    return Err(BridgeError::PluginNotFound(format!(
                        "r2x_plugin entry point not found for package: {} (normalized: {})",
                        package_name, normalized_package_name
                    )))
                }
            };

            // Load and call the entry point function
            let register_fn = ep.call_method0("load")?;
            let package_obj = register_fn.call0()?;

            // Serialize Package to JSON
            let model_dump_json = package_obj.getattr("model_dump_json")?;
            let json_str = model_dump_json.call0()?.extract::<String>()?;

            Ok(json_str)
        })?;

        logger::debug(&format!(
            "Slow path took: {:?}, total load time: {:?}",
            slow_path_start.elapsed(),
            load_start.elapsed()
        ));

        Ok(result)
    }

    /// Load plugin package directly from entry point without discovery
    ///
    /// This is a fast path that bypasses importlib.metadata discovery by:
    /// 1. Reading entry_points.txt from the installed package's dist-info
    /// 2. Parsing it to find the entry point module and function
    /// 3. Calling the function directly
    ///
    /// Falls back to the slow discovery path if needed.
    fn load_plugin_package_fast(
        _package_name: &str,
        full_package_name: &str,
    ) -> Option<Result<String, BridgeError>> {
        // Try to find and parse entry_points.txt
        let parse_start = std::time::Instant::now();
        let ep_info = Self::parse_entry_point_from_dist_info(full_package_name)?;
        logger::debug(&format!(
            "parse_entry_point_from_dist_info took: {:?}",
            parse_start.elapsed()
        ));

        logger::debug(&format!("Parsed entry point: {}", ep_info));

        Python::attach(|py| {
            // Parse module:function format
            let parts: Vec<&str> = ep_info.split(':').collect();
            if parts.len() != 2 {
                return Some(Err(BridgeError::InvalidEntryPoint(format!(
                    "Invalid entry point format: {}",
                    ep_info
                ))));
            }

            let module_name = parts[0];
            let func_name = parts[1];

            logger::debug(&format!(
                "Importing module '{}' and calling function '{}'",
                module_name, func_name
            ));

            // Directly import and call the function
            let result = (|| -> Result<String, BridgeError> {
                let wall_start = SystemTime::now();
                let import_start = std::time::Instant::now();
                let module = PyModule::import(py, module_name)
                    .map_err(|e| BridgeError::Import(module_name.to_string(), format!("{}", e)))?;
                let wall_elapsed = wall_start.elapsed().unwrap_or_default();
                logger::debug(&format!(
                    "PyModule::import took: {:?} (Instant), {:?} (SystemTime)",
                    import_start.elapsed(),
                    wall_elapsed
                ));

                let getattr_start = std::time::Instant::now();
                let func = module.getattr(func_name).map_err(|_| {
                    BridgeError::PluginNotFound(format!(
                        "Function '{}' not found in module '{}'",
                        func_name, module_name
                    ))
                })?;
                logger::debug(&format!(
                    "module.getattr took: {:?}",
                    getattr_start.elapsed()
                ));

                let call_start = std::time::Instant::now();
                let package_obj = func.call0()?;
                logger::debug(&format!("func.call0() took: {:?}", call_start.elapsed()));

                // Serialize Package to JSON
                let serialize_start = std::time::Instant::now();
                let model_dump_json = package_obj.getattr("model_dump_json")?;
                let json_str = model_dump_json.call0()?.extract::<String>()?;
                logger::debug(&format!(
                    "Serialization took: {:?}",
                    serialize_start.elapsed()
                ));

                Ok(json_str)
            })();

            Some(result)
        })
    }

    /// Parse entry point from dist-info/entry_points.txt
    fn parse_entry_point_from_dist_info(full_package_name: &str) -> Option<String> {
        use std::fs;

        // Get the venv path from config
        let config = Config::load().ok()?;
        let venv_path = PathBuf::from(config.get_venv_path());

        logger::debug(&format!(
            "Looking for entry_points.txt for package: {}",
            full_package_name
        ));
        logger::debug(&format!(
            "Venv path: {}",
            venv_path.display()
        ));

        // Find site-packages directory using centralized resolver
        let site_packages_path = match super::resolve_site_package_path(&venv_path) {
            Ok(path) => {
                logger::debug(&format!(
                    "Found site-packages at: {}",
                    path.display()
                ));
                path
            }
            Err(e) => {
                logger::debug(&format!(
                    "Failed to resolve site-packages path: {}",
                    e
                ));
                return None;
            }
        };

        // Find dist-info directory matching the package name (with version)
        // dist-info dirs are named like: r2x_reeds-0.0.1.dist-info
        // We need to match exactly: package_name + "-" to avoid matching r2x_sienna when looking for r2x_sienna_to_plexos
        logger::debug(&format!(
            "Searching for dist-info directory in: {}",
            site_packages_path.display()
        ));
        let mut dist_info_dir = None;
        if let Ok(entries) = fs::read_dir(&site_packages_path) {
            for entry in entries.flatten() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                // Match package name followed by hyphen (for version) to ensure exact match
                let expected_prefix = format!("{}-", full_package_name);
                if file_name.starts_with(&expected_prefix) && file_name.ends_with(".dist-info") {
                    logger::debug(&format!(
                        "Found dist-info directory: {}",
                        file_name
                    ));
                    dist_info_dir = Some(entry.path());
                    break;
                }
            }
        } else {
            logger::debug(&format!(
                "Failed to read site-packages directory: {}",
                site_packages_path.display()
            ));
        }

        let dist_info_dir = dist_info_dir?;
        let entry_points_path = dist_info_dir.join("entry_points.txt");
        logger::debug(&format!(
            "Looking for entry_points.txt at: {}",
            entry_points_path.display()
        ));

        logger::debug(&format!(
            "Entry points path: {}",
            entry_points_path.display()
        ));

        if !entry_points_path.exists() {
            logger::debug("Entry points file not found");
            return None;
        }

        // Parse entry_points.txt for r2x_plugin group
        let content = fs::read_to_string(&entry_points_path).ok()?;
        let mut in_r2x_plugin_section = false;

        for line in content.lines() {
            let line = line.trim();

            // Check for [r2x_plugin] section
            if line == "[r2x_plugin]" {
                in_r2x_plugin_section = true;
                continue;
            }

            // Check for new section (starts with [)
            if line.starts_with('[') {
                in_r2x_plugin_section = false;
                continue;
            }

            // If we're in r2x_plugin section, parse the entry
            if in_r2x_plugin_section && line.contains('=') {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    // Return the right side (module:function)
                    return Some(parts[1].trim().to_string());
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_entry_point_parsing() {
        let ep = "module.path:function_name";
        let parts: Vec<&str> = ep.split(':').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "module.path");
        assert_eq!(parts[1], "function_name");
    }

    #[test]
    fn test_invalid_entry_point() {
        let ep = "no_colon_here";
        let parts: Vec<&str> = ep.split(':').collect();
        assert_ne!(parts.len(), 2);
    }
}
