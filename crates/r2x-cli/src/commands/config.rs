use crate::config_manager::Config;
use crate::logger;
use crate::GlobalOpts;
use clap::Subcommand;
use colored::*;
use std::io::{self, Write};

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
                if config.is_empty() {
                    if opts.verbosity_level() > 0 {
                        println!("  {}", "(empty)".yellow());
                    }
                } else {
                    for (key, value) in config.values_iter() {
                        println!("  {}: {}", key.cyan(), value);
                    }
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
                        "cache-path" | "verbosity" | "default-python-version" | "r2x-core-version"
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
                        "Unknown config key: {}. Currently supported keys: cache-path, verbosity, default-python-version, r2x-core-version",
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
