use crate::schema::{build_config_dict, get_plugin_schema};
use crate::{R2xError, Result};
use clap::Args;
use std::path::PathBuf;
use std::process::Command;
use tracing::info;

#[derive(Args, Debug)]
pub struct ReadArgs {
    /// Model name (e.g., reeds, switch, plexos)
    model: String,

    /// Input folder containing model data
    #[arg(short, long)]
    input: PathBuf,

    /// Output file for system JSON
    /// If not specified, saves to cache: ~/.cache/r2x/systems/<model>_system.json (or ~/Library/Caches on macOS)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Print JSON to stdout instead of saving to file
    #[arg(long, conflicts_with = "output")]
    stdout: bool,

    /// Show model-specific help for the selected model
    #[arg(long = "model-help")]
    model_help: bool,

    /// Model-specific arguments (key=value pairs)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    model_args: Vec<String>,
}

pub fn execute(args: ReadArgs, verbose: u8, quiet: bool) -> Result<()> {
    let uv_python_path = crate::python::venv::get_uv_python_path()?;
    let uv_python_str = uv_python_path.to_str().ok_or(R2xError::ConfigError(
        "Virtual environment path is not valid UTF-8".to_string(),
    ))?;
    let schema = get_plugin_schema(&args.model)?;

    if args.model_help {
        print_model_help(&args.model, &schema);
        return Ok(());
    }

    info!("Reading {} model from: {:?}", args.model, args.input);

    let matches = parse_model_args(&args.model, &schema, &args.model_args)?;

    // Get module file
    let module_name = format!("r2x_{}", args.model);
    let output = Command::new("uv")
        .args([
            "-c",
            &format!("import {}; print({}.__file__)", module_name, module_name),
        ])
        .output()?;

    if !output.status.success() {
        return Err(R2xError::SubprocessError(format!(
            "Failed to get module file for {}: {}",
            module_name,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let module_file = String::from_utf8(output.stdout)
        .map_err(|e| R2xError::ConfigError(format!("Invalid UTF8: {}", e)))?
        .trim()
        .to_string();
    let module_path = PathBuf::from(&module_file);
    let module_dir = module_path.parent().ok_or_else(|| {
        R2xError::ConfigError(format!("Cannot find parent directory of {}", module_file))
    })?;
    let file_mapping_path = module_dir.join("config").join("file_mapping.json");

    info!("Using file mapping: {:?}", file_mapping_path);

    let config_json = build_config_dict(&schema, &matches)?;

    let output_path = if args.stdout {
        PathBuf::new()
    } else if let Some(path) = &args.output {
        path.clone()
    } else {
        // Default to cache directory
        let cache_dir = dirs::cache_dir()
            .ok_or(R2xError::NoCacheDir)?
            .join("r2x")
            .join("systems");

        // Create systems directory if it doesn't exist
        std::fs::create_dir_all(&cache_dir)?;

        cache_dir.join(format!("{}_system.json", args.model))
    };

    // Define loguru code once based on verbosity/quiet settings
    let loguru_code = if quiet || verbose == 0 {
        "\nfrom loguru import logger\nlogger.disable(\"\")\n"
    } else {
        ""
    };

    let script = format!(
        r#"
import sys
import json
from r2x_core import PluginManager, DataStore
import infrasys{}
manager = PluginManager()
parser_class = manager.load_parser("{}")
if parser_class is None:
    sys.exit(1)
config_class = manager.load_config_class("{}")
if config_class is None:
    sys.exit(1)
config_dict = json.loads('{}')
config = config_class(**config_dict)
data_store = DataStore.from_json('{}', '{}')
parser = parser_class(config=config, data_store=data_store)
system = parser.build_system()
if {}:
    print(system.to_json())
else:
    system.to_json('{}')
"#,
        loguru_code,
        args.model,
        args.model,
        config_json.replace("'", "\\'"),
        file_mapping_path.display(),
        args.input.display(),
        if args.stdout { "True" } else { "False" },
        output_path.display()
    );

    let status = Command::new("uv")
        .args(["run", "--python", uv_python_str, "python", "-c", &script])
        .status()?;

    if !status.success() {
        return Err(R2xError::SubprocessError(
            "Failed to run parsing script".to_string(),
        ));
    }

    if !args.stdout {
        println!("✓ System written to: {}", output_path.display());
    }

    Ok(())
}

fn parse_model_args(
    _model: &str,
    schema: &[crate::schema::FieldSchema],
    args: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    use std::collections::HashMap;

    let mut result = HashMap::new();
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        if arg.starts_with("--") {
            let key = arg.trim_start_matches("--").to_string();
            if let Some(value) = iter.next() {
                result.insert(key.replace('-', "_"), value.clone());
            } else {
                return Err(R2xError::ConfigError(format!(
                    "Missing value for argument: {}",
                    arg
                )));
            }
        } else {
            return Err(R2xError::ConfigError(format!(
                "Unexpected argument: {}. Use --<arg-name> <value> format",
                arg
            )));
        }
    }

    for field in schema {
        if field.required && !result.contains_key(&field.name) {
            return Err(R2xError::ConfigError(format!(
                "Required argument missing: --{}",
                field.name.replace('_', "-")
            )));
        }
    }

    Ok(result)
}

fn print_model_help(model: &str, schema: &[crate::schema::FieldSchema]) {
    println!("Model-specific options for {}:\n", model);

    for field in schema {
        let arg_name = field.name.replace('_', "-");
        let required = if field.required { " (required)" } else { "" };
        let default = if let Some(ref def) = field.default {
            format!(" [default: {}]", def)
        } else {
            String::new()
        };

        println!(
            "  --{:<20} {}{}{}",
            arg_name,
            field.description.as_deref().unwrap_or(""),
            required,
            default
        );
    }
}
