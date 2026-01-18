use clap::{Parser, Subcommand};
use r2x::{
    commands::{
        config::{self, ConfigAction},
        init, plugins, read, run,
    },
    config_manager, logger, GlobalOpts,
};

#[derive(Parser)]
#[command(name = "r2x")]
#[command(version = env!("CARGO_PKG_VERSION"))]
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

#[derive(Subcommand)]
enum Commands {
    /// Configure r2x tool
    #[command(subcommand_required = false, arg_required_else_help = false)]
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// List installed plugins
    List {
        /// Optional plugin name to filter by (e.g., r2x-reeds)
        plugin: Option<String>,
        /// Optional module/function name to filter by (e.g., break_gens)
        module: Option<String>,
    },
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
    /// Sync plugin manifest (re-run plugin discovery for all installed packages)
    /// Useful when developing plugins locally with -e to refresh the plugin registry
    Sync,
    /// Clean the plugin manifest (removes all installed plugins)
    Clean {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Initialize a new pipeline file
    Init {
        /// Optional filename for the pipeline (default: pipeline.yaml)
        file: Option<String>,
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

fn main() {
    let cli = Cli::parse();

    // Initialize logger with verbosity level, log_python flag, and no_stdout flag
    if let Err(e) = logger::init_with_verbosity(
        cli.global.verbosity_level(),
        cli.global.log_python,
        cli.global.no_stdout,
    ) {
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
        Commands::Config { action } => {
            config::handle_config(action, cli.global);
        }
        Commands::List { plugin, module } => {
            if let Err(e) = plugins::list_plugins(&cli.global, plugin, module) {
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
                if let Err(e) = plugins::install_plugin(
                    &pkg,
                    editable,
                    no_cache,
                    plugins::GitOptions {
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
                if let Err(e) = plugins::show_install_help() {
                    logger::error(&e);
                }
            }
        },
        Commands::Remove { plugin } => {
            if let Err(e) = plugins::remove_plugin(&plugin, &cli.global) {
                logger::error(&e);
            }
        }
        Commands::Sync => {
            if let Err(e) = plugins::sync_manifest(&cli.global) {
                logger::error(&e);
            }
        }
        Commands::Clean { yes } => {
            if let Err(e) = plugins::clean_manifest(yes, &cli.global) {
                logger::error(&e);
            }
        }
        Commands::Init { file } => {
            init::handle_init(file, cli.global);
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
