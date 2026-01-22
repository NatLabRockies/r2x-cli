use super::{
    logger, BridgeError, PluginInvocationResult, PluginInvocationTimings, RuntimeBindings,
};
use crate::Bridge;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyDictMethods, PyModule};
use pyo3::PyResult;
use std::time::{Duration, Instant};

/// Guard that suppresses Python stdout and restores it on drop.
pub(super) struct StdoutGuard<'py> {
    py: pyo3::Python<'py>,
    original: Option<pyo3::Py<PyAny>>,
}

impl<'py> StdoutGuard<'py> {
    pub(super) fn new(py: pyo3::Python<'py>, suppress: bool) -> Result<Self, BridgeError> {
        let original = if suppress {
            let sys = PyModule::import(py, "sys")?;
            let io = PyModule::import(py, "io")?;
            let original_stdout = sys.getattr("stdout")?;
            let string_io = io.getattr("StringIO")?.call0()?;
            sys.setattr("stdout", &string_io)?;
            logger::debug("Python stdout suppressed");
            Some(original_stdout.unbind())
        } else {
            None
        };
        Ok(Self { py, original })
    }
}

impl Drop for StdoutGuard<'_> {
    fn drop(&mut self) {
        if let Some(ref stdout) = self.original {
            if let Ok(sys) = PyModule::import(self.py, "sys") {
                let _ = sys.setattr("stdout", stdout.bind(self.py));
                logger::debug("Python stdout restored");
            }
        }
    }
}

impl Bridge {
    pub(super) fn invoke_plugin_regular(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        pyo3::Python::attach(|py| {
            let _guard = StdoutGuard::new(py, logger::get_no_stdout())?;

            logger::debug(&format!("Parsing target: {}", target));
            let parts: Vec<&str> = target.split(':').collect();
            if parts.len() != 2 {
                return Err(BridgeError::InvalidEntryPoint(target.to_string()));
            }
            let module_path = parts[0];
            let callable_path = parts[1];

            logger::debug(&format!("Importing module: {}", module_path));
            let module = PyModule::import(py, module_path)
                .map_err(|e| BridgeError::Import(module_path.to_string(), format!("{}", e)))?;
            let json_module = PyModule::import(py, "json")
                .map_err(|e| BridgeError::Import("json".to_string(), format!("{}", e)))?;
            let loads = json_module.getattr("loads")?;

            logger::debug("Parsing config JSON");
            let config_dict = loads
                .call1((config_json,))?
                .cast::<pyo3::types::PyDict>()
                .map_err(|e| BridgeError::Python(format!("Config must be a JSON object: {}", e)))?
                .clone();

            let stdin_obj = if let Some(stdin) = stdin_json {
                logger::debug("Parsing stdin JSON");
                Some(loads.call1((stdin,))?)
            } else {
                None
            };

            logger::debug("Starting plugin invocation");
            let call_start = Instant::now();
            let result_py = if callable_path.contains('.') {
                Self::invoke_class_callable(
                    self,
                    &module,
                    callable_path,
                    &config_dict,
                    stdin_obj.as_ref(),
                    runtime_bindings,
                )?
            } else {
                logger::debug("Building kwargs for function invocation");
                let kwargs =
                    self.build_kwargs(py, &config_dict, stdin_obj.as_ref(), runtime_bindings)?;
                Self::invoke_function_callable(
                    py,
                    &module,
                    callable_path,
                    stdin_obj.as_ref(),
                    &kwargs,
                    &json_module,
                )?
            };
            let call_elapsed = call_start.elapsed();
            logger::debug(&format!(
                "Python invocation for '{}' took {}",
                callable_path,
                format_duration(call_elapsed)
            ));
            logger::debug("Plugin execution completed");
            logger::debug("Serializing result to JSON");

            let result_unwrapped = {
                let type_name: String = result_py
                    .get_type()
                    .getattr("__name__")
                    .and_then(|n| n.extract())
                    .unwrap_or_default();
                if type_name == "Ok" {
                    logger::debug("Unwrapping Ok result type");
                    result_py
                        .getattr("ok_value")
                        .or_else(|_| result_py.getattr("value"))?
                } else if type_name == "Err" {
                    // rust_ok library uses 'error' property, others might use 'err_value' or 'value'
                    let err_value = result_py
                        .getattr("error")
                        .or_else(|_| result_py.getattr("err_value"))
                        .or_else(|_| result_py.getattr("value"))?;
                    return Err(BridgeError::Python(format!(
                        "Plugin returned Err: {}",
                        err_value
                    )));
                } else {
                    result_py
                }
            };

            let result_to_serialize =
                if result_unwrapped.hasattr("system")? && result_unwrapped.hasattr("config")? {
                    logger::debug("Result is PluginContext, extracting system for serialization");
                    result_unwrapped.getattr("system")?
                } else {
                    result_unwrapped
                };

            let (json_str, ser_elapsed) = if result_to_serialize.hasattr("to_json")? {
                let ser_start = Instant::now();
                let to_json_result = result_to_serialize.call_method0("to_json")?;
                let json_str = if let Ok(json_bytes) = to_json_result.extract::<Vec<u8>>() {
                    String::from_utf8(json_bytes).map_err(|e| {
                        BridgeError::Python(format!("Invalid UTF-8 in JSON output: {}", e))
                    })?
                } else {
                    let dumps = json_module.getattr("dumps")?;
                    dumps.call1((&result_to_serialize,))?.extract::<String>()?
                };
                let ser_elapsed = ser_start.elapsed();
                logger::debug(&format!(
                    "Serialization for '{}' took {}",
                    callable_path,
                    format_duration(ser_elapsed)
                ));
                (json_str, ser_elapsed)
            } else {
                let ser_start = Instant::now();
                let dumps = json_module.getattr("dumps")?;
                let json_str = dumps.call1((&result_to_serialize,))?.extract::<String>()?;
                let ser_elapsed = ser_start.elapsed();
                logger::debug(&format!(
                    "Serialization for '{}' took {}",
                    callable_path,
                    format_duration(ser_elapsed)
                ));
                (json_str, ser_elapsed)
            };

            Ok(PluginInvocationResult {
                output: json_str,
                timings: Some(PluginInvocationTimings {
                    python_invocation: call_elapsed,
                    serialization: ser_elapsed,
                }),
            })
        })
    }

    fn invoke_class_callable<'py>(
        bridge: &Bridge,
        module: &pyo3::Bound<'py, PyModule>,
        callable_path: &str,
        config_dict: &pyo3::Bound<'py, PyDict>,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let parts: Vec<&str> = callable_path.split('.').collect();
        if parts.len() != 2 {
            return Err(BridgeError::InvalidEntryPoint(callable_path.to_string()));
        }
        let (class_name, method_name) = (parts[0], parts[1]);

        let class = module.getattr(class_name).map_err(|e| {
            BridgeError::Python(format_python_error(
                module.py(),
                e,
                &format!("Failed to get class '{}'", class_name),
            ))
        })?;

        let py = module.py();

        let bindings = runtime_bindings.ok_or_else(|| {
            BridgeError::Python(format!(
                "Runtime bindings required for class-based plugin '{}'",
                class_name
            ))
        })?;

        logger::step(&format!(
            "Instantiating plugin '{}' via from_context (PluginContext interface)",
            class_name
        ));

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

        let config_instance = if bindings.config.is_some() {
            bridge.instantiate_config_class(py, &config_params, bindings.config.as_ref())?
        } else {
            logger::debug(&format!(
                "Config metadata missing for '{}', discovering from plugin class",
                class_name
            ));
            discover_and_instantiate_config(py, &class, &config_params)?
        };
        logger::step(&format!("Config class instantiated for '{}'", class_name));

        let store_value = config_dict
            .get_item("store")?
            .or_else(|| config_dict.get_item("store_path").ok().flatten())
            .or_else(|| config_dict.get_item("path").ok().flatten());

        let store_instance = if let Some(value) = store_value {
            logger::debug("Creating DataStore from path for PluginContext");
            Some(bridge.instantiate_data_store(
                py,
                &value,
                Some(&config_instance),
                bindings.config.as_ref(),
            )?)
        } else {
            None
        };

        let system_instance = if let Some(stdin) = stdin_obj {
            use r2x_manifest::PluginKind;
            if bindings.plugin_kind == PluginKind::Exporter {
                logger::step("Deserializing system from stdin for PluginContext");

                let system_module = PyModule::import(py, "infrasys")?;
                let system_class = system_module.getattr("System")?;
                let from_dict = system_class.getattr("from_dict")?;

                let tempfile = PyModule::import(py, "tempfile")?;
                let mkdtemp = tempfile.getattr("mkdtemp")?;
                let temp_dir = mkdtemp.call0()?.extract::<String>()?;

                let kwargs_dict = PyDict::new(py);
                kwargs_dict.set_item("time_series_read_only", true)?;
                let system_obj = from_dict.call((stdin, temp_dir), Some(&kwargs_dict))?;

                Some(system_obj)
            } else {
                None
            }
        } else {
            None
        };

        let ctx = bridge.instantiate_plugin_context(
            py,
            &config_instance,
            store_instance.as_ref(),
            system_instance.as_ref(),
        )?;
        logger::step("PluginContext created");

        let from_context = class.getattr("from_context").map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                &format!(
                    "Plugin class '{}' missing from_context classmethod",
                    class_name
                ),
            ))
        })?;

        let instance = from_context.call1((ctx,)).map_err(|err| {
            let raw_msg = err.to_string();
            let mut formatted = format_python_error(
                py,
                err,
                &format!("Failed to instantiate '{}' via from_context", class_name),
            );
            if raw_msg.contains("missing") && raw_msg.contains("required positional argument") {
                formatted.push_str("\n\nHint: This may happen if the plugin metadata cache is stale. Try running:\n  r2x sync");
            }
            BridgeError::Python(formatted)
        })?;
        logger::step(&format!(
            "Plugin '{}' instantiated via from_context",
            class_name
        ));

        let actual_method_name = if instance.hasattr("run")? {
            "run"
        } else {
            method_name
        };
        logger::debug(&format!(
            "Using method '{}' for plugin '{}'",
            actual_method_name, class_name
        ));
        let method = instance.getattr(actual_method_name).map_err(|e| {
            BridgeError::Python(format_python_error(
                instance.py(),
                e,
                &format!(
                    "Failed to get method '{}.{}'",
                    class_name, actual_method_name
                ),
            ))
        })?;

        let accepts_stdin = if stdin_obj.is_some() {
            match method_accepts_stdin(&method) {
                Ok(result) => result,
                Err(err) => {
                    logger::debug(&format!(
                        "Failed to inspect method '{}.{}' signature for stdin support: {}",
                        class_name, method_name, err
                    ));
                    false
                }
            }
        } else {
            false
        };

        if accepts_stdin {
            let stdin = stdin_obj.expect("checked Some above");
            method.call1((stdin,)).map_err(|e| {
                BridgeError::Python(format_python_error(
                    method.py(),
                    e,
                    &format!("Method '{}.{}' failed", class_name, method_name),
                ))
            })
        } else {
            if stdin_obj.is_some() {
                logger::debug(&format!(
                    "Method '{}.{}' does not declare 'system'/'stdin'; skipping stdin payload",
                    class_name, method_name
                ));
            }
            method.call0().map_err(|e| {
                BridgeError::Python(format_python_error(
                    method.py(),
                    e,
                    &format!("Method '{}.{}' failed", class_name, method_name),
                ))
            })
        }
    }

    fn invoke_function_callable<'py>(
        py: pyo3::Python<'py>,
        module: &pyo3::Bound<'py, PyModule>,
        callable_path: &str,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        kwargs: &pyo3::Bound<'py, PyDict>,
        json_module: &pyo3::Bound<'py, PyModule>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        logger::debug(&format!("Function pattern: {}", callable_path));
        let func = module.getattr(callable_path).map_err(|e| {
            BridgeError::Python(format_python_error(
                module.py(),
                e,
                &format!("Failed to get function '{}'", callable_path),
            ))
        })?;

        logger::step("Function kwargs prepared (before system injection)");
        if let Some(stdin) = stdin_obj {
            logger::step("Function has stdin - deserializing to System object");
            let dumps = json_module.getattr("dumps")?;
            let json_str = dumps.call1((stdin,))?.extract::<String>()?;
            let json_bytes = json_str.as_bytes();

            let system_module = PyModule::import(py, "r2x_core.system")?;
            let system_class = system_module.getattr("System")?;
            let from_json = system_class.getattr("from_json")?;
            let system_obj = from_json.call1((json_bytes,))?;
            kwargs.set_item("system", system_obj)?;
        }

        let kwarg_keys: Vec<String> = kwargs
            .keys()
            .into_iter()
            .filter_map(|k| k.extract::<String>().ok())
            .collect();
        logger::step(&format!("Final function kwargs keys: {:?}", kwarg_keys));
        func.call((), Some(kwargs)).map_err(|e| {
            BridgeError::Python(format_python_error(
                func.py(),
                e,
                &format!("Function '{}' failed", callable_path),
            ))
        })
    }
}

fn format_duration(duration: Duration) -> String {
    let total_ms = duration.as_millis();
    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}

fn format_python_error(py: pyo3::Python<'_>, err: pyo3::PyErr, context: &str) -> String {
    if let Some(traceback_text) = render_traceback(py, &err) {
        format!("{}:\n{}", context, traceback_text)
    } else {
        format!("{}: {}", context, err)
    }
}

fn render_traceback(py: pyo3::Python<'_>, err: &pyo3::PyErr) -> Option<String> {
    let traceback = err.traceback(py)?;
    let traceback_module = PyModule::import(py, "traceback").ok()?;
    let formatter = traceback_module.getattr("format_exception").ok()?;
    let formatted = formatter
        .call1((err.get_type(py), err.value(py), traceback))
        .ok()?;
    let lines: Vec<String> = formatted.extract().ok()?;
    Some(lines.join(""))
}

fn method_accepts_stdin(method: &pyo3::Bound<'_, PyAny>) -> PyResult<bool> {
    let code = method.getattr("__code__")?;
    let argcount: usize = code.getattr("co_argcount")?.extract()?;
    if argcount <= 1 {
        return Ok(false);
    }

    let varnames: Vec<String> = code.getattr("co_varnames")?.extract()?;
    let usable = argcount.min(varnames.len());
    if usable <= 1 {
        return Ok(false);
    }

    Ok(varnames[1..usable]
        .iter()
        .any(|name| name == "system" || name == "stdin"))
}

fn discover_and_instantiate_config<'py>(
    py: pyo3::Python<'py>,
    plugin_class: &pyo3::Bound<'py, PyAny>,
    config_params: &pyo3::Bound<'py, PyDict>,
) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
    if let Ok(orig_bases) = plugin_class.getattr("__orig_bases__") {
        if let Ok(bases_iter) = orig_bases.try_iter() {
            for base in bases_iter.flatten() {
                if let Ok(args) = base.getattr("__args__") {
                    if let Ok(mut args_list) = args.try_iter() {
                        // Only check the first type argument (the config type)
                        if let Some(Ok(config_type)) = args_list.next() {
                            if config_type.is_callable() {
                                let type_name: String = config_type
                                    .getattr("__name__")
                                    .and_then(|n| n.extract())
                                    .unwrap_or_default();
                                if type_name.contains("Config") || type_name.contains("config") {
                                    logger::debug(&format!(
                                        "Discovered config class '{}' from __orig_bases__",
                                        type_name
                                    ));
                                    return config_type
                                        .call((), Some(config_params))
                                        .map_err(|e| {
                                            BridgeError::Python(format!(
                                                "Failed to instantiate discovered config class '{}': {}",
                                                type_name, e
                                            ))
                                        });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Ok(config_class) = plugin_class.getattr("Config") {
        if config_class.is_callable() {
            logger::debug("Discovered nested Config class");
            return config_class.call((), Some(config_params)).map_err(|e| {
                BridgeError::Python(format!("Failed to instantiate nested Config class: {}", e))
            });
        }
    }

    if let Ok(config_class) = plugin_class.getattr("config_class") {
        if config_class.is_callable() {
            let type_name: String = config_class
                .getattr("__name__")
                .and_then(|n| n.extract())
                .unwrap_or_default();
            logger::debug(&format!(
                "Discovered config class '{}' from config_class attribute",
                type_name
            ));
            return config_class.call((), Some(config_params)).map_err(|e| {
                BridgeError::Python(format!(
                    "Failed to instantiate config class from config_class attribute: {}",
                    e
                ))
            });
        }
    }

    logger::debug("No config class discovered, using PluginConfig from r2x_core");
    let r2x_core = PyModule::import(py, "r2x_core")
        .map_err(|e| BridgeError::Python(format!("Failed to import r2x_core: {}", e)))?;
    let plugin_config_class = r2x_core
        .getattr("PluginConfig")
        .map_err(|e| BridgeError::Python(format!("Failed to get PluginConfig class: {}", e)))?;
    plugin_config_class
        .call((), Some(config_params))
        .map_err(|e| BridgeError::Python(format!("Failed to instantiate PluginConfig: {}", e)))
}
