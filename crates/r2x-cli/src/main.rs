use clap::{Parser, Subcommand};
use r2x::commands::{
    cache,
    config::{self, ConfigAction, PythonAction},
    init,
    log::{self, LogAction},
    plugins, read, run, self_update, venv,
};
use r2x::common::GlobalOpts;
use r2x_config as config_manager;
use r2x_logger as logger;
use r2x_python::python_bridge::process_exit;
use std::ffi::OsString;

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
        /// Output in JSON format
        #[arg(long)]
        json: bool,
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
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        action: Option<LogAction>,
    },
    /// List installed plugins
    List {
        /// Optional plugin name to filter by (e.g., r2x-reeds)
        plugin: Option<String>,
        /// Optional module/function name to filter by (e.g., break_gens)
        module: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
    /// Install a plugin
    Install {
        plugin: Option<String>,
        /// Install in editable mode (-e)
        #[arg(short, long)]
        editable: bool,
        /// Force re-discovery of plugins (ignore cached metadata)
        #[arg(long)]
        no_cache: bool,
        /// Git host (default: github.com). Use with gh:owner/repo or full URLs.
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
        /// Install a package from a repository subdirectory
        #[arg(long)]
        subdirectory: Option<String>,
    },
    /// Remove a plugin
    Remove { plugin: String },
    /// Sync plugin manifest (re-run plugin discovery for all installed packages)
    Sync {
        /// Upgrade installed plugin packages before syncing metadata
        #[arg(long)]
        upgrade: bool,
    },
    /// Clean plugins and cache (removes installed plugins and cleans cache folder)
    Clean {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Virtual environment management
    Venv {
        #[command(subcommand)]
        command: venv::VenvCommand,
    },
    /// Cache management
    Cache {
        #[command(subcommand)]
        command: cache::CacheCommand,
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
    /// Manage the r2x executable
    Self_(self_update::SelfNamespace),
}

/// Move root global flags ahead of `run` before Clap parses its trailing target.
///
/// `RunCommand` captures the target and all following values verbatim so it
/// can distinguish a pipeline path from a plugin reference. Without this
/// normalization, root-global flags written after `run` would be mistaken for
/// plugin or pipeline arguments and would miss logger initialization.
fn normalize_run_global_args(args: Vec<OsString>) -> Vec<OsString> {
    let Some(command_index) = args.iter().enumerate().skip(1).find_map(|(index, arg)| {
        let arg = arg.to_string_lossy();
        (!is_global_run_option(&arg) && arg != "--").then_some(index)
    }) else {
        return args;
    };

    if args[command_index].to_string_lossy() != "run" {
        return args;
    }

    let mut normalized = Vec::with_capacity(args.len());
    normalized.extend(args[..command_index].iter().cloned());

    let mut run_args = Vec::new();
    let mut moved_globals = Vec::new();
    let mut parse_options = true;
    for arg in args[command_index + 1..].iter().cloned() {
        let arg_text = arg.to_string_lossy();
        if parse_options && arg_text == "--" {
            parse_options = false;
            run_args.push(arg);
        } else if parse_options && is_global_run_option(&arg_text) {
            moved_globals.push(arg);
        } else {
            run_args.push(arg);
        }
    }

    normalized.extend(moved_globals);
    normalized.push(OsString::from("run"));
    normalized.extend(run_args);
    normalized
}

fn is_global_run_option(arg: &str) -> bool {
    matches!(
        arg,
        "--quiet" | "--verbose" | "--log-python" | "--python-log" | "--no-stdout"
    ) || is_global_short_flag(arg)
}

fn is_global_short_flag(arg: &str) -> bool {
    arg.starts_with('-')
        && !arg.starts_with("--")
        && arg.len() > 1
        && arg[1..]
            .chars()
            .all(|character| matches!(character, 'q' | 'v'))
}

fn with_plugin_context<F>(action: F) -> Result<(), r2x::plugins::error::PluginError>
where
    F: FnOnce(&mut plugins::context::PluginContext) -> Result<(), r2x::plugins::error::PluginError>,
{
    let mut ctx = plugins::context::PluginContext::load()?;
    action(&mut ctx)
}

fn exit_on_plugin_error(result: Result<(), r2x::plugins::error::PluginError>) {
    if let Err(e) = result {
        logger::error(&e.to_string());
        std::process::exit(1);
    }
}

fn main() {
    // Respect NO_COLOR and TERM=dumb for accessibility and automation
    if std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").ok().as_deref() == Some("dumb")
    {
        colored::control::set_override(false);
    }

    let cli = Cli::parse_from(normalize_run_global_args(std::env::args_os().collect()));

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
        cli.global.quiet,
        effective_log_python,
        effective_no_stdout,
        saved_log_path,
        saved_log_max_size,
    ) {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }

    if !matches!(cli.command, Commands::Self_(_)) {
        if let Some(cfg) = startup_config.as_mut() {
            if let Err(e) = cfg.ensure_uv_path().and_then(|_| cfg.ensure_cache_path()) {
                logger::warn(&format!("Failed to setup CLI: {}", e));
            }
        }
    }

    match cli.command {
        Commands::Config { json, action } => {
            config::handle_config(action, json, cli.global);
        }
        Commands::Python { action } => {
            config::handle_python(action, cli.global);
        }
        Commands::Log { json, action } => {
            log::handle_log(action, json);
        }
        Commands::List {
            plugin,
            module,
            json,
        } => {
            exit_on_plugin_error(with_plugin_context(|ctx| {
                plugins::list::list_plugins(&cli.global, plugin, module, json, ctx)
            }));
        }
        Commands::Install {
            plugin,
            editable,
            no_cache,
            host,
            branch,
            tag,
            commit,
            subdirectory,
        } => match plugin {
            Some(pkg) => {
                exit_on_plugin_error(with_plugin_context(|ctx| {
                    plugins::install::install_plugin(
                        &pkg,
                        editable,
                        no_cache,
                        plugins::install::GitOptions {
                            host,
                            branch,
                            tag,
                            commit,
                            subdirectory,
                        },
                        ctx,
                    )
                }));
            }
            None => {
                if let Err(e) = plugins::install::show_install_help() {
                    logger::error(&e.to_string());
                    std::process::exit(1);
                }
            }
        },
        Commands::Remove { plugin } => {
            exit_on_plugin_error(with_plugin_context(|ctx| {
                plugins::remove::remove_plugin(&plugin, ctx)
            }));
        }
        Commands::Sync { upgrade } => {
            exit_on_plugin_error(with_plugin_context(|ctx| {
                plugins::sync::sync_manifest(ctx, upgrade)
            }));
        }
        Commands::Clean { yes } => {
            exit_on_plugin_error(with_plugin_context(|ctx| {
                plugins::clean::clean_manifest(yes, ctx)
            }));
        }
        Commands::Venv { command } => {
            venv::handle_venv(command, cli.global);
        }
        Commands::Cache { command } => {
            cache::handle_cache(command, cli.global);
        }
        Commands::Init { file } => {
            init::handle_init(file, cli.global);
        }

        Commands::Run(cmd) => {
            if let Err(e) = run::handle_run(cmd, cli.global) {
                logger::error(&format!("Run command failed: {}", e));
                std::process::exit(1);
            }
            // Re-acquire the GIL on the main thread before exit so that
            // PyO3's atexit handler can call PyEval_SaveThread() without
            // crashing. See python_bridge::process_exit for details.
            process_exit(0);
        }
        Commands::Read(cmd) => {
            if let Err(e) = read::handle_read(cmd, cli.global) {
                logger::error(&format!("Read command failed: {}", e));
                std::process::exit(1);
            }
        }
        Commands::Self_(args) => match self_update::handle_self_command(args) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                logger::error(&e.to_string());
                std::process::exit(1);
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_run_global_args;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn moves_global_options_before_run_for_clap() {
        assert_eq!(
            normalize_run_global_args(args(&[
                "r2x",
                "run",
                "r2x-reeds.reeds-parser",
                "-qv",
                "--python-log",
                "--no-stdout",
            ])),
            args(&[
                "r2x",
                "-qv",
                "--python-log",
                "--no-stdout",
                "run",
                "r2x-reeds.reeds-parser",
            ])
        );
    }

    #[test]
    fn keeps_options_after_double_dash_with_the_plugin() {
        assert_eq!(
            normalize_run_global_args(args(&[
                "r2x",
                "run",
                "r2x-reeds.reeds-parser",
                "--",
                "--quiet",
            ])),
            args(&["r2x", "run", "r2x-reeds.reeds-parser", "--", "--quiet",])
        );
    }

    #[test]
    fn leaves_non_run_commands_unchanged() {
        let input = args(&["r2x", "install", "--no-stdout", "r2x-reeds"]);
        assert_eq!(normalize_run_global_args(input.clone()), input);
    }
}
