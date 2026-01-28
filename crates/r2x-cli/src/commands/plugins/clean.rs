use std::process::Command;

use colored::Colorize;

use crate::logger;
use crate::plugins::PluginError;

use super::PluginContext;

pub fn clean_manifest(yes: bool, ctx: &mut PluginContext) -> Result<(), PluginError> {
    let manifest = &mut ctx.manifest;

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

    for package_name in &package_names {
        uninstall_package(&ctx.uv_path, &ctx.python_path, package_name);
    }

    manifest.clear()?;

    println!("{}", format!("Removed {total} plugin(s)").dimmed());
    Ok(())
}

fn uninstall_package(uv_path: &str, python_path: &str, package_name: &str) {
    logger::debug(&format!(
        "Running: {uv_path} pip uninstall --python {python_path} {package_name}"
    ));

    let output = Command::new(uv_path)
        .args(["pip", "uninstall", "--python", python_path, package_name])
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
