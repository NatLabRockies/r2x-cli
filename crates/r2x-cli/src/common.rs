//! Common types and utilities shared across modules

use clap::Parser;

/// Global CLI options available to all commands
#[derive(Parser, Debug, Clone)]
pub struct GlobalOpts {
    #[arg(short, long, global = true, help = "Decrease verbosity")]
    pub quiet: bool,

    #[arg(short, long, global = true, action = clap::ArgAction::Count, help = "Increase verbosity (-v for debug, -vv for trace)")]
    pub verbose: u8,

    #[arg(
        long,
        global = true,
        help = "Show Python logs on console (always logged to file)"
    )]
    pub log_python: bool,
}

impl GlobalOpts {
    /// Get the effective verbosity level
    /// - 0: quiet/warn only
    /// - 1: debug (-v)
    /// - 2: trace (-vv)
    pub fn verbosity_level(&self) -> u8 {
        if self.quiet {
            0
        } else {
            self.verbose
        }
    }
}
