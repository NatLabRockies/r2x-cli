use crate::errors::PipelineError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Pipeline configuration from YAML
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PipelineConfig {
    /// Variables for substitution (${var} and $(var) syntax)
    #[serde(default)]
    pub variables: HashMap<String, serde_yaml::Value>,

    /// Named pipelines (each is a list of plugin names)
    #[serde(default)]
    pub pipelines: HashMap<String, Vec<String>>,

    /// Output folder for pipeline results
    #[serde(default)]
    pub output_folder: Option<String>,

    /// Plugin configuration (keyed by plugin name)
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

impl PipelineConfig {
    /// Load pipeline configuration from YAML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, PipelineError> {
        let path_ref = path.as_ref();
        let content = match fs::read_to_string(path_ref) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if let Some(fallback) = Self::resolve_fallback_path(path_ref) {
                    fs::read_to_string(fallback)?
                } else {
                    return Err(PipelineError::Io(err));
                }
            }
            Err(err) => return Err(PipelineError::Io(err)),
        };
        let config: PipelineConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// List all available pipeline names
    pub fn list_pipelines(&self) -> Vec<String> {
        let mut names: Vec<String> = self.pipelines.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get a specific pipeline by name
    pub fn get_pipeline(&self, name: &str) -> Option<&Vec<String>> {
        self.pipelines.get(name)
    }

    /// Substitute variables in a string (supports ${var} and $(var) syntax)
    pub fn substitute_string(&self, input: &str) -> Result<String, PipelineError> {
        let mut result = input.to_string();

        // Handle ${var} syntax
        while let Some(start) = result.find("${") {
            if let Some(end) = result[start..].find('}') {
                let var_name = &result[start + 2..start + end];
                let value = self.get_variable_string(var_name)?;
                result.replace_range(start..=(start + end), &value);
            } else {
                return Err(PipelineError::InvalidConfig(
                    "Unclosed variable substitution ${".to_string(),
                ));
            }
        }

        // Handle $(var) syntax
        while let Some(start) = result.find("$(") {
            if let Some(end) = result[start..].find(')') {
                let var_name = &result[start + 2..start + end];
                let value = self.get_variable_string(var_name)?;
                result.replace_range(start..=(start + end), &value);
            } else {
                return Err(PipelineError::InvalidConfig(
                    "Unclosed variable substitution $(".to_string(),
                ));
            }
        }

        Ok(result)
    }

    /// Get a variable value as a string
    fn get_variable_string(&self, name: &str) -> Result<String, PipelineError> {
        let value = self
            .variables
            .get(name)
            .ok_or_else(|| PipelineError::VariableNotFound(name.to_string()))?;

        match value {
            serde_yaml::Value::String(s) => Ok(s.clone()),
            serde_yaml::Value::Number(n) => Ok(n.to_string()),
            serde_yaml::Value::Bool(b) => Ok(b.to_string()),
            serde_yaml::Value::Null => Ok("null".to_string()),
            _ => Err(PipelineError::InvalidConfig(format!(
                "Variable '{}' has complex type that cannot be substituted as string",
                name
            ))),
        }
    }

    /// Substitute variables in a YAML value recursively
    pub fn substitute_value(
        &self,
        value: &serde_yaml::Value,
    ) -> Result<serde_yaml::Value, PipelineError> {
        match value {
            serde_yaml::Value::String(s) => {
                let substituted = self.substitute_string(s)?;
                Ok(serde_yaml::Value::String(substituted))
            }
            serde_yaml::Value::Mapping(map) => {
                let mut new_map = serde_yaml::Mapping::new();
                for (k, v) in map {
                    let new_key = self.substitute_value(k)?;
                    let new_value = self.substitute_value(v)?;
                    new_map.insert(new_key, new_value);
                }
                Ok(serde_yaml::Value::Mapping(new_map))
            }
            serde_yaml::Value::Sequence(seq) => {
                let mut new_seq = Vec::new();
                for item in seq {
                    new_seq.push(self.substitute_value(item)?);
                }
                Ok(serde_yaml::Value::Sequence(new_seq))
            }
            // Numbers, booleans, null don't need substitution
            _ => Ok(value.clone()),
        }
    }

    /// Get plugin configuration with variable substitution
    pub fn get_plugin_config(&self, plugin_name: &str) -> Result<serde_yaml::Value, PipelineError> {
        let config = self.config.get(plugin_name).ok_or_else(|| {
            PipelineError::InvalidConfig(format!(
                "No configuration found for plugin '{}'",
                plugin_name
            ))
        })?;

        self.substitute_value(config)
    }

    /// Get plugin configuration as JSON string (for Python bridge)
    pub fn get_plugin_config_json(&self, plugin_name: &str) -> Result<String, PipelineError> {
        let config = self.get_plugin_config(plugin_name)?;
        serde_json::to_string(&config).map_err(|e| {
            PipelineError::InvalidConfig(format!("Failed to serialize config to JSON: {}", e))
        })
    }

    /// Get all plugin configurations with variable substitution
    pub fn get_all_configs(&self) -> Result<HashMap<String, serde_yaml::Value>, PipelineError> {
        let mut configs = HashMap::new();
        for (name, config) in &self.config {
            configs.insert(name.clone(), self.substitute_value(config)?);
        }
        Ok(configs)
    }

    /// Resolve and print the configuration for a specific pipeline
    pub fn print_pipeline_config(&self, pipeline_name: &str) -> Result<String, PipelineError> {
        let pipeline = self
            .get_pipeline(pipeline_name)
            .ok_or_else(|| PipelineError::PipelineNotFound(pipeline_name.to_string()))?;

        let mut output = String::new();
        output.push_str(&format!("Pipeline: {}\n", pipeline_name));
        output.push_str(&format!("Steps: {:?}\n\n", pipeline));

        output.push_str("Variables:\n");
        for (key, value) in &self.variables {
            output.push_str(&format!("  {}: {:?}\n", key, value));
        }

        output.push_str("\nResolved Configuration:\n");
        for plugin_name in pipeline {
            if let Ok(config) = self.get_plugin_config(plugin_name) {
                output.push_str(&format!("\n{}:\n", plugin_name));
                let yaml_str = serde_yaml::to_string(&config).unwrap_or_else(|_| "{}".to_string());
                for line in yaml_str.lines() {
                    output.push_str(&format!("  {}\n", line));
                }
            }
        }

        if let Some(output_folder) = &self.output_folder {
            let resolved = self.substitute_string(output_folder)?;
            output.push_str(&format!("\nOutput Folder: {}\n", resolved));
        }

        Ok(output)
    }

    fn resolve_fallback_path(original: &Path) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if original.extension().is_none() {
            candidates.push(original.with_extension("yaml"));
            candidates.push(original.with_extension("yml"));
        }

        candidates.into_iter().find(|candidate| candidate.exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_variable_substitution_dollar_brace() {
        let mut vars = HashMap::new();
        vars.insert("year".to_string(), serde_yaml::Value::Number(2032.into()));
        vars.insert(
            "scenario".to_string(),
            serde_yaml::Value::String("test".to_string()),
        );

        let config = PipelineConfig {
            variables: vars,
            pipelines: HashMap::new(),
            output_folder: None,
            config: HashMap::new(),
        };

        let result = config.substitute_string("Year is ${year}");
        assert!(result.is_ok_and(|r| r == "Year is 2032"));

        let result = config.substitute_string("Scenario: ${scenario}, Year: ${year}");
        assert!(result.is_ok_and(|r| r == "Scenario: test, Year: 2032"));
    }

    #[test]
    fn test_variable_substitution_dollar_paren() {
        let mut vars = HashMap::new();
        vars.insert("year".to_string(), serde_yaml::Value::Number(2032.into()));

        let config = PipelineConfig {
            variables: vars,
            pipelines: HashMap::new(),
            output_folder: None,
            config: HashMap::new(),
        };

        let result = config.substitute_string("Year is $(year)");
        assert!(result.is_ok_and(|r| r == "Year is 2032"));
    }

    #[test]
    fn test_variable_not_found() {
        let config = PipelineConfig {
            variables: HashMap::new(),
            pipelines: HashMap::new(),
            output_folder: None,
            config: HashMap::new(),
        };

        let result = config.substitute_string("Year is ${year}");
        assert!(result.is_err());
        assert!(matches!(result, Err(PipelineError::VariableNotFound(_))));
    }

    #[test]
    fn test_substitute_yaml_value() {
        let mut vars = HashMap::new();
        vars.insert("year".to_string(), serde_yaml::Value::Number(2032.into()));
        vars.insert(
            "folder".to_string(),
            serde_yaml::Value::String("/data".to_string()),
        );

        let config = PipelineConfig {
            variables: vars,
            pipelines: HashMap::new(),
            output_folder: None,
            config: HashMap::new(),
        };

        let input = serde_yaml::Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String("solve_year".to_string()),
                serde_yaml::Value::String("${year}".to_string()),
            );
            map.insert(
                serde_yaml::Value::String("folder_path".to_string()),
                serde_yaml::Value::String("${folder}/inputs".to_string()),
            );
            map
        });

        let result = config.substitute_value(&input);
        assert!(result.is_ok());
        let Ok(serde_yaml::Value::Mapping(map)) = result else {
            assert!(false, "Expected mapping");
            return;
        };
        let year = map.get(serde_yaml::Value::String("solve_year".to_string()));
        assert!(year.is_some_and(|y| y == &serde_yaml::Value::String("2032".to_string())));

        let folder = map.get(serde_yaml::Value::String("folder_path".to_string()));
        assert!(folder.is_some_and(|f| f == &serde_yaml::Value::String("/data/inputs".to_string())));
    }

    #[test]
    fn test_load_with_fallback_extension() {
        let Ok(dir) = TempDir::new() else {
            return;
        };
        let yaml_path = dir.path().join("sample-pipeline.yaml");
        if fs::write(
            &yaml_path,
            r#"
pipelines:
  demo: ["step"]
"#,
        )
        .is_err()
        {
            return;
        }

        let config = PipelineConfig::load(dir.path().join("sample-pipeline"));
        assert!(config.is_ok_and(|c| c.get_pipeline("demo").is_some()));
    }
}
