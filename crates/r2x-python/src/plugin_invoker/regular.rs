use super::*;
use crate::Bridge;
use pyo3::types::{PyDict, PyModule};
use std::time::{Duration, Instant};

impl Bridge {
    pub(super) fn invoke_plugin_regular(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<String, BridgeError> {
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

            if result_py.hasattr("to_json")? {
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
                Ok(json_str)
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
                Ok(json_str)
            }
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
            BridgeError::Python(format!("Failed to get class '{}': {}", class_name, e))
        })?;

        let instance = class.call((), Some(kwargs)).map_err(|e| {
            let error_msg = format!("{}", e);
            let enhanced_msg = if error_msg.contains("missing")
                && error_msg.contains("required positional argument")
            {
                format!(
                    "Failed to instantiate '{}': {}\n\nHint: This error may occur when plugin metadata is incomplete. Try running:\n  r2x sync\n\nThis will refresh the plugin metadata cache.",
                    class_name, error_msg
                )
            } else {
                format!("Failed to instantiate '{}': {}", class_name, error_msg)
            };
            BridgeError::Python(enhanced_msg)
        })?;

        let method = instance.getattr(method_name).map_err(|e| {
            BridgeError::Python(format!("Failed to get method '{}': {}", method_name, e))
        })?;

        if let Some(stdin) = stdin_obj {
            method.call1((stdin,)).map_err(|e| {
                BridgeError::Python(format!(
                    "Method '{}.{}' failed: {}",
                    class_name, method_name, e
                ))
            })
        } else {
            method.call0().map_err(|e| {
                BridgeError::Python(format!(
                    "Method '{}.{}' failed: {}",
                    class_name, method_name, e
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
            BridgeError::Python(format!("Failed to get function '{}': {}", callable_path, e))
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
        func.call((), Some(kwargs))
            .map_err(|e| BridgeError::Python(format!("Function '{}' failed: {}", callable_path, e)))
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
