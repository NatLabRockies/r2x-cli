use super::*;
use crate::Bridge;
use pyo3::types::PyModule;
use r2x_manifest::{runtime::RuntimeBindings, DiscoveryPlugin};

impl Bridge {
    pub(super) fn invoke_upgrader_plugin(
        &self,
        target: &str,
        config_json: &str,
        runtime_bindings: Option<&RuntimeBindings>,
        plugin_metadata: Option<&DiscoveryPlugin>,
    ) -> Result<String, BridgeError> {
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
                .cast::<pyo3::types::PyDict>()
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
                instance
                    .call_method0("run")?
                    .extract::<String>()
                    .map_err(|e| {
                        BridgeError::Python(format!(
                            "Failed to run upgrader '{}': {}",
                            callable_path, e
                        ))
                    })
            } else {
                logger::debug("Upgrader missing run() method, invoking registered steps directly");
                Self::invoke_registered_steps(&instance)
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

        let path = instance.getattr("path").map_err(|e| {
            BridgeError::Python(format!("Upgrader missing 'path' attribute: {}", e))
        })?;

        for step in steps.try_iter()? {
            let step_obj = step.map_err(|e| BridgeError::Python(format!("{}", e)))?;
            let func = step_obj
                .getattr("func")
                .map_err(|e| BridgeError::Python(format!("Invalid upgrade step: {}", e)))?;
            logger::debug(&format!(
                "Executing upgrade step: {}",
                step_obj
                    .getattr("name")
                    .and_then(|n| n.extract::<String>())
                    .unwrap_or_else(|_| "<unknown>".to_string())
            ));
            func.call1((path.clone(),)).map_err(|e| {
                BridgeError::Python(format!("Upgrade step execution failed: {}", e))
            })?;
        }

        // Upgrader steps modify files in place, no JSON output expected
        Ok("null".to_string())
    }
}
