// Core plugin infrastructure modules
pub mod config;
pub mod discovery;
pub mod install;
pub mod package_resolver;
pub mod package_spec;
pub mod utils;

// Re-export public functions from core infrastructure
pub use install::get_package_info;
pub use package_resolver::{find_package_path, find_package_path_with_venv};
// Re-export AstDiscovery from new location
pub use crate::r2x_ast::AstDiscovery;

#[cfg(test)]
mod tests {

    #[test]
    fn test_plugins_module() {
        // Module-level tests
    }
}
