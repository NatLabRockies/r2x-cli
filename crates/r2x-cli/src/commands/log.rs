use clap::ArgAction;
use clap::Subcommand;
use colored::Colorize;
use r2x_config::Config;
use r2x_logger as logger;

#[derive(Subcommand, Debug, Clone)]
pub enum LogAction {
    /// Display logging settings and current log file path
    Show,
    /// Print current log path or set a new path override
    Path {
        /// Optional new log file path to set
        new_path: Option<String>,
    },
    /// Update logging settings
    Set {
        #[command(subcommand)]
        setting: LogSetAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum LogSetAction {
    /// Set max log file size in bytes (example: 26214400 for 25 MiB)
    MaxSize {
        /// Maximum size in bytes
        bytes: u64,
    },
    /// Enable or disable Python logs on console by default
    LogPython {
        /// true or false
        #[arg(action = ArgAction::Set)]
        enabled: bool,
    },
    /// Enable or disable plugin stdout capture in logs by default
    NoStdout {
        /// true or false
        #[arg(action = ArgAction::Set)]
        enabled: bool,
    },
}

pub fn handle_log(action: Option<LogAction>) {
    let action = if let Some(action) = action {
        action
    } else {
        println!(
            "{}",
            "Tip: run `r2x log show` or `r2x log set <key> <value>`.".dimmed()
        );
        return;
    };

    match action {
        LogAction::Show => show_logging_config(),
        LogAction::Path { new_path } => handle_log_path(new_path),
        LogAction::Set { setting } => set_logging_config(setting),
    }
}

fn show_logging_config() {
    match Config::load() {
        Ok(config) => {
            println!("{}", "Logging Configuration:".bold().green());
            println!(
                "  {}: {}",
                "log-python".cyan(),
                config.log_python.unwrap_or(false)
            );
            println!(
                "  {}: {}",
                "no-stdout".cyan(),
                config.no_stdout.unwrap_or(false)
            );
            println!(
                "  {}: {}",
                "max-size".cyan(),
                format_max_size(config.log_max_size)
            );
            println!("  {}: {}", "path".cyan(), resolve_log_path(&config));
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn handle_log_path(new_path: Option<String>) {
    match Config::load() {
        Ok(mut config) => {
            if let Some(path) = new_path {
                if path.trim().is_empty() {
                    logger::error("Log path cannot be empty.");
                    return;
                }

                config.log_path = Some(path.clone());
                if let Err(e) = config.save() {
                    logger::error(&format!("Failed to save config: {}", e));
                    return;
                }
                logger::success(&format!("Set log path to {}", path));
            }

            println!("{}", resolve_log_path(&config));
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn set_logging_config(setting: LogSetAction) {
    match Config::load() {
        Ok(mut config) => {
            let (key, value_display) = match setting {
                LogSetAction::MaxSize { bytes } => {
                    config.log_max_size = Some(bytes);
                    ("max-size", bytes.to_string())
                }
                LogSetAction::LogPython { enabled } => {
                    config.log_python = Some(enabled);
                    ("log-python", enabled.to_string())
                }
                LogSetAction::NoStdout { enabled } => {
                    config.no_stdout = Some(enabled);
                    ("no-stdout", enabled.to_string())
                }
            };

            if let Err(e) = config.save() {
                logger::error(&format!("Failed to save config: {}", e));
                return;
            }

            logger::success(&format!("Set {} = {}", key, value_display));
            println!(
                "{}",
                "Tip: CLI flags still override these settings for a single run.".dimmed()
            );
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn resolve_log_path(config: &Config) -> String {
    config
        .log_path
        .clone()
        .unwrap_or_else(logger::get_log_path_string)
}

fn format_max_size(max_size: Option<u64>) -> String {
    match max_size {
        Some(bytes) => format!("{} bytes", bytes),
        None => "unlimited".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::log::{handle_log, LogAction, LogSetAction};
    use crate::test_support::with_temp_config;
    use r2x_config::Config;

    #[test]
    fn test_log_set_no_stdout() {
        with_temp_config(|| {
            handle_log(Some(LogAction::Set {
                setting: LogSetAction::NoStdout { enabled: true },
            }));

            let Ok(config) = Config::load() else {
                return;
            };
            assert_eq!(config.no_stdout, Some(true));
        });
    }

    #[test]
    fn test_log_set_size() {
        with_temp_config(|| {
            handle_log(Some(LogAction::Set {
                setting: LogSetAction::MaxSize {
                    bytes: 10 * 1024 * 1024,
                },
            }));

            let Ok(config) = Config::load() else {
                return;
            };
            assert_eq!(config.log_max_size, Some(10 * 1024 * 1024));
        });
    }
}
