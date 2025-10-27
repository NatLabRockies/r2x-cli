use crate::{R2xError, Result};
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct FieldSchema {
    pub name: String,
    pub field_type: FieldType,
    pub description: Option<String>,
    pub default: Option<Value>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub enum FieldType {
    String,
    Integer,
    Float,
    Boolean,
    IntegerArray,
    StringArray,
}

impl FieldType {
    fn from_json_type(type_str: &str, items: Option<&Value>) -> Self {
        match type_str {
            "string" => FieldType::String,
            "integer" => FieldType::Integer,
            "number" => FieldType::Float,
            "boolean" => FieldType::Boolean,
            "array" => {
                if let Some(items) = items {
                    if let Some(item_type) = items.get("type").and_then(|t| t.as_str()) {
                        match item_type {
                            "integer" => FieldType::IntegerArray,
                            "string" => FieldType::StringArray,
                            _ => FieldType::StringArray,
                        }
                    } else {
                        FieldType::StringArray
                    }
                } else {
                    FieldType::StringArray
                }
            }
            _ => FieldType::String,
        }
    }
}

pub fn get_plugin_schema(plugin_name: &str) -> Result<Vec<FieldSchema>> {
    println!("Getting plugin schema for {}", plugin_name);
    let uv_path = crate::python::uv::ensure_uv()?;
    let python_path = crate::python::venv::get_uv_python_path()?;

    let python_script = format!(
        r#"
        import json
        import r2x_core

        manager = r2x_core.PluginManager()
        config_class = manager.load_config_class('{}')
        if config_class is None:
            print('{{}}')
        else:
            schema = config_class.model_json_schema()
            print(json.dumps(schema))
        "#,
        plugin_name
    );

    let output = Command::new(&uv_path)
        .args([
            "run",
            "--python",
            &python_path.to_string_lossy(),
            "python",
            &python_script,
        ])
        .output()
        .map_err(|e| R2xError::PythonInit(format!("Failed to run schema retrieval: {}", e)))?;

    if !output.status.success() {
        return Err(R2xError::PythonInit(format!(
            "Schema retrieval failed with status {}",
            output.status
        )));
    }

    let schema_json = String::from_utf8(output.stdout).map_err(|e| {
        R2xError::PythonInit(format!("Failed to parse schema JSON to UTF-8: {}", e))
    })?;

    if schema_json.trim().is_empty() {
        return Err(R2xError::PythonInit("Schema JSON is empty".to_string()));
    }

    let schema: Value = serde_json::from_str(&schema_json)
        .map_err(|e| R2xError::PythonInit(format!("Invalid JSON output: {}", e)))?;

    parse_schema(&schema)
}

fn parse_schema(schema: &Value) -> Result<Vec<FieldSchema>> {
    println!("Parsing schema");
    let mut fields = Vec::new();

    let properties = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .ok_or_else(|| crate::R2xError::ConfigError("No properties in schema".to_string()))?;

    let required_fields: Vec<String> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for (name, prop) in properties {
        let field_type_str = prop
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("string");

        let items = prop.get("items");
        let field_type = FieldType::from_json_type(field_type_str, items);

        let description = prop
            .get("description")
            .and_then(|d| d.as_str())
            .map(String::from);

        let default = prop.get("default").cloned();

        let required = required_fields.contains(name);

        fields.push(FieldSchema {
            name: name.clone(),
            field_type,
            description,
            default,
            required,
        });
    }

    Ok(fields)
}

pub fn build_config_dict(
    fields: &[FieldSchema],
    args: &std::collections::HashMap<String, String>,
) -> Result<String> {
    // Return JSON string instead of PyDict
    use serde_json::json;

    let mut config = serde_json::Map::new();

    for field in fields {
        if let Some(value_str) = args.get(&field.name) {
            match &field.field_type {
                FieldType::String => {
                    config.insert(field.name.clone(), json!(value_str));
                }
                FieldType::Integer => {
                    let value: i64 = value_str.parse().map_err(|_| {
                        R2xError::ConfigError(format!(
                            "Invalid integer value for {}: {}",
                            field.name, value_str
                        ))
                    })?;
                    config.insert(field.name.clone(), json!(value));
                }
                FieldType::Float => {
                    let value: f64 = value_str.parse().map_err(|_| {
                        R2xError::ConfigError(format!(
                            "Invalid float value for {}: {}",
                            field.name, value_str
                        ))
                    })?;
                    config.insert(field.name.clone(), json!(value));
                }
                FieldType::Boolean => {
                    let value: bool = value_str.parse().map_err(|_| {
                        R2xError::ConfigError(format!(
                            "Invalid boolean value for {}: {}",
                            field.name, value_str
                        ))
                    })?;
                    config.insert(field.name.clone(), json!(value));
                }
                FieldType::IntegerArray => {
                    let values: Vec<i64> = value_str
                        .split(',')
                        .map(|s| {
                            s.trim().parse().map_err(|_| {
                                R2xError::ConfigError(format!(
                                    "Invalid integer in array for {}: {}",
                                    field.name, s
                                ))
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    config.insert(field.name.clone(), json!(values));
                }
                FieldType::StringArray => {
                    let values: Vec<String> =
                        value_str.split(',').map(|s| s.trim().to_string()).collect();
                    config.insert(field.name.clone(), json!(values));
                }
            }
        }
    }

    Ok(serde_json::to_string(&config)
        .map_err(|e| R2xError::ConfigError(format!("JSON serialization failed: {}", e)))?)
}
