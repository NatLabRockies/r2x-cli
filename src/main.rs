use clap::{Parser, Subcommand};

mod commands;
pub mod config_manager;
pub mod errors;
pub mod help;
pub mod logger;
pub mod package_verification;
pub mod pipeline_config;
pub mod plugin_cache;
pub mod plugin_manifest;
mod plugins;
pub mod python_bridge;
use commands::{cache, config, python, read, run};
use plugins::{
    clean_manifest, install_plugin, list_plugins, remove_plugin, show_install_help, GitOptions,
};

#[derive(Parser)]
#[command(name = "r2x")]
#[command(version = "0.1.0")]
#[command(
    about = "Energy translator framework",
    long_about = "R2X is a CLI tool for translating models."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

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
    pub fn verbosity_level(&self) -> u8 {
        if self.quiet {
            0
        } else {
            // 0 = quiet/warn only, 1 = debug (-v), 2 = trace (-vv)
            self.verbose
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Cache configuration
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Configure r2x tool
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// List installed plugins
    List,
    /// Install a plugin
    Install {
        plugin: Option<String>,
        /// Install in editable mode (-e)
        #[arg(short, long)]
        editable: bool,
        /// Skip metadata cache and force rebuild
        #[arg(long)]
        no_cache: bool,
        /// Git host (default: github.com). Use with org/repo format or full URLs.
        #[arg(long)]
        host: Option<String>,
        /// Install from a git branch
        #[arg(long, conflicts_with_all = ["tag", "commit"])]
        branch: Option<String>,
        /// Install from a git tag
        #[arg(long, conflicts_with_all = ["branch", "commit"])]
        tag: Option<String>,
        /// Install from a git commit hash
        #[arg(long, conflicts_with_all = ["branch", "tag"])]
        commit: Option<String>,
    },
    /// Remove a plugin
    Remove { plugin: String },
    /// Clean the plugin manifest (removes all installed plugins)
    Clean {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
    ///  Configure Python installation and virtual environment
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Python {
        #[command(subcommand)]
        action: PythonAction,
    },
    /// Run pipelines or plugins
    Run(run::RunCommand),
    /// Read a system from JSON (stdin or file) and open an interactive IPython session
    Read {
        /// Path to JSON file to read. If not provided, reads from stdin
        file: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// Clean the cache folder
    Clean,
    /// Get or set cache path
    Path {
        /// Optional new cache path to set
        new_path: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    Show,
    Set {
        key: String,
        value: String,
    },
    /// Get or set the path to the config file.
    /// If `new_path` is provided, the CLI will set the config path to that value.
    /// If omitted, the CLI will print the current configuration file path.
    Path {
        /// Optional new config path to set
        new_path: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginsAction {
    /// List current plugins.
    List,
    /// Install a package with plugins
    Install {
        plugin: String,
        /// Install in editable mode (-e)
        #[arg(short, long)]
        editable: bool,
        /// Skip metadata cache and force rebuild
        #[arg(long)]
        no_cache: bool,
        /// Git host (default: github.com). Use with org/repo format or full URLs.
        #[arg(long)]
        host: Option<String>,
        /// Install from a git branch
        #[arg(long, conflicts_with_all = ["tag", "commit"])]
        branch: Option<String>,
        /// Install from a git tag
        #[arg(long, conflicts_with_all = ["branch", "commit"])]
        tag: Option<String>,
        /// Install from a git commit hash
        #[arg(long, conflicts_with_all = ["branch", "tag"])]
        commit: Option<String>,
    },
    /// Remove a plugin
    Remove { plugin: String },
    /// Clean the plugin manifest (removes all installed plugins)
    Clean {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum PythonAction {
    /// Install a different python version.
    Install {
        #[arg(long, help = "Python version to install")]
        version: Option<String>,
    },
    /// Create or manage virtual environment
    Venv {
        #[command(subcommand)]
        subcommand: Option<VenvSubcommand>,

        /// Skip confirmation and clear existing venv
        #[arg(long, global = true)]
        clear: bool,
    },
    /// Show the configured Python version and venv information
    Show,
}

#[derive(Subcommand, Clone, Debug)]
pub enum VenvSubcommand {
    /// Get or set the venv path
    Path {
        /// Optional new venv path to set
        new_path: Option<String>,
    },
    /// Update r2x-core in the venv to the configured version
    UpdateCore,
}

fn main() {
    let cli = Cli::parse();

    // Initialize logger with verbosity level and log_python flag
    if let Err(e) = logger::init_with_verbosity(cli.global.verbosity_level(), cli.global.log_python)
    {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }

    if let Err(e) = config_manager::Config::load().and_then(|mut cfg| {
        cfg.ensure_uv_path()?;
        cfg.ensure_cache_path()?;
        Ok(())
    }) {
        logger::warn(&format!("Failed to setup CLI: {}", e));
    }

    match cli.command {
        Commands::Cache { action } => {
            cache::handle_cache(action, cli.global);
        }
        Commands::Config { action } => {
            config::handle_config(action, cli.global);
        }
        Commands::List => {
            if let Err(e) = list_plugins(&cli.global) {
                logger::error(&e);
            }
        }
        Commands::Install {
            plugin,
            editable,
            no_cache,
            host,
            branch,
            tag,
            commit,
        } => match plugin {
            Some(pkg) => {
                if let Err(e) = install_plugin(
                    &pkg,
                    editable,
                    no_cache,
                    GitOptions {
                        host,
                        branch,
                        tag,
                        commit,
                    },
                    &cli.global,
                ) {
                    logger::error(&e);
                }
            }
            None => {
                if let Err(e) = show_install_help() {
                    logger::error(&e);
                }
            }
        },
        Commands::Remove { plugin } => {
            if let Err(e) = remove_plugin(&plugin, &cli.global) {
                logger::error(&e);
            }
        }
        Commands::Clean { yes } => {
            if let Err(e) = clean_manifest(yes, &cli.global) {
                logger::error(&e);
            }
        }
        Commands::Python { action } => {
            python::handle_python(action, cli.global);
        }
        Commands::Run(cmd) => {
            if let Err(e) = run::handle_run(cmd, cli.global) {
                logger::error(&format!("Run command failed: {}", e));
                std::process::exit(1);
            }
        }
        Commands::Read { file } => {
            let cmd = read::ReadCommand { file };
            if let Err(e) = read::handle_read(cmd, cli.global) {
                logger::error(&format!("Read command failed: {}", e));
                std::process::exit(1);
            }
        }
    }
}
