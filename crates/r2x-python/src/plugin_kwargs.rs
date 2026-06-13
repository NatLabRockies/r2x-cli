//! Keyword argument building for plugin invocation

use crate::errors::BridgeError;
use crate::plugin_regular::format_python_error;
use crate::python_bridge::Bridge;
use pyo3::exceptions::PyFileNotFoundError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use r2x_logger as logger;
use r2x_manifest::runtime::{PluginRole, RuntimeBindings, RuntimeConfig};
use std::collections::HashSet;
use std::path::Path;

impl Bridge {
    pub(crate) fn build_kwargs<'py>(
        py: pyo3::Python<'py>,
        config_dict: &pyo3::Bound<'py, PyDict>,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<pyo3::Bound<'py, PyDict>, BridgeError> {
        let kwargs = PyDict::new(py);

        logger::debug_lazy(|| {
            let config_keys: Vec<String> = config_dict
                .keys()
                .iter()
                .filter_map(|k| k.extract::<String>().ok())
                .collect();
            format!("build_kwargs: input config_dict keys: {:?}", config_keys)
        });

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

        logger::debug_lazy(|| {
            let param_names: Vec<&str> =
                runtime.parameters.iter().map(|p| p.name.as_ref()).collect();
            format!(
                "build_kwargs: runtime parameters to process: {:?}",
                param_names
            )
        });

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

        // Track argument reconstruction details only when debug logging is enabled.
        let collect_debug_details = logger::debug_enabled();
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
                    logger::debug_lazy(|| {
                        format!(
                            "Config parameter detected: '{}' (type matches config class '{}')",
                            param.name, config_class_name
                        )
                    });
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
        let mut config_field_names: Option<HashSet<String>> = None;
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
            if collect_debug_details {
                created_args.push(format!("{} (config class)", config_param_name));
            }
            config_instance = Some(config_obj.unbind());
            if let Some(ref config_obj) = config_instance {
                config_field_names = snapshot_config_field_names(config_obj.bind(py));
            }
        }

        for param in &runtime.parameters {
            // Skip the config parameter - it was already handled above
            if needs_config_class && param.name.as_ref() == config_param_name {
                if collect_debug_details {
                    skipped_args.push((
                        param.name.to_string(),
                        "already handled as config class".to_string(),
                    ));
                }
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
                    if collect_debug_details {
                        created_args.push(format!("{} (DataStore)", param.name));
                    }
                } else if collect_debug_details {
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
                if let Some(names) = config_field_names.as_ref() {
                    names.contains(param.name.as_ref())
                } else if let Some(ref config_obj) = config_instance {
                    // Fallback to per-parameter Python lookup when a field snapshot
                    // is unavailable for the config object implementation.
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
                logger::debug_lazy(|| {
                    format!(
                        "Skipping '{}' as separate kwarg - it's a config field",
                        param.name
                    )
                });
                if collect_debug_details {
                    skipped_args.push((
                        param.name.to_string(),
                        "already in config object".to_string(),
                    ));
                }
                continue;
            }

            if let Some(value) = config_dict.get_item(param.name.as_ref()).ok().flatten() {
                let path_alias = value.clone();
                kwargs.set_item(param.name.as_ref(), value)?;
                if collect_debug_details {
                    created_args.push(param.name.to_string());
                }
                if param.name.as_ref() == "folder_path" && !kwargs.contains("path")? {
                    kwargs.set_item("path", path_alias)?;
                    if collect_debug_details {
                        created_args.push("path (alias of folder_path)".to_string());
                    }
                }
            } else if param.required {
                let stdin_param = param.name.as_ref() == "stdin" || param.name.as_ref() == "system";
                if stdin_param && stdin_obj.is_some() {
                    logger::debug_lazy(|| {
                        format!(
                            "Required parameter '{}' will be provided via stdin",
                            param.name
                        )
                    });
                    if collect_debug_details {
                        skipped_args.push((
                            param.name.to_string(),
                            "will be provided via stdin".to_string(),
                        ));
                    }
                } else {
                    logger::warn(&format!(
                        "Required parameter '{}' missing in config",
                        param.name
                    ));
                    if collect_debug_details {
                        skipped_args.push((
                            param.name.to_string(),
                            "missing in config (required)".to_string(),
                        ));
                    }
                }
            } else if collect_debug_details {
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
                if collect_debug_details {
                    created_args.push("stdin (from pipeline)".to_string());
                }
            } else {
                logger::debug(
                    "Plugin received stdin payload but exposes no 'stdin' parameter; skipping kwargs injection",
                );
                if collect_debug_details {
                    skipped_args.push((
                        "stdin".to_string(),
                        "plugin has no stdin parameter".to_string(),
                    ));
                }
            }
        }

        // Log summary of argument reconstruction
        if collect_debug_details {
            logger::debug_lazy(|| {
                format!(
                    "build_kwargs: created {} arguments: {:?}",
                    created_args.len(),
                    created_args
                )
            });
            if !skipped_args.is_empty() {
                logger::debug_lazy(|| {
                    format!(
                        "build_kwargs: skipped {} arguments from pipeline:",
                        skipped_args.len()
                    )
                });
                for (name, reason) in &skipped_args {
                    logger::debug_lazy(|| format!("  - '{}': {}", name, reason));
                }
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
                    logger::debug_lazy(|| {
                        format!("Config metadata present: {}", config_metadata.is_some())
                    });
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
                    logger::debug_lazy(|| {
                        format!(
                            "DataStore(path) failed; config metadata present: {}",
                            config_metadata.is_some()
                        )
                    });
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::snapshot_config_field_names;
    use crate::python_bridge::Bridge;
    use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods, PyModule};
    use r2x_manifest::runtime::{PluginRole, RuntimeBindings, RuntimeConfig};
    use r2x_manifest::types::{Parameter, PluginType};
    use std::ffi::CString;

    #[test]
    fn snapshot_config_field_names_reads_fields_from_dict() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
class Config:
    def __init__(self):
        self.alpha = 1
        self.beta = 2
",
            )
            .map_err(|error| error.to_string())?;
            let file =
                CString::new("config_snapshot_dict_test.py").map_err(|error| error.to_string())?;
            let module_name =
                CString::new("config_snapshot_dict_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let instance = module
                .getattr("Config")
                .and_then(|class| class.call0())
                .map_err(|error| error.to_string())?;

            let names = snapshot_config_field_names(&instance)
                .ok_or_else(|| "missing snapshot".to_string())?;
            assert!(names.contains("alpha"));
            assert!(names.contains("beta"));
            Ok(())
        })
    }

    #[test]
    fn snapshot_config_field_names_falls_back_to_dir_for_slots() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r#"
class Config:
    __slots__ = ("gamma",)
    def __init__(self):
        self.gamma = 3
"#,
            )
            .map_err(|error| error.to_string())?;
            let file =
                CString::new("config_snapshot_slots_test.py").map_err(|error| error.to_string())?;
            let module_name =
                CString::new("config_snapshot_slots_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let instance = module
                .getattr("Config")
                .and_then(|class| class.call0())
                .map_err(|error| error.to_string())?;

            let names = snapshot_config_field_names(&instance)
                .ok_or_else(|| "missing snapshot".to_string())?;
            assert!(names.contains("gamma"));
            Ok(())
        })
    }

    #[test]
    fn build_kwargs_uses_config_field_snapshot_without_per_param_hasattr() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r#"
HASATTR_CHECKS = 0

class Config:
    def __init__(self, alpha=0, beta=0):
        self.alpha = alpha
        self.beta = beta

    def __getattribute__(self, name):
        global HASATTR_CHECKS
        if name in ("alpha", "beta"):
            HASATTR_CHECKS += 1
        return object.__getattribute__(self, name)
"#,
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("build_kwargs_config_snapshot_test.py")
                .map_err(|error| error.to_string())?;
            let module_name = CString::new("build_kwargs_config_snapshot_test")
                .map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;

            let config_dict = PyDict::new(py);
            config_dict
                .set_item("alpha", 1)
                .map_err(|error| error.to_string())?;
            config_dict
                .set_item("beta", 2)
                .map_err(|error| error.to_string())?;

            let runtime_bindings = RuntimeBindings {
                entry_module: "m".to_string(),
                entry_name: "f".to_string(),
                plugin_type: PluginType::Function,
                role: PluginRole::Utility,
                call_method: None,
                config: Some(RuntimeConfig {
                    module: "build_kwargs_config_snapshot_test".to_string(),
                    name: "Config".to_string(),
                }),
                parameters: vec![
                    Parameter {
                        name: "config".into(),
                        required: true,
                        default: None,
                        types: vec!["Config".into()].into(),
                        module: None,
                        description: None,
                    },
                    Parameter {
                        name: "alpha".into(),
                        required: false,
                        default: None,
                        types: vec!["int".into()].into(),
                        module: None,
                        description: None,
                    },
                    Parameter {
                        name: "beta".into(),
                        required: false,
                        default: None,
                        types: vec!["int".into()].into(),
                        module: None,
                        description: None,
                    },
                ],
                requires_store: false,
            };

            let kwargs = Bridge::build_kwargs(py, &config_dict, None, Some(&runtime_bindings))
                .map_err(|error| error.to_string())?;

            let has_config = kwargs
                .contains("config")
                .map_err(|error| error.to_string())?;
            let has_alpha = kwargs
                .contains("alpha")
                .map_err(|error| error.to_string())?;
            let has_beta = kwargs.contains("beta").map_err(|error| error.to_string())?;
            assert!(has_config);
            assert!(!has_alpha);
            assert!(!has_beta);

            let checks = module
                .getattr("HASATTR_CHECKS")
                .and_then(|value| value.extract::<i64>())
                .map_err(|error| error.to_string())?;
            assert_eq!(checks, 0);
            Ok(())
        })
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

fn snapshot_config_field_names(
    config_instance: &pyo3::Bound<'_, PyAny>,
) -> Option<HashSet<String>> {
    let mut names = HashSet::new();

    if let Ok(dict_obj) = config_instance.getattr("__dict__") {
        if let Ok(dict) = dict_obj.cast::<PyDict>() {
            for key in dict.keys().iter() {
                if let Ok(name) = key.extract::<String>() {
                    names.insert(name);
                }
            }
        }
    }

    if names.is_empty() {
        if let Ok(dir_list) = config_instance.dir() {
            for key in dir_list.iter() {
                if let Ok(name) = key.extract::<String>() {
                    names.insert(name);
                }
            }
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
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
            logger::debug_lazy(|| format!("Python exception context[{}]: {}", depth, repr));
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
    logger::debug_lazy(|| format!("Validating ReEDS data files under {}", folder_path));
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
            logger::debug_lazy(|| {
                format!(
                    "Detected missing data file during ReEDS run: {}",
                    full_path.display()
                )
            });
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
