use super::*;

impl PluginExtractor {
    pub(super) fn extract_keyword_arguments_from_text(
        &self,
        call_text: &str,
    ) -> Result<Vec<ConstructorArg>> {
        let mut args = Vec::new();

        if let Some(start) = call_text.find('(') {
            if let Some(end) = call_text.rfind(')') {
                let args_str = &call_text[start + 1..end];

                for arg in args_str.split(',') {
                    let arg = arg.trim();
                    if arg.is_empty() {
                        continue;
                    }

                    if let Some(eq_idx) = arg.find('=') {
                        let key = arg[..eq_idx].trim().to_string();
                        let value_str = arg[eq_idx + 1..].trim();
                        let arg_type = self.infer_argument_type(value_str);
                        let value = if arg_type == "string" {
                            value_str
                                .trim_matches(|c: char| c == '"' || c == '\'')
                                .to_string()
                        } else {
                            value_str.to_string()
                        };

                        args.push(ConstructorArg {
                            name: key.clone(),
                            value: value.clone(),
                            arg_type: arg_type.clone(),
                        });

                        debug!("Extracted arg: {} = {} (type: {})", key, value, arg_type);
                    }
                }
            }
        }

        Ok(args)
    }

    #[allow(dead_code)]
    pub(super) fn extract_keyword_arguments<'r, D: ast_grep_core::Doc>(
        &self,
        call_node: &ast_grep_core::Node<'r, D>,
    ) -> Result<Vec<ConstructorArg>> {
        let mut args = Vec::new();

        for arg_match in call_node.find_all("$_") {
            let arg_text = arg_match.text();
            if arg_text.contains('=') && !arg_text.contains('(') {
                if let Some(eq_idx) = arg_text.find('=') {
                    let param_name = arg_text[..eq_idx].trim().to_string();
                    let param_value = arg_text[eq_idx + 1..].trim().to_string();
                    let arg_type = self.infer_argument_type(&param_value);

                    args.push(ConstructorArg {
                        name: param_name,
                        value: param_value,
                        arg_type,
                    });

                    debug!("Extracted kwarg via ast-grep");
                }
            }
        }

        Ok(args)
    }

    pub(super) fn infer_argument_type(&self, value_str: &str) -> String {
        let value_str = value_str.trim();

        if (value_str.starts_with('"') && value_str.ends_with('"'))
            || (value_str.starts_with('\'') && value_str.ends_with('\''))
        {
            return "string".to_string();
        }

        if value_str.parse::<i64>().is_ok() {
            return "number".to_string();
        }

        if value_str.parse::<f64>().is_ok() {
            return "float".to_string();
        }

        if value_str == "True" || value_str == "False" {
            return "boolean".to_string();
        }

        if value_str.contains('.')
            && value_str
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return "enum_value".to_string();
        }

        if value_str.chars().next().map_or(false, |c| c.is_uppercase()) {
            return "class_reference".to_string();
        }

        if value_str.starts_with('[') || value_str.starts_with('{') {
            return "complex".to_string();
        }

        "identifier".to_string()
    }

    pub(super) fn find_kwarg_value(&self, kwargs: &[ConstructorArg], name: &str) -> Result<String> {
        kwargs
            .iter()
            .find(|arg| arg.name == name)
            .map(|arg| arg.value.clone())
            .ok_or_else(|| anyhow!("Argument '{}' not found", name))
    }
}
