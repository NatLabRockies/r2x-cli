use super::*;
use crate::Bridge;
use pyo3::types::PyDict;
use r2x_manifest::ConfigMetadata;

impl Bridge {
    pub(super) fn build_kwargs<'py>(
        &self,
        py: pyo3::Python<'py>,
        config_dict: &pyo3::Bound<'py, PyDict>,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<pyo3::Bound<'py, PyDict>, BridgeError> {
        let kwargs = PyDict::new(py);

        let runtime = match runtime_bindings {
            Some(binding) => binding,
            None => {
                for (k, v) in config_dict {
                    kwargs.set_item(k, v)?;
                }
                if let Some(stdin) = stdin_obj {
                    kwargs.set_item("stdin", stdin)?;
                }
                return Ok(kwargs);
            }
        };
        let obj = &runtime.callable;

        let mut needs_config_class = false;
        let mut config_param_name = String::new();
        for (param_name, param_meta) in &obj.parameters {
            let annotation = param_meta.annotation.as_deref().unwrap_or("");
            if param_name == "config" || annotation.contains("Config") {
                needs_config_class = true;
                config_param_name = param_name.clone();
                break;
            }
        }

        let mut config_instance: Option<pyo3::Py<pyo3::PyAny>> = None;
        if needs_config_class {
            let config_params = if let Ok(Some(existing_config)) = config_dict.get_item("config") {
                if let Ok(config_dict_value) = existing_config.cast::<PyDict>() {
                    config_dict_value.clone()
                } else {
                    PyDict::new(py)
                }
            } else {
                let params = PyDict::new(py);
                for (key, value) in config_dict.iter() {
                    let key_str = key.extract::<String>()?;
                    if key_str != "data_store" && key_str != "store_path" {
                        params.set_item(key, value)?;
                    }
                }
                params
            };

            let config_obj =
                self.instantiate_config_class(py, &config_params, runtime.config.as_ref())?;
            kwargs.set_item(&config_param_name, &config_obj)?;
            config_instance = Some(config_obj.unbind());
        }

        for (param_name, param_meta) in &obj.parameters {
            let annotation = param_meta.annotation.as_deref().unwrap_or("");
            if param_name == "config" || annotation.contains("Config") {
                continue;
            }

            if param_name == "data_store" || annotation.contains("DataStore") {
                logger::step(&format!("Processing data_store parameter: {}", param_name));
                let mut value = config_dict
                    .get_item("store_path")?
                    .or_else(|| config_dict.get_item(param_name).ok().flatten());
                if value.is_none() {
                    value = config_dict.get_item("path").ok().flatten();
                }

                if let Some(value) = value {
                    let config_binding = config_instance.as_ref().map(|obj| obj.bind(py));
                    let store_instance = match config_binding {
                        Some(ref binding) => {
                            self.instantiate_data_store(py, &value, Some(binding))?
                        }
                        None => self.instantiate_data_store(py, &value, None)?,
                    };
                    kwargs.set_item(param_name, store_instance)?;
                }
                continue;
            }

            if let Some(value) = config_dict.get_item(param_name).ok().flatten() {
                kwargs.set_item(param_name, value)?;
            } else if param_meta.is_required {
                logger::warn(&format!(
                    "Required parameter '{}' missing in config",
                    param_name
                ));
            }
        }

        if let Some(stdin) = stdin_obj {
            kwargs.set_item("stdin", stdin)?;
        }

        Ok(kwargs)
    }

    pub(super) fn instantiate_config_class<'py>(
        &self,
        py: pyo3::Python<'py>,
        config_params: &pyo3::Bound<'py, PyDict>,
        config_metadata: Option<&ConfigMetadata>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let config_meta = config_metadata
            .ok_or_else(|| BridgeError::Python("Plugin config metadata missing".to_string()))?;

        let config_module = PyModule::import(py, &config_meta.module).map_err(|e| {
            BridgeError::Python(format!(
                "Failed to import config module '{}': {}",
                config_meta.module, e
            ))
        })?;
        let config_class = config_module.getattr(&config_meta.name).map_err(|e| {
            BridgeError::Python(format!(
                "Failed to get config class '{}': {}",
                config_meta.name, e
            ))
        })?;

        config_class.call((), Some(&config_params)).map_err(|e| {
            BridgeError::Python(format!(
                "Failed to instantiate config class '{}': {}",
                config_meta.name, e
            ))
        })
    }

    pub(super) fn instantiate_data_store<'py>(
        &self,
        py: pyo3::Python<'py>,
        value: &pyo3::Bound<'py, PyAny>,
        config_instance: Option<&pyo3::Bound<'py, PyAny>>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let path = if let Ok(store_dict) = value.cast::<PyDict>() {
            let path = store_dict
                .get_item("path")?
                .ok_or_else(|| BridgeError::Python("data_store path missing".to_string()))?
                .extract::<String>()?;
            path
        } else if let Ok(path_str) = value.extract::<String>() {
            path_str
        } else {
            return Err(BridgeError::Python(
                "Invalid data_store format. Provide dict or store path".to_string(),
            ));
        };

        let data_store_module = PyModule::import(py, "r2x_core.store")?;
        let data_store_class = data_store_module.getattr("DataStore")?;

        if let Some(config) = config_instance {
            let from_config = data_store_class
                .getattr("from_plugin_config")
                .map_err(|e| {
                    BridgeError::Python(format!("DataStore missing from_plugin_config: {}", e))
                })?;
            from_config
                .call1((config, path))
                .map_err(|e| BridgeError::Python(format!("Failed to instantiate DataStore: {}", e)))
        } else {
            data_store_class
                .call1((path,))
                .map_err(|e| BridgeError::Python(format!("Failed to instantiate DataStore: {}", e)))
        }
    }
}
