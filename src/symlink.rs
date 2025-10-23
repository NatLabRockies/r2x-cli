//! Symlink-based command routing
//!
//! Handles execution when r2x-cli is invoked via a symlink name.

use crate::cli::Cli;
use crate::entrypoints::{EntryPoint, EntryPointKind};
use crate::schema;
use crate::Result;
use clap::Parser;

pub fn execute_from_symlink(command_name: &str) -> Result<()> {
    let entry_point = EntryPoint::from_command_name(command_name);

    // Check if --help was requested
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        crate::python::init()?;
        print_dynamic_help(command_name, &entry_point)?;
        return Ok(());
    }

    let mut args_vec = vec![
        "r2x-cli".to_string(),
        entry_point.subcommand().to_string(),
        entry_point.name.clone(),
    ];
    args_vec.extend(args);

    let cli = Cli::parse_from(args_vec);
    cli.execute()
}

fn print_dynamic_help(command_name: &str, entry_point: &EntryPoint) -> Result<()> {
    match entry_point.kind {
        EntryPointKind::Parser => print_parser_help(command_name, &entry_point.name),
        EntryPointKind::Exporter => print_exporter_help(command_name, &entry_point.name),
        EntryPointKind::Modifier => print_modifier_help(command_name, &entry_point.name),
    }
}

fn print_parser_help(command_name: &str, model_name: &str) -> Result<()> {
    println!("Read {} model data and output system as JSON\n", model_name);
    println!("Usage: {} --input <INPUT> [OPTIONS] [MODEL_ARGS]...\n", command_name);

    println!("Options:");
    println!("  -i, --input <INPUT>    Input folder containing model data (required)");
    println!("  -o, --output <OUTPUT>  Output file for system JSON");
    println!("                         [default: ~/.cache/r2x/systems/{}_system.json]", model_name);
    println!("      --stdout           Print JSON to stdout instead of saving");
    println!("  -v, --verbose          Enable verbose logging");
    println!("      --quiet            Disable progress output");
    println!("  -h, --help             Print help\n");

    if let Ok(schema) = schema::get_plugin_schema(model_name) {
        if !schema.is_empty() {
            println!("Model-specific options for {}:", model_name);
            for field in schema {
                let arg_name = field.name.replace('_', "-");
                let required = if field.required { " (required)" } else { "" };
                let default = if let Some(ref def) = field.default {
                    format!(" [default: {}]", def)
                } else {
                    String::new()
                };

                println!("  --{:<20} {}{}{}",
                    arg_name,
                    field.description.as_deref().unwrap_or(""),
                    required,
                    default
                );
            }
        }
    }

    Ok(())
}

fn print_exporter_help(command_name: &str, model_name: &str) -> Result<()> {
    println!("Write system JSON to {} format\n", model_name);
    println!("Usage: {} [OPTIONS]\n", command_name);

    println!("Options:");
    println!("  -v, --verbose  Enable verbose logging");
    println!("      --quiet    Disable progress output");
    println!("  -h, --help     Print help");

    Ok(())
}

fn print_modifier_help(command_name: &str, modifier_name: &str) -> Result<()> {
    println!("Run {} system modifier\n", modifier_name);
    println!("Usage: {} [OPTIONS]\n", command_name);

    println!("Options:");
    println!("  -v, --verbose  Enable verbose logging");
    println!("      --quiet    Disable progress output");
    println!("  -h, --help     Print help\n");

    if let Ok(schema) = schema::get_plugin_schema(modifier_name) {
        if !schema.is_empty() {
            println!("Modifier-specific options for {}:", modifier_name);
            for field in schema {
                let arg_name = field.name.replace('_', "-");
                let required = if field.required { " (required)" } else { "" };
                let default = if let Some(ref def) = field.default {
                    format!(" [default: {}]", def)
                } else {
                    String::new()
                };

                println!("  --{:<20} {}{}{}",
                    arg_name,
                    field.description.as_deref().unwrap_or(""),
                    required,
                    default
                );
            }
        }
    }

    Ok(())
}
