//! Cache management — top-level shortcut for `r2x config cache *`

use crate::commands::config::{self, CacheAction};
use crate::common::GlobalOpts;
use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum CacheCommand {
    /// Clean the cache folder
    Clean,
    /// Get or set cache path
    Path {
        /// Optional new cache path to set
        new_path: Option<String>,
    },
}

pub fn handle_cache(command: CacheCommand, _opts: GlobalOpts) {
    let action = match command {
        CacheCommand::Clean => CacheAction::Clean,
        CacheCommand::Path { new_path } => CacheAction::Path { new_path },
    };
    config::handle_cache(action, _opts);
}
