use crate::logger;
use crate::plugins::PluginError;
use std::process::Command;

/// Query package info via a single pip show call.
/// Returns (version, dependencies) tuple.
/// Returns (None, empty_vec) on any error (best-effort, non-fatal).
pub fn get_package_info(
    uv_path: &str,
    python_path: &str,
    package: &str,
) -> Result<(Option<String>, Vec<String>), PluginError> {
    let show_output = Command::new(uv_path)
        .args(["pip", "show", "--python", python_path, package])
        .output()
        .map_err(|e| {
            logger::debug(&format!(
                "Failed to query package info for '{}': {}",
                package, e
            ));
            PluginError::Io(e)
        })?;

    if !show_output.status.success() {
        logger::debug(&format!(
            "pip show failed for package '{}' with status: {}",
            package, show_output.status
        ));
        return Err(PluginError::CommandFailed {
            command: format!("{} pip show {}", uv_path, package),
            status: show_output.status.code(),
        });
    }

    let stdout = String::from_utf8_lossy(&show_output.stdout);
    let mut version = None;
    let mut dependencies = Vec::new();

    for line in stdout.lines() {
        if line.starts_with("Version:") {
            version = Some(line.trim_start_matches("Version:").trim().to_string());
        }
        if line.starts_with("Requires:") {
            let requires_str = line.trim_start_matches("Requires:").trim();
            if !requires_str.is_empty() {
                for dep in requires_str.split(',') {
                    let dep_name = dep.trim();
                    if let Some(pkg_name) = dep_name.split(['>', '<', '=', '!', '~']).next() {
                        let clean_name = pkg_name.trim();
                        if !clean_name.is_empty() {
                            dependencies.push(clean_name.to_string());
                        }
                    }
                }
            }
        }
    }

    logger::debug(&format!(
        "Package '{}': version={:?}, {} dependencies",
        package,
        version,
        dependencies.len()
    ));

    Ok((version, dependencies))
}
