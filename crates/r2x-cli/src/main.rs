use clap::{Parser, Subcommand};
use r2x::commands::{
    config::{self, ConfigAction, PythonAction},
    init,
    log::{self, LogAction},
    plugins, read, run,
};
use r2x::common::GlobalOpts;
use r2x_config as config_manager;
use r2x_logger as logger;

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
    /// Python runtime management
    Python {
        #[command(subcommand)]
        action: PythonAction,
    },
    /// Logging configuration
    #[command(subcommand_required = false, arg_required_else_help = false)]
    Log {
        #[command(subcommand)]
        action: Option<LogAction>,
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
    Read(read::ReadCommand),
}

fn with_plugin_context<F>(action: F)
where
    F: FnOnce(&mut plugins::context::PluginContext) -> Result<(), r2x::plugins::error::PluginError>,
{
    let mut ctx = match plugins::context::PluginContext::load() {
        Ok(ctx) => ctx,
        Err(e) => {
            logger::error(&e.to_string());
            return;
        }
    };

    if let Err(e) = action(&mut ctx) {
        logger::error(&e.to_string());
    }
}

fn main() {
    let cli = Cli::parse();

    let mut startup_config = match config_manager::Config::load() {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            eprintln!("Warning: Failed to load config: {}", e);
            None
        }
    };

    let (saved_log_python, saved_no_stdout, saved_log_path, saved_log_max_size) =
        match startup_config.as_ref() {
            Some(cfg) => (
                cfg.log_python.unwrap_or(false),
                cfg.no_stdout.unwrap_or(false),
                cfg.log_path.as_deref(),
                cfg.log_max_size,
            ),
            None => (false, false, None, None),
        };
    let effective_log_python = cli.global.log_python || saved_log_python;
    let effective_no_stdout = cli.global.no_stdout || saved_no_stdout;

    // Initialize logger with verbosity level, log_python flag, and no_stdout flag
    if let Err(e) = logger::init_with_config(
        cli.global.verbosity_level(),
        effective_log_python,
        effective_no_stdout,
        saved_log_path,
        saved_log_max_size,
    ) {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }

    if let Some(cfg) = startup_config.as_mut() {
        if let Err(e) = cfg.ensure_uv_path().and_then(|_| cfg.ensure_cache_path()) {
            logger::warn(&format!("Failed to setup CLI: {}", e));
        }
    }

    match cli.command {
        Commands::Config { action } => {
            config::handle_config(action, cli.global);
        }
        Commands::Python { action } => {
            config::handle_python(action, cli.global);
        }
        Commands::Log { action } => {
            log::handle_log(action);
        }
        Commands::List { plugin, module } => {
            with_plugin_context(|ctx| {
                plugins::list::list_plugins(&cli.global, plugin, module, ctx)
            });
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
                with_plugin_context(|ctx| {
                    plugins::install::install_plugin(
                        &pkg,
                        editable,
                        no_cache,
                        plugins::install::GitOptions {
                            host,
                            branch,
                            tag,
                            commit,
                        },
                        ctx,
                    )
                });
            }
            None => {
                if let Err(e) = plugins::install::show_install_help() {
                    logger::error(&e.to_string());
                }
            }
        },
        Commands::Remove { plugin } => {
            with_plugin_context(|ctx| plugins::remove::remove_plugin(&plugin, ctx));
        }
        Commands::Sync => {
            with_plugin_context(plugins::sync::sync_manifest);
        }
        Commands::Clean { yes } => {
            with_plugin_context(|ctx| plugins::clean::clean_manifest(yes, ctx));
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
        Commands::Read(cmd) => {
            if let Err(e) = read::handle_read(cmd, cli.global) {
                logger::error(&format!("Read command failed: {}", e));
                std::process::exit(1);
            }
        }
    }
}
