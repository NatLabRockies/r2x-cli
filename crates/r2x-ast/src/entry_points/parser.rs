//! Entry points.txt parser
//!
//! Parses entry_points.txt files from Python packages to discover r2x plugins.

use crate::discovery_types::EntryPointInfo;
use r2x_logger as logger;

/// Parse all r2x-related entry points from entry_points.txt content
///
/// This function scans for all sections that are r2x-related:
/// - `[r2x_plugin]` - main plugin registration
/// - `[r2x.*]` - any section starting with "r2x." (e.g., r2x.transforms, r2x.parsers)
///
/// Returns a vector of EntryPointInfo for all discovered entry points.
pub fn parse_all_entry_points(content: &str) -> Vec<EntryPointInfo> {
    let mut entries = Vec::new();
    let mut current_section: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Check for section header
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            // Check if this section is r2x-related
            if is_r2x_section(section) {
                current_section = Some(section.to_string());
            } else {
                current_section = None;
            }
            continue;
        }

        // Parse entry point within a relevant section
        if let Some(ref section) = current_section {
            if let Some(entry) = parse_entry_point_line(line, section) {
                entries.push(entry);
            }
        }
    }

    logger::debug(&format!(
        "Parsed {} entry points from entry_points.txt",
        entries.len()
    ));
    entries
}

/// Check if a section name is r2x-related
pub fn is_r2x_section(section: &str) -> bool {
    section == "r2x_plugin" || section.starts_with("r2x.")
}

/// Parse a single entry point line in the format: name = module:symbol
pub fn parse_entry_point_line(line: &str, section: &str) -> Option<EntryPointInfo> {
    let eq_idx = line.find('=')?;
    let name = line[..eq_idx].trim();
    let value = line[eq_idx + 1..].trim();

    // Handle quoted values (e.g., name = "module:symbol")
    let value = value.trim_matches('"').trim_matches('\'');

    let colon_idx = value.find(':')?;
    let module = value[..colon_idx].trim();
    let symbol = value[colon_idx + 1..].trim();

    if name.is_empty() || module.is_empty() || symbol.is_empty() {
        return None;
    }

    Some(EntryPointInfo {
        name: name.to_string(),
        module: module.to_string(),
        symbol: symbol.to_string(),
        section: section.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use crate::entry_points::parser::*;

    #[test]
    fn test_parse_all_entry_points_r2x_plugin() {
        let content = r"[r2x_plugin]
reeds = r2x_reeds:ReEDSParser
";
        let entries = parse_all_entry_points(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "reeds");
        assert_eq!(entries[0].module, "r2x_reeds");
        assert_eq!(entries[0].symbol, "ReEDSParser");
        assert_eq!(entries[0].section, "r2x_plugin");
    }

    #[test]
    fn test_parse_all_entry_points_multiple_sections() {
        let content = r"[r2x_plugin]
reeds = r2x_reeds:ReEDSParser

[r2x.transforms]
add-pcm-defaults = r2x_reeds.sysmod.pcm_defaults:add_pcm_defaults
add-emission-cap = r2x_reeds.sysmod.emission_cap:add_emission_cap

[console_scripts]
some-cli = some_module:main
";
        let entries = parse_all_entry_points(content);
        assert_eq!(entries.len(), 3);

        // Check r2x_plugin entry
        assert_eq!(entries[0].name, "reeds");
        assert_eq!(entries[0].section, "r2x_plugin");

        // Check r2x.transforms entries
        assert_eq!(entries[1].name, "add-pcm-defaults");
        assert_eq!(entries[1].section, "r2x.transforms");
        assert_eq!(entries[1].module, "r2x_reeds.sysmod.pcm_defaults");
        assert_eq!(entries[1].symbol, "add_pcm_defaults");

        assert_eq!(entries[2].name, "add-emission-cap");
        assert_eq!(entries[2].section, "r2x.transforms");
    }

    #[test]
    fn test_parse_all_entry_points_ignores_non_r2x_sections() {
        let content = r"[console_scripts]
some-cli = some_module:main

[gui_scripts]
some-gui = some_module:gui_main
";
        let entries = parse_all_entry_points(content);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_is_r2x_section() {
        assert!(is_r2x_section("r2x_plugin"));
        assert!(is_r2x_section("r2x.transforms"));
        assert!(is_r2x_section("r2x.parsers"));
        assert!(is_r2x_section("r2x.exporters"));
        assert!(!is_r2x_section("console_scripts"));
        assert!(!is_r2x_section("gui_scripts"));
    }

    #[test]
    fn test_parse_entry_point_line() {
        let entry = parse_entry_point_line("reeds = r2x_reeds:ReEDSParser", "r2x_plugin");
        assert!(entry.is_some());
        let entry = entry.unwrap_or_else(|| unreachable!());
        assert_eq!(entry.name, "reeds");
        assert_eq!(entry.module, "r2x_reeds");
        assert_eq!(entry.symbol, "ReEDSParser");

        // Test with quoted value
        let entry_quoted =
            parse_entry_point_line("reeds = \"r2x_reeds:ReEDSParser\"", "r2x_plugin");
        assert!(entry_quoted.is_some());
        assert!(entry_quoted.is_some_and(|e| e.symbol == "ReEDSParser"));

        // Test invalid lines
        assert!(parse_entry_point_line("no equals sign", "r2x_plugin").is_none());
        assert!(parse_entry_point_line("name = no_colon", "r2x_plugin").is_none());
    }
}
