//! Keyword argument building for plugin invocation

use crate::errors::BridgeError;
use crate::plugin_regular::format_python_error;
use crate::python_bridge::Bridge;
use pyo3::exceptions::PyFileNotFoundError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use r2x_logger as logger;
use r2x_manifest::runtime::{PluginRole, RuntimeBindings, RuntimeConfig};
use std::path::Path;

impl Bridge {
    pub(crate) fn build_kwargs<'py>(
        py: pyo3::Python<'py>,
        config_dict: &pyo3::Bound<'py, PyDict>,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<pyo3::Bound<'py, PyDict>, BridgeError> {
        let kwargs = PyDict::new(py);

        // Log the input config dict keys for debugging
        let config_keys: Vec<String> = config_dict
            .keys()
            .iter()
            .filter_map(|k| k.extract::<String>().ok())
            .collect();
        logger::debug(&format!(
            "build_kwargs: input config_dict keys: {:?}",
            config_keys
        ));

        let Some(runtime) = runtime_bindings else {
            logger::debug(
                "build_kwargs: no runtime bindings, passing all config_dict keys as kwargs",
            );
            for (k, v) in config_dict {
                kwargs.set_item(k, v)?;
            }
            if let Some(stdin) = stdin_obj {
                kwargs.set_item("stdin", stdin)?;
            }
            return Ok(kwargs);
        };

        // Log the runtime parameters we're working with
        let param_names: Vec<&str> = runtime.parameters.iter().map(|p| p.name.as_ref()).collect();
        logger::debug(&format!(
            "build_kwargs: runtime parameters to process: {:?}",
            param_names
        ));

        // For upgrader plugins without config metadata, pass all config values directly as kwargs.
        // Upgraders typically have simple constructors (path, folder_path, etc.) and don't use
        // the complex config class machinery that parsers/exporters use.
        if runtime.role == PluginRole::Upgrader
            && runtime.config.is_none()
            && runtime.parameters.is_empty()
        {
            logger::debug(
                "build_kwargs: upgrader plugin without config metadata, passing all config_dict keys as kwargs",
            );
            for (k, v) in config_dict {
                kwargs.set_item(k, v)?;
            }
            return Ok(kwargs);
        }

        let mut needs_config_class = false;
        let mut config_param_name = String::new();

        // Track which arguments are created vs skipped for logging
        let mut created_args: Vec<String> = Vec::new();
        let mut skipped_args: Vec<(String, String)> = Vec::new(); // (name, reason)

        // Use ConfigSpec metadata as the authoritative source for config parameter detection.
        // Match parameters by their annotation against the config class name from the manifest,
        // allowing plugin authors to name their config parameter anything they want.
        if let Some(config_spec) = &runtime.config {
            let config_class_name = &config_spec.name;
            logger::step(&format!(
                "Looking for config parameter with annotation matching '{}'",
                config_class_name
            ));

            // Find the parameter whose type annotation matches the config class name
            for param in &runtime.parameters {
                let type_matches = param
                    .types
                    .iter()
                    .any(|t| t.as_ref() == config_class_name || t.contains(config_class_name));
                if type_matches {
                    needs_config_class = true;
                    config_param_name = param.name.to_string();
                    logger::debug(&format!(
                        "Config parameter detected: '{}' (type matches config class '{}')",
                        param.name, config_class_name
                    ));
                    break;
                }
            }

            // Fallback: if no annotation match, look for a param explicitly named after the config
            if !needs_config_class {
                for param in &runtime.parameters {
                    if param.name.as_ref() == "config" {
                        needs_config_class = true;
                        config_param_name = "config".to_string();
                        logger::debug(
                            "Config parameter detected by fallback: param named 'config'",
                        );
                        break;
                    }
                }
            }

            // Last resort: we have config metadata but no matching param, use "config" as default
            // This is expected for function plugins where entry_parameters contains config fields
            // rather than the actual function signature parameters
            if !needs_config_class {
                needs_config_class = true;
                config_param_name = "config".to_string();
                logger::step("Using default config parameter name 'config'");
            }
        }

        let mut config_instance: Option<pyo3::Py<pyo3::PyAny>> = None;
        if needs_config_class {
            // Always pass the full config dict to the config class.
            // The config class (e.g., ZonalToNodal which extends PluginConfig) may have
            // its own nested "config" field, but it needs ALL top-level fields too
            // (name, output_folder, etc.). We filter out store-related keys since those
            // are handled separately.
            let config_params = {
                let params = PyDict::new(py);
                for (key, value) in config_dict.iter() {
                    let key_str = key.extract::<String>()?;
                    if key_str != "store" && key_str != "data_store" && key_str != "store_path" {
                        params.set_item(key, value)?;
                    }
                }
                params
            };

            logger::step("Instantiating config class with params");
            let config_obj =
                Self::instantiate_config_class(py, &config_params, runtime.config.as_ref())?;
            logger::step(&format!(
                "Config class instantiated, setting as kwarg '{}'",
                config_param_name
            ));
            kwargs.set_item(&config_param_name, &config_obj)?;
            created_args.push(format!("{} (config class)", config_param_name));
            config_instance = Some(config_obj.unbind());
        }

        for param in &runtime.parameters {
            // Skip the config parameter - it was already handled above
            if needs_config_class && param.name.as_ref() == config_param_name {
                skipped_args.push((
                    param.name.to_string(),
                    "already handled as config class".to_string(),
                ));
                continue;
            }

            let has_data_store_type = param.types.iter().any(|t| t.contains("DataStore"));
            if param.name.as_ref() == "store"
                || param.name.as_ref() == "data_store"
                || has_data_store_type
            {
                logger::step(&format!("Processing store parameter: {}", param.name));
                // Look for store value: prefer "store" key, then param name, then "path"
                let value = config_dict
                    .get_item("store")?
                    .or_else(|| config_dict.get_item(param.name.as_ref()).ok().flatten())
                    .or_else(|| config_dict.get_item("store_path").ok().flatten())
                    .or_else(|| config_dict.get_item("path").ok().flatten());

                if let Some(value) = value {
                    let config_binding = config_instance.as_ref().map(|obj| obj.bind(py));
                    let store_instance = if let Some(binding) = config_binding.as_ref() {
                        Self::instantiate_data_store(
                            py,
                            &value,
                            Some(binding),
                            runtime.config.as_ref(),
                        )?
                    } else {
                        Self::instantiate_data_store(py, &value, None, runtime.config.as_ref())?
                    };
                    kwargs.set_item(param.name.as_ref(), store_instance)?;
                    created_args.push(format!("{} (DataStore)", param.name));
                } else {
                    skipped_args.push((
                        param.name.to_string(),
                        "no store path found in config".to_string(),
                    ));
                }
                continue;
            }

            // Skip parameters that are config fields when we have a config class
            // (those values are already inside the config object)
            let is_config_field = if needs_config_class {
                // Check the config instance directly for the attribute
                if let Some(ref config_obj) = config_instance {
                    config_obj
                        .bind(py)
                        .hasattr(param.name.as_ref())
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };

            if is_config_field {
                logger::debug(&format!(
                    "Skipping '{}' as separate kwarg - it's a config field",
                    param.name
                ));
                skipped_args.push((
                    param.name.to_string(),
                    "already in config object".to_string(),
                ));
                continue;
            }

            if let Some(value) = config_dict.get_item(param.name.as_ref()).ok().flatten() {
                let path_alias = value.clone();
                kwargs.set_item(param.name.as_ref(), value)?;
                created_args.push(param.name.to_string());
                if param.name.as_ref() == "folder_path" && !kwargs.contains("path")? {
                    kwargs.set_item("path", path_alias)?;
                    created_args.push("path (alias of folder_path)".to_string());
                }
            } else if param.required {
                let stdin_param = param.name.as_ref() == "stdin" || param.name.as_ref() == "system";
                if stdin_param && stdin_obj.is_some() {
                    logger::debug(&format!(
                        "Required parameter '{}' will be provided via stdin",
                        param.name
                    ));
                    skipped_args.push((
                        param.name.to_string(),
                        "will be provided via stdin".to_string(),
                    ));
                } else {
                    logger::warn(&format!(
                        "Required parameter '{}' missing in config",
                        param.name
                    ));
                    skipped_args.push((
                        param.name.to_string(),
                        "missing in config (required)".to_string(),
                    ));
                }
            } else {
                skipped_args.push((
                    param.name.to_string(),
                    "not found in config (optional)".to_string(),
                ));
            }
        }

        if let Some(stdin) = stdin_obj {
            if runtime
                .parameters
                .iter()
                .any(|p| p.name.as_ref() == "stdin")
            {
                kwargs.set_item("stdin", stdin)?;
                created_args.push("stdin (from pipeline)".to_string());
            } else {
                logger::debug(
                    "Plugin received stdin payload but exposes no 'stdin' parameter; skipping kwargs injection",
                );
                skipped_args.push((
                    "stdin".to_string(),
                    "plugin has no stdin parameter".to_string(),
                ));
            }
        }

        // Log summary of argument reconstruction
        logger::debug(&format!(
            "build_kwargs: created {} arguments: {:?}",
            created_args.len(),
            created_args
        ));
        if !skipped_args.is_empty() {
            logger::debug(&format!(
                "build_kwargs: skipped {} arguments from pipeline:",
                skipped_args.len()
            ));
            for (name, reason) in &skipped_args {
                logger::debug(&format!("  - '{}': {}", name, reason));
            }
        }

        Ok(kwargs)
    }

    pub(crate) fn instantiate_config_class<'py>(
        py: pyo3::Python<'py>,
        config_params: &pyo3::Bound<'py, PyDict>,
        config_metadata: Option<&RuntimeConfig>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let config_meta = config_metadata
            .ok_or_else(|| BridgeError::Python("Plugin config metadata missing".to_string()))?;

        let config_module = PyModule::import(py, &config_meta.module).map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                &format!("Failed to import config module '{}'", config_meta.module),
            ))
        })?;
        let config_class = config_module.getattr(&config_meta.name).map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                &format!("Failed to get config class '{}'", config_meta.name),
            ))
        })?;

        config_class.call((), Some(config_params)).map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                &format!("Failed to instantiate config class '{}'", config_meta.name),
            ))
        })
    }

    pub(crate) fn instantiate_data_store<'py>(
        py: pyo3::Python<'py>,
        value: &pyo3::Bound<'py, PyAny>,
        config_instance: Option<&pyo3::Bound<'py, PyAny>>,
        config_metadata: Option<&RuntimeConfig>,
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
            // path is keyword-only in from_plugin_config(plugin_config, *, path)
            let kwargs = PyDict::new(py);
            kwargs.set_item("path", &path)?;
            match from_config.call((config,), Some(&kwargs)) {
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
        BridgeError::Python(format_python_error(
            py,
            err,
            "Failed to instantiate DataStore",
        ))
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
            logger::debug(&format!("Python exception context[{}]: {}", depth, repr));
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
    metadata: Option<&RuntimeConfig>,
    folder_path: &str,
) -> Option<String> {
    let class_obj = resolve_config_class(py, None, metadata)?;
    detect_missing_data_file_from_mapping(&class_obj, folder_path)
}

pub(crate) fn resolve_config_class<'py>(
    py: pyo3::Python<'py>,
    config_instance: Option<&pyo3::Bound<'py, PyAny>>,
    metadata: Option<&RuntimeConfig>,
) -> Option<pyo3::Bound<'py, PyAny>> {
    if let Some(instance) = config_instance {
        return instance.getattr("__class__").ok();
    }

    let meta = metadata?;
    let module = PyModule::import(py, meta.module.as_str()).ok()?;
    module.getattr(meta.name.as_str()).ok()
}

impl Bridge {
    /// Instantiate a PluginContext from r2x_core with config (positional) and optional
    /// keyword-only arguments (store, system).
    pub(crate) fn instantiate_plugin_context<'py>(
        py: pyo3::Python<'py>,
        config_instance: &pyo3::Bound<'py, PyAny>,
        store_instance: Option<&pyo3::Bound<'py, PyAny>>,
        system_instance: Option<&pyo3::Bound<'py, PyAny>>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let context_module = PyModule::import(py, "r2x_core").map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                "Failed to import r2x_core for PluginContext",
            ))
        })?;
        let context_class = context_module.getattr("PluginContext").map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                "Failed to get PluginContext class",
            ))
        })?;

        let kwargs = PyDict::new(py);
        if let Some(store) = store_instance {
            kwargs.set_item("store", store)?;
        }
        if let Some(system) = system_instance {
            kwargs.set_item("system", system)?;
        }

        // config is positional (first argument), rest are keyword-only
        context_class
            .call((config_instance,), Some(&kwargs))
            .map_err(|e| {
                BridgeError::Python(format_python_error(py, e, "Failed to create PluginContext"))
            })
    }
}
