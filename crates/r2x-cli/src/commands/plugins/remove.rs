use super::PluginContext;
use crate::logger;
use crate::plugins::PluginError;
use colored::Colorize;
use std::process::Command;

pub fn remove_plugin(package: &str, ctx: &mut PluginContext) -> Result<(), PluginError> {
    let removed = ctx.manifest.remove_package_with_deps_summary(package);
    let removed_plugin_count: usize = removed.iter().map(|pkg| pkg.plugin_count).sum();
    let orphaned_dependencies: Vec<String> = removed
        .iter()
        .filter(|pkg| pkg.name != package)
        .map(|pkg| pkg.name.clone())
        .collect();

    if removed.is_empty() {
        logger::info(&format!(
            "No plugins found for package '{}' in manifest",
            package
        ));
    } else {
        ctx.manifest.save()?;
    }

    logger::info(&format!("Using venv: {}", ctx.venv_path));

    if is_package_installed(&ctx.uv_path, &ctx.python_path, package)? {
        uninstall_package(&ctx.uv_path, &ctx.python_path, package)?;
    } else {
        logger::warn(&format!("Package '{}' is not installed", package));
    }

    for orphan_pkg in &orphaned_dependencies {
        if is_package_installed(&ctx.uv_path, &ctx.python_path, orphan_pkg)? {
            uninstall_package(&ctx.uv_path, &ctx.python_path, orphan_pkg)?;
        }
    }

    println!(
        "{}",
        format!("Uninstalled {} plugin(s)", removed_plugin_count).dimmed()
    );
    println!(" {} {}", "-".bold().red(), package.bold());

    for dep in &orphaned_dependencies {
        println!(
            " {} {} {}",
            "-".bold().red(),
            dep.bold(),
            "(dependency)".dimmed()
        );
    }

    Ok(())
}

fn is_package_installed(
    uv_path: &str,
    python_path: &str,
    package: &str,
) -> Result<bool, PluginError> {
    let output = Command::new(uv_path)
        .args(["pip", "show", "--python", python_path, package])
        .output()
        .map_err(PluginError::Io)?;
    Ok(output.status.success())
}

fn uninstall_package(uv_path: &str, python_path: &str, package: &str) -> Result<(), PluginError> {
    logger::debug(&format!(
        "Running: {} pip uninstall --python {} {}",
        uv_path, python_path, package
    ));

    let output = Command::new(uv_path)
        .args(["pip", "uninstall", "--python", python_path, package])
        .output()
        .map_err(|e| {
            logger::error(&format!("Failed to run pip uninstall: {}", e));
            PluginError::Io(e)
        })?;

    logger::capture_output(&format!("uv pip uninstall {}", package), &output);

    if !output.status.success() {
        logger::error(&format!("pip uninstall failed for package '{}'", package));
        return Err(PluginError::CommandFailed {
            command: format!("{} pip uninstall {}", uv_path, package),
            status: output.status.code(),
        });
    }

    logger::info(&format!("Package '{}' uninstalled successfully", package));
    Ok(())
}
