use std::path::PathBuf;
use std::process::Command;

use crate::commands::config::clean_cache_folder;
use crate::plugins::error::PluginError;
use colored::Colorize;
use r2x_logger as logger;

use crate::commands::plugins::context::PluginContext;

pub fn clean_manifest(yes: bool, ctx: &mut PluginContext) -> Result<(), PluginError> {
    if !yes {
        println!("To actually clean, run with --yes flag.");
        return Ok(());
    }

    let manifest = &mut ctx.manifest;

    if manifest.is_empty() {
        let manifest_path = PathBuf::from(ctx.config.get_cache_path()).join("manifest.toml");
        println!(
            "No manifest found at: {}",
            manifest_path.display().to_string().cyan()
        );
        clean_cache_folder();
    } else {
        let total = manifest.total_plugin_count();
        logger::debug(&format!("Manifest has {total} plugin entries."));

        let package_names: Vec<String> = manifest
            .packages
            .iter()
            .map(|p| p.name.to_string())
            .collect();

        for package_name in &package_names {
            uninstall_package(&ctx.uv_path, &ctx.python_path, package_name);
        }

        manifest.clear()?;
        clean_cache_folder();
        println!("Removed {total} plugin(s)");
    }
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
