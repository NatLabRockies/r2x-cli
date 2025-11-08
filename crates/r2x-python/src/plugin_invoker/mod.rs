//! Plugin invocation and execution

use crate::errors::BridgeError;
use pyo3::prelude::*;
use r2x_logger as logger;
use r2x_manifest::{
    runtime::{build_runtime_bindings, RuntimeBindings},
    DiscoveryPlugin,
};

mod kwargs;
mod regular;
mod upgrader;

impl super::Bridge {
    pub fn invoke_plugin(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        plugin_metadata: Option<&DiscoveryPlugin>,
    ) -> Result<String, BridgeError> {
        let runtime_bindings = match plugin_metadata {
            Some(meta) => Some(
                build_runtime_bindings(meta)
                    .map_err(|e| BridgeError::Python(format!("Invalid plugin metadata: {}", e)))?,
            ),
            None => None,
        };

        if let Some(plugin) = plugin_metadata {
            if plugin.plugin_type == "UpgraderPlugin" {
                logger::debug("Routing to upgrader plugin handler");
                return self.invoke_upgrader_plugin(
                    target,
                    config_json,
                    runtime_bindings.as_ref(),
                    plugin_metadata,
                );
            }
        }

        self.invoke_plugin_regular(target, config_json, stdin_json, runtime_bindings.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_invocation_placeholder() {
        assert!(true);
    }
}
