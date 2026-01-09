use super::*;
use crate::Bridge;
use pyo3::exceptions::PyFileNotFoundError;
use pyo3::types::{PyDict, PyList, PyModule};
use r2x_logger as logger;
use r2x_manifest::ConfigSpec;
use std::path::Path;

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

        let mut needs_config_class = false;
        let mut config_param_name = String::new();
        for param in &runtime.entry_parameters {
            let annotation = param.annotation.as_deref().unwrap_or("");
            if param.name == "config" || annotation.contains("Config") {
                needs_config_class = true;
                config_param_name = param.name.clone();
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
                    if key_str != "store" && key_str != "data_store" && key_str != "store_path" {
                        params.set_item(key, value)?;
                    }
                }
                params
            };

            logger::step(&format!(
                "Instantiating config class with params: {:?}",
                config_params
            ));
            let config_obj =
                self.instantiate_config_class(py, &config_params, runtime.config.as_ref())?;
            logger::step(&format!(
                "Config class instantiated, setting as kwarg '{}'",
                config_param_name
            ));
            kwargs.set_item(&config_param_name, &config_obj)?;
            config_instance = Some(config_obj.unbind());
        }

        for param in &runtime.entry_parameters {
            let annotation = param.annotation.as_deref().unwrap_or("");
            if param.name == "config" || annotation.contains("Config") {
                continue;
            }

            if param.name == "store"
                || param.name == "data_store"
                || annotation.contains("DataStore")
            {
                logger::step(&format!("Processing store parameter: {}", param.name));
                // Look for store value: prefer "store" key, then param name, then "path"
                let value = config_dict
                    .get_item("store")?
                    .or_else(|| config_dict.get_item(&param.name).ok().flatten())
                    .or_else(|| config_dict.get_item("store_path").ok().flatten())
                    .or_else(|| config_dict.get_item("path").ok().flatten());

                if let Some(value) = value {
                    let config_binding = config_instance.as_ref().map(|obj| obj.bind(py));
                    let store_instance = match config_binding {
                        Some(ref binding) => self.instantiate_data_store(
                            py,
                            &value,
                            Some(binding),
                            runtime.config.as_ref(),
                        )?,
                        None => {
                            self.instantiate_data_store(py, &value, None, runtime.config.as_ref())?
                        }
                    };
                    kwargs.set_item(&param.name, store_instance)?;
                }
                continue;
            }

            if let Some(value) = config_dict.get_item(&param.name).ok().flatten() {
                let path_alias = value.clone();
                kwargs.set_item(&param.name, value)?;
                if param.name == "folder_path" && !kwargs.contains("path")? {
                    kwargs.set_item("path", path_alias)?;
                }
            } else if param.required {
                let stdin_param = param.name == "stdin" || param.name == "system";
                if stdin_param && stdin_obj.is_some() {
                    logger::debug(&format!(
                        "Required parameter '{}' will be provided via stdin",
                        param.name
                    ));
                } else {
                    logger::warn(&format!(
                        "Required parameter '{}' missing in config",
                        param.name
                    ));
                }
            }
        }

        if let Some(stdin) = stdin_obj {
            if runtime.entry_parameters.iter().any(|p| p.name == "stdin") {
                kwargs.set_item("stdin", stdin)?;
            } else {
                logger::debug(
                    "Plugin received stdin payload but exposes no 'stdin' parameter; skipping kwargs injection",
                );
            }
        }

        Ok(kwargs)
    }

    pub(super) fn instantiate_config_class<'py>(
        &self,
        py: pyo3::Python<'py>,
        config_params: &pyo3::Bound<'py, PyDict>,
        config_metadata: Option<&ConfigSpec>,
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
        config_metadata: Option<&ConfigSpec>,
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
            let store_path = path.clone();
            let from_config = data_store_class
                .getattr("from_plugin_config")
                .map_err(|e| {
                    BridgeError::Python(format!("DataStore missing from_plugin_config: {}", e))
                })?;
            match from_config.call1((config, path)) {
                Ok(store) => Ok(store),
                Err(err) => {
                    logger::debug(
                        "DataStore.from_plugin_config failed; attempting targeted diagnostics",
                    );
                    logger::debug(&format!(
                        "Config metadata present: {}",
                        config_metadata.is_some()
                    ));
                    if let Some(class_obj) = resolve_config_class(py, Some(config), config_metadata)
                    {
                        if let Some(missing) =
                            detect_missing_data_file_from_mapping(&class_obj, &store_path)
                        {
                            return Err(BridgeError::Python(format!(
                                "Missing required ReEDS data file: {}. \
Verify the data folder contains all expected outputs (did you unpack the full `inputs_case` directory?).",
                                missing
                            )));
                        }
                    } else if let Some(missing) =
                        detect_missing_data_file_from_metadata(py, config_metadata, &store_path)
                    {
                        return Err(BridgeError::Python(format!(
                            "Missing required ReEDS data file: {}. \
Verify the data folder contains all expected outputs (did you unpack the full `inputs_case` directory?).",
                            missing
                        )));
                    }
                    Err(transform_data_store_error(py, err))
                }
            }
        } else {
            let store_path = path.clone();
            match data_store_class.call1((path,)) {
                Ok(store) => Ok(store),
                Err(err) => {
                    logger::debug(&format!(
                        "DataStore(path) failed; config metadata present: {}",
                        config_metadata.is_some()
                    ));
                    if let Some(missing) =
                        detect_missing_data_file_from_metadata(py, config_metadata, &store_path)
                    {
                        Err(BridgeError::Python(format!(
                            "Missing required ReEDS data file: {}. \
Verify the data folder contains all expected outputs (did you unpack the full `inputs_case` directory?).",
                            missing
                        )))
                    } else {
                        Err(transform_data_store_error(py, err))
                    }
                }
            }
        }
    }
}

fn transform_data_store_error(py: pyo3::Python<'_>, err: pyo3::PyErr) -> BridgeError {
    if let Some(missing) = extract_missing_data_file(py, &err) {
        BridgeError::Python(format!(
            "Missing required ReEDS data file: {}. \
Verify the data folder contains all expected outputs (did you unpack the full `inputs_case` directory?).",
            missing
        ))
    } else {
        BridgeError::Python(format!("Failed to instantiate DataStore: {}", err))
    }
}

fn extract_missing_data_file(py: pyo3::Python<'_>, err: &pyo3::PyErr) -> Option<String> {
    let mut current = err.value(py).getattr("__context__").ok();
    let mut depth = 0;
    loop {
        let Some(ctx) = current else { break };
        if ctx.is_none() {
            break;
        }
        if let Ok(repr) = ctx.str() {
            logger::debug(&format!(
                "Python exception context[{}]: {}",
                depth,
                repr.to_string()
            ));
        }
        if ctx.is_instance_of::<PyFileNotFoundError>() {
            if let Ok(text) = ctx.str() {
                return Some(text.to_string());
            }
        }
        current = ctx.getattr("__context__").ok();
        depth += 1;
    }
    None
}

fn detect_missing_data_file_from_mapping(
    class_obj: &pyo3::Bound<'_, PyAny>,
    folder_path: &str,
) -> Option<String> {
    logger::debug(&format!(
        "Validating ReEDS data files under {}",
        folder_path
    ));
    let loader = class_obj.getattr("load_file_mapping").ok()?;
    let records = loader.call0().ok()?;
    let records = records.cast::<PyList>().ok()?;
    let base = Path::new(folder_path);

    for record in records {
        let record = record.cast::<PyDict>().ok()?;
        let optional = record
            .get_item("optional")
            .ok()
            .flatten()
            .and_then(|val| val.extract::<bool>().ok())
            .unwrap_or(false);
        if optional {
            continue;
        }

        let Some(fpath_obj) = record.get_item("fpath").ok().flatten() else {
            continue;
        };
        let Ok(rel_path) = fpath_obj.extract::<String>() else {
            continue;
        };
        let full_path = base.join(rel_path);
        if !full_path.exists() {
            logger::debug(&format!(
                "Detected missing data file during ReEDS run: {}",
                full_path.display()
            ));
            return Some(full_path.to_string_lossy().to_string());
        }
    }

    None
}

fn detect_missing_data_file_from_metadata(
    py: pyo3::Python<'_>,
    metadata: Option<&ConfigSpec>,
    folder_path: &str,
) -> Option<String> {
    let class_obj = resolve_config_class(py, None, metadata)?;
    detect_missing_data_file_from_mapping(&class_obj, folder_path)
}

fn resolve_config_class<'py>(
    py: pyo3::Python<'py>,
    config_instance: Option<&pyo3::Bound<'py, PyAny>>,
    metadata: Option<&ConfigSpec>,
) -> Option<pyo3::Bound<'py, PyAny>> {
    if let Some(instance) = config_instance {
        return instance.getattr("__class__").ok();
    }

    let meta = metadata?;
    let module = PyModule::import(py, &meta.module).ok()?;
    module.getattr(&meta.name).ok()
}
