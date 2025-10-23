//! Plugin management commands

use crate::python::plugin_cache::{invalidate_cache, load_cached_plugins, save_cached_plugins};
use crate::{R2xError, Result};
use clap::{Args, Subcommand};
use tracing::debug;

#[derive(Args)]
pub struct PluginsArgs {
    #[command(subcommand)]
    command: PluginsCommand,
}

#[derive(Debug, Subcommand)]
pub enum PluginsCommand {
    /// List all installed plugins
    List,
    /// Install a plugin package
    Install(InstallArgs),
    /// Uninstall a plugin package
    Uninstall(UninstallArgs),
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Plugin name (e.g., r2x-switch)
    plugin: String,
}

#[derive(Debug, Args)]
struct UninstallArgs {
    /// Plugin name to uninstall (e.g., r2x-switch)
    plugin: String,
}

pub fn execute(args: PluginsArgs) -> Result<()> {
    match args.command {
        PluginsCommand::List => execute_list(),
        PluginsCommand::Install(args) => execute_install(args),
        PluginsCommand::Uninstall(args) => execute_uninstall(args),
    }
}

fn execute_list() -> Result<()> {
    // Try to load cached plugin list first
    match load_cached_plugins() {
        Ok(Some(cached)) => {
            debug!(
                "Using cached plugin list (age: {}s)",
                cached.age().as_secs()
            );
            display_plugins(&cached.plugins);
            return Ok(());
        }
        Ok(None) => {
            debug!("Cache miss or expired, discovering plugins via Python");
        }
        Err(e) => {
            debug!(
                "Failed to load cache: {}, discovering plugins via Python",
                e
            );
        }
    }

    // Cache miss or error - discover plugins via Python
    crate::python::init()?;
    let registry = crate::python::plugin::discover_plugins()?;

    // Save to cache for next time
    if let Err(e) = save_cached_plugins(&registry) {
        debug!("Failed to cache plugin list: {}", e);
    }

    display_plugins(&registry);

    Ok(())
}

fn display_plugins(registry: &crate::python::plugin::PluginRegistry) {
    if registry.is_empty() {
        println!("No plugins installed.");
        println!();
        println!("To install plugins, run:");
        println!("  r2x plugin install <plugin-name>");
        println!();
        println!("Example:");
        println!("  r2x plugin install r2x-plexos");
        return;
    }

    println!("Installed plugins:");
    println!();

    if !registry.parsers.is_empty() {
        println!("Parsers (readers):");
        for (name, info) in &registry.parsers {
            if let Some(pkg) = &info.package_name {
                println!("  • {} (from {})", name, pkg);
            } else {
                println!("  • {}", name);
            }
        }
        println!();
    }

    if !registry.exporters.is_empty() {
        println!("Exporters (writers):");
        for (name, info) in &registry.exporters {
            if let Some(pkg) = &info.package_name {
                println!("  • {} (from {})", name, pkg);
            } else {
                println!("  • {}", name);
            }
        }
        println!();
    }

    if !registry.modifiers.is_empty() {
        println!("System Modifiers:");
        for (name, info) in &registry.modifiers {
            if let Some(pkg) = &info.package_name {
                println!("  • {} (from {})", name, pkg);
            } else {
                println!("  • {}", name);
            }
        }
        println!();
    }

    if !registry.filters.is_empty() {
        println!("Filters:");
        for (name, info) in &registry.filters {
            if let Some(pkg) = &info.package_name {
                println!("  • {} (from {})", name, pkg);
            } else {
                println!("  • {}", name);
            }
        }
        println!();
    }
}

fn execute_install(args: InstallArgs) -> Result<()> {
    let uv_path = crate::python::uv::ensure_uv()?;
    let venv_path = crate::python::venv::get_venv_path()?;
    let python_exe = crate::python::venv::get_venv_python(&venv_path)?;

    println!("Installing plugin: {}", args.plugin);

    let status = std::process::Command::new(&uv_path)
        .args([
            "pip",
            "install",
            &args.plugin,
            "--python",
            python_exe.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| R2xError::VenvError(format!("Failed to run UV: {}", e)))?;

    if !status.success() {
        return Err(R2xError::VenvError(format!(
            "Plugin installation failed with exit code: {}",
            status.code().unwrap_or(-1)
        )));
    }

    println!("✓ Plugin '{}' installed successfully", args.plugin);

    // Invalidate cache since we just installed a new plugin
    if let Err(e) = invalidate_cache() {
        debug!("Failed to invalidate plugin cache: {}", e);
    }

    // Create wrappers for all entry points
    println!("Creating entry point wrappers...");
    match crate::entrypoints::create_all_wrappers() {
        Ok(created) if !created.is_empty() => {
            println!("✓ Created entry points:");
            for name in created {
                println!("  • {}", name);
            }
        }
        Ok(_) => {
            debug!("No entry points to create");
        }
        Err(e) => {
            eprintln!("⚠ Warning: Failed to create entry point wrappers: {}", e);
            eprintln!("  You can still use: r2x run <modifier>, r2x read <parser>, etc.");
        }
    }

    Ok(())
}

fn execute_uninstall(args: UninstallArgs) -> Result<()> {
    let uv_path = crate::python::uv::ensure_uv()?;
    let venv_path = crate::python::venv::get_venv_path()?;
    let python_exe = crate::python::venv::get_venv_python(&venv_path)?;

    // First attempt: try with the provided name directly
    println!("Uninstalling plugin: {}", args.plugin);

    let output = std::process::Command::new(&uv_path)
        .args([
            "pip",
            "uninstall",
            &args.plugin,
            "--python",
            python_exe.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| R2xError::VenvError(format!("Failed to run UV: {}", e)))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let package_not_found = stderr.contains("No packages to uninstall") || stderr.contains("Skipping");

    // If first attempt failed, try to find package name from registry
    if package_not_found {
        debug!("Package '{}' not found, checking plugin registry", args.plugin);

        // Discover plugins to get package mapping
        crate::python::init()?;
        if let Ok(registry) = crate::python::plugin::discover_plugins() {
            if let Some(package_name) = registry.find_package_name(&args.plugin) {
                println!("Found plugin '{}' provided by package '{}'", args.plugin, package_name);
                println!("Attempting to uninstall package '{}'...", package_name);

                let output2 = std::process::Command::new(&uv_path)
                    .args([
                        "pip",
                        "uninstall",
                        &package_name,
                        "--python",
                        python_exe.to_str().unwrap(),
                    ])
                    .output()
                    .map_err(|e| R2xError::VenvError(format!("Failed to run UV: {}", e)))?;

                let stderr2 = String::from_utf8_lossy(&output2.stderr);
                if stderr2.contains("No packages to uninstall") || stderr2.contains("Skipping") {
                    eprintln!("\n⚠ Package '{}' not found", package_name);
                    return Err(R2xError::VenvError(format!("Package '{}' not found", package_name)));
                }

                if !output2.status.success() {
                    return Err(R2xError::VenvError(format!(
                        "Package '{}' uninstallation failed",
                        package_name
                    )));
                }

                println!("✓ Package '{}' uninstalled successfully", package_name);
                // Continue to wrapper cleanup
            } else {
                eprintln!("\n⚠ Plugin '{}' not found in registry", args.plugin);
                eprintln!("\n💡 Tip: Use 'r2x plugin list' to see installed plugins and their package names");
                return Err(R2xError::VenvError(format!("Plugin '{}' not found", args.plugin)));
            }
        } else {
            eprintln!("\n⚠ Package '{}' not found", args.plugin);
            eprintln!("\n💡 Tip: Use the exact package name shown in 'r2x plugin list'");
            return Err(R2xError::VenvError(format!("Package '{}' not found", args.plugin)));
        }
    } else if !output.status.success() {
        return Err(R2xError::VenvError(format!(
            "Plugin '{}' uninstallation failed",
            args.plugin
        )));
    } else {
        println!("✓ Plugin '{}' uninstalled successfully", args.plugin);
    }

    // Invalidate cache since we just removed a plugin
    if let Err(e) = invalidate_cache() {
        debug!("Failed to invalidate plugin cache: {}", e);
    }

    // Rebuild wrappers for remaining plugins (in background to not slow down uninstall)
    println!("Updating entry point wrappers...");

    // Remove old wrappers and recreate for remaining plugins
    // This requires Python init but we do it after reporting success to the user
    if let Err(e) = crate::entrypoints::remove_all_wrappers() {
        debug!("Failed to remove old wrappers: {}", e);
    }

    if let Err(e) = crate::entrypoints::create_all_wrappers() {
        debug!("Failed to recreate wrappers: {}", e);
    }

    Ok(())
}
