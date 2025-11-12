use crate::logger;
use crate::r2x_manifest::Manifest;
use crate::GlobalOpts;
use colored::Colorize;

pub fn clean_manifest(yes: bool, _opts: &GlobalOpts) -> Result<(), String> {
    let mut manifest = Manifest::load().map_err(|e| format!("Failed to load manifest: {}", e))?;

    if manifest.is_empty() {
        logger::warn("Manifest is empty.");
        return Ok(());
    }

    let total = manifest.total_plugin_count();
    logger::debug(&format!("Manifest has {} plugin entries.", total));

    if !yes {
        println!("To actually clean, run with --yes flag.");
        return Ok(());
    }

    manifest.packages.clear();
    manifest
        .save()
        .map_err(|e| format!("Failed to save manifest: {}", e))?;

    println!("{}", format!("Removed {} plugin(s)", total).dimmed());
    Ok(())
}
