use super::setup_config;
use crate::logger;
use crate::plugins::{
    discovery::{discover_and_register_entry_points_with_deps, DiscoveryOptions},
    install::get_package_info,
    package_spec::{build_package_spec, extract_package_name},
};
use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;
use colored::Colorize;
use std::process::Command;

/// Options for git-based package installation
pub struct GitOptions {
    pub host: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub commit: Option<String>,
}

/// Install a plugin package
pub fn install_plugin(
    package: &str,
    editable: bool,
    no_cache: bool,
    git_opts: GitOptions,
    _opts: &GlobalOpts,
) -> Result<(), String> {
    logger::debug("Loading configuration for plugin installation");

    let (uv_path, venv_path, python_path) = setup_config()?;
    logger::debug(&format!("Using venv: {}", venv_path));

    let total_start = std::time::Instant::now();
    let package_spec = build_package_spec(
        package,
        git_opts.host,
        git_opts.branch,
        git_opts.tag,
        git_opts.commit,
    )?;

    let package_name_for_query = extract_package_name(package)?;

    let check_start = std::time::Instant::now();
    let is_already_installed = if no_cache {
        None
    } else {
        match get_package_info(&uv_path, &python_path, &package_name_for_query) {
            Ok((version, _deps)) => {
                let manifest = Manifest::load().unwrap_or_default();
                let has_plugins = manifest
                    .packages
                    .iter()
                    .any(|pkg| pkg.name == package_name_for_query && !pkg.plugins.is_empty());

                if has_plugins {
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

    if is_already_installed.is_some() {
        let elapsed_ms = total_start.elapsed().as_millis();
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
            logger::debug(&format!("pip install took: {:?}", start.elapsed()));
        }
        Err(e) => {
            logger::spinner_error(&format!("Failed to install: {}", package));
            return Err(e);
        }
    }

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

    print_install_summary(
        &package_name_for_query,
        package_version.as_deref().unwrap_or(""),
        entry_count,
        total_start.elapsed(),
    );

    Ok(())
}

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
    println!("  Install from PyPI:\n    r2x install r2x-reeds");
    println!("\n  Install from local path:\n    r2x install ./packages/r2x-reeds");
    println!("\n  Install from GitHub (org/repo format):\n    r2x install NREL/r2x-reeds");
    println!("\n  Install from specific branch:\n    r2x install NREL/r2x-reeds --branch develop");
    println!("\n  Install from git tag:\n    r2x install NREL/r2x-reeds --tag v0.1.0");
    println!(
        "\n  Install in editable mode for development:\n    r2x install -e ./packages/r2x-reeds"
    );
    println!();
    Ok(())
}

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
        "--prerelease=allow".to_string(),
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        logger::error(&format!("pip install failed for package '{}'", package));

        if !stderr.is_empty() {
            logger::error(&format!("STDERR:\n{}", stderr));
            eprintln!("Error details:\n{}", stderr);
        }

        if !stdout.is_empty() {
            logger::debug(&format!("STDOUT:\n{}", stdout));
        }

        return Err(format!(
            "pip install failed for package '{}': {}",
            package,
            stderr.lines().next().unwrap_or("unknown error")
        ));
    }

    Ok(())
}

fn print_install_summary(pkg: &str, version: &str, count: usize, elapsed: std::time::Duration) {
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
