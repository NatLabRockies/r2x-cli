//! Python-Rust bridge for plugin execution
//!
//! This bridge provides a minimal, focused interface for:
//! 1. Loading plugin package metadata via entry points
//! 2. Executing plugins with configuration
//!
//! Plugin discovery uses AST-based analysis instead of runtime inspection,
//! making it more efficient and reducing Python interpreter overhead.

pub mod errors;
mod initialization;
pub mod plugin_invoker;
mod utils;

pub use errors::BridgeError;
pub use initialization::{configure_python_venv, Bridge};
pub use utils::{PYTHON_BIN_DIR, PYTHON_EXE, PYTHON_LIB_DIR, SITE_PACKAGES};

#[cfg(test)]
mod tests {
    #[test]
    fn test_bridge_module_exports() {
        // Verify that key types are exported
        // Bridge and configure_python_venv should be publicly accessible
    }
}
