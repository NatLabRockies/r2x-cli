use crate::config_manager::Config;
use crate::logger;
use crate::plugins::get_package_info;
use crate::python_bridge::configure_python_venv;
use crate::GlobalOpts;
use clap::Subcommand;
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigAction {
    /// Display the current configuration values.
    Show,
    /// Update a configuration key (e.g. `r2x config set default-python-version 3.13`).
    Set { key: String, value: String },
    /// Show the config path or set it when `new_path` is provided.
    Path {
        /// Optional new config path to set
        new_path: Option<String>,
    },
    /// Reset configuration back to defaults.
    Reset {
        /// Skip confirmation prompt
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
    /// Python version management
    #[command(subcommand)]
    Python(PythonAction),
    /// Virtual environment management
    #[command(subcommand)]
    Venv(VenvAction),
    /// Cache management
    #[command(subcommand)]
    Cache(CacheAction),
}

#[derive(Subcommand, Debug, Clone)]
pub enum PythonAction {
    /// Install a different Python version
    Install {
        /// Python version to install (e.g., 3.13, 3.12.1)
        version: Option<String>,
    },
    /// Get the Python executable path in the configured venv
    Path,
    /// Show the configured Python version and venv information
    Show,
}

#[derive(Subcommand, Debug, Clone)]
pub enum VenvAction {
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

#[derive(Subcommand, Debug, Clone)]
pub enum CacheAction {
    /// Clean the cache folder
    Clean,
    /// Get or set cache path
    Path {
        /// Optional new cache path to set
        new_path: Option<String>,
    },
}

pub fn handle_config(action: Option<ConfigAction>, opts: GlobalOpts) {
    let action = match action {
        Some(action) => action,
        None => {
            println!(
                "{}",
                "Tip: run `r2x config show` to inspect settings or `r2x config set <key> <value>` to update them."
                    .dimmed()
            );
            return;
        }
    };

    match action {
        ConfigAction::Show => match Config::load() {
            Ok(config) => {
                println!("{}", "Configuration:".bold().green());

                // Show Python version (explicit or default)
                let python_version = config.python_version.as_deref().unwrap_or("3.12");
                let python_suffix = if config.python_version.is_none() {
                    " (default)"
                } else {
                    ""
                };
                println!(
                    "  {}: {}{}",
                    "python-version".cyan(),
                    python_version,
                    python_suffix.dimmed()
                );

                // Show venv path (computed)
                let venv_path = config.get_venv_path();
                let venv_suffix = if config.venv_path.is_some() {
                    ""
                } else {
                    " (default)"
                };
                println!(
                    "  {}: {}{}",
                    "venv-path".cyan(),
                    venv_path,
                    venv_suffix.dimmed()
                );

                // Show cache path (computed)
                let cache_path = config.get_cache_path();
                let cache_suffix = if config.cache_path.is_some() {
                    ""
                } else {
                    " (default)"
                };
                println!(
                    "  {}: {}{}",
                    "cache-path".cyan(),
                    cache_path,
                    cache_suffix.dimmed()
                );

                // Show other explicit config values
                if let Some(ref uv) = config.uv_path {
                    println!("  {}: {}", "uv-path".cyan(), uv);
                }
                if let Some(ref core_ver) = config.r2x_core_version {
                    println!("  {}: {}", "r2x-core-version".cyan(), core_ver);
                }

                // Show installed r2x-core version
                let python_path = config.get_venv_python_path();
                if PathBuf::from(&python_path).exists() {
                    // Try to get uv_path from config, or use "uv" from PATH
                    let uv_path = config.uv_path.as_deref().unwrap_or("uv");
                    match get_package_info(uv_path, &python_path, "r2x-core") {
                        Ok((Some(version), _)) => {
                            println!("  {}: {}", "r2x-core-version".cyan(), version);
                        }
                        Ok((None, _)) => {
                            println!("  {}: {}", "r2x-core-version".cyan(), "not found".dimmed());
                        }
                        Err(_) => {
                            logger::debug("Could not query r2x-core package info");
                        }
                    }
                } else {
                    logger::debug("Venv does not exist, skipping r2x-core version check");
                }
            }
            Err(e) => {
                logger::error(&format!("Failed to load config: {}", e));
            }
        },
        ConfigAction::Set { key, value } => match Config::load() {
            Ok(mut config) => {
                if config.get(&key).is_some()
                    || matches!(
                        key.as_str(),
                        "cache-path"
                            | "verbosity"
                            | "python-version"
                            | "venv-path"
                            | "r2x-core-version"
                    )
                {
                    config.set(&key, value.clone());
                    match config.save() {
                        Ok(_) => {
                            logger::success(&format!("Set {} = {}", key, value));
                        }
                        Err(e) => {
                            logger::error(&format!("Failed to save config: {}", e));
                        }
                    }
                    println!(
                        "{}",
                        "Tip: run `r2x config show` to confirm the updated value.".dimmed()
                    );
                } else {
                    logger::error(&format!(
                        "Unknown config key: {}. Currently supported keys: cache-path, verbosity, python-version, venv-path, r2x-core-version",
                        key
                    ));
                }
            }
            Err(e) => {
                logger::error(&format!("Failed to load config: {}", e));
            }
        },
        ConfigAction::Path { new_path } => {
            // Show or set the configuration file path.
            // When `new_path` is provided, write it to a pointer file next to the default config dir.
            // When omitted, print the current resolved config path.
            let config_path = Config::path();
            logger::debug(&format!("Reading config from: {}", config_path.display()));

            match new_path {
                Some(p) => {
                    // Pointer file path: same directory as default config, file named `.r2x_config_path`
                    let pointer_path = config_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(".r2x_config_path");

                    // Ensure pointer directory exists
                    if let Some(parent) = pointer_path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            logger::error(&format!("Failed to set config path: {}", e));
                            return;
                        }
                    }

                    if let Err(e) = std::fs::write(&pointer_path, p.as_bytes()) {
                        logger::error(&format!("Failed to set config path: {}", e));
                        return;
                    }

                    logger::success(&format!("Config path set to {}", p));
                }
                None => {
                    // Print the resolved config path
                    println!("{}", config_path.display());

                    // If pointer file exists, also show the override
                    let pointer_path = config_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(".r2x_config_path");
                    if pointer_path.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&pointer_path) {
                            let trimmed = contents.trim();
                            if !trimmed.is_empty() {
                                println!("{} {}", "overridden-by".cyan(), trimmed);
                            }
                        }
                    }
                }
            }
        }
        ConfigAction::Reset { yes } => {
            let config_path = Config::path();
            if !yes {
                print!(
                    "{} Reset R2X configuration at `{}` to default settings? {} ",
                    "?".bold().cyan(),
                    config_path.display(),
                    "[y/n] ›".dimmed()
                );
                if let Err(e) = io::stdout().flush() {
                    logger::error(&format!("Failed to flush stdout: {}", e));
                    return;
                }
                let mut input = String::new();
                match io::stdin().read_line(&mut input) {
                    Ok(_) => {
                        let response = input.trim().to_lowercase();
                        if response != "y" && response != "yes" {
                            println!("{}", "Reset cancelled.".yellow());
                            return;
                        }
                    }
                    Err(e) => {
                        logger::error(&format!("Failed to read confirmation: {}", e));
                        return;
                    }
                }
            }

            if opts.verbosity_level() > 0 {
                logger::step("Resetting configuration to defaults");
            }
            match Config::reset() {
                Ok(_) => {
                    println!(
                        "{} configuration {} has been reset to default settings.",
                        "\u{2714}".green().bold(),
                        config_path.display()
                    );
                }
                Err(e) => {
                    logger::error(&format!("Failed to reset config: {}", e));
                }
            }
        }
        ConfigAction::Python(python_action) => {
            handle_python(python_action, opts);
        }
        ConfigAction::Venv(venv_action) => {
            handle_venv(venv_action, opts);
        }
        ConfigAction::Cache(cache_action) => {
            handle_cache(cache_action, opts);
        }
    }
}

/// Handle Python version management
fn handle_python(action: PythonAction, opts: GlobalOpts) {
    match action {
        PythonAction::Show => {
            handle_python_show(opts);
        }
        PythonAction::Path => {
            handle_python_path(opts);
        }
        PythonAction::Install { version } => {
            handle_python_install(version, opts);
        }
    }
}

/// Handle virtual environment management
fn handle_venv(action: VenvAction, opts: GlobalOpts) {
    match action {
        VenvAction::Create { yes } => {
            handle_venv_create(yes);
        }
        VenvAction::Path { new_path } => {
            handle_venv_path(new_path, opts);
        }
    }
}

/// Handle cache management
fn handle_cache(action: CacheAction, opts: GlobalOpts) {
    match action {
        CacheAction::Clean => {
            clean_cache(opts);
        }
        CacheAction::Path { new_path } => {
            handle_cache_path(new_path, opts);
        }
    }
}

/// Install a specific Python version
fn handle_python_install(version: Option<String>, _opts: GlobalOpts) {
    logger::debug("Handling Python install command");
    match Config::load() {
        Ok(mut config) => {
            let version_str = version
                .or_else(|| config.python_version.clone())
                .unwrap_or_else(|| "3.12".to_string());

            config.python_version = Some(version_str.clone());
            if let Err(e) = config.save() {
                logger::error(&format!("Failed to save config: {}", e));
                return;
            }

            let venv_path = config.get_venv_path();
            logger::step(&format!(
                "Installing Python {} and creating venv...",
                version_str
            ));
            if let Err(e) = remove_existing_venv(&venv_path) {
                logger::error(&e);
                return;
            }

            match configure_python_venv() {
                Ok(python_env) => {
                    logger::info(&format!(
                        "Configuration saved with Python version {}",
                        version_str
                    ));
                    if let Some(actual_version) = verify_python_version(&python_env.interpreter) {
                        logger::success(&format!(
                            "Python {} installed (reported {}). Venv ready at {}",
                            version_str,
                            actual_version,
                            PathBuf::from(&venv_path).display()
                        ));
                    } else {
                        logger::success(&format!(
                            "Python {} installed and venv created at {}",
                            version_str, venv_path
                        ));
                    }
                }
                Err(e) => {
                    logger::error(&format!("Failed to configure Python environment: {}", e));
                }
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

/// Output the Python executable path
fn handle_python_path(_opts: GlobalOpts) {
    logger::debug("Handling python path command");
    match Config::load() {
        Ok(config) => {
            println!("{}", config.get_venv_python_path());
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn handle_venv_create(skip_confirmation: bool) {
    logger::debug(&format!(
        "Handling venv create command (skip_confirmation: {})",
        skip_confirmation
    ));
    match Config::load() {
        Ok(config) => {
            let venv_path = config.get_venv_path();
            let venv_dir = PathBuf::from(&venv_path);

            if venv_dir.exists() {
                let should_skip = skip_confirmation || std::env::var("R2X_VENV_YES").is_ok();

                if !should_skip {
                    print!(
                        "{} A virtual environment already exists at `{}`. Do you want to replace it? {} ",
                        "?".bold().cyan(),
                        venv_path,
                        "[y/n] ›".dimmed()
                    );
                    io::stdout().flush().unwrap();
                    logger::debug("Prompting user for venv replacement confirmation");

                    let mut response = String::new();
                    if io::stdin().read_line(&mut response).is_ok() {
                        let response = response.trim().to_lowercase();
                        if response != "y" && response != "yes" {
                            logger::info("Operation cancelled by user");
                            println!("Operation cancelled.");
                            return;
                        }
                        logger::debug("User confirmed venv replacement");
                    } else {
                        logger::error("Failed to read input");
                        return;
                    }
                } else {
                    logger::debug("Skipping confirmation (--yes flag or R2X_VENV_YES set)");
                }

                if let Err(e) = remove_existing_venv(&venv_path) {
                    logger::error(&e);
                    return;
                }
            }

            match configure_python_venv() {
                Ok(python_env) => {
                    logger::success(&format!(
                        "Virtual environment ready at {} (python {})",
                        venv_path,
                        python_env.interpreter.display()
                    ));
                }
                Err(e) => logger::error(&format!("Failed to configure venv: {}", e)),
            }

            if !skip_confirmation && std::env::var("R2X_VENV_YES").is_err() {
                println!(
                    "\n{} Use the `{}` flag or set `{}` to skip this prompt",
                    "hint:".dimmed(),
                    "-y/--yes".bold(),
                    "R2X_VENV_YES=1".bold()
                );
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn handle_venv_path(new_path: Option<String>, _opts: GlobalOpts) {
    logger::debug("Handling venv path command");
    match Config::load() {
        Ok(mut config) => {
            // Ensure uv is installed first
            if let Err(e) = config.ensure_uv_path() {
                logger::error(&format!("Failed to setup uv: {}", e));
                return;
            }

            if let Some(path) = new_path {
                logger::debug(&format!("Setting venv path to: {}", path));
                let venv_path = PathBuf::from(&path);

                if !venv_path.exists() {
                    logger::error(&format!("Path does not exist: {}", path));
                    return;
                }

                if !is_valid_venv(&venv_path) {
                    logger::error(&format!("Path is not a valid venv: {}", path));
                    return;
                }

                config.venv_path = Some(path.clone());
                if let Err(e) = config.save() {
                    logger::error(&format!("Failed to save config: {}", e));
                    return;
                }

                logger::success(&format!("Venv path set to {}", path));
            } else {
                let venv_path = config.get_venv_path();
                logger::debug(&format!("Current venv path: {}", venv_path));

                if !PathBuf::from(&venv_path).exists() {
                    logger::error(&format!("Venv path does not exist: {}", venv_path));
                    return;
                }

                if !is_valid_venv(&PathBuf::from(&venv_path)) {
                    logger::error(&format!("Venv path is not a valid venv: {}", venv_path));
                    return;
                }

                println!("{}", venv_path);
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

// Simple check to avoid people setting virtual environments to not executable folders.
fn is_valid_venv(path: &Path) -> bool {
    logger::debug(&format!("Validating venv at: {}", path.display()));
    if !path.exists() || !path.is_dir() {
        logger::debug("Path does not exist or is not a directory");
        return false;
    }

    let bin_dir = if cfg!(windows) {
        path.join("Scripts")
    } else {
        path.join("bin")
    };

    bin_dir.exists() && bin_dir.is_dir()
}

fn remove_existing_venv(venv_path: &str) -> Result<(), String> {
    let venv_dir = PathBuf::from(venv_path);
    if venv_dir.exists() {
        logger::debug(&format!("Removing existing venv at {}", venv_path));
        fs::remove_dir_all(&venv_dir)
            .map_err(|e| format!("Failed to remove existing venv: {}", e))?;
    }
    Ok(())
}

fn verify_python_version(python_path: &Path) -> Option<String> {
    if !python_path.exists() {
        return None;
    }

    match Command::new(python_path).args(["--version"]).output() {
        Ok(output) if output.status.success() => {
            let raw = if output.stdout.is_empty() {
                output.stderr
            } else {
                output.stdout
            };
            Some(String::from_utf8_lossy(&raw).trim().to_string())
        }
        _ => None,
    }
}

fn handle_python_show(_opts: GlobalOpts) {
    logger::debug("Handling python show command");
    match Config::load() {
        Ok(config) => {
            let version = config.python_version.as_deref().unwrap_or("not configured");

            let venv_path = config.get_venv_path();
            let python_path = PathBuf::from(config.get_venv_python_path());
            let venv_exists = python_path.exists();

            let mut actual_version_str = String::new();
            let mut version_mismatch = false;
            if venv_exists {
                if let Some(actual_version) = verify_python_version(&python_path) {
                    actual_version_str = actual_version.clone();

                    if let Some(version_num) = actual_version.split_whitespace().nth(1) {
                        let configured_short =
                            version.split('.').take(2).collect::<Vec<_>>().join(".");
                        let actual_short =
                            version_num.split('.').take(2).collect::<Vec<_>>().join(".");
                        if configured_short != actual_short && version != "not configured" {
                            version_mismatch = true;
                        }
                    }
                } else {
                    logger::debug("Could not determine actual Python version");
                }
            }

            // Show warning first if there's a version mismatch
            if version_mismatch {
                logger::warn(&format!(
                    "Version mismatch: config has {}, venv has {}. Run 'r2x config venv create --yes' to recreate.",
                    version, actual_version_str.trim()
                ));
            }

            println!("{}", "Python Configuration:".bold().green());
            println!("  version: {}", version);
            println!("  venv path: {}", venv_path);
            println!("  venv exists: {}", if venv_exists { "yes" } else { "no" });
            if !actual_version_str.is_empty() {
                println!("  Actual venv version: {}", actual_version_str.trim());
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn clean_cache(_opts: GlobalOpts) {
    match Config::load() {
        Ok(config) => {
            let cache_path = config.get_cache_path();
            let cache_dir = PathBuf::from(&cache_path);

            if !cache_dir.exists() {
                logger::debug("Cache folder already clean");
                return;
            }

            match fs::remove_dir_all(&cache_dir) {
                Ok(_) => {
                    logger::success("Cache folder cleaned");
                }
                Err(e) => {
                    logger::error(&format!("Failed to clean cache folder: {}", e));
                }
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn handle_cache_path(new_path: Option<String>, _opts: GlobalOpts) {
    match Config::load() {
        Ok(mut config) => {
            if let Some(path) = new_path {
                let cache_path = PathBuf::from(&path);

                if let Err(e) = fs::create_dir_all(&cache_path) {
                    logger::error(&format!("Failed to create cache directory: {}", e));
                    return;
                }

                config.cache_path = Some(path.clone());
                if let Err(e) = config.save() {
                    logger::error(&format!("Failed to save config: {}", e));
                    return;
                }

                logger::success(&format!("Cache path set to {}", path));
            } else {
                let cache_path = config.get_cache_path();
                println!("{}", cache_path);
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: 1,
            verbose: 0,
            log_python: false,
        }
    }

    fn verbose_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: 0,
            verbose: 1,
            log_python: false,
        }
    }

    fn normal_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: 0,
            verbose: 0,
            log_python: false,
        }
    }

    #[test]
    fn test_config_show() {
        handle_config(Some(ConfigAction::Show), normal_opts());
    }

    #[test]
    fn test_config_set() {
        handle_config(
            Some(ConfigAction::Set {
                key: "cache-path".to_string(),
                value: "test-value".to_string(),
            }),
            normal_opts(),
        );
    }

    #[test]
    fn test_config_set_quiet() {
        handle_config(
            Some(ConfigAction::Set {
                key: "cache-path".to_string(),
                value: "test-value".to_string(),
            }),
            quiet_opts(),
        );
    }

    #[test]
    fn test_config_set_verbose() {
        handle_config(
            Some(ConfigAction::Set {
                key: "cache-path".to_string(),
                value: "test-value".to_string(),
            }),
            verbose_opts(),
        );
    }

    #[test]
    fn test_config_reset() {
        handle_config(Some(ConfigAction::Reset { yes: true }), normal_opts());
    }

    #[test]
    fn test_config_no_action_tip() {
        handle_config(None, normal_opts());
    }
}
