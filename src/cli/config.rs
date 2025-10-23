//! Configuration management commands

use crate::{R2xError, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommands,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show current configuration
    Show,
    /// Get a configuration value
    Get(GetArgs),
    /// Show the path to the config file
    Path,
    /// Initialize a workflow configuration file
    Init(InitArgs),
}

#[derive(Args)]
struct GetArgs {
    /// Configuration key (e.g., python.version)
    key: String,
}

#[derive(Args)]
struct InitArgs {
    /// Input plugin name (e.g., reeds, switch)
    #[arg(short, long)]
    input: String,

    /// Output plugin name (e.g., plexos, switch)
    #[arg(short, long)]
    output: String,

    /// Output file path (default: r2x-workflow.yaml)
    #[arg(short, long, default_value = "r2x-workflow.yaml")]
    file: PathBuf,

    /// Include example modifiers and config overrides
    #[arg(short = 'e', long)]
    with_examples: bool,

    /// Force overwrite if file exists
    #[arg(short, long)]
    force: bool,
}

pub fn execute(args: ConfigArgs) -> Result<()> {
    match args.command {
        ConfigCommands::Show => execute_show(),
        ConfigCommands::Get(args) => execute_get(args),
        ConfigCommands::Path => execute_path(),
        ConfigCommands::Init(args) => execute_init(args),
    }
}

fn get_config_path() -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir().ok_or(R2xError::NoCacheDir)?;
    Ok(cache_dir.join("r2x").join("config.toml"))
}

fn load_config() -> crate::config::Config {
    if let Ok(path) = get_config_path() {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(config) = toml::from_str(&content) {
                    return config;
                }
            }
        }
    }
    crate::config::Config::default()
}

fn execute_show() -> Result<()> {
    let config = load_config();
    let toml = toml::to_string_pretty(&config)
        .map_err(|e| R2xError::ConfigError(format!("Failed to serialize: {}", e)))?;
    println!("{}", toml);
    Ok(())
}

fn execute_get(args: GetArgs) -> Result<()> {
    let config = load_config();

    let parts: Vec<&str> = args.key.split('.').collect();
    let value = match parts.as_slice() {
        ["python", "version"] => config.python.version,
        ["python", "uv_version"] => config
            .python
            .uv_version
            .unwrap_or_else(|| "(not set)".to_string()),
        ["python", "venv_path"] => config
            .python
            .venv_path
            .unwrap_or_else(|| "(not set)".to_string()),
        ["plugins", "auto_update"] => config.plugins.auto_update.to_string(),
        ["plugins", "cache_ttl_hours"] => config.plugins.cache_ttl_hours.to_string(),
        ["logging", "level"] => config.logging.level,
        ["logging", "format"] => config.logging.format,
        _ => {
            return Err(R2xError::ConfigError(format!(
                "Unknown config key: {}",
                args.key
            )))
        }
    };

    println!("{}", value);
    Ok(())
}

fn execute_path() -> Result<()> {
    let config_path = get_config_path()?;
    println!("{}", config_path.display());
    Ok(())
}

fn execute_init(args: InitArgs) -> Result<()> {
    use crate::config::workflow::WorkflowConfig;

    // Check if file exists
    if args.file.exists() && !args.force {
        return Err(R2xError::ConfigError(format!(
            "File '{}' already exists. Use --force to overwrite.",
            args.file.display()
        )));
    }

    // Create workflow config
    let workflow = if args.with_examples {
        WorkflowConfig::example_with_modifiers(&args.input, &args.output)
    } else {
        WorkflowConfig::new(&args.input, &args.output)
    };

    // Validate
    workflow
        .validate()
        .map_err(|e| R2xError::ConfigError(format!("Invalid workflow: {}", e)))?;

    // Save to file
    workflow
        .save_to_file(&args.file)
        .map_err(|e| R2xError::ConfigError(format!("Failed to save workflow: {}", e)))?;

    println!("✓ Created workflow configuration: {}", args.file.display());
    
    if args.with_examples {
        println!("\nThis file includes example modifiers and configuration overrides.");
        println!("Edit the file to customize:");
        println!("  - Input/output paths");
        println!("  - Configuration overrides (override defaults.json from plugins)");
        println!("  - System modifiers");
    } else {
        println!("\nEdit the file to add:");
        println!("  - Configuration overrides under 'input.config' and 'output.config'");
        println!("  - System modifiers under 'modifiers'");
        println!("\nFor an example with modifiers, run:");
        println!("  r2x config init --input {} --output {} --with-examples", args.input, args.output);
    }

    println!("\nTo run the workflow:");
    println!("  r2x run --workflow {}", args.file.display());

    Ok(())
}
