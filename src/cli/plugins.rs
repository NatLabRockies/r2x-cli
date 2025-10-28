//! Plugin management commands

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
pub struct InstallArgs {
    /// Plugin name (e.g., r2x-switch)
    plugin: String,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
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
    let registry = crate::python::plugin::get_plugin_registry()?;

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
    let python_exe_path = crate::python::venv::get_uv_python_path()?;

    println!("Installing plugins: {}", args.plugin);

    // Build args: pip install {plugin} --python <venv_path>
    let status = std::process::Command::new(&uv_path)
        .args([
            "pip",
            "install",
            &args.plugin,
            "--python",
            &python_exe_path.to_str().unwrap(),
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
    if let Err(e) = crate::python::plugin_cache::invalidate_cache() {
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
    let package_not_found =
        stderr.contains("No packages to uninstall") || stderr.contains("Skipping");

    // Check if first attempt succeeded
    if output.status.success() && !package_not_found {
        println!("✓ Plugin '{}' uninstalled successfully", args.plugin);

        // Update cache by removing plugins from the uninstalled package
        if let Ok(mut registry) = crate::python::plugin::get_plugin_registry() {
            registry
                .parsers
                .retain(|_, info| info.package_name.as_ref() != Some(&args.plugin));
            registry
                .exporters
                .retain(|_, info| info.package_name.as_ref() != Some(&args.plugin));
            registry
                .modifiers
                .retain(|_, info| info.package_name.as_ref() != Some(&args.plugin));
            registry
                .filters
                .retain(|_, info| info.package_name.as_ref() != Some(&args.plugin));
            if let Err(e) = crate::python::plugin_cache::save_cached_plugins(&registry) {
                debug!("Failed to save updated plugin cache: {}", e);
            }
        }
    } else {
        // If first attempt failed, try to find package name from registry
        debug!(
            "Package '{}' not found or uninstall failed, checking plugin registry",
            args.plugin
        );

        // Load plugin registry from cache if available, else discover
        let mut registry = match crate::python::plugin::get_plugin_registry() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("\n⚠ Failed to load plugin registry: {}", e);
                eprintln!(
                    "\n💡 Tip: Use 'r2x plugin list' to refresh cache, or the exact package name"
                );
                return Err(R2xError::VenvError(format!(
                    "Failed to load plugin registry: {}",
                    e
                )));
            }
        };

        if let Some(package_name) = registry.find_package_name(&args.plugin) {
            println!(
                "Found plugin '{}' provided by package '{}'",
                args.plugin, package_name
            );
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
                return Err(R2xError::VenvError(format!(
                    "Package '{}' not found",
                    package_name
                )));
            }

            if !output2.status.success() {
                return Err(R2xError::VenvError(format!(
                    "Package '{}' uninstallation failed",
                    package_name
                )));
            }

            println!("✓ Package '{}' uninstalled successfully", package_name);

            // Update cache by removing plugins from the uninstalled package
            registry
                .parsers
                .retain(|_, info| info.package_name.as_ref() != Some(&package_name));
            registry
                .exporters
                .retain(|_, info| info.package_name.as_ref() != Some(&package_name));
            registry
                .modifiers
                .retain(|_, info| info.package_name.as_ref() != Some(&package_name));
            registry
                .filters
                .retain(|_, info| info.package_name.as_ref() != Some(&package_name));
            if let Err(e) = crate::python::plugin_cache::save_cached_plugins(&registry) {
                debug!("Failed to save updated plugin cache: {}", e);
            }
        } else {
            eprintln!("\n⚠ Plugin '{}' not found in registry", args.plugin);
            eprintln!(
                "\n💡 Tip: Use 'r2x plugin list' to see installed plugins and their package names"
            );
            return Err(R2xError::VenvError(format!(
                "Package '{}' not found",
                args.plugin
            )));
        }
    }

    // Rebuild wrappers for remaining plugins (in background to not slow down uninstall)
    println!("Updating entry point wrappers...");

    // Remove all existing wrappers
    if let Err(e) = crate::entrypoints::remove_all_wrappers() {
        debug!("Failed to remove old wrappers: {}", e);
    }

    // Recreate wrappers for remaining plugins
    match crate::entrypoints::create_all_wrappers() {
        Ok(created) if !created.is_empty() => {
            println!("✓ Created entry points:");
            for name in created {
                println!("  • {}", name);
            }
        }
        Ok(_) => {}
        Err(e) => {
            debug!("Failed to recreate wrappers: {}", e);
        }
    }

    Ok(())
}
