// Core plugin infrastructure modules
pub mod discovery;
pub mod error;
pub mod install;
pub mod package_spec;
pub mod utils;

// Re-export public functions from core infrastructure
pub use error::PluginError;
pub use install::get_package_info;
// Re-export AstDiscovery from new location
pub use crate::r2x_ast::AstDiscovery;
