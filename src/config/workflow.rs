//! Workflow configuration for r2x pipelines
//!
//! This module defines the structure for YAML workflow files that specify
//! complete data pipelines: read → modify → write with configuration overrides.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete workflow configuration for an r2x pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    /// Workflow metadata
    #[serde(default)]
    pub metadata: WorkflowMetadata,

    /// Input configuration (reader)
    pub input: InputConfig,

    /// System modifiers to apply (optional)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<ModifierConfig>,

    /// Output configuration (writer)
    pub output: OutputConfig,
}

/// Workflow metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowMetadata {
    /// Workflow name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Workflow description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Workflow version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Author
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Input (reader) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    /// Plugin name (e.g., "reeds", "switch", "plexos")
    pub plugin: String,

    /// Input path (file or directory)
    pub path: String,

    /// Plugin-specific configuration overrides
    /// These override the defaults from the plugin's Pydantic config
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config: HashMap<String, serde_json::Value>,
}

/// System modifier configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifierConfig {
    /// Modifier name (e.g., "add_storage", "scale_renewables")
    pub name: String,

    /// Modifier-specific parameters
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub params: HashMap<String, serde_json::Value>,
}

/// Output (writer) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Plugin name (e.g., "plexos", "switch", "reeds")
    pub plugin: String,

    /// Output path (file or directory)
    pub path: String,

    /// Plugin-specific configuration overrides
    /// These override the defaults from the plugin's Pydantic config
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config: HashMap<String, serde_json::Value>,
}

impl WorkflowConfig {
    /// Create a new workflow config with sensible defaults
    pub fn new(input_plugin: &str, output_plugin: &str) -> Self {
        Self {
            metadata: WorkflowMetadata {
                name: Some(format!("{}_to_{}", input_plugin, output_plugin)),
                description: Some(format!(
                    "Convert {} data to {} format",
                    input_plugin, output_plugin
                )),
                version: Some("1.0.0".to_string()),
                author: None,
            },
            input: InputConfig {
                plugin: input_plugin.to_string(),
                path: format!("./input/{}", input_plugin),
                config: HashMap::new(),
            },
            modifiers: Vec::new(),
            output: OutputConfig {
                plugin: output_plugin.to_string(),
                path: format!("./output/{}", output_plugin),
                config: HashMap::new(),
            },
        }
    }

    /// Create an example workflow with modifiers
    pub fn example_with_modifiers(input_plugin: &str, output_plugin: &str) -> Self {
        let mut workflow = Self::new(input_plugin, output_plugin);

        workflow.metadata.description = Some(format!(
            "Example workflow: {} → modifiers → {}",
            input_plugin, output_plugin
        ));

        // Add example input config overrides
        workflow
            .input
            .config
            .insert("weather_year".to_string(), serde_json::json!(2012));

        // Add example modifiers
        workflow.modifiers.push(ModifierConfig {
            name: "add_storage".to_string(),
            params: {
                let mut params = HashMap::new();
                params.insert("capacity_mw".to_string(), serde_json::json!(100.0));
                params.insert("duration_hours".to_string(), serde_json::json!(4.0));
                params
            },
        });

        workflow.modifiers.push(ModifierConfig {
            name: "scale_renewables".to_string(),
            params: {
                let mut params = HashMap::new();
                params.insert("scale_factor".to_string(), serde_json::json!(1.5));
                params
            },
        });

        // Add example output config overrides
        workflow
            .output
            .config
            .insert("version".to_string(), serde_json::json!("8.2"));
        workflow
            .output
            .config
            .insert("include_comments".to_string(), serde_json::json!(true));

        workflow
    }

    /// Validate the workflow configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.input.plugin.is_empty() {
            return Err("Input plugin name cannot be empty".to_string());
        }

        if self.input.path.is_empty() {
            return Err("Input path cannot be empty".to_string());
        }

        if self.output.plugin.is_empty() {
            return Err("Output plugin name cannot be empty".to_string());
        }

        if self.output.path.is_empty() {
            return Err("Output path cannot be empty".to_string());
        }

        for modifier in &self.modifiers {
            if modifier.name.is_empty() {
                return Err("Modifier name cannot be empty".to_string());
            }
        }

        Ok(())
    }

    /// Convert to YAML string
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Load from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Load from YAML file
    pub fn from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config = Self::from_yaml(&content)?;
        config.validate().map_err(|e| e.to_string())?;
        Ok(config)
    }

    /// Save to YAML file
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        self.validate().map_err(|e| e.to_string())?;
        let yaml = self.to_yaml()?;
        std::fs::write(path, yaml)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_new() {
        let workflow = WorkflowConfig::new("reeds", "plexos");
        assert_eq!(workflow.input.plugin, "reeds");
        assert_eq!(workflow.output.plugin, "plexos");
        assert!(workflow.modifiers.is_empty());
    }

    #[test]
    fn test_workflow_validation() {
        let workflow = WorkflowConfig::new("reeds", "plexos");
        assert!(workflow.validate().is_ok());

        let mut invalid = workflow.clone();
        invalid.input.plugin = String::new();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_workflow_yaml_roundtrip() {
        let workflow = WorkflowConfig::example_with_modifiers("reeds", "plexos");
        let yaml = workflow.to_yaml().unwrap();
        let loaded = WorkflowConfig::from_yaml(&yaml).unwrap();
        assert_eq!(workflow.input.plugin, loaded.input.plugin);
        assert_eq!(workflow.output.plugin, loaded.output.plugin);
        assert_eq!(workflow.modifiers.len(), loaded.modifiers.len());
    }
}
