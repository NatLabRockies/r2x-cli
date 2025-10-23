use crate::Result;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use serde_json::Value;

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
    Python::with_gil(|py| {
        let r2x_core = py.import_bound("r2x_core")?;
        let plugin_manager = r2x_core.getattr("PluginManager")?.call0()?;

        let config_class = plugin_manager.call_method1("load_config_class", (plugin_name,))?;
        if config_class.is_none() {
            return Ok(vec![]);
        }

        let schema_dict = config_class.call_method0("model_json_schema")?;
        let schema_json: String = py
            .import_bound("json")?
            .call_method1("dumps", (schema_dict,))?
            .extract()?;

        let schema: Value = serde_json::from_str(&schema_json)?;
        parse_schema(&schema)
    })
}

fn parse_schema(schema: &Value) -> Result<Vec<FieldSchema>> {
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

pub fn build_config_dict<'py>(
    py: Python<'py>,
    fields: &[FieldSchema],
    args: &std::collections::HashMap<String, String>,
) -> Result<Bound<'py, PyDict>> {
    let config_dict = PyDict::new_bound(py);

    for field in fields {
        if let Some(value_str) = args.get(&field.name) {
            match &field.field_type {
                FieldType::String => {
                    config_dict.set_item(&field.name, value_str)?;
                }
                FieldType::Integer => {
                    let value: i64 = value_str.parse().map_err(|_| {
                        crate::R2xError::ConfigError(format!(
                            "Invalid integer value for {}: {}",
                            field.name, value_str
                        ))
                    })?;
                    config_dict.set_item(&field.name, value)?;
                }
                FieldType::Float => {
                    let value: f64 = value_str.parse().map_err(|_| {
                        crate::R2xError::ConfigError(format!(
                            "Invalid float value for {}: {}",
                            field.name, value_str
                        ))
                    })?;
                    config_dict.set_item(&field.name, value)?;
                }
                FieldType::Boolean => {
                    let value: bool = value_str.parse().map_err(|_| {
                        crate::R2xError::ConfigError(format!(
                            "Invalid boolean value for {}: {}",
                            field.name, value_str
                        ))
                    })?;
                    config_dict.set_item(&field.name, value)?;
                }
                FieldType::IntegerArray => {
                    let values: Vec<i64> = value_str
                        .split(',')
                        .map(|s| {
                            s.trim().parse().map_err(|_| {
                                crate::R2xError::ConfigError(format!(
                                    "Invalid integer in array for {}: {}",
                                    field.name, s
                                ))
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let py_list = PyList::new_bound(py, values);
                    config_dict.set_item(&field.name, py_list)?;
                }
                FieldType::StringArray => {
                    let values: Vec<&str> = value_str.split(',').map(|s| s.trim()).collect();
                    let py_list = PyList::new_bound(py, values);
                    config_dict.set_item(&field.name, py_list)?;
                }
            }
        }
    }

    Ok(config_dict)
}
