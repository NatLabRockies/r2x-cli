//! r2x: Rust CLI wrapper for r2x-core Python plugins

mod cli;
pub mod config;
pub mod entrypoints;
mod error;
pub mod python;
pub mod schema;
mod symlink;

pub use error::{R2xError, Result};

use clap::Parser;
use std::path::Path;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn run() -> Result<()> {
    init_logging();
    python::prepare_environment()?;

    // Detect how we were invoked (symlink vs direct)
    let program_name = std::env::args()
        .next()
        .and_then(|p| {
            Path::new(&p)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "r2x-cli".to_string());

    // If invoked as a symlink (not r2x-cli or r2x), route to symlink handler
    if program_name != "r2x-cli" && program_name != "r2x" {
        return symlink::execute_from_symlink(&program_name);
    }

    // Normal subcommand-based execution
    let cli = cli::Cli::parse();
    cli.execute()
}

fn init_logging() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "r2x_cli=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer().compact())
        .init();
}
