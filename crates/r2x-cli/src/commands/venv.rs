//! Virtual environment management — top-level shortcut for `r2x config venv *`

use crate::commands::config::{self, VenvAction};
use crate::common::GlobalOpts;
use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum VenvCommand {
    /// Create or recreate the virtual environment
    Create {
        /// Skip confirmation prompt
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
    /// Get or set the venv path
    Path {
        /// Optional new venv path to set
        new_path: Option<String>,
    },
}

pub fn handle_venv(command: VenvCommand, _opts: GlobalOpts) {
    let action = match command {
        VenvCommand::Create { yes } => VenvAction::Create { yes },
        VenvCommand::Path { new_path } => VenvAction::Path { new_path },
    };
    config::handle_venv(action, _opts);
}
