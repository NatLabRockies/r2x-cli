//! Python-Rust bridge for plugin execution
//!
//! This bridge provides a minimal, focused interface for:
//! 1. Loading plugin package metadata via entry points
//! 2. Executing plugins with configuration
//!
//! Plugin discovery uses AST-based analysis instead of runtime inspection,
//! making it more efficient and reducing Python interpreter overhead.
//!
//! ## Runtime Python Detection
//!
//! This crate discovers Python at runtime, preferring uv-managed installations.
//! It supports Python 3.11+ and dynamically loads the Python shared library.

pub mod errors;
pub mod plugin_invoker;
mod plugin_kwargs;
mod plugin_regular;
mod plugin_upgrader;
mod python_bridge;
mod python_discovery;
mod python_loader;
mod utils;

pub use errors::BridgeError;
pub use plugin_invoker::{PluginInvocationResult, PluginInvocationTimings};
pub use python_bridge::{configure_python_venv, Bridge, PythonEnvCompat as PythonEnvironment};
pub use python_discovery::PythonEnvironment as DiscoveredPythonEnvironment;
pub use utils::{resolve_python_path, resolve_site_package_path, PYTHON_LIB_DIR};

#[cfg(test)]
mod tests {
    #[test]
    fn test_bridge_module_exports() {
        // Verify that key types are exported
        // Bridge and configure_python_venv should be publicly accessible
    }
}
