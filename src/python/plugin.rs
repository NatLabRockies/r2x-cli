use crate::{R2xError, Result};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    Python::with_gil(|py| {
        // Import r2x_core first
        let r2x_core = py
            .import_bound("r2x_core")
            .map_err(|e| R2xError::PythonInit(format!("Failed to import r2x_core: {}", e)))?;

        // Import r2x package to trigger auto-registration of its modifiers
        // Ignore errors if r2x is not installed
        let _ = py.import_bound("r2x");

        let plugin_manager_class = r2x_core.getattr("PluginManager")?;
        let manager = plugin_manager_class.call0()?;

        // Get package mapping from entry points
        let package_map = get_entry_point_packages(py)?;

        let parsers_attr = manager.getattr("registered_parsers")?;
        let parsers_dict = parsers_attr
            .downcast::<PyDict>()
            .map_err(|e| R2xError::Python(PyErr::from(e)))?;
        let parsers = dict_to_plugin_info(parsers_dict, &package_map)?;
        debug!("Found {} parsers", parsers.len());

        let exporters_attr = manager.getattr("registered_exporters")?;
        let exporters_dict = exporters_attr
            .downcast::<PyDict>()
            .map_err(|e| R2xError::Python(PyErr::from(e)))?;
        let exporters = dict_to_plugin_info(exporters_dict, &package_map)?;
        debug!("Found {} exporters", exporters.len());

        let modifiers_attr = manager.getattr("registered_modifiers")?;
        let modifiers_dict = modifiers_attr
            .downcast::<PyDict>()
            .map_err(|e| R2xError::Python(PyErr::from(e)))?;
        let modifiers = dict_keys_to_plugin_info(modifiers_dict, &package_map)?;
        debug!("Found {} modifiers", modifiers.len());

        let filters_attr = manager.getattr("registered_filters")?;
        let filters_dict = filters_attr
            .downcast::<PyDict>()
            .map_err(|e| R2xError::Python(PyErr::from(e)))?;
        let filters = dict_keys_to_plugin_info(filters_dict, &package_map)?;
        debug!("Found {} filters", filters.len());

        Ok(PluginRegistry {
            parsers,
            exporters,
            modifiers,
            filters,
        })
    })
}

fn get_entry_point_packages(py: Python) -> Result<HashMap<String, String>> {
    let code = r#"
import importlib.metadata
result = {}
eps = importlib.metadata.entry_points()

# Try the new select API (Python 3.10+)
if hasattr(eps, 'select'):
    for ep in eps.select(group='r2x_plugin'):
        if ep.dist:
            result[ep.name] = ep.dist.name
else:
    # Fallback for older API
    for ep in eps:
        group = ep.group if hasattr(ep, 'group') else getattr(ep, '_group', '')
        if group == 'r2x_plugin':
            name = ep.name if hasattr(ep, 'name') else getattr(ep, '_name', '')
            dist_name = ep.dist.name if hasattr(ep, 'dist') and ep.dist else ''
            if name and dist_name:
                result[name] = dist_name
result
"#;

    let locals = PyDict::new_bound(py);
    py.run_bound(code, None, Some(&locals))?;

    let result = locals.get_item("result")?
        .ok_or_else(|| R2xError::PythonInit("Failed to get entry point mapping".to_string()))?;

    let result_dict = result.downcast::<PyDict>()
        .map_err(|e| R2xError::Python(PyErr::from(e)))?;

    let mut map = HashMap::new();
    for (key, value) in result_dict {
        let key_str: String = key.extract()?;
        let value_str: String = value.extract()?;
        map.insert(key_str, value_str);
    }

    debug!("Found {} entry point -> package mappings", map.len());
    Ok(map)
}

/// Convert Python dict to HashMap of PluginInfo (for parsers/exporters which have class values)
fn dict_to_plugin_info(
    py_dict: &Bound<PyDict>,
    package_map: &HashMap<String, String>,
) -> Result<HashMap<String, PluginInfo>> {
    let mut map = HashMap::new();

    for (key, value) in py_dict {
        let key_str: String = key.extract()?;
        let class_name = value.to_string();
        let package_name = package_map.get(&key_str).cloned();

        map.insert(
            key_str.clone(),
            PluginInfo {
                class_name,
                package_name,
            },
        );
    }

    Ok(map)
}

fn dict_keys_to_plugin_info(
    py_dict: &Bound<PyDict>,
    package_map: &HashMap<String, String>,
) -> Result<HashMap<String, PluginInfo>> {
    let mut map = HashMap::new();

    for (key, value) in py_dict {
        let key_str: String = key.extract()?;
        let class_name = if let Ok(name) = value.getattr("__name__") {
            name.extract().unwrap_or_else(|_| "unknown".to_string())
        } else {
            "unknown".to_string()
        };
        let package_name = package_map.get(&key_str).cloned();

        map.insert(
            key_str.clone(),
            PluginInfo {
                class_name,
                package_name,
            },
        );
    }

    Ok(map)
}

pub fn get_plugin_schema(plugin_name: &str) -> Result<serde_json::Value> {
    Python::with_gil(|py| {
        let r2x_core = py.import_bound("r2x_core")?;
        let plugin_manager_class = r2x_core.getattr("PluginManager")?;
        let manager = plugin_manager_class.call0()?;

        let config_class_result = manager.call_method1("get_config_class", (plugin_name,))?;

        if config_class_result.is_none() {
            return Err(R2xError::PluginNotFound(plugin_name.to_string()));
        }

        let schema = config_class_result.call_method0("model_json_schema")?;

        let json_module = py.import_bound("json")?;
        let schema_str: String = json_module.call_method1("dumps", (schema,))?.extract()?;

        Ok(serde_json::from_str(&schema_str)?)
    })
}
