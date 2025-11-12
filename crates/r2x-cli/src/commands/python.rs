use crate::config_manager::Config;
use crate::logger;
use crate::python_bridge::configure_python_venv;
use crate::GlobalOpts;
use clap::Subcommand;
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    /// Get or set the venv path
    Path {
        /// Optional new venv path to set
        new_path: Option<String>,
    },
}

/// Handle Python version management
pub fn handle_python(action: PythonAction, opts: GlobalOpts) {
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
pub fn handle_venv(action: Option<VenvAction>, skip_confirmation: bool, opts: GlobalOpts) {
    if let Some(action) = action {
        // Handle subcommands
        match action {
            VenvAction::Path { new_path } => {
                handle_venv_path(new_path, opts);
            }
        }
    } else {
        // No subcommand: create/recreate venv
        handle_venv_create(skip_confirmation);
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
                Ok(python_path) => {
                    logger::info(&format!(
                        "Configuration saved with Python version {}",
                        version_str
                    ));
                    if let Some(actual_version) = verify_python_version(&python_path) {
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
                Ok(python_path) => {
                    logger::success(&format!(
                        "Virtual environment ready at {} (python {})",
                        venv_path,
                        python_path.display()
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
                    "Version mismatch: config has {}, venv has {}. Run 'r2x venv --yes' to recreate.",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn normal_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: false,
            verbose: 0,
            log_python: false,
        }
    }

    #[test]
    fn test_python_install() {
        handle_python(
            PythonAction::Install {
                version: Some("3.12".to_string()),
            },
            normal_opts(),
        );
    }

    #[test]
    fn test_python_install_no_version() {
        handle_python(PythonAction::Install { version: None }, normal_opts());
    }

    #[test]
    fn test_python_path() {
        handle_python(PythonAction::Path, normal_opts());
    }

    #[test]
    fn test_python_show() {
        handle_python(PythonAction::Show, normal_opts());
    }

    #[test]
    fn test_venv_create() {
        handle_venv(None, false, normal_opts());
    }

    #[test]
    fn test_venv_create_skip_confirm() {
        handle_venv(None, true, normal_opts());
    }

    #[test]
    fn test_venv_path() {
        handle_venv(
            Some(VenvAction::Path { new_path: None }),
            false,
            normal_opts(),
        );
    }
}
