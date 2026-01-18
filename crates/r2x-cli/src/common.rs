//! Common types and utilities shared across modules

use clap::{ArgAction, Parser};

/// Global CLI options available to all commands
#[derive(Parser, Debug, Clone)]
pub struct GlobalOpts {
    #[arg(
        short = 'q',
        long = "quiet",
        global = true,
        action = ArgAction::Count,
        help = "Decrease verbosity (-q suppresses logs, -qq also hides plugin stdout)"
    )]
    pub quiet: u8,

    #[arg(short, long, global = true, action = ArgAction::Count, help = "Increase verbosity (-v for debug, -vv for trace)")]
    pub verbose: u8,

    #[arg(
        long,
        global = true,
        help = "Show Python logs on console (always logged to file)"
    )]
    pub log_python: bool,

    #[arg(
        long,
        global = true,
        help = "Disable logging stdout to file (useful with --log-python to avoid large system objects in logs)"
    )]
    pub no_stdout: bool,
}

impl GlobalOpts {
    /// Get the effective verbosity level
    /// - 0: quiet/warn only
    /// - 1: debug (-v)
    /// - 2: trace (-vv)
    pub fn verbosity_level(&self) -> u8 {
        if self.quiet > 0 {
            0
        } else {
            self.verbose
        }
    }

    /// Returns true when output (plugin stdout) should be fully suppressed
    pub fn suppress_stdout(&self) -> bool {
        self.quiet >= 2
    }
}
