// Re-export public operations
pub mod ast_discovery;
pub mod clean;
pub mod discovery;
pub mod install;
pub mod list;
pub mod package_spec;
pub mod remove;
pub mod sync;

// Re-export public functions from submodules
pub use ast_discovery::AstDiscovery;
pub use clean::clean_manifest;
pub use install::{install_plugin, show_install_help, GitOptions};
pub use list::list_plugins;
pub use remove::remove_plugin;
pub use sync::sync_manifest;

#[cfg(test)]
mod tests {

    #[test]
    fn test_plugins_module() {
        // Module-level tests
    }
}
