//! Python-Rust bridge for plugin execution
//!
//! This bridge provides a minimal, focused interface for:
//! 1. Loading plugin package metadata via entry points
//! 2. Building manifest entries from package schemas
//! 3. Executing plugins with configuration
//!
//! The Package JSON is the single source of truth for all plugin metadata.
//! All plugin information flows through: Python Package → JSON → Manifest

mod initialization;
mod manifest_builder;
mod package_loader;
mod plugin_invoker;
mod utils;

pub use initialization::configure_python_venv;
pub use initialization::Bridge;
pub use utils::{PYTHON_BIN_DIR, PYTHON_EXE, PYTHON_LIB_DIR, SITE_PACKAGES};

#[cfg(test)]
mod tests {
    #[test]
    fn test_bridge_module_exports() {
        // Verify that key types are exported
        // Bridge and configure_python_venv should be publicly accessible
    }
}
