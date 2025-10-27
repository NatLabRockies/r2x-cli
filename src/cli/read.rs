use crate::schema::{build_config_dict, get_plugin_schema};
use crate::{R2xError, Result};
use clap::Args;
use pyo3::prelude::*;
use std::path::PathBuf;
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
    //crate::python::init()?;

    let schema = get_plugin_schema(&args.model)?;

    if args.model_help {
        print_model_help(&args.model, &schema);
        return Ok(());
    }

    info!("Reading {} model from: {:?}", args.model, args.input);

    let matches = parse_model_args(&args.model, &schema, &args.model_args)?;

    Python::with_gil(|py| -> Result<()> {
        if quiet || verbose == 0 {
            let logger = py.import_bound("loguru")?.getattr("logger")?;
            logger.call_method1("disable", ("",))?;
        }

        let r2x_core = py.import_bound("r2x_core")?;
        let plugin_manager_class = r2x_core.getattr("PluginManager")?;
        let manager = plugin_manager_class.call0()?;

        let parser_class = manager.call_method1("load_parser", (&args.model,))?;
        if parser_class.is_none() {
            return Err(R2xError::PluginNotFound(args.model.clone()));
        }

        let config_class = manager.call_method1("load_config_class", (&args.model,))?;
        if config_class.is_none() {
            return Err(R2xError::PluginNotFound(args.model.clone()));
        }

        let config_dict = build_config_dict(py, &schema, &matches)?;
        let config = config_class.call((), Some(&config_dict))?;

        let data_store_class = r2x_core.getattr("DataStore")?;
        let plugin_module_name = format!("r2x_{}", &args.model);
        let plugin_module = py.import_bound(plugin_module_name.as_str())?;
        let module_file = plugin_module.getattr("__file__")?.extract::<String>()?;

        let module_path = PathBuf::from(&module_file);
        let module_dir = module_path.parent().ok_or_else(|| {
            R2xError::ConfigError(format!("Cannot find parent directory of {}", module_file))
        })?;
        let file_mapping_path = module_dir.join("config").join("file_mapping.json");

        info!("Using file mapping: {:?}", file_mapping_path);

        let data_store = data_store_class.call_method1(
            "from_json",
            (
                file_mapping_path.to_str().unwrap(),
                args.input.to_str().unwrap(),
            ),
        )?;

        let parser_kwargs = pyo3::types::PyDict::new_bound(py);
        parser_kwargs.set_item("config", config)?;
        parser_kwargs.set_item("data_store", data_store)?;

        let parser = parser_class.call((), Some(&parser_kwargs))?;

        info!("Building system from {} data...", args.model);
        let system = parser.call_method0("build_system")?;

        if args.stdout {
            // Write to stdout
            let json_str: String = system.call_method0("to_json")?.extract()?;
            println!("{}", json_str);
        } else {
            // Determine output path
            let output_path = if let Some(path) = &args.output {
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

            // Write to file
            info!("Writing system to: {:?}", output_path);
            system.call_method1("to_json", (output_path.to_str().unwrap(),))?;
            println!("✓ System written to: {}", output_path.display());
        }

        Ok(())
    })?;

    Ok(())
}

fn parse_model_args(
    model: &str,
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
