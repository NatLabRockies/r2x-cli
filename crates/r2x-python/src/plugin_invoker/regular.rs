use super::{
    logger, BridgeError, PluginInvocationResult, PluginInvocationTimings, RuntimeBindings,
};
use crate::Bridge;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyModule};
use pyo3::PyResult;
use std::time::{Duration, Instant};

impl Bridge {
    pub(super) fn invoke_plugin_regular(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        pyo3::Python::attach(|py| {
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

            logger::debug("Building kwargs for plugin invocation");
            let kwargs =
                self.build_kwargs(py, &config_dict, stdin_obj.as_ref(), runtime_bindings)?;

            logger::debug("Starting plugin invocation");
            let call_start = Instant::now();
            let result_py = if callable_path.contains('.') {
                Self::invoke_class_callable(&module, callable_path, stdin_obj.as_ref(), &kwargs)?
            } else {
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

            let (json_str, ser_elapsed) = if result_py.hasattr("to_json")? {
                let ser_start = Instant::now();
                let to_json_result = result_py.call_method0("to_json")?;
                let json_str = if let Ok(json_bytes) = to_json_result.extract::<Vec<u8>>() {
                    String::from_utf8(json_bytes).map_err(|e| {
                        BridgeError::Python(format!("Invalid UTF-8 in JSON output: {}", e))
                    })?
                } else {
                    let dumps = json_module.getattr("dumps")?;
                    dumps.call1((result_py,))?.extract::<String>()?
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
                let json_str = dumps.call1((result_py,))?.extract::<String>()?;
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
        module: &pyo3::Bound<'py, PyModule>,
        callable_path: &str,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        kwargs: &pyo3::Bound<'py, PyDict>,
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

        logger::step(&format!("Class '{}' constructor kwargs: {:?}", class_name, kwargs));
        let instance = class.call((), Some(kwargs)).map_err(|err| {
            let raw_msg = err.to_string();
            let mut formatted = format_python_error(
                class.py(),
                err,
                &format!("Failed to instantiate '{}'", class_name),
            );
            if raw_msg.contains("missing") && raw_msg.contains("required positional argument") {
                formatted.push_str("\n\nHint: This may happen if the plugin metadata cache is stale. Try running:\n  r2x sync");
            }
            BridgeError::Python(formatted)
        })?;

        let method = instance.getattr(method_name).map_err(|e| {
            BridgeError::Python(format_python_error(
                instance.py(),
                e,
                &format!("Failed to get method '{}.{}'", class_name, method_name),
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

        logger::step(&format!("Function kwargs before system: {:?}", kwargs));
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

        logger::step(&format!("Final function kwargs: {:?}", kwargs));
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
