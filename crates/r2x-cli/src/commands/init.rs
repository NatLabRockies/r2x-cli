use crate::logger;
use crate::GlobalOpts;
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

const DEFAULT_FILENAME: &str = "pipeline.yaml";

const PIPELINE_TEMPLATE: &str = r#"# R2X Pipeline Configuration
# This file defines pipelines for processing power system data

# Variables for substitution (use ${var} or $(var) syntax)
variables:
  input_file: "input.json"
  output_dir: "output"
  model_name: "example_model"

# Named pipelines - each is a sequence of plugins to execute
pipelines:
  # Simple pipeline with two steps
  simple:
    - plugin-1
    - plugin-2

  # Multi-step pipeline with data flow
  # Each plugin consumes stdout from previous and produces stdout for next
  process:
    - reader
    - transformer
    - writer

  # Example: data processing pipeline
  analyze:
    - load-data
    - validate
    - analyze
    - export

# Plugin-specific configuration
# Configuration is passed to each plugin when it runs
config:
  # Reader plugin configuration
  reader:
    input: ${input_file}
    format: json

  # Transformer plugin configuration
  transformer:
    normalize: true
    validate: true

  # Writer plugin configuration
  writer:
    output: ${output_dir}/${model_name}.json
    format: json

  # Example data processing plugins
  load-data:
    source: ${input_file}
    strict: true

  validate:
    check_required: true
    check_types: true

  analyze:
    method: detailed
    include_stats: true

  export:
    destination: ${output_dir}/results.json
    compress: false

# Optional: default output folder for pipeline results
output_folder: ${output_dir}
"#;

/// Initialize a new pipeline file
pub fn handle_init(filename: Option<String>, _opts: GlobalOpts) {
    logger::debug("Handling init command");

    let target_filename = filename.unwrap_or_else(|| DEFAULT_FILENAME.to_string());
    let target_path = Path::new(&target_filename);

    logger::debug(&format!("Target file: {}", target_filename));

    // Check if file exists
    if target_path.exists() {
        // Check for skip confirmation flag
        let should_skip = std::env::var("R2X_INIT_YES").is_ok();

        if !should_skip {
            print!(
                "{} File '{}' already exists. Overwrite? {} ",
                "?".bold().cyan(),
                target_filename,
                "[y/n] ›".dimmed()
            );
            let _ = io::stdout().flush();

            let mut response = String::new();
            if io::stdin().read_line(&mut response).is_ok() {
                let response = response.trim().to_lowercase();
                if response != "y" && response != "yes" {
                    logger::info("Operation cancelled by user");
                    println!("Operation cancelled.");
                    return;
                }
            } else {
                logger::error("Failed to read input");
                return;
            }
        } else {
            logger::debug("Skipping confirmation (R2X_INIT_YES set)");
        }
    }

    // Write the pipeline template
    match fs::write(&target_filename, PIPELINE_TEMPLATE) {
        Ok(_) => {
            logger::success(&format!("Created pipeline file: {}", target_filename));
            println!();
            println!("{}  Pipeline file created successfully!", "✔".green());
            println!();
            println!("Next steps:");
            println!(
                "  1. Edit {} with your pipeline configuration",
                target_filename.bold()
            );
            println!("  2. Install plugins: r2x install <package>");
            println!(
                "  3. List available pipelines: r2x run {} --list",
                target_filename
            );
            println!(
                "  4. Run a pipeline: r2x run {} <pipeline-name>",
                target_filename
            );
            println!(
                "  5. Preview pipeline: r2x run {} <pipeline-name> --dry-run",
                target_filename
            );
        }
        Err(e) => {
            logger::error(&format!("Failed to create pipeline file: {}", e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_filename() {
        assert_eq!(DEFAULT_FILENAME, "pipeline.yaml");
    }

    #[test]
    fn test_template_contains_variables() {
        assert!(PIPELINE_TEMPLATE.contains("variables:"));
    }

    #[test]
    fn test_template_contains_pipelines() {
        assert!(PIPELINE_TEMPLATE.contains("pipelines:"));
    }

    #[test]
    fn test_template_contains_config() {
        assert!(PIPELINE_TEMPLATE.contains("config:"));
    }
}
