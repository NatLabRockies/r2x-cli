use std::path::PathBuf;

use crate::commands::config::clean_cache_folder;
use crate::plugins::error::PluginError;
use colored::Colorize;
use r2x_logger as logger;

use crate::commands::plugins::context::PluginContext;
use crate::uv;

pub fn clean_manifest(yes: bool, ctx: &mut PluginContext) -> Result<(), PluginError> {
    if !yes {
        let total = ctx.manifest.total_plugin_count();
        if total > 0 {
            println!(
                "This will remove {} plugin(s) and clear the cache folder.",
                total
            );
        } else {
            println!("This will clear the cache folder.");
        }
        println!("Run with {} to confirm.", "--yes".bold().cyan());
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
    let uninstall_args = vec![
        "pip".to_string(),
        "uninstall".to_string(),
        "--python".to_string(),
        python_path.to_string(),
        package_name.to_string(),
    ];

    match uv::run(uv_path, "Uninstalling", package_name, uninstall_args) {
        Ok(_) => logger::status(&format!("Uninstalled '{package_name}'")),
        Err(error) => logger::warn(&error.to_string()),
    }
}
