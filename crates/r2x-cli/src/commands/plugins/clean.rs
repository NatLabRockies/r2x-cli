use std::process::Command;

use colored::Colorize;

use crate::logger;
use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;

use super::setup_config;

pub fn clean_manifest(yes: bool, _opts: &GlobalOpts) -> Result<(), String> {
    let mut manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {e}"))?;

    if manifest.is_empty() {
        logger::warn("Manifest is empty.");
        return Ok(());
    }

    let total = manifest.total_plugin_count();
    logger::debug(&format!("Manifest has {total} plugin entries."));

    if !yes {
        println!("To actually clean, run with --yes flag.");
        return Ok(());
    }

    let package_names: Vec<String> = manifest
        .packages
        .iter()
        .map(|p| p.name.to_string())
        .collect();

    let (uv_path, venv_path, _python_path) = setup_config()?;

    for package_name in &package_names {
        uninstall_package(&uv_path, &venv_path, package_name);
    }

    manifest.packages.clear();
    manifest
        .save()
        .map_err(|e| format!("Failed to save manifest: {e}"))?;

    println!("{}", format!("Removed {total} plugin(s)").dimmed());
    Ok(())
}

fn uninstall_package(uv_path: &str, venv_path: &str, package_name: &str) {
    logger::debug(&format!(
        "Running: {uv_path} pip uninstall --python {venv_path} {package_name}"
    ));

    let output = Command::new(uv_path)
        .args(["pip", "uninstall", "--python", venv_path, package_name])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            logger::info(&format!("Uninstalled '{package_name}'"));
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            logger::debug(&format!("Failed to uninstall '{package_name}': {stderr}"));
        }
        Err(e) => {
            logger::warn(&format!(
                "Failed to run uv pip uninstall for '{package_name}': {e}"
            ));
        }
    }
}
