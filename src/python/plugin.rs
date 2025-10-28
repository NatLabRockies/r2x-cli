use crate::python::plugin_cache;
use crate::{R2xError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub class_name: String,
    pub package_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRegistry {
    pub parsers: HashMap<String, PluginInfo>,
    pub exporters: HashMap<String, PluginInfo>,
    pub modifiers: HashMap<String, PluginInfo>,
    pub filters: HashMap<String, PluginInfo>,
}

impl PluginRegistry {
    pub fn is_empty(&self) -> bool {
        self.parsers.is_empty()
            && self.exporters.is_empty()
            && self.modifiers.is_empty()
            && self.filters.is_empty()
    }

    pub fn find_package_name(&self, plugin_name: &str) -> Option<String> {
        // Try all plugin types
        if let Some(info) = self.parsers.get(plugin_name) {
            return info.package_name.clone();
        }
        if let Some(info) = self.exporters.get(plugin_name) {
            return info.package_name.clone();
        }
        if let Some(info) = self.modifiers.get(plugin_name) {
            return info.package_name.clone();
        }
        if let Some(info) = self.filters.get(plugin_name) {
            return info.package_name.clone();
        }
        None
    }
}

pub fn discover_plugins() -> Result<PluginRegistry> {
    debug!("Discovering plugins via UV subprocess");
    let uv_path = crate::python::uv::ensure_uv()?;
    let python_path = crate::python::venv::get_uv_python_path()?;

    // Python script to discover plugins and output JSON
    let python_script = r#"
import importlib.metadata
import json
import r2x_core

# Import r2x to trigger auto-registration (ignore errors if not installed)
try:
    import r2x
except ImportError:
    pass

manager = r2x_core.PluginManager()

# Get package mapping from entry points (equivalent to get_entry_point_packages)
package_map = {}
eps = importlib.metadata.entry_points()
try:
    # Try new API (Python 3.10+)
    for ep in eps.select(group='r2x_plugin'):
        if ep.dist:
            package_map[ep.name] = ep.dist.name
except AttributeError:
    # Fallback for older API
    for ep in eps:
        group = getattr(ep, 'group', '')
        if group == 'r2x_plugin':
            name = getattr(ep, 'name', '')
            dist_name = getattr(getattr(ep, 'dist', None), 'name', '')
            if name and dist_name:
                package_map[name] = dist_name

# Equivalent to dict_to_plugin_info (for parsers/exporters with class values)
def dict_to_plugin_info(py_dict, package_map):
    return {k: {'class_name': str(v), 'package_name': package_map.get(k)} for k, v in py_dict.items()}

# Equivalent to dict_keys_to_plugin_info (for modifiers/filters with keys only)
def dict_keys_to_plugin_info(py_dict, package_map):
    return {k: {'class_name': getattr(v, '__name__', 'unknown'), 'package_name': package_map.get(k)} for k, v in py_dict.items()}

# Collect plugin info
parsers = dict_to_plugin_info(manager.registered_parsers, package_map)
exporters = dict_to_plugin_info(manager.registered_exporters, package_map)
modifiers = dict_keys_to_plugin_info(manager.registered_modifiers, package_map)
filters = dict_keys_to_plugin_info(manager.registered_filters, package_map)

# Output JSON
print(json.dumps({'parsers': parsers, 'exporters': exporters, 'modifiers': modifiers, 'filters': filters}))
"#;

    let output = Command::new(&uv_path)
        .args([
            "run",
            "--python",
            &python_path.to_string_lossy(),
            "python",
            "-c",
            python_script,
        ])
        .output()
        .map_err(|e| R2xError::PythonInit(format!("Failed to run plugin discovery: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(R2xError::PythonInit(format!(
            "Plugin discovery failed: {}",
            stderr
        )));
    }

    let json_str = String::from_utf8(output.stdout)
        .map_err(|e| R2xError::PythonInit(format!("Invalid UTF-8 output: {}", e)))?;
    debug!("Plugin discovery JSON: {}", json_str);

    let data: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| R2xError::PythonInit(format!("Invalid JSON output: {}", e)))?;

    let parsers = parse_plugin_info(&data["parsers"])?;
    let exporters = parse_plugin_info(&data["exporters"])?;
    let modifiers = parse_plugin_info(&data["modifiers"])?;
    let filters = parse_plugin_info(&data["filters"])?;

    debug!("Found {} parsers", parsers.len());
    debug!("Found {} exporters", exporters.len());
    debug!("Found {} modifiers", modifiers.len());
    debug!("Found {} filters", filters.len());

    Ok(PluginRegistry {
        parsers,
        exporters,
        modifiers,
        filters,
    })
}

// Helper to parse JSON into HashMap<PluginInfo>
fn parse_plugin_info(value: &serde_json::Value) -> Result<HashMap<String, PluginInfo>> {
    let mut map = HashMap::new();
    if let serde_json::Value::Object(obj) = value {
        for (k, v) in obj {
            if let serde_json::Value::Object(info) = v {
                let class_name = info["class_name"].as_str().unwrap_or("unknown").to_string();
                let package_name = info["package_name"].as_str().map(|s| s.to_string());
                map.insert(
                    k.clone(),
                    PluginInfo {
                        class_name,
                        package_name,
                    },
                );
            }
        }
    }
    Ok(map)
}

pub fn get_plugin_registry() -> Result<PluginRegistry> {
    if let Ok(Some(cached)) = plugin_cache::load_cached_plugins() {
        debug!("Using cached plugin registry");
        Ok(cached.plugins)
    } else {
        debug!("No valid cache, discovering plugins");
        let registry = discover_plugins()?;
        if let Err(e) = plugin_cache::save_cached_plugins(&registry) {
            debug!("Failed to save plugin cache: {}", e);
        }
        Ok(registry)
    }
}
