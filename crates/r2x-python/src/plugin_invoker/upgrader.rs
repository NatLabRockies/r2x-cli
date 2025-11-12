use super::{logger, BridgeError, PluginInvocationResult, RuntimeBindings};
use crate::Bridge;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyDictMethods, PyModule, PyString};
use r2x_manifest::DiscoveryPlugin;
use std::path::{Path, PathBuf};

impl Bridge {
    pub(super) fn invoke_upgrader_plugin(
        &self,
        target: &str,
        config_json: &str,
        runtime_bindings: Option<&RuntimeBindings>,
        plugin_metadata: Option<&DiscoveryPlugin>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        pyo3::Python::attach(|py| {
            logger::debug(&format!("Invoking upgrader plugin: {}", target));
            let parts: Vec<&str> = target.split(':').collect();
            if parts.len() != 2 {
                return Err(BridgeError::InvalidEntryPoint(target.to_string()));
            }
            let module_path = parts[0];
            let callable_path = parts[1];

            let module = PyModule::import(py, module_path)
                .map_err(|e| BridgeError::Import(module_path.to_string(), format!("{}", e)))?;
            let json_module = PyModule::import(py, "json")
                .map_err(|e| BridgeError::Import("json".to_string(), format!("{}", e)))?;
            let loads = json_module.getattr("loads")?;
            let config_dict = loads
                .call1((config_json,))?
                .cast::<PyDict>()
                .map_err(|e| BridgeError::Python(format!("Config must be a JSON object: {}", e)))?
                .clone();

            let kwargs = self.build_kwargs(py, &config_dict, None, runtime_bindings)?;
            let upgrader_class = module.getattr(callable_path).map_err(|e| {
                BridgeError::Python(format!(
                    "Failed to get upgrader class '{}': {}",
                    callable_path, e
                ))
            })?;

            if let Some(plugin) = plugin_metadata {
                if let Some(strategy) = find_arg_value(plugin, "version_strategy") {
                    kwargs.set_item("version_strategy", strategy)?;
                }
                if let Some(reader) = find_arg_value(plugin, "version_reader") {
                    kwargs.set_item("version_reader", reader)?;
                }
                if let Some(steps) = find_arg_value(plugin, "upgrade_steps") {
                    kwargs.set_item("upgrade_steps", steps)?;
                }
            }

            let instance = upgrader_class.call((), Some(&kwargs)).map_err(|e| {
                BridgeError::Python(format!(
                    "Failed to instantiate upgrader '{}': {}",
                    callable_path, e
                ))
            })?;

            if instance.hasattr("run")? {
                let output = instance
                    .call_method0("run")?
                    .extract::<String>()
                    .map_err(|e| {
                        BridgeError::Python(format!(
                            "Failed to run upgrader '{}': {}",
                            callable_path, e
                        ))
                    })?;
                Ok(PluginInvocationResult {
                    output,
                    timings: None,
                })
            } else {
                logger::debug("Upgrader missing run() method, invoking registered steps directly");
                let output = Self::invoke_registered_steps(&instance)?;
                Ok(PluginInvocationResult {
                    output,
                    timings: None,
                })
            }
        })
    }
}

fn find_arg_value<'a>(plugin: &'a DiscoveryPlugin, name: &str) -> Option<&'a str> {
    plugin
        .constructor_args
        .iter()
        .find(|arg| arg.name == name)
        .map(|arg| arg.value.as_str())
}

impl Bridge {
    fn invoke_registered_steps<'py>(
        instance: &pyo3::Bound<'py, pyo3::PyAny>,
    ) -> Result<String, BridgeError> {
        let steps = instance
            .getattr("steps")
            .map_err(|e| BridgeError::Python(format!("Failed to access upgrader steps: {}", e)))?;

        let path_obj = instance.getattr("path").map_err(|e| {
            BridgeError::Python(format!("Upgrader missing 'path' attribute: {}", e))
        })?;
        let path_str = path_obj
            .str()
            .map_err(|e| BridgeError::Python(format!("Invalid upgrader path: {}", e)))?
            .to_string();
        let path_buf = PathBuf::from(path_str);
        let path_handle = path_obj.clone().unbind();

        let py = instance.py();

        let upgrader_utils = PyModule::import(py, "r2x_core.upgrader_utils").map_err(|e| {
            BridgeError::Import("r2x_core.upgrader_utils".to_string(), format!("{}", e))
        })?;
        let run_upgrade_step = upgrader_utils.getattr("run_upgrade_step").map_err(|e| {
            BridgeError::Python(format!(
                "Failed to import r2x_core.upgrader_utils.run_upgrade_step: {}",
                e
            ))
        })?;

        let json_module = PyModule::import(py, "json")
            .map_err(|e| BridgeError::Import("json".to_string(), format!("{}", e)))?;
        let loads = json_module.getattr("loads")?;
        let dumps = json_module.getattr("dumps")?;

        let mut system_data: Option<pyo3::Py<PyAny>> = None;
        let mut system_json_path: Option<PathBuf> = None;

        for step in steps.try_iter()? {
            let step_obj = step.map_err(|e| BridgeError::Python(format!("{}", e)))?;
            let upgrade_type_obj = step_obj.getattr("upgrade_type").map_err(|e| {
                BridgeError::Python(format!("Invalid upgrade step (missing type): {}", e))
            })?;

            let upgrade_value = upgrade_type_obj
                .getattr("value")
                .or_else(|_| Ok(upgrade_type_obj.clone()))
                .and_then(|obj| obj.str().map(|s| s.to_string()))
                .map_err(|e| BridgeError::Python(format!("Invalid upgrade type: {}", e)))?;
            logger::debug(&format!(
                "Upgrade step type for current step: {}",
                upgrade_value
            ));
            let upgrade_is_system =
                upgrade_value.eq_ignore_ascii_case("SYSTEM") || upgrade_value.ends_with(".SYSTEM");
            let upgrade_is_file =
                upgrade_value.eq_ignore_ascii_case("FILE") || upgrade_value.ends_with(".FILE");

            logger::debug(&format!(
                "Executing upgrade step: {}",
                step_obj
                    .getattr("name")
                    .and_then(|n| n.extract::<String>())
                    .unwrap_or_else(|_| "<unknown>".to_string())
            ));

            let data_arg = if upgrade_is_system {
                if system_data.is_none() {
                    let resolved =
                        resolve_system_json_path(&path_buf).map_err(BridgeError::Python)?;
                    let data = load_system_data(py, &loads, &resolved)?;
                    system_data = Some(data);
                    system_json_path = Some(resolved);
                }
                system_data
                    .as_ref()
                    .expect("system_data populated")
                    .clone_ref(py)
            } else {
                path_handle.clone_ref(py)
            };

            let kwargs = PyDict::new(py);
            kwargs
                .set_item("upgrader_context", instance)
                .map_err(|e| BridgeError::Python(format!("Failed to set context: {}", e)))?;

            let result = run_upgrade_step
                .call((step_obj.clone(), data_arg), Some(&kwargs))
                .map_err(|e| {
                    BridgeError::Python(format!("Upgrade step execution failed: {}", e))
                })?;

            let is_err = result
                .getattr("is_err")?
                .call0()
                .and_then(|v| v.is_truthy())
                .map_err(|e| BridgeError::Python(format!("Failed to inspect result: {}", e)))?;

            if is_err {
                let err_obj = result
                    .getattr("unwrap_err")?
                    .call0()
                    .map_err(|e| BridgeError::Python(format!("Failed to fetch error: {}", e)))?;
                let err_text = err_obj
                    .str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| "<unknown error>".to_string());
                return Err(BridgeError::Python(format!(
                    "Upgrade step execution failed: {}",
                    err_text
                )));
            }

            if upgrade_is_system {
                let value_obj = result.getattr("unwrap")?.call0().map_err(|e| {
                    BridgeError::Python(format!("Failed to unwrap upgrade result: {}", e))
                })?;
                if !value_obj.is_none() {
                    system_data = Some(value_obj.into());
                }
            } else if !upgrade_is_file {
                logger::warn(&format!(
                    "Unknown upgrade type '{}' for step {}; defaulting to pass-through",
                    upgrade_value,
                    step_obj
                        .getattr("name")
                        .and_then(|n| n.extract::<String>())
                        .unwrap_or_else(|_| "<unknown>".into())
                ));
            }
        }

        let final_json_path = if let Some(json_path) = system_json_path {
            if let Some(ref data) = system_data {
                write_system_data(py, &dumps, data, &json_path)?;
            }
            json_path
        } else {
            resolve_system_json_path(&path_buf).unwrap_or(path_buf.clone())
        };

        if let Some(data) = system_data {
            let json_str: String = dumps
                .call1((data.bind(py),))
                .map_err(|e| {
                    BridgeError::Python(format!("Failed to serialize upgraded system: {}", e))
                })?
                .extract()
                .map_err(|e| {
                    BridgeError::Python(format!("Failed to extract upgraded system JSON: {}", e))
                })?;
            Ok(json_str)
        } else {
            let contents = std::fs::read_to_string(&final_json_path).map_err(|e| {
                BridgeError::Python(format!(
                    "Failed to read upgraded system JSON {}: {}",
                    final_json_path.display(),
                    e
                ))
            })?;
            Ok(contents)
        }
    }
}

fn resolve_system_json_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }

    if path.is_dir() {
        let candidate = path.join("system.json");
        if candidate.exists() {
            return Ok(candidate);
        }

        if let Ok(mut entries) = std::fs::read_dir(path) {
            while let Some(Ok(entry)) = entries.next() {
                let entry_path = entry.path();
                if entry_path
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
                {
                    return Ok(entry_path);
                }
            }
        }
    }

    Err(format!(
        "Unable to locate JSON file for upgrader at {}",
        path.display()
    ))
}

fn load_system_data<'py>(
    py: pyo3::Python<'py>,
    loads: &pyo3::Bound<'py, pyo3::PyAny>,
    json_path: &Path,
) -> Result<pyo3::Py<PyAny>, BridgeError> {
    let content = std::fs::read_to_string(json_path).map_err(|e| {
        BridgeError::Python(format!(
            "Failed to read system JSON {}: {}",
            json_path.display(),
            e
        ))
    })?;
    let py_str = PyString::new(py, &content);
    let data = loads.call1((py_str,)).map_err(|e| {
        BridgeError::Python(format!(
            "Failed to parse system JSON {}: {}",
            json_path.display(),
            e
        ))
    })?;
    Ok(data.into())
}

fn write_system_data<'py>(
    py: pyo3::Python<'py>,
    dumps: &pyo3::Bound<'py, pyo3::PyAny>,
    data: &pyo3::Py<PyAny>,
    json_path: &Path,
) -> Result<(), BridgeError> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("indent", 2)?;
    kwargs.set_item("ensure_ascii", false)?;
    let json_str: String = dumps
        .call((data.bind(py),), Some(&kwargs))
        .map_err(|e| {
            BridgeError::Python(format!(
                "Failed to serialize upgraded system JSON {}: {}",
                json_path.display(),
                e
            ))
        })?
        .extract()
        .map_err(|e| {
            BridgeError::Python(format!(
                "Failed to convert upgraded system JSON {}: {}",
                json_path.display(),
                e
            ))
        })?;
    std::fs::write(json_path, json_str).map_err(|e| {
        BridgeError::Python(format!(
            "Failed to write upgraded system JSON {}: {}",
            json_path.display(),
            e
        ))
    })
}
