use crate::config_manager::Config;
use crate::logger;
use crate::plugin_manifest::PluginManifest;
use crate::GlobalOpts;
use colored::Colorize;
use std::process::Command;

use super::discovery::{discover_and_register_entry_points_with_deps, DiscoveryOptions};
use super::package_spec::{build_package_spec, extract_package_name};

/// Options for git-based package installation
pub struct GitOptions {
    pub host: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub commit: Option<String>,
}

pub fn install_plugin(
    package: &str,
    editable: bool,
    no_cache: bool,
    git_opts: GitOptions,
    _opts: &GlobalOpts,
) -> Result<(), String> {
    logger::debug("Loading configuration for plugin installation");

    let (uv_path, venv_path, python_path) = setup_install_config()?;
    logger::debug(&format!("Using venv: {}", venv_path));

    let total_start = std::time::Instant::now();

    // Build the package specifier for pip install
    let package_spec = build_package_spec(
        package,
        git_opts.host,
        git_opts.branch,
        git_opts.tag,
        git_opts.commit,
    )?;

    // Extract package name for version and dependency query
    let package_name_for_query = extract_package_name(package)?;

    // AUDIT CHECK: Is package already installed and registered?
    // Skip this check if --no-cache is specified
    let check_start = std::time::Instant::now();
    let is_already_installed = if no_cache {
        None
    } else {
        match get_package_info(&uv_path, &python_path, &package_name_for_query) {
            Ok((version, _deps)) => {
                let manifest = PluginManifest::load().unwrap_or_default();
                let has_plugins_in_manifest = manifest.plugins.iter().any(|(_, plugin)| {
                    plugin.package_name.as_deref() == Some(package_name_for_query.as_str())
                });

                if has_plugins_in_manifest {
                    logger::debug(&format!(
                        "Package '{}' already installed and registered (check took {:?})",
                        package_name_for_query,
                        check_start.elapsed()
                    ));
                    Some(version)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    };

    // If already installed, skip pip install and just audit
    if is_already_installed.is_some() {
        let total_elapsed = total_start.elapsed();
        let elapsed_ms = total_elapsed.as_millis();
        println!(
            "{}",
            format!("Audited 1 package in {}ms", elapsed_ms)
                .bold()
                .dimmed()
        );
        return Ok(());
    }

    logger::spinner_start(&format!("Installing: {}", package));
    let start = std::time::Instant::now();
    match run_pip_install(&uv_path, &python_path, &package_spec, editable, no_cache) {
        Ok(_) => {
            let elapsed = start.elapsed();
            logger::debug(&format!("pip install took: {:?}", elapsed));
        }
        Err(e) => {
            logger::spinner_error(&format!("Failed to install: {}", package));
            return Err(e);
        }
    }

    // Get version and dependencies in single pip show call
    let start = std::time::Instant::now();
    let (package_version, dependencies) =
        match get_package_info(&uv_path, &python_path, &package_name_for_query) {
            Ok((version, deps)) => (version, deps),
            Err(e) => {
                logger::debug(&format!("Failed to get package info: {}", e));
                (None, Vec::new())
            }
        };
    logger::debug(&format!("get_package_info took: {:?}", start.elapsed()));

    let start = std::time::Instant::now();
    let entry_count = discover_and_register_entry_points_with_deps(
        &uv_path,
        &python_path,
        DiscoveryOptions {
            package: package.to_string(),
            package_name_full: package_name_for_query.to_string(),
            dependencies,
            package_version: package_version.clone(),
            no_cache,
        },
    )?;
    logger::debug(&format!(
        "discover_and_register_entry_points took: {:?}",
        start.elapsed()
    ));

    logger::spinner_stop();

    let total_elapsed = total_start.elapsed();
    print_summary(
        &package_name_for_query,
        package_version.as_deref().unwrap_or(""),
        entry_count,
        total_elapsed,
    );

    Ok(())
}

fn setup_install_config() -> Result<(String, String, String), String> {
    let mut config = Config::load().map_err(|e| {
        logger::error(&format!("Failed to load config: {}", e));
        format!("Failed to load config: {}", e)
    })?;

    config.ensure_uv_path().map_err(|e| {
        logger::error(&format!("Failed to setup uv: {}", e));
        format!("Failed to setup uv: {}", e)
    })?;
    config.ensure_cache_path().map_err(|e| {
        logger::error(&format!("Failed to setup cache: {}", e));
        format!("Failed to setup cache: {}", e)
    })?;
    config.ensure_venv_path().map_err(|e| {
        logger::error(&format!("Failed to setup venv: {}", e));
        format!("Failed to setup venv: {}", e)
    })?;

    let uv_path = config
        .uv_path
        .as_ref()
        .cloned()
        .ok_or_else(|| "uv path not configured".to_string())?;
    let venv_path = config.get_venv_path();
    let python_path = config.get_venv_python_path();

    Ok((uv_path, venv_path, python_path))
}

/// Run pip install via uv
fn run_pip_install(
    uv_path: &str,
    python_path: &str,
    package: &str,
    editable: bool,
    no_cache: bool,
) -> Result<(), String> {
    let mut install_args: Vec<String> = vec![
        "pip".to_string(),
        "install".to_string(),
        "--python".to_string(),
        python_path.to_string(),
    ];

    if no_cache {
        install_args.push("--no-cache".to_string());
    }

    if editable {
        install_args.push("-e".to_string());
    }

    install_args.push(package.to_string());

    let debug_flags = if editable && no_cache {
        "-e --no-cache"
    } else if editable {
        "-e"
    } else if no_cache {
        "--no-cache"
    } else {
        ""
    };

    logger::debug(&format!(
        "Running: {} pip install {} --python {} {}",
        uv_path, debug_flags, python_path, package
    ));

    let output = Command::new(uv_path)
        .args(&install_args)
        .output()
        .map_err(|e| {
            logger::error(&format!("Failed to run pip install: {}", e));
            format!("Failed to run pip install: {}", e)
        })?;

    logger::capture_output(&format!("uv pip install {}", package), &output);

    if !output.status.success() {
        logger::error(&format!("pip install failed for package '{}'", package));
        return Err(format!("pip install failed for package '{}'", package));
    }

    Ok(())
}

/// Query package info via a single pip show call.
/// Returns (version, dependencies) tuple.
/// Returns (None, empty_vec) on any error (best-effort, non-fatal).
pub fn get_package_info(
    uv_path: &str,
    python_path: &str,
    package: &str,
) -> Result<(Option<String>, Vec<String>), String> {
    let show_output = Command::new(uv_path)
        .args(["pip", "show", "--python", python_path, package])
        .output()
        .map_err(|e| {
            logger::debug(&format!(
                "Failed to query package info for '{}': {}",
                package, e
            ));
            format!("Failed to query package info: {}", e)
        })?;

    if !show_output.status.success() {
        logger::debug(&format!(
            "pip show failed for package '{}' with status: {}",
            package, show_output.status
        ));
        return Err("pip show failed".to_string());
    }

    let stdout = String::from_utf8_lossy(&show_output.stdout);
    let mut version = None;
    let mut dependencies = Vec::new();

    for line in stdout.lines() {
        if line.starts_with("Version:") {
            version = Some(line.trim_start_matches("Version:").trim().to_string());
        }
        if line.starts_with("Requires:") {
            let requires_str = line.trim_start_matches("Requires:").trim();
            if !requires_str.is_empty() {
                for dep in requires_str.split(',') {
                    let dep_name = dep.trim();
                    if let Some(pkg_name) = dep_name.split(['>', '<', '=', '!', '~']).next() {
                        let clean_name = pkg_name.trim();
                        if !clean_name.is_empty() {
                            dependencies.push(clean_name.to_string());
                        }
                    }
                }
            }
        }
    }

    logger::debug(&format!(
        "Package '{}': version={:?}, {} dependencies",
        package,
        version,
        dependencies.len()
    ));

    Ok((version, dependencies))
}

/// Print installation summary
fn print_summary(pkg: &str, version: &str, count: usize, elapsed: std::time::Duration) {
    let elapsed_ms = elapsed.as_millis();
    println!(
        "{}",
        format!("Installed {} entry point(s) in {}ms", count, elapsed_ms).dimmed()
    );
    let disp = if version.is_empty() {
        format!("{}", pkg.bold())
    } else {
        format!("{}=={}", pkg.bold(), version)
    };
    println!(" {} {}", "+".bold().green(), disp);
}

/// Show help for the install command
pub fn show_install_help() -> Result<(), String> {
    println!();
    println!("{}", "Install a plugin package".bold());
    println!();
    println!("{}", "Usage:".bold());
    println!("  r2x install <PLUGIN> [OPTIONS]");
    println!();
    println!("{}", "Arguments:".bold());
    println!("  <PLUGIN>           Package name, local path, or git URL to install");
    println!();
    println!("{}", "Options:".bold());
    println!("  -e, --editable     Install in editable mode (for development)");
    println!("  --no-cache         Skip metadata cache and force rebuild");
    println!("  --host <HOST>      Git host (default: github.com)");
    println!("  --branch <BRANCH>  Install from a git branch");
    println!("  --tag <TAG>        Install from a git tag");
    println!("  --commit <COMMIT>  Install from a git commit hash");
    println!();
    println!("{}", "Examples:".bold());
    println!("  Install from PyPI:");
    println!("    r2x install r2x-reeds");
    println!();
    println!("  Install from local path:");
    println!("    r2x install ./packages/r2x-reeds");
    println!();
    println!("  Install from GitHub (org/repo format):");
    println!("    r2x install NREL/r2x-reeds");
    println!();
    println!("  Install from specific branch:");
    println!("    r2x install NREL/r2x-reeds --branch develop");
    println!();
    println!("  Install from git tag:");
    println!("    r2x install NREL/r2x-reeds --tag v0.1.0");
    println!();
    println!("  Install in editable mode for development:");
    println!("    r2x install -e ./packages/r2x-reeds");
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_setup_install_config() {
        // Test configuration setup
    }

    #[test]
    fn test_get_package_info() {
        // Test package info extraction
    }

    #[test]
    fn test_install_plugin_success() {
        // Test successful plugin installation
    }
}
