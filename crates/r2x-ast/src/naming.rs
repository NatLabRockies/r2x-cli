//! Naming utilities for converting between different case conventions
//!
//! This module provides functions for converting between CamelCase, snake_case,
//! and kebab-case naming conventions commonly used in plugin discovery.

/// Convert CamelCase class name to kebab-case plugin name
///
/// Preserves suffixes (unlike stripping them):
/// - ReEDSParser -> reeds-parser
/// - MyExporter -> my-exporter
/// - SimplePlugin -> simple-plugin
///
/// Handles acronyms (consecutive uppercase) correctly:
/// - XMLParser -> xml-parser
/// - HTTPClient -> http-client
pub fn camel_to_kebab(class_name: &str) -> String {
    let mut result = String::new();

    for (i, ch) in class_name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            let prev_upper = class_name
                .chars()
                .nth(i - 1)
                .map(|c| c.is_uppercase())
                .unwrap_or(false);
            let next_upper = class_name
                .chars()
                .nth(i + 1)
                .map(|c| c.is_uppercase())
                .unwrap_or(false);
            let next_lower = class_name
                .chars()
                .nth(i + 1)
                .map(|c| c.is_lowercase())
                .unwrap_or(false);

            // Add hyphen when:
            // 1. Starting new word from lowercase, unless entering an acronym
            //    (e.g., "my" -> "P" in "myParser", but NOT "Re" -> "E" in "ReEDS")
            // 2. End of acronym transitioning to new word
            //    (e.g., "S" -> "P" in "ReEDSParser")
            let start_new_word = !prev_upper && !next_upper;
            let end_of_acronym = prev_upper && next_lower;

            if start_new_word || end_of_acronym {
                result.push('-');
            }
        }
        result.push(ch.to_ascii_lowercase());
    }

    result
}

/// Convert snake_case function name to kebab-case plugin name
pub fn snake_to_kebab(func_name: &str) -> String {
    func_name.replace('_', "-")
}

/// Find the position of matching closing paren, handling nested parens/brackets
///
/// Returns the index of the closing paren that matches the implicit opening paren
/// at position -1 (i.e., we start at depth 0 looking for the first ')' at depth 0).
pub fn find_matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::naming::*;

    #[test]
    fn test_camel_to_kebab() {
        // Simple CamelCase
        assert_eq!(camel_to_kebab("MyParser"), "my-parser");
        assert_eq!(camel_to_kebab("SimplePlugin"), "simple-plugin");
        assert_eq!(camel_to_kebab("MyExporter"), "my-exporter");

        // Acronyms (consecutive uppercase)
        assert_eq!(camel_to_kebab("ReEDSParser"), "reeds-parser");
        assert_eq!(camel_to_kebab("XMLParser"), "xml-parser");
        assert_eq!(camel_to_kebab("HTTPClient"), "http-client");

        // Single word
        assert_eq!(camel_to_kebab("Parser"), "parser");
        assert_eq!(camel_to_kebab("Reeds"), "reeds");

        // All uppercase acronym
        assert_eq!(camel_to_kebab("HTTP"), "http");
        assert_eq!(camel_to_kebab("API"), "api");
    }

    #[test]
    fn test_snake_to_kebab() {
        assert_eq!(snake_to_kebab("add_pcm_defaults"), "add-pcm-defaults");
        assert_eq!(snake_to_kebab("break_gens"), "break-gens");
        assert_eq!(snake_to_kebab("simple"), "simple");
        assert_eq!(snake_to_kebab("a_b_c"), "a-b-c");
    }

    #[test]
    fn test_find_matching_paren() {
        // Simple case
        assert_eq!(find_matching_paren("abc)"), Some(3));

        // Nested parens
        assert_eq!(find_matching_paren("(inner))"), Some(7));
        assert_eq!(find_matching_paren("a, b, (c, d))"), Some(12));

        // Mixed brackets
        assert_eq!(find_matching_paren("a: List[int])"), Some(12));
        assert_eq!(find_matching_paren("a: Dict[str, int], b: int)"), Some(25));

        // No match
        assert_eq!(find_matching_paren("no close paren"), None);
    }
}
