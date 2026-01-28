#![allow(private_interfaces)]

use crate::extractor::PluginExtractor;
use anyhow::Result;
use ast_grep_core::AstGrep;
use ast_grep_language::Python;
use r2x_logger::debug;

pub(super) struct ParameterEntry {
    pub name: String,
    pub annotation: Option<String>,
    pub default: Option<String>,
    pub is_required: bool,
}

impl PluginExtractor {
    pub(super) fn extract_class_parameters_from_content(
        &self,
        content: &str,
        class_name: &str,
    ) -> Result<Vec<ParameterEntry>> {
        if let Some(init_signature) = Self::find_init_signature(content, class_name) {
            let params = self.parse_parameters_to_entries(&init_signature);
            if !params.is_empty() {
                return Ok(params);
            }
        }

        if let Some(class_signature) = Self::find_class_signature(content, class_name) {
            return Ok(self.parse_parameters_to_entries(&class_signature));
        }

        if Self::class_exists(content, class_name) {
            return Ok(Vec::new());
        }

        Err(anyhow!("Class not found: {}", class_name))
    }

    pub(super) fn extract_function_parameters_from_content(
        &self,
        content: &str,
        function_name: &str,
    ) -> Result<Vec<ParameterEntry>> {
        if let Some(function_signature) = Self::find_function_signature(content, function_name) {
            return Ok(self.parse_parameters_to_entries(&function_signature));
        }

        if Self::function_exists(content, function_name) {
            return Ok(Vec::new());
        }

        Err(anyhow!("Function not found: {}", function_name))
    }

    pub(super) fn extract_function_return_type_from_content(
        &self,
        content: &str,
        function_name: &str,
    ) -> Option<String> {
        let func_text = Self::find_function_signature(content, function_name)?;

        let arrow_pos = func_text.find("->")?;
        let colon_pos = func_text[arrow_pos..].find(':')?;
        Some(
            func_text[arrow_pos + 2..arrow_pos + colon_pos]
                .trim()
                .to_string(),
        )
    }

    fn find_class_signature(content: &str, class_name: &str) -> Option<String> {
        let sg = AstGrep::new(content, Python);
        let root = sg.root();
        // Use ast-grep pattern to match class definitions
        let pattern = format!("class {}($$$BASES)", class_name);
        let mut matches = root.find_all(pattern.as_str());
        matches.next().map(|m| m.text().to_string())
    }

    fn find_function_signature(content: &str, function_name: &str) -> Option<String> {
        let sg = AstGrep::new(content, Python);
        let root = sg.root();

        // Use tree-sitter to find function_definition nodes
        let function_defs = root
            .children()
            .filter(|node| node.kind() == "function_definition");

        for func_node in function_defs {
            // Get the function name from the identifier
            if let Some(name_node) = func_node.field("name") {
                let found_name = name_node.text();
                debug(&format!("Found function in AST: {}", found_name));

                if found_name == function_name {
                    debug(&format!(
                        "Match! Extracting signature for: {}",
                        function_name
                    ));
                    // Get the parameters field from the function_definition
                    if let Some(params_node) = func_node.field("parameters") {
                        let params_text = params_node.text().to_string();
                        debug(&format!(
                            "Parameters text: {}",
                            params_text.chars().take(100).collect::<String>()
                        ));
                        // Build the signature: def function_name(params)
                        let signature = format!(
                            "def {}({})",
                            function_name,
                            params_text.trim_start_matches('(').trim_end_matches(')')
                        );
                        return Some(signature);
                    }
                }
            }
        }

        debug(&format!("Function '{}' not found in AST", function_name));

        None
    }

    fn find_init_signature(content: &str, class_name: &str) -> Option<String> {
        let lines: Vec<&str> = content.lines().collect();
        let mut in_target_class = false;
        let mut class_indent = 0usize;
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];
            let trimmed = line.trim_start();

            if !in_target_class {
                if let Some(rest) = trimmed.strip_prefix("class ") {
                    if let Some(after_name) = rest.strip_prefix(class_name) {
                        if after_name.starts_with('(') || after_name.starts_with(':') {
                            in_target_class = true;
                            class_indent = line.chars().take_while(|c| c.is_whitespace()).count();
                        }
                    }
                }
                idx += 1;
                continue;
            }

            if trimmed.is_empty() {
                idx += 1;
                continue;
            }

            let indent = line.chars().take_while(|c| c.is_whitespace()).count();
            if indent <= class_indent && !trimmed.starts_with('#') {
                break;
            }

            if trimmed.starts_with("def __init__") {
                let mut signature = trimmed.to_string();
                let mut inner_idx = idx + 1;
                while !signature.contains("):")
                    && !signature.contains(")->")
                    && inner_idx < lines.len()
                {
                    let continuation = lines[inner_idx].trim();
                    // Strip comments from the line before adding to signature
                    let continuation_no_comment = if let Some(hash_pos) = continuation.find('#') {
                        continuation[..hash_pos].trim()
                    } else {
                        continuation
                    };
                    if !continuation_no_comment.is_empty() {
                        signature.push(' ');
                        signature.push_str(continuation_no_comment);
                    }
                    inner_idx += 1;
                }
                return Some(signature);
            }

            idx += 1;
        }

        None
    }

    fn class_exists(content: &str, class_name: &str) -> bool {
        content.contains(&format!("class {}", class_name))
    }

    fn function_exists(content: &str, function_name: &str) -> bool {
        content.contains(&format!("def {}", function_name))
    }

    fn parse_parameters_to_entries(&self, func_text: &str) -> Vec<ParameterEntry> {
        let mut parameters = Vec::new();

        let Some(start) = func_text.find('(') else {
            return parameters;
        };
        let Some(end) = func_text[start..].find(')') else {
            return parameters;
        };

        let params_str = &func_text[start + 1..start + end];
        let mut current_param = String::new();
        let mut depth = 0;

        for ch in params_str.chars() {
            match ch {
                '[' | '(' | '{' => {
                    depth += 1;
                    current_param.push(ch);
                }
                ']' | ')' | '}' => {
                    depth -= 1;
                    current_param.push(ch);
                }
                ',' if depth == 0 => {
                    if let Some(entry) = Self::parse_single_parameter_entry(&current_param) {
                        parameters.push(entry);
                    }
                    current_param.clear();
                }
                _ => current_param.push(ch),
            }
        }

        if let Some(entry) = Self::parse_single_parameter_entry(&current_param) {
            parameters.push(entry);
        }

        parameters
    }

    fn parse_single_parameter_entry(raw: &str) -> Option<ParameterEntry> {
        let param_str = raw.trim();

        // Strip inline comments from the parameter string
        let param_str = if let Some(hash_pos) = param_str.find('#') {
            param_str[..hash_pos].trim()
        } else {
            param_str
        };

        if param_str.is_empty()
            || param_str == "self"
            || param_str == "/"
            || param_str.starts_with('*')
            || param_str.starts_with('#')
        {
            return None;
        }

        let (name_part, rest) = if let Some(colon_idx) = param_str.find(':') {
            (
                param_str[..colon_idx].trim(),
                Some(param_str[colon_idx + 1..].trim()),
            )
        } else if let Some(eq_idx) = param_str.find('=') {
            (param_str[..eq_idx].trim(), Some(param_str[eq_idx..].trim()))
        } else {
            (param_str, None)
        };

        let name = name_part.to_string();
        if name.is_empty() {
            return None;
        }

        let (annotation, default, is_required) = match rest {
            Some(rest) => {
                if let Some(eq_idx) = rest.find('=') {
                    let annotation = rest[..eq_idx].trim();
                    let default = rest[eq_idx + 1..].trim();
                    (
                        (!annotation.is_empty()).then(|| annotation.to_string()),
                        (!default.is_empty()).then(|| default.to_string()),
                        false,
                    )
                } else {
                    let annotation = rest.trim();
                    (
                        (!annotation.is_empty()).then(|| annotation.to_string()),
                        None,
                        true,
                    )
                }
            }
            None => {
                if let Some(eq_idx) = param_str.find('=') {
                    let default = param_str[eq_idx + 1..].trim();
                    (
                        None,
                        (!default.is_empty()).then(|| default.to_string()),
                        false,
                    )
                } else {
                    (None, None, true)
                }
            }
        };

        Some(ParameterEntry {
            name,
            annotation,
            default,
            is_required,
        })
    }
}
