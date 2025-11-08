use crate::config_manager::Config;
use crate::logger;
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
        handle_venv_create(skip_confirmation, opts);
    }
}

/// Install a specific Python version
fn handle_python_install(version: Option<String>, _opts: GlobalOpts) {
    logger::debug("Handling Python install command");
    match Config::load() {
        Ok(mut config) => {
            if let Err(e) = config.ensure_uv_path() {
                logger::error(&format!("Failed to setup uv: {}", e));
                return;
            }

            let uv_path = match config.uv_path {
                Some(ref path) => path.clone(),
                None => {
                    logger::error("uv path not found");
                    return;
                }
            };

            let version_str = version.unwrap_or_else(|| {
                config
                    .python_version
                    .clone()
                    .unwrap_or_else(|| "3.12".to_string())
            });

            config.python_version = Some(version_str.clone());

            let venv_path = PathBuf::from(config.get_venv_path());

            logger::step(&format!(
                "Installing Python {} and creating venv...",
                version_str
            ));
            logger::debug(&format!("Using uv path: {}", uv_path));
            logger::debug(&format!("Venv path: {}", venv_path.display()));

            let venv_output = Command::new(&uv_path)
                .args([
                    "venv",
                    "--clear",
                    "--python",
                    &version_str,
                    venv_path.to_str().unwrap(),
                ])
                .output();

            match venv_output {
                Ok(output) if output.status.success() => {
                    logger::capture_output(&format!("uv venv --python {}", version_str), &output);
                    config.venv_path = Some(venv_path.to_str().unwrap().to_string());

                    if let Err(e) = config.save() {
                        logger::error(&format!("Failed to save config: {}", e));
                        return;
                    }

                    logger::info(&format!(
                        "Configuration saved with Python version {}",
                        version_str
                    ));

                    let python_check = Command::new(&uv_path)
                        .args(["run", "--python", &version_str, "python", "--version"])
                        .output();

                    match python_check {
                        Ok(output) if output.status.success() => {
                            logger::capture_output(
                                &format!("uv run --python {} python --version", version_str),
                                &output,
                            );
                            logger::success(&format!(
                                "Python {} installed and venv created successfully",
                                version_str
                            ));
                        }
                        _ => {
                            logger::warn("Failed to verify Python installation");
                        }
                    }
                }
                Ok(output) => {
                    logger::capture_output(&format!("uv venv --python {}", version_str), &output);
                    logger::error(&format!(
                        "Failed to create virtual environment for Python {}",
                        version_str
                    ));
                }
                Err(e) => {
                    logger::error(&format!("Failed to execute uv command: {}", e));
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
            let venv_path = config.get_venv_path();

            #[cfg(unix)]
            let python_path = format!("{}/bin/python", venv_path);

            #[cfg(windows)]
            let python_path = format!("{}\\Scripts\\python.exe", venv_path);

            println!("{}", python_path);
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn handle_venv_create(skip_confirmation: bool, opts: GlobalOpts) {
    logger::debug(&format!(
        "Handling venv create command (skip_confirmation: {})",
        skip_confirmation
    ));
    match Config::load() {
        Ok(mut config) => {
            // Ensure uv is installed first
            if let Err(e) = config.ensure_uv_path() {
                logger::error(&format!("Failed to setup uv: {}", e));
                return;
            }

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

                if let Err(e) = fs::remove_dir_all(&venv_dir) {
                    logger::error(&format!("Failed to remove existing venv: {}", e));
                    return;
                }
                logger::debug(&format!("Removed existing venv at {}", venv_path));
            }

            create_venv(&config, opts);

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

// Create virtual environment using UV and specified python version
fn create_venv(config: &Config, _opts: GlobalOpts) {
    let venv_path = config.get_venv_path();
    let version = config.python_version.as_deref().unwrap_or("3.12");

    if let Some(ref uv_path) = config.uv_path {
        logger::debug(&format!("Creating virtual environment at: {}", venv_path));
        logger::debug(&format!(
            "Running {} venv {} --python {}",
            uv_path, venv_path, version
        ));

        let output = Command::new(uv_path)
            .args(["venv", &venv_path, "--python", version])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                logger::capture_output("uv venv", &out);
                logger::success(&format!(
                    "Virtual environment created successfully at {}",
                    venv_path
                ));
            }
            Ok(out) => {
                logger::capture_output("uv venv", &out);
                logger::error("Failed to create virtual environment");
            }
            Err(e) => {
                logger::error(&format!("Failed to execute uv command: {}", e));
            }
        }
    } else {
        logger::error("uv path not configured");
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

fn handle_python_show(_opts: GlobalOpts) {
    logger::debug("Handling python show command");
    match Config::load() {
        Ok(config) => {
            let version = config.python_version.as_deref().unwrap_or("not configured");

            let venv_path = config.get_venv_path();
            let venv_exists = PathBuf::from(&venv_path).exists();

            // Try to get the actual Python version from the venv if it exists
            let mut actual_version_str = String::new();
            let mut version_mismatch = false;
            if venv_exists {
                let python_path = if cfg!(windows) {
                    PathBuf::from(&venv_path).join("Scripts").join("python.exe")
                } else {
                    PathBuf::from(&venv_path).join("bin").join("python")
                };

                if python_path.exists() {
                    match Command::new(&python_path).args(["--version"]).output() {
                        Ok(output) if output.status.success() => {
                            let actual_version =
                                String::from_utf8_lossy(&output.stdout).trim().to_string();
                            actual_version_str = actual_version.clone();

                            // Check for version mismatch
                            // Extract version number from "Python X.Y.Z" format
                            if let Some(version_num) = actual_version.split_whitespace().nth(1) {
                                let configured_short =
                                    version.split('.').take(2).collect::<Vec<_>>().join(".");
                                let actual_short =
                                    version_num.split('.').take(2).collect::<Vec<_>>().join(".");
                                if configured_short != actual_short && version != "not configured" {
                                    version_mismatch = true;
                                }
                            }
                        }
                        _ => {
                            logger::debug("Could not determine actual Python version");
                        }
                    }
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
