//! pyproject.toml entry point parser (PEP 621)
//!
//! Parses entry points from pyproject.toml files for plugin discovery.

use crate::discovery_types::EntryPointInfo;
use crate::entry_points::parser::{is_r2x_section, parse_entry_point_line};
use r2x_logger as logger;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Find pyproject.toml path by searching common locations
///
/// Searches in order:
/// 1. package_path directory
/// 2. Parent of package_path
/// 3. Parent of discovery_root
/// 4. Grandparent of discovery_root
pub fn find_pyproject_toml_path(package_path: &Path, discovery_root: &Path) -> Option<PathBuf> {
    let mut seen = HashSet::new();
    let candidates = [
        Some(package_path),
        package_path.parent(),
        discovery_root.parent(),
        discovery_root.parent().and_then(|p| p.parent()),
    ];

    for candidate in candidates.into_iter().flatten() {
        let candidate = candidate.to_path_buf();
        if !seen.insert(candidate.clone()) {
            continue;
        }

        let pyproject = candidate.join("pyproject.toml");
        if pyproject.exists() {
            return Some(pyproject);
        }
    }

    None
}

/// Parse entry points from pyproject.toml content (PEP 621 format)
///
/// Looks for entry points in `[project.entry-points]` table.
pub fn parse_pyproject_entry_points(content: &str) -> Vec<EntryPointInfo> {
    let mut entries = Vec::new();
    let parsed: toml::Value = match toml::from_str(content) {
        Ok(parsed) => parsed,
        Err(_) => return entries,
    };

    let entry_points = match parsed
        .get("project")
        .and_then(|project| project.get("entry-points"))
        .and_then(|value| value.as_table())
    {
        Some(entry_points) => entry_points,
        None => return entries,
    };

    for (section, values) in entry_points {
        if !is_r2x_section(section) {
            continue;
        }

        let Some(table) = values.as_table() else {
            continue;
        };

        for (name, target) in table {
            let Some(target_str) = target.as_str() else {
                continue;
            };

            let line = format!("{} = {}", name, target_str);
            if let Some(entry) = parse_entry_point_line(&line, section) {
                entries.push(entry);
            }
        }
    }

    if !entries.is_empty() {
        logger::debug(&format!(
            "Parsed {} entry points from pyproject.toml",
            entries.len()
        ));
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pyproject_entry_points() {
        let content = r#"
[project]
name = "r2x-reeds"
version = "0.1.0"

[project.entry-points.r2x_plugin]
reeds = "r2x_reeds:ReEDSParser"

[project.entry-points."r2x.transforms"]
add-pcm-defaults = "r2x_reeds.sysmod:add_pcm_defaults"
"#;
        let entries = parse_pyproject_entry_points(content);
        assert_eq!(entries.len(), 2);

        let plugin_entry = entries.iter().find(|e| e.section == "r2x_plugin");
        assert!(plugin_entry.is_some());
        assert!(plugin_entry.is_some_and(|e| e.name == "reeds" && e.symbol == "ReEDSParser"));

        let transform_entry = entries.iter().find(|e| e.section == "r2x.transforms");
        assert!(transform_entry.is_some());
        assert!(transform_entry.is_some_and(|e| e.name == "add-pcm-defaults"));
    }

    #[test]
    fn test_parse_pyproject_entry_points_no_r2x_sections() {
        let content = r#"
[project]
name = "some-package"

[project.entry-points.console_scripts]
cli = "some_package:main"
"#;
        let entries = parse_pyproject_entry_points(content);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_pyproject_entry_points_invalid_toml() {
        let content = "not valid toml {{{";
        let entries = parse_pyproject_entry_points(content);
        assert!(entries.is_empty());
    }
}
