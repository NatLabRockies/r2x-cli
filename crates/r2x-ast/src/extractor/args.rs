use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KwArgRole {
    Name,
    EntryReference,
    Method,
    Description,
    Config,
    Store,
    IoType,
    Other,
}

impl KwArgRole {
    pub fn from_identifier(name: &str) -> Self {
        match name {
            "name" => KwArgRole::Name,
            "obj" | "callable" | "target" | "function" | "factory" | "entry" => {
                KwArgRole::EntryReference
            }
            "call_method" | "method" | "call_function" => KwArgRole::Method,
            "description" => KwArgRole::Description,
            "config" => KwArgRole::Config,
            "store" => KwArgRole::Store,
            "io_type" => KwArgRole::IoType,
            _ => KwArgRole::Other,
        }
    }
}

pub(super) struct KwArg {
    pub name: String,
    pub value: String,
    #[allow(dead_code)]
    pub arg_type: String,
    pub role: KwArgRole,
}

impl PluginExtractor {
    pub(super) fn extract_keyword_arguments_from_text(
        &self,
        call_text: &str,
    ) -> Result<Vec<KwArg>> {
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

                        args.push(KwArg {
                            name: key.clone(),
                            value: value.clone(),
                            arg_type: arg_type.clone(),
                            role: KwArgRole::from_identifier(&key),
                        });

                        debug!("Extracted arg: {} = {} (type: {})", key, value, arg_type);
                    }
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

    pub(super) fn find_kwarg_value(&self, kwargs: &[KwArg], name: &str) -> Result<String> {
        kwargs
            .iter()
            .find(|arg| arg.name == name)
            .map(|arg| arg.value.clone())
            .ok_or_else(|| anyhow!("Argument '{}' not found", name))
    }
}
