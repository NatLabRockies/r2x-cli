use crate::config_manager::Config;
use crate::logger;
use crate::{GlobalOpts, PythonAction, VenvSubcommand};
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

// This function handle the python installation for the user.
// It defaults to create a venv based on the specified python-version on the config
// If there is not python-version, we default to 3.12
pub fn handle_python(action: PythonAction, opts: GlobalOpts) {
    match action {
        PythonAction::Show => {
            handle_python_show(opts);
        }
        PythonAction::Install { version } => {
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
                            logger::capture_output(
                                &format!("uv venv --python {}", version_str),
                                &output,
                            );
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
                                        &format!(
                                            "uv run --python {} python --version",
                                            version_str
                                        ),
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
                            logger::capture_output(
                                &format!("uv venv --python {}", version_str),
                                &output,
                            );
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
        PythonAction::Venv { subcommand, clear } => match subcommand {
            None => {
                // No subcommand: create/recreate venv
                handle_venv_create(clear, opts);
            }
            Some(VenvSubcommand::Path { new_path }) => {
                handle_venv_path(new_path, opts);
            }
            Some(VenvSubcommand::UpdateCore) => {
                handle_update_core(opts);
            }
        },
    }
}

fn handle_venv_create(clear: bool, opts: GlobalOpts) {
    logger::debug(&format!("Handling venv create command (clear: {})", clear));
    match Config::load() {
        Ok(mut config) => {
            // Ensure uv is installed first
            if let Err(e) = config.ensure_uv_path() {
                logger::error(&format!("Failed to setup uv: {}", e));
                return;
            }

            let venv_path = config.get_venv_path();
            let venv_dir = PathBuf::from(&venv_path);
            let mut prompted = false;

            if venv_dir.exists() {
                if !clear {
                    print!(
                        "{} A virtual environment already exists at `{}`. Do you want to replace it? {} ",
                        "?".bold().cyan(),
                        venv_path,
                        "[y/n] ›".dimmed()
                    );
                    io::stdout().flush().unwrap();
                    prompted = true;
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
                    logger::debug("Clearing existing venv...");
                }

                if let Err(e) = fs::remove_dir_all(&venv_dir) {
                    logger::error(&format!("Failed to remove existing venv: {}", e));
                    return;
                }
                logger::debug(&format!("Removed existing venv at {}", venv_path));
            }

            create_venv(&config, opts);

            if prompted {
                println!(
                    "\n{} Use the `{}` flag or set `{}` to skip this prompt",
                    "hint:".dimmed(),
                    "--clear".bold(),
                    "UV_VENV_CLEAR=1".bold()
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

fn handle_update_core(_opts: GlobalOpts) {
    logger::debug("Handling update-core command");
    match Config::load() {
        Ok(mut config) => {
            // Ensure uv is installed first
            if let Err(e) = config.ensure_uv_path() {
                logger::error(&format!("Failed to setup uv: {}", e));
                return;
            }

            let venv_path = config.get_venv_path();
            let venv_dir = PathBuf::from(&venv_path);

            if !venv_dir.exists() {
                logger::error(&format!("Venv does not exist at: {}", venv_path));
                return;
            }

            if !is_valid_venv(&venv_dir) {
                logger::error(&format!("Path is not a valid venv: {}", venv_path));
                return;
            }

            let target_spec = config.get_r2x_core_package_spec();
            let target_version = config.r2x_core_version.as_deref().unwrap_or("0.1.0rc1");

            if let Some(ref uv_path) = config.uv_path {
                let python_path = if cfg!(windows) {
                    PathBuf::from(&venv_path).join("Scripts").join("python.exe")
                } else {
                    PathBuf::from(&venv_path).join("bin").join("python")
                };

                // Check currently installed version
                let show_output = Command::new(uv_path)
                    .args(["pip", "show", "--python", python_path.to_str().unwrap(), "r2x-core"])
                    .output();

                match show_output {
                    Ok(out) if out.status.success() => {
                        let output_str = String::from_utf8_lossy(&out.stdout);
                        let installed_version = output_str
                            .lines()
                            .find(|line| line.starts_with("Version:"))
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or("unknown");

                        logger::debug(&format!(
                            "Installed: r2x-core=={}, Target: {}",
                            installed_version, target_version
                        ));

                        // Compare versions - extract base version without operators
                        let target_base = target_version
                            .trim_start_matches(|c: char| !c.is_ascii_digit())
                            .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
                            .next()
                            .unwrap_or(target_version);

                        if installed_version == target_base || installed_version == target_version {
                            logger::success(&format!(
                                "r2x-core is already at version {}",
                                installed_version
                            ));
                            return;
                        }

                        // Versions differ, install target version
                        logger::step(&format!("Updating r2x-core to: {}", target_spec));

                        let install_output = Command::new(uv_path)
                            .args([
                                "pip",
                                "install",
                                "--python",
                                python_path.to_str().unwrap(),
                                &target_spec,
                            ])
                            .output();

                        match install_output {
                            Ok(out) if out.status.success() => {
                                logger::capture_output(&format!("uv pip install {}", target_spec), &out);
                                logger::success(&format!(
                                    "Successfully updated r2x-core from {} to {}",
                                    installed_version, target_version
                                ));
                            }
                            Ok(out) => {
                                logger::capture_output(&format!("uv pip install {}", target_spec), &out);
                                logger::error(&format!("Failed to update r2x-core to {}", target_spec));
                            }
                            Err(e) => {
                                logger::error(&format!("Failed to execute update command: {}", e));
                            }
                        }
                    }
                    Ok(out) => {
                        logger::capture_output("uv pip show r2x-core", &out);
                        logger::error("r2x-core is not installed in the venv. Install a plugin first to set up r2x-core.");
                    }
                    Err(e) => {
                        logger::error(&format!("Failed to check r2x-core version: {}", e));
                    }
                }
            } else {
                logger::error("uv path not configured");
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
                    "Version mismatch: config has {}, venv has {}. Run 'r2x python venv --clear' to fix.",
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

    fn verbose_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: false,
            verbose: 1,
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
    fn test_python_install_verbose() {
        handle_python(PythonAction::Install { version: None }, verbose_opts());
    }

    #[test]
    fn test_python_venv_create() {
        handle_python(
            PythonAction::Venv {
                subcommand: None,
                clear: false,
            },
            normal_opts(),
        );
    }

    #[test]
    fn test_python_venv_clear() {
        handle_python(
            PythonAction::Venv {
                subcommand: None,
                clear: true,
            },
            normal_opts(),
        );
    }

    #[test]
    fn test_python_venv_path() {
        handle_python(
            PythonAction::Venv {
                subcommand: Some(VenvSubcommand::Path { new_path: None }),
                clear: false,
            },
            verbose_opts(),
        );
    }
}
