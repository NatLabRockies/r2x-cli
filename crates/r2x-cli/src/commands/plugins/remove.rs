use crate::commands::plugins::context::PluginContext;
use crate::plugins::error::PluginError;
use crate::uv;
use r2x_logger as logger;
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

    logger::status(&format!(
        "Uninstalled {removed_plugin_count} plugin(s): {package}"
    ));
    for dep in &orphaned_dependencies {
        logger::status(&format!("Uninstalled dependency: {dep}"));
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
    let uninstall_args = vec![
        "pip".to_string(),
        "uninstall".to_string(),
        "--python".to_string(),
        python_path.to_string(),
        package.to_string(),
    ];
    uv::run(uv_path, "Uninstalling", package, uninstall_args)
        .map(|_| ())
        .map_err(PluginError::from)
}
