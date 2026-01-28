//! Plugin invocation and execution

use crate::errors::BridgeError;
use r2x_logger as logger;
use r2x_manifest::execution_types::{PluginKind, PluginSpec};
use r2x_manifest::runtime::{build_runtime_bindings, RuntimeBindings};
use std::time::Duration;

/// Timings for a plugin invocation phase
pub struct PluginInvocationTimings {
    pub python_invocation: Duration,
    pub serialization: Duration,
}

/// Result of running a plugin through the Python bridge
pub struct PluginInvocationResult {
    /// JSON text emitted by the plugin (may be `"null"`)
    pub output: String,
    /// Optional per-phase timings for diagnostics
    pub timings: Option<PluginInvocationTimings>,
}

impl crate::python_bridge::Bridge {
    pub fn invoke_plugin(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        plugin_metadata: Option<&PluginSpec>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        let runtime_bindings = plugin_metadata.map(build_runtime_bindings);

        if let Some(plugin) = plugin_metadata {
            if plugin.kind == PluginKind::Upgrader {
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

    pub fn invoke_plugin_with_bindings(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        if let Some(bindings) = runtime_bindings {
            if bindings.plugin_kind == PluginKind::Upgrader {
                logger::debug("Routing to upgrader plugin handler (runtime bindings)");
                return self.invoke_upgrader_plugin(target, config_json, Some(bindings), None);
            }
        }

        self.invoke_plugin_regular(target, config_json, stdin_json, runtime_bindings)
    }
}

#[cfg(test)]
mod tests {
    use crate::plugin_invoker::*;

    #[test]
    fn plugin_invocation_result_basics() {
        let result = PluginInvocationResult {
            output: String::new(),
            timings: None,
        };
        assert!(result.output.is_empty());
    }
}
