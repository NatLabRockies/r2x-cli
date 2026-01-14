use super::setup_config;
use crate::logger;
use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;
use colored::Colorize;
use std::process::Command;

pub fn remove_plugin(package: &str, _opts: &GlobalOpts) -> Result<(), String> {
    let mut removed_count = 0usize;
    let mut orphaned_dependencies = Vec::new();

    match Manifest::load() {
        Ok(mut manifest) => {
            orphaned_dependencies = find_orphaned_dependencies(&manifest, package);
            removed_count = manifest.remove_plugins_by_package(package);
            manifest.remove_decorator_registrations(package);

            if removed_count > 0 {
                // Remove the package entirely from the manifest
                manifest.remove_package(package);

                for dep in &orphaned_dependencies {
                    let count = manifest.remove_plugins_by_package(dep);
                    manifest.remove_decorator_registrations(dep);
                    if count > 0 {
                        logger::info(&format!("Removing orphaned dependency package '{}'", dep));
                        manifest.remove_package(dep);
                        removed_count += count;
                    }
                }

                if let Err(e) = manifest.save() {
                    logger::warn(&format!("Failed to update manifest: {}", e));
                }
            } else {
                logger::info(&format!(
                    "No plugins found for package '{}' in manifest",
                    package
                ));
            }
        }
        Err(e) => {
            logger::warn(&format!(
                "Failed to load manifest: {}. Continuing with uninstall...",
                e
            ));
        }
    }

    let (uv_path, venv_path, _python_path) = setup_config()?;
    logger::info(&format!("Using venv: {}", venv_path));

    let check_output = Command::new(&uv_path)
        .args(["pip", "show", "--python", &venv_path, package])
        .output()
        .map_err(|e| format!("Failed to check package status: {}", e))?;

    if !check_output.status.success() {
        logger::warn(&format!("Package '{}' is not installed", package));
        return Ok(());
    }

    logger::debug(&format!(
        "Running: {} pip uninstall --python {} {}",
        uv_path, venv_path, package
    ));

    let output = Command::new(&uv_path)
        .args(["pip", "uninstall", "--python", &venv_path, package])
        .output()
        .map_err(|e| {
            logger::error(&format!("Failed to run pip uninstall: {}", e));
            format!("Failed to run pip uninstall: {}", e)
        })?;

    logger::capture_output(&format!("uv pip uninstall {}", package), &output);

    if !output.status.success() {
        logger::error(&format!("pip uninstall failed for package '{}'", package));
        return Err(format!("pip uninstall failed for package '{}'", package));
    }

    logger::info(&format!("Package '{}' uninstalled successfully", package));

    for orphan_pkg in &orphaned_dependencies {
        let check_orphan = Command::new(&uv_path)
            .args(["pip", "show", "--python", &venv_path, orphan_pkg])
            .output()
            .map_err(|e| {
                format!(
                    "Failed to check orphaned package '{}' status: {}",
                    orphan_pkg, e
                )
            })?;

        if check_orphan.status.success() {
            logger::debug(&format!(
                "Running: {} pip uninstall --python {} {}",
                uv_path, venv_path, orphan_pkg
            ));

            let orphan_output = Command::new(&uv_path)
                .args(["pip", "uninstall", "--python", &venv_path, orphan_pkg])
                .output()
                .map_err(|e| {
                    logger::error(&format!(
                        "Failed to run pip uninstall for orphaned package '{}': {}",
                        orphan_pkg, e
                    ));
                    format!(
                        "Failed to run pip uninstall for orphaned package '{}': {}",
                        orphan_pkg, e
                    )
                })?;

            logger::capture_output(
                &format!("uv pip uninstall {} (orphaned dependency)", orphan_pkg),
                &orphan_output,
            );

            if orphan_output.status.success() {
                logger::info(&format!(
                    "Orphaned dependency package '{}' uninstalled successfully",
                    orphan_pkg
                ));
            } else {
                logger::warn(&format!(
                    "Failed to uninstall orphaned dependency package '{}'",
                    orphan_pkg
                ));
            }
        }
    }

    println!(
        "{}",
        format!("Uninstalled {} plugins(s)", removed_count).dimmed()
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

fn find_orphaned_dependencies(_manifest: &Manifest, _package: &str) -> Vec<String> {
    Vec::new()
}
