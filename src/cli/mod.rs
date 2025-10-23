mod config;
mod plugins;
pub mod read;
pub mod run;
mod shell;
mod upgrade;
pub mod write;

use crate::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "r2x")]
#[command(about = "Energy model data converter", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Disable progress output
    #[arg(long, global = true)]
    quiet: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage plugins (list, install, uninstall)
    Plugin(plugins::PluginsArgs),

    /// Read model data and output system as JSON
    Read(read::ReadArgs),

    /// Write system JSON to model format
    Write(write::WriteArgs),

    /// Run a system modifier
    Run(run::RunArgs),

    /// Load system and start interactive IPython shell
    Shell(shell::ShellArgs),

    /// Manage r2x configuration
    Config(config::ConfigArgs),

    /// Upgrade r2x to the latest version
    Upgrade(upgrade::UpgradeArgs),
}

impl Cli {
    pub fn execute(self) -> Result<()> {
        let verbose = self.verbose;
        let quiet = self.quiet;

        match self.command {
            Commands::Plugin(args) => plugins::execute(args),
            Commands::Read(args) => read::execute(args, verbose, quiet),
            Commands::Write(args) => write::execute(args),
            Commands::Run(args) => run::execute(args),
            Commands::Shell(args) => shell::execute(args, verbose, quiet),
            Commands::Config(args) => config::execute(args),
            Commands::Upgrade(args) => upgrade::execute(args),
        }
    }
}
