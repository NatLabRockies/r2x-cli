use crate::config_manager::Config;
use crate::logger;
use crate::GlobalOpts;
use clap::Subcommand;
use colored::*;

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigAction {
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

pub fn handle_config(action: ConfigAction, opts: GlobalOpts) {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: true,
            verbose: 0,
            log_python: false,
        }
    }

    fn verbose_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: false,
            verbose: 1,
            log_python: false,
        }
    }

    fn normal_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: false,
            verbose: 0,
            log_python: false,
        }
    }

    #[test]
    fn test_config_show() {
        handle_config(ConfigAction::Show, normal_opts());
    }

    #[test]
    fn test_config_set() {
        handle_config(
            ConfigAction::Set {
                key: "cache-path".to_string(),
                value: "test-value".to_string(),
            },
            normal_opts(),
        );
    }

    #[test]
    fn test_config_set_quiet() {
        handle_config(
            ConfigAction::Set {
                key: "cache-path".to_string(),
                value: "test-value".to_string(),
            },
            quiet_opts(),
        );
    }

    #[test]
    fn test_config_set_verbose() {
        handle_config(
            ConfigAction::Set {
                key: "cache-path".to_string(),
                value: "test-value".to_string(),
            },
            verbose_opts(),
        );
    }
}
