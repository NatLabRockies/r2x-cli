use super::PluginContext;
use crate::logger;
use crate::plugins::{
    discovery::{discover_and_register_entry_points_with_deps, DiscoveryOptions},
    install::get_package_info,
    package_spec::{build_package_spec, extract_package_name},
    PluginError,
};
use colored::Colorize;
use r2x_manifest::PackageDiscoverer;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

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
    ctx: &mut PluginContext,
) -> Result<(), PluginError> {
    logger::debug("Loading configuration for plugin installation");

    let total_start = std::time::Instant::now();
    let package_spec = build_package_spec(
        package,
        git_opts.host.clone(),
        git_opts.branch.clone(),
        git_opts.tag.clone(),
        git_opts.commit.clone(),
    )?;

    // Check if this is a workspace installation
    if is_workspace_package(&package_spec)? {
        logger::info("Detected workspace repository, installing all members...");
        // Just install the workspace - uv will handle all members
        run_pip_install(
            &ctx.uv_path,
            &ctx.python_path,
            &package_spec,
            editable,
            no_cache,
        )?;

        // Now discover all packages with entry points (like sync command)
        logger::info("Discovering plugins from installed packages...");
        return discover_all_installed_packages(ctx, no_cache, total_start);
    }

    let package_name_for_query = extract_package_name(package)?;

    let check_start = std::time::Instant::now();
    let is_already_installed = if no_cache {
        None
    } else {
        match get_package_info(&ctx.uv_path, &ctx.python_path, &package_name_for_query) {
            Ok((version, _deps)) => {
                let has_plugins = ctx.manifest.packages.iter().any(|pkg| {
                    pkg.name.as_ref() == package_name_for_query && !pkg.plugins.is_empty()
                });

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

    // Print status without spinner since we need interactive terminal for SSH prompts
    logger::info(&format!("Installing: {}", package));
    let start = std::time::Instant::now();
    match run_pip_install(
        &ctx.uv_path,
        &ctx.python_path,
        &package_spec,
        editable,
        no_cache,
    ) {
        Ok(_) => {
            logger::debug(&format!("pip install took: {:?}", start.elapsed()));
        }
        Err(e) => {
            logger::error(&format!("Failed to install: {}", package));
            return Err(e);
        }
    }

    let start = std::time::Instant::now();
    let (package_version, dependencies) =
        match get_package_info(&ctx.uv_path, &ctx.python_path, &package_name_for_query) {
            Ok((version, deps)) => (version, deps),
            Err(e) => {
                logger::debug(&format!("Failed to get package info: {}", e));
                (None, Vec::new())
            }
        };
    logger::debug(&format!("get_package_info took: {:?}", start.elapsed()));

    // Resolve source path for editable installs
    let source_path = if editable {
        // If it's a local path, canonicalize it
        if Path::new(package).exists() {
            fs::canonicalize(package)
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
        } else {
            None
        }
    } else {
        None
    };

    let start = std::time::Instant::now();
    let entry_count = discover_and_register_entry_points_with_deps(
        &ctx.locator,
        Some(&ctx.venv_path),
        &mut ctx.manifest,
        DiscoveryOptions {
            package: package.to_string(),
            package_name_full: package_name_for_query.to_string(),
            dependencies,
            package_version: package_version.clone(),
            no_cache,
            editable,
            source_path,
        },
    )?;
    logger::debug(&format!(
        "discover_and_register_entry_points took: {:?}",
        start.elapsed()
    ));

    print_install_summary(
        &package_name_for_query,
        package_version.as_deref().unwrap_or(""),
        entry_count,
        total_start.elapsed(),
    );

    Ok(())
}

pub fn show_install_help() -> Result<(), PluginError> {
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
    println!("\n  Install workspace (all packages in monorepo):\n    r2x install https://github.com/NREL/R2X --branch v2.0.0");
    println!("\n  Install local workspace:\n    r2x install ./R2X");
    println!();
    println!("{}", "Workspace Installation:".bold());
    println!("  When installing from a repository with [tool.uv.workspace] in its");
    println!("  pyproject.toml, r2x will automatically detect and install all workspace");
    println!("  members (e.g., packages in packages/*), registering their entry points.");
    println!();
    Ok(())
}

fn run_pip_install(
    uv_path: &str,
    python_path: &str,
    package: &str,
    editable: bool,
    no_cache: bool,
) -> Result<(), PluginError> {
    let mut install_args: Vec<String> = vec![
        "pip".to_string(),
        "install".to_string(),
        "--python".to_string(),
        python_path.to_string(),
        "--prerelease=allow".to_string(),
        "--no-progress".to_string(),
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

    // Use inherited stdio to allow interactive prompts (e.g., SSH key passphrases)
    let status = Command::new(uv_path)
        .args(&install_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| {
            logger::error(&format!("Failed to run pip install: {}", e));
            PluginError::Io(e)
        })?;

    if !status.success() {
        logger::error(&format!("pip install failed for package '{}'", package));
        return Err(PluginError::CommandFailed {
            command: format!("{} pip install {}", uv_path, package),
            status: status.code(),
        });
    }

    Ok(())
}

fn print_install_summary(pkg: &str, version: &str, count: usize, elapsed: std::time::Duration) {
    let elapsed_ms = elapsed.as_millis();
    logger::debug(&format!(
        "Installed {} entry point(s) in {}ms",
        count, elapsed_ms
    ));
    let disp = if version.is_empty() {
        format!("{}", pkg.bold())
    } else {
        format!("{}=={}", pkg.bold(), version)
    };
    println!(" {} {}", "+".bold().green(), disp);
}

/// Check if a package is a workspace (by detecting [tool.uv.workspace] in pyproject.toml)
fn is_workspace_package(package_spec: &str) -> Result<bool, PluginError> {
    // Only check for local paths or git URLs
    let is_local_path = package_spec.starts_with("./")
        || package_spec.starts_with("../")
        || package_spec.starts_with('/');

    let is_git_url = package_spec.starts_with("git+") || package_spec.starts_with("git@");

    if !is_local_path && !is_git_url {
        return Ok(false);
    }

    // For local paths, check directly
    if is_local_path {
        let pyproject_path = Path::new(package_spec).join("pyproject.toml");
        if !pyproject_path.exists() {
            return Ok(false);
        }

        let content = fs::read_to_string(&pyproject_path).map_err(|e| {
            PluginError::PackageSpec(format!("Failed to read pyproject.toml: {}", e))
        })?;

        return Ok(content.contains("[tool.uv.workspace]"));
    }

    // For git URLs, use heuristic: if it's a git URL pointing to NREL/R2X, assume it's a workspace
    if is_git_url && (package_spec.contains("NREL/R2X") || package_spec.contains("NREL/r2x")) {
        return Ok(true);
    }

    Ok(false)
}

/// Discover all installed packages with r2x_plugin entry points
fn discover_all_installed_packages(
    ctx: &mut PluginContext,
    no_cache: bool,
    total_start: std::time::Instant,
) -> Result<(), PluginError> {
    let discoverer =
        PackageDiscoverer::new(ctx.locator.site_packages().to_path_buf()).map_err(|e| {
            PluginError::Discovery(format!("Failed to initialize package discovery: {e}"))
        })?;
    let packages = discoverer
        .discover_packages()
        .map_err(|e| PluginError::Discovery(format!("Failed to discover packages: {e}")))?;

    if packages.is_empty() {
        logger::warn("No packages with r2x_plugin entry points found");
        return Ok(());
    }

    logger::info(&format!(
        "Found {} package(s) with r2x_plugin entry points",
        packages.len()
    ));

    let mut discovered_count = 0;
    let mut total_entry_points = 0;

    for package in packages {
        let package_name = package.name.clone();
        logger::debug(&format!("Checking for plugins in: {}", package_name));

        // Get package info
        let (package_version, dependencies) =
            match get_package_info(&ctx.uv_path, &ctx.python_path, &package_name) {
                Ok((version, deps)) => (version, deps),
                Err(_) => continue,
            };

        let source_path = package
            .resolved_source_path
            .as_ref()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .or_else(|| {
                if package.is_editable {
                    Some(package.location.to_string_lossy().to_string())
                } else {
                    None
                }
            });

        // Try to discover entry points
        match discover_and_register_entry_points_with_deps(
            &ctx.locator,
            Some(&ctx.venv_path),
            &mut ctx.manifest,
            DiscoveryOptions {
                package: package_name.clone(),
                package_name_full: package_name.clone(),
                dependencies,
                package_version: package_version.clone(),
                no_cache,
                editable: package.is_editable,
                source_path,
            },
        ) {
            Ok(entry_count) => {
                if entry_count > 0 {
                    let version_str = package_version.as_deref().unwrap_or("");
                    let disp = if version_str.is_empty() {
                        format!("{}", package_name.bold())
                    } else {
                        format!("{}=={}", package_name.bold(), version_str)
                    };
                    println!(" {} {}", "+".bold().green(), disp);
                    discovered_count += 1;
                    total_entry_points += entry_count;
                }
            }
            Err(_) => {
                // Not every package has r2x_plugin entry points, skip silently
            }
        }
    }

    let elapsed_ms = total_start.elapsed().as_millis();
    println!(
        "{}",
        format!(
            "Discovered {} package(s) with {} plugin(s) in {}ms",
            discovered_count, total_entry_points, elapsed_ms
        )
        .bold()
        .dimmed()
    );

    Ok(())
}
