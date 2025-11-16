#![allow(private_interfaces)]

use super::*;

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
        if let Some(init_signature) = self.find_init_signature(content, class_name) {
            let params = self.parse_parameters_to_entries(&init_signature);
            if !params.is_empty() {
                return Ok(params);
            }
        }

        if let Some(class_signature) = self.find_class_signature(content, class_name) {
            return Ok(self.parse_parameters_to_entries(&class_signature));
        }

        if self.class_exists(content, class_name) {
            return Ok(Vec::new());
        }

        Err(anyhow!("Class not found: {}", class_name))
    }

    pub(super) fn extract_function_parameters_from_content(
        &self,
        content: &str,
        function_name: &str,
    ) -> Result<Vec<ParameterEntry>> {
        if let Some(function_signature) = self.find_function_signature(content, function_name) {
            return Ok(self.parse_parameters_to_entries(&function_signature));
        }

        if self.function_exists(content, function_name) {
            return Ok(Vec::new());
        }

        Err(anyhow!("Function not found: {}", function_name))
    }

    pub(super) fn extract_function_return_type_from_content(
        &self,
        content: &str,
        function_name: &str,
    ) -> Option<String> {
        let func_text = self.find_function_signature(content, function_name)?;

        let arrow_pos = func_text.find("->")?;
        let colon_pos = func_text[arrow_pos..].find(':')?;
        Some(
            func_text[arrow_pos + 2..arrow_pos + colon_pos]
                .trim()
                .to_string(),
        )
    }

    fn find_class_signature(&self, content: &str, class_name: &str) -> Option<String> {
        let sg = AstGrep::new(content, Python);
        let root = sg.root();
        let pattern = format!("class {}(", class_name);
        let mut matches = root.find_all(pattern.as_str());
        matches.next().map(|m| m.text().to_string())
    }

    fn find_function_signature(&self, content: &str, function_name: &str) -> Option<String> {
        let sg = AstGrep::new(content, Python);
        let root = sg.root();
        let pattern = format!("def {}(", function_name);
        let mut matches = root.find_all(pattern.as_str());
        matches.next().map(|m| m.text().to_string())
    }

    fn find_init_signature(&self, content: &str, class_name: &str) -> Option<String> {
        let lines: Vec<&str> = content.lines().collect();
        let mut in_target_class = false;
        let mut class_indent = 0usize;
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];
            let trimmed = line.trim_start();

            if !in_target_class {
                if let Some(rest) = trimmed.strip_prefix("class ") {
                    if rest.starts_with(class_name) {
                        let after_name = &rest[class_name.len()..];
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
                    signature.push(' ');
                    signature.push_str(continuation);
                    inner_idx += 1;
                }
                return Some(signature);
            }

            idx += 1;
        }

        None
    }

    fn class_exists(&self, content: &str, class_name: &str) -> bool {
        content.contains(&format!("class {}", class_name))
    }

    fn function_exists(&self, content: &str, function_name: &str) -> bool {
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
                    if let Some(entry) = self.parse_single_parameter_entry(&current_param) {
                        parameters.push(entry);
                    }
                    current_param.clear();
                }
                _ => current_param.push(ch),
            }
        }

        if let Some(entry) = self.parse_single_parameter_entry(&current_param) {
            parameters.push(entry);
        }

        parameters
    }

    fn parse_single_parameter_entry(&self, raw: &str) -> Option<ParameterEntry> {
        let param_str = raw.trim();

        if param_str.is_empty()
            || param_str == "self"
            || param_str == "/"
            || param_str.starts_with('*')
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
            Some(rest) if rest.contains('=') => {
                let eq_idx = rest.find('=').unwrap();
                let annotation = rest[..eq_idx].trim();
                let default = rest[eq_idx + 1..].trim();
                (
                    (!annotation.is_empty()).then(|| annotation.to_string()),
                    (!default.is_empty()).then(|| default.to_string()),
                    false,
                )
            }
            Some(rest) => {
                let annotation = rest.trim();
                (
                    (!annotation.is_empty()).then(|| annotation.to_string()),
                    None,
                    true,
                )
            }
            None if param_str.contains('=') => {
                let eq_idx = param_str.find('=').unwrap();
                let default = param_str[eq_idx + 1..].trim();
                (
                    None,
                    (!default.is_empty()).then(|| default.to_string()),
                    false,
                )
            }
            None => (None, None, true),
        };

        Some(ParameterEntry {
            name,
            annotation,
            default,
            is_required,
        })
    }
}
