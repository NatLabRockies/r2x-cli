//! Schema extraction from Python config classes using AST analysis
//!
//! This module extracts configuration schemas from Python source code using ast-grep.
//! It supports:
//! - Pydantic Field() constraints (ge, le, gt, lt, min_length, max_length)
//! - Literal type enums
//! - Generic types (List, Optional, Dict)
//! - Default values
//! - Required/optional field detection

use anyhow::{anyhow, Result};
use ast_grep_core::AstGrep;
use ast_grep_language::Python;
use r2x_manifest::{Constraint, DefaultValue, FieldType, NestedInfo, SchemaField, SchemaFields};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;

type ParsedTypeInfo = (
    FieldType,
    Option<Arc<str>>,
    Option<Arc<NestedInfo>>,
    Option<Arc<[Arc<str>]>>,
);

/// Schema extractor for Python config classes
pub struct SchemaExtractor {
    /// Import map for resolving type references
    import_map: HashMap<String, String>,
}

impl SchemaExtractor {
    /// Create a new schema extractor
    pub fn new() -> Self {
        SchemaExtractor {
            import_map: HashMap::new(),
        }
    }

    /// Create a schema extractor with an import map for type resolution
    pub fn with_imports(import_map: HashMap<String, String>) -> Self {
        SchemaExtractor { import_map }
    }

    /// Extract schema from a config class in source code
    ///
    /// Optimized to use a single AST parse - finds the class and extracts
    /// fields in one pass without re-parsing.
    pub fn extract(&self, content: &str, class_name: &str) -> Result<SchemaFields> {
        let mut fields = SchemaFields::default();

        let sg = AstGrep::new(content, Python);
        let root = sg.root();

        // Find the class definition
        let class_pattern = format!("class {}($$$): $$$BODY", class_name);
        let class_matches: Vec<_> = root.find_all(class_pattern.as_str()).collect();

        if class_matches.is_empty() {
            return Err(anyhow!("Class '{}' not found in source", class_name));
        }

        // Extract fields directly from the class match (no re-parsing)
        self.extract_fields_from_class_node(&class_matches[0], &mut fields)?;

        Ok(fields)
    }

    /// Extract fields from class node using the already-parsed AST
    fn extract_fields_from_class_node(
        &self,
        class_match: &ast_grep_core::matcher::NodeMatch<'_, ast_grep_core::source::StrDoc<Python>>,
        fields: &mut SchemaFields,
    ) -> Result<()> {
        // Find annotated assignments within the class match
        // Pattern for annotated assignment: name: Type = value or name: Type
        let pattern = "$NAME: $TYPE";
        let matches: Vec<_> = class_match.find_all(pattern).collect();

        for m in matches {
            let full_text = m.text();

            // Parse name and type from the match text (format: "name: type" or "name: type = value")
            let Some(colon_pos) = full_text.find(':') else {
                continue;
            };

            let name = full_text[..colon_pos].trim().to_string();

            // Skip private/magic attributes and non-identifier names
            if name.starts_with('_')
                || !name
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic())
                    .unwrap_or(false)
            {
                continue;
            }

            // Extract type - everything after the colon until = or end
            let after_colon = &full_text[colon_pos + 1..];
            let type_text = if let Some(eq_pos) = after_colon.rfind(" = ") {
                after_colon[..eq_pos].trim().to_string()
            } else {
                after_colon.trim().to_string()
            };

            // Parse the type annotation
            let (field_type, items, nested, enum_values) = self.parse_type_annotation(&type_text);

            // Check if there's a default value
            let has_default = full_text.contains(" = ") || full_text.contains("default=");

            // Determine if required
            let required =
                !has_default && !type_text.contains("None") && !type_text.starts_with("Optional");

            // Try to extract default value
            let default = if has_default {
                self.extract_default_from_text(&full_text)
            } else {
                None
            };

            // Parse Field() constraints if present
            let constraints = if full_text.contains("Field(") {
                self.parse_field_constraints(&full_text)
            } else {
                SmallVec::new()
            };

            let field = SchemaField {
                field_type,
                required,
                default,
                constraints,
                enum_values,
                items,
                nested,
                properties: None,
            };

            fields.insert(Arc::from(name), field);
        }

        Ok(())
    }

    /// Extract default value from field definition text
    fn extract_default_from_text(&self, text: &str) -> Option<DefaultValue> {
        // Look for = value after the type annotation
        if let Some(eq_pos) = text.rfind(" = ") {
            let value_part = text[eq_pos + 3..].trim();
            // Skip if it's a Field() call - we handle that separately
            if !value_part.starts_with("Field(") {
                return self.parse_literal_value(value_part);
            }
        }

        // Try to extract from Field(default=...)
        if let Some(start) = text.find("default=") {
            let rest = &text[start + 8..];
            // Find the end of the default value
            let mut depth = 0;
            let mut end = 0;
            for (i, ch) in rest.char_indices() {
                match ch {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => {
                        if depth == 0 {
                            end = i;
                            break;
                        }
                        depth -= 1;
                    }
                    ',' if depth == 0 => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            if end > 0 {
                let default_str = rest[..end].trim();
                return self.parse_literal_value(default_str);
            }
        }

        None
    }

    /// Parse a type annotation into field type and metadata
    fn parse_type_annotation(&self, annotation: &str) -> ParsedTypeInfo {
        let annotation = annotation.trim();

        // Handle Annotated[X, Field(...)] - extract the actual type
        if annotation.starts_with("Annotated[") && annotation.ends_with(']') {
            let inner = &annotation[10..annotation.len() - 1];
            // Find first argument (actual type) - handle nested brackets
            if let Some(type_end) = find_first_arg_end(inner) {
                let actual_type = inner[..type_end].trim();
                return self.parse_type_annotation(actual_type);
            }
        }

        // Handle Optional[X]
        if annotation.starts_with("Optional[") && annotation.ends_with(']') {
            let inner = &annotation[9..annotation.len() - 1];
            let (inner_type, items, nested, enum_values) = self.parse_type_annotation(inner);
            return (inner_type, items, nested, enum_values);
        }

        // Handle Literal["a", "b", "c"]
        if annotation.starts_with("Literal[") && annotation.ends_with(']') {
            let inner = &annotation[8..annotation.len() - 1];
            let values: Vec<Arc<str>> = inner
                .split(',')
                .map(|s| Arc::from(s.trim().trim_matches(|c| c == '"' || c == '\'')))
                .collect();
            return (FieldType::Str, None, None, Some(Arc::from(values)));
        }

        // Handle List[X] or list[X]
        if (annotation.starts_with("List[") || annotation.starts_with("list["))
            && annotation.ends_with(']')
        {
            let start = 5;
            let inner = &annotation[start..annotation.len() - 1];
            return (FieldType::Array, Some(Arc::from(inner)), None, None);
        }

        // Handle Dict[K, V] or dict[K, V]
        if (annotation.starts_with("Dict[") || annotation.starts_with("dict["))
            && annotation.ends_with(']')
        {
            return (FieldType::Object, None, None, None);
        }

        // Handle basic types
        let field_type = match annotation.to_lowercase().as_str() {
            "str" | "string" => FieldType::Str,
            "int" | "integer" => FieldType::Int,
            "float" | "double" => FieldType::Float,
            "bool" | "boolean" => FieldType::Bool,
            "datetime" | "date" => FieldType::Datetime,
            "any" => FieldType::Any,
            _ => {
                // Assume it's a nested object type
                let nested_info = NestedInfo {
                    class: Some(Arc::from(annotation)),
                    module: self
                        .import_map
                        .get(annotation)
                        .map(|m| Arc::from(m.as_str())),
                };
                return (FieldType::Object, None, Some(Arc::new(nested_info)), None);
            }
        };

        (field_type, None, None, None)
    }

    /// Parse Field() constraints
    fn parse_field_constraints(&self, field_call: &str) -> SmallVec<[Constraint; 2]> {
        let mut constraints = SmallVec::new();

        // Extract keyword arguments from Field(...)
        if let Some(start) = field_call.find("Field(") {
            let args_start = start + 6;
            if let Some(end) = field_call[args_start..].find(')') {
                let args = &field_call[args_start..args_start + end];

                // Parse each keyword argument
                for arg in args.split(',') {
                    let arg = arg.trim();

                    // ge=X
                    if let Some(value) = arg.strip_prefix("ge=") {
                        if let Ok(v) = value.trim().parse::<f64>() {
                            constraints.push(Constraint::Ge(v));
                        }
                    }
                    // le=X
                    else if let Some(value) = arg.strip_prefix("le=") {
                        if let Ok(v) = value.trim().parse::<f64>() {
                            constraints.push(Constraint::Le(v));
                        }
                    }
                    // gt=X
                    else if let Some(value) = arg.strip_prefix("gt=") {
                        if let Ok(v) = value.trim().parse::<f64>() {
                            constraints.push(Constraint::Gt(v));
                        }
                    }
                    // lt=X
                    else if let Some(value) = arg.strip_prefix("lt=") {
                        if let Ok(v) = value.trim().parse::<f64>() {
                            constraints.push(Constraint::Lt(v));
                        }
                    }
                    // min_length=X
                    else if let Some(value) = arg.strip_prefix("min_length=") {
                        if let Ok(v) = value.trim().parse::<u32>() {
                            constraints.push(Constraint::MinLen(v));
                        }
                    }
                    // max_length=X
                    else if let Some(value) = arg.strip_prefix("max_length=") {
                        if let Ok(v) = value.trim().parse::<u32>() {
                            constraints.push(Constraint::MaxLen(v));
                        }
                    }
                    // multiple_of=X
                    else if let Some(value) = arg.strip_prefix("multiple_of=") {
                        if let Ok(v) = value.trim().parse::<f64>() {
                            constraints.push(Constraint::MultipleOf(v));
                        }
                    }
                    // pattern=X
                    else if let Some(value) = arg.strip_prefix("pattern=") {
                        let pattern = value
                            .trim()
                            .trim_matches(|c| c == '"' || c == '\'')
                            .to_string();
                        constraints.push(Constraint::Pattern(Arc::from(pattern)));
                    }
                }
            }
        }

        constraints
    }

    /// Parse a literal value
    fn parse_literal_value(&self, value: &str) -> Option<DefaultValue> {
        let value = value.trim();

        // Boolean
        if value == "True" || value == "true" {
            return Some(DefaultValue::Bool(true));
        }
        if value == "False" || value == "false" {
            return Some(DefaultValue::Bool(false));
        }

        // None
        if value == "None" {
            return None;
        }

        // String
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            let inner = &value[1..value.len() - 1];
            return Some(DefaultValue::Str(Arc::from(inner)));
        }

        // Empty list
        if value == "[]" {
            return Some(DefaultValue::Array(Arc::from([])));
        }

        // Integer
        if let Ok(i) = value.parse::<i64>() {
            return Some(DefaultValue::Int(i));
        }

        // Float
        if let Ok(f) = value.parse::<f64>() {
            return Some(DefaultValue::Float(f));
        }

        None
    }
}

impl Default for SchemaExtractor {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// HELPER FUNCTIONS FOR TYPE PARSING
// =============================================================================

/// Find the end of the first argument in a comma-separated list, handling nested brackets
fn find_first_arg_end(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    // If no comma found, the whole text is the first argument
    Some(text.len())
}

/// Extract description from Field(description="...") or Field(..., description="...")
pub fn extract_description_from_field(text: &str) -> Option<String> {
    if let Some(start) = text.find("description=") {
        let rest = &text[start + 12..];
        // Find opening quote
        let quote_start = rest.find(|c| ['"', '\''].contains(&c))?;
        let quote_char = rest.chars().nth(quote_start)?;
        let after_quote = &rest[quote_start + 1..];
        // Find closing quote
        let quote_end = after_quote.find(quote_char)?;
        Some(after_quote[..quote_end].to_string())
    } else {
        None
    }
}

/// Parse union types from a type annotation string
/// Handles:
/// - "int | str | None" -> vec!["int", "str", "None"]
/// - "Annotated[int | str, Field(...)]" -> vec!["int", "str"]
/// - "str" -> vec!["str"]
pub fn parse_union_types_from_annotation(annotation: &str) -> Vec<String> {
    let annotation = annotation.trim();

    // Handle Annotated[X, Field(...)] - extract the actual type first
    let actual_type = if annotation.starts_with("Annotated[") && annotation.ends_with(']') {
        let inner = &annotation[10..annotation.len() - 1];
        if let Some(type_end) = find_first_arg_end(inner) {
            inner[..type_end].trim()
        } else {
            annotation
        }
    } else {
        annotation
    };

    // Now parse union types
    if actual_type.contains(" | ") {
        actual_type
            .split(" | ")
            .map(|t| t.trim().to_string())
            .collect()
    } else {
        vec![actual_type.to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_fields() {
        let source = r"
class MyConfig(BaseModel):
    name: str
    count: int = 10
    enabled: bool = True
";

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "MyConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        assert!(!fields.is_empty());

        let name_field = fields.get("name");
        assert!(name_field.is_some_and(|f| f.field_type == FieldType::Str && f.required));

        let count_field = fields.get("count");
        assert!(count_field.is_some_and(|f| f.field_type == FieldType::Int
            && !f.required
            && f.default == Some(DefaultValue::Int(10))));
    }

    #[test]
    fn test_extract_field_constraints() {
        let source = r"
class MyConfig(BaseModel):
    threshold: float = Field(ge=0.0, le=1.0)
    name: str = Field(min_length=1, max_length=50)
";

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "MyConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        let threshold = fields.get("threshold");
        assert!(threshold.is_some_and(|t| t.constraints.len() == 2
            && t.constraints.contains(&Constraint::Ge(0.0))
            && t.constraints.contains(&Constraint::Le(1.0))));

        let name = fields.get("name");
        assert!(name.is_some_and(|n| n.constraints.len() == 2
            && n.constraints.contains(&Constraint::MinLen(1))
            && n.constraints.contains(&Constraint::MaxLen(50))));
    }

    #[test]
    fn test_extract_literal_enum() {
        let source = r#"
class MyConfig(BaseModel):
    mode: Literal["fast", "slow", "balanced"]
"#;

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "MyConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        let mode = fields.get("mode");
        assert!(mode.is_some_and(|m| m.enum_values.as_ref().is_some_and(|values| values.len() == 3
            && values.iter().any(|v| v.as_ref() == "fast")
            && values.iter().any(|v| v.as_ref() == "slow")
            && values.iter().any(|v| v.as_ref() == "balanced"))));
    }

    #[test]
    fn test_extract_list_type() {
        let source = r"
class MyConfig(BaseModel):
    items: List[str] = []
";

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "MyConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        let items = fields.get("items");
        assert!(items.is_some_and(|i| i.field_type == FieldType::Array
            && i.items.as_ref().map(|s| s.as_ref()) == Some("str")));
    }

    #[test]
    fn test_extract_optional_type() {
        let source = r"
class MyConfig(BaseModel):
    optional_value: Optional[int] = None
";

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "MyConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        let optional = fields.get("optional_value");
        assert!(optional.is_some_and(|o| o.field_type == FieldType::Int && !o.required));
    }

    #[test]
    fn test_extract_nested_type() {
        let source = r"
class MyConfig(BaseModel):
    database: DatabaseConfig
";

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "MyConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        let database = fields.get("database");
        assert!(database.is_some_and(|d| d.field_type == FieldType::Object
            && d.nested.as_ref().is_some_and(|n| n.class.as_ref().map(|s| s.as_ref())
                == Some("DatabaseConfig"))));
    }

    #[test]
    fn test_extract_multiline_field_definitions() {
        // Tests the actual pattern from pcm_defaults.py
        let source = r#"
class PCMDefaultsConfig(PluginConfig):
    pcm_defaults_fpath: Path | str | None = Field(
        default=None,
        description="Path for JSON file containing PCM defaults.",
    )
    pcm_defaults_dict: dict[str, dict[str, Any]] | None = Field(
        default=None,
        description="Dictionary of PCM defaults to apply.",
    )
    pcm_defaults_override: bool = Field(
        default=False,
        description="Flag to override existing PCM fields with JSON values.",
    )
"#;

        let extractor = SchemaExtractor::new();
        let fields = extractor.extract(source, "PCMDefaultsConfig");
        assert!(fields.is_ok());
        let fields = fields.unwrap_or_default();

        eprintln!(
            "Extracted fields: {:?}",
            fields.fields.keys().collect::<Vec<_>>()
        );

        assert!(!fields.is_empty(), "Should extract at least one field");
        assert!(
            fields.get("pcm_defaults_fpath").is_some(),
            "Should have pcm_defaults_fpath field"
        );
        assert!(
            fields.get("pcm_defaults_override").is_some(),
            "Should have pcm_defaults_override field"
        );

        let override_field = fields.get("pcm_defaults_override");
        assert!(
            override_field.is_some_and(|f| f.field_type == FieldType::Bool && !f.required),
            "pcm_defaults_override should be bool and not required"
        );
    }
}
