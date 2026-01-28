pub mod clean;
pub mod context;
pub mod install;
pub mod list;
pub mod remove;
pub mod sync;

pub use crate::plugins::PluginError;
pub use clean::clean_manifest;
pub use context::PluginContext;
pub use install::{install_plugin, show_install_help, GitOptions};
pub use list::list_plugins;
pub use remove::remove_plugin;
pub use sync::sync_manifest;
