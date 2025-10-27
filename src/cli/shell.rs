//! Interactive IPython shell with loaded system

use crate::schema::{build_config_dict, get_plugin_schema};
use crate::{R2xError, Result};
use clap::Args;
use std::path::PathBuf;
use std::process::Command;
use std::process::Command;

#[derive(Args)]
pub struct ShellArgs {
    /// Plugin name to use for reading (e.g., reeds, switch, plexos)
    plugin: String,

    /// Input path (file or directory with source data)
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Load system from JSON file instead of parsing source data
    /// If not specified, looks for cached system in ~/.cache/r2x/systems/<plugin>_system.json
    #[arg(short = 'j', long, conflicts_with = "input")]
    json: Option<PathBuf>,

    /// Read system JSON from stdin
    #[arg(long, conflicts_with_all = ["input", "json"])]
    stdin: bool,

    /// Additional arguments for the plugin (key=value pairs, only used when parsing source data)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    model_args: Vec<String>,
}

pub fn execute(args: ShellArgs, verbose: u8, quiet: bool) -> Result<()> {
    // Initialize Python to get system loaded
    crate::python::init()?;

    let temp_system_path = Python::with_gil(|py| -> Result<PathBuf> {
        // Control Python logging
        if quiet || verbose == 0 {
            let logger = py.import_bound("loguru")?.getattr("logger")?;
            logger.call_method1("disable", ("",))?;
        }

        // Import infrasys
        let infrasys = py.import_bound("infrasys")?;
        let system_class = infrasys.getattr("System")?;

        // Determine how to load the system
        let system = if args.stdin {
            // Load from stdin
            println!("Reading system from stdin...");
            let mut json_input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut json_input)?;

            // Create temp file for stdin input
            let temp_path = std::env::temp_dir().join("r2x_stdin_system.json");
            std::fs::write(&temp_path, &json_input)?;

            system_class.call_method1("from_json", (temp_path.to_str().unwrap(),))?
        } else if let Some(json_path) = &args.json {
            // Load from JSON file
            println!("Loading system from JSON file: {}", json_path.display());
            system_class.call_method1("from_json", (json_path.to_str().unwrap(),))?
        } else if args.input.is_none() {
            // Try to load from cache
            let cache_dir = dirs::cache_dir()
                .ok_or(R2xError::NoCacheDir)?
                .join("r2x")
                .join("systems");

            let cached_path = cache_dir.join(format!("{}_system.json", args.plugin));

            if !cached_path.exists() {
                return Err(R2xError::ConfigError(format!(
                    "No cached system found at: {}\n\
                     Run 'r2x read {} <path>' first to create the cache, or use --input or --json",
                    cached_path.display(),
                    args.plugin
                )));
            }

            println!("Loading cached system from: {}", cached_path.display());

            system_class.call_method1("from_json", (cached_path.to_str().unwrap(),))?
        } else if let Some(input_path) = &args.input {
            // Parse from source data
            // Get plugin schema for argument parsing
            let schema = get_plugin_schema(&args.plugin)?;

            // Parse model-specific arguments
            let model_config = if !args.model_args.is_empty() {
                parse_model_args(&args.plugin, &schema, &args.model_args)?
            } else {
                std::collections::HashMap::new()
            };

            println!(
                "Loading system from {} using plugin '{}'...",
                input_path.display(),
                args.plugin
            );

            // Import r2x_core and get PluginManager
            let r2x_core = py.import_bound("r2x_core")?;
            let plugin_manager_class = r2x_core.getattr("PluginManager")?;
            let manager = plugin_manager_class.call0()?;

            // Load parser class
            let parser_class = manager.call_method1("load_parser", (&args.plugin,))?;
            if parser_class.is_none() {
                return Err(R2xError::PluginNotFound(args.plugin.clone()));
            }

            // Load config class
            let config_class = manager.call_method1("load_config_class", (&args.plugin,))?;
            if config_class.is_none() {
                return Err(R2xError::PluginNotFound(args.plugin.clone()));
            }

            // Build config dict
            let config_dict = build_config_dict(py, &schema, &model_config)?;

            // Add data_path to config
            config_dict.set_item("data_path", input_path.to_str().unwrap())?;

            // Create config instance
            let config_obj = config_class.call((), Some(&config_dict))?;

            // Create DataStore
            let data_store_class = r2x_core.getattr("DataStore")?;
            let data_store = data_store_class.call0()?;

            // Create parser instance
            let parser_kwargs = PyDict::new_bound(py);
            parser_kwargs.set_item("config", &config_obj)?;
            parser_kwargs.set_item("data_store", &data_store)?;
            let parser = parser_class.call((), Some(&parser_kwargs))?;

            // Build system
            println!("Building system...");
            parser.call_method0("build_system")?
        } else {
            return Err(R2xError::ConfigError(
                "Must specify one of: --input, --json, or --stdin".to_string(),
            ));
        };

        println!("✓ System loaded successfully\n");

        // Save system to a temporary file that IPython can load
        let temp_dir = std::env::temp_dir();
        let temp_system_path = temp_dir.join("r2x_shell_system.json");

        println!("Preparing IPython environment...");


    // Now spawn IPython as a subprocess with the system pre-loaded
    launch_ipython_subprocess(&args.plugin, &temp_system_path)?;

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_system_path);

    Ok(())
}

fn parse_model_args(
    _model: &str,
    schema: &[crate::schema::FieldSchema],
    args: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    let mut result = std::collections::HashMap::new();

    for arg in args {
        if let Some((key, value)) = arg.split_once('=') {
            let key = key.trim_start_matches("--").replace('-', "_");

            // Validate against schema
            if !schema.iter().any(|f| f.name == key) {
                return Err(R2xError::ConfigError(format!(
                    "Unknown argument: {}. Use --model-help to see available options.",
                    key
                )));
            }

            result.insert(key, value.to_string());
        } else {
            return Err(R2xError::ConfigError(format!(
                "Invalid argument format: '{}'. Expected key=value",
                arg
            )));
        }
    }

    Ok(result)
}

fn launch_ipython_subprocess(plugin: &str, temp_system_path: &PathBuf) -> Result<()> {
    // Get venv path
    let venv_path = crate::python::venv::get_venv_path()?;
    let python_exe = venv_path.join("bin/python");

    // Create startup script for IPython
    let startup_script = format!(
        r#"
# r2x shell startup script
import sys
from typing import Type, Optional, List

# Import required modules
import infrasys
import r2x_core
try:
    import polars as pl
except ImportError:
    pl = None

# Load the system
print("Loading system...")
system = infrasys.System.from_json("{}")
print("✓ System loaded successfully\n")

# Define helper functions for easy exploration
def info():
    """Display system summary information."""
    print("\n" + "=" * 70)
    print("System Information")
    print("=" * 70)

    # Get all component types
    comp_types = system.get_component_types()

    print(f"\nTotal Component Types: {{len(comp_types)}}")
    print("\nComponents by Type:")
    print("-" * 70)

    for comp_type in sorted(comp_types):
        components = list(system.get_components_by_type(comp_type))
        print(f"  {{comp_type:<40}} {{len(components):>6}}")

    print("-" * 70)

    # Time series info
    ts_metadata = system.list_time_series_metadata()
    print(f"\nTime Series: {{len(ts_metadata)}} datasets")

    print("=" * 70 + "\n")

def show_components(component_type: str, limit: int = 10):
    """
    Show components of a specific type.

    Args:
        component_type: Type of component (e.g., 'Generator', 'Bus', 'Load')
        limit: Maximum number to display (default: 10)
    """
    components = list(system.get_components_by_type(component_type))

    print(f"\n{{component_type}} (showing {{min(limit, len(components))}} of {{len(components)}}):")
    print("-" * 70)

    for i, comp in enumerate(components[:limit]):
        print(f"  {{i+1:>3}}. {{comp.name}}")
        # Show a few key attributes if available
        attrs = []
        if hasattr(comp, 'capacity') and comp.capacity is not None:
            attrs.append(f"capacity={{comp.capacity}}")
        if hasattr(comp, 'technology') and comp.technology:
            attrs.append(f"tech={{comp.technology}}")
        if hasattr(comp, 'category') and comp.category:
            attrs.append(f"category={{comp.category}}")

        if attrs:
            print(f"      {{', '.join(attrs)}}")

    if len(components) > limit:
        print(f"\n  ... and {{len(components) - limit}} more")

    print("-" * 70 + "\n")

def get_component(component_type: str, name: str):
    """
    Get a specific component by type and name.

    Args:
        component_type: Type of component
        name: Name of the component

    Returns:
        The component object or None if not found
    """
    components = list(system.get_components_by_type(component_type))

    for comp in components:
        if comp.name == name:
            return comp

    print(f"Component '{{name}}' of type '{{component_type}}' not found")
    print(f"Available components: {{len(components)}}")
    return None

def list_types():
    """List all available component types."""
    comp_types = sorted(system.get_component_types())

    print("\nAvailable Component Types:")
    print("-" * 70)
    for comp_type in comp_types:
        count = len(list(system.get_components_by_type(comp_type)))
        print(f"  {{comp_type:<40}} ({{count}} components)")
    print("-" * 70 + "\n")

def export(filename: str = "system_export.json"):
    """
    Export the system to a JSON file.

    Args:
        filename: Output filename (default: 'system_export.json')
    """
    print(f"Exporting system to {{filename}}...")
    system.to_json(filename)
    print(f"✓ Exported to {{filename}}")

def search(query: str, component_type: Optional[str] = None):
    """
    Search for components by name.

    Args:
        query: Search string (case-insensitive)
        component_type: Optional component type to filter by
    """
    query_lower = query.lower()
    results = []

    if component_type:
        types_to_search = [component_type]
    else:
        types_to_search = system.get_component_types()

    for comp_type in types_to_search:
        components = list(system.get_components_by_type(comp_type))
        for comp in components:
            if query_lower in comp.name.lower():
                results.append((comp_type, comp))

    print(f"\nSearch results for '{{query}}' ({{len(results)}} found):")
    print("-" * 70)

    for comp_type, comp in results[:20]:  # Limit to 20 results
        print(f"  [{{comp_type}}] {{comp.name}}")

    if len(results) > 20:
        print(f"\n  ... and {{len(results) - 20}} more results")

    print("-" * 70 + "\n")

    return results

# Print welcome message
print("=" * 70)
print("                      r2x Interactive Shell")
print("=" * 70)
print()
print("Loaded System:")
print(f"  Plugin: {}")
print()
print("Available Helper Functions:")
print("  info()                           - Show system summary")
print("  list_types()                     - List all component types")
print("  show_components(type, limit=10)  - Show components of a type")
print("  get_component(type, name)        - Get a specific component")
print("  search(query, type=None)         - Search components by name")
print("  export(filename)                 - Export system to JSON")
print()
print("Available Objects:")
print("  system      - The infrasys.System object")
print("  infrasys    - InfraSys module")
print("  r2x_core    - R2X Core module")
if pl:
    print("  pl          - Polars (DataFrame library)")
print()
print("Quick Start:")
print("  >>> info()                       # See what's in the system")
print("  >>> list_types()                 # List component types")
print("  >>> show_components('Generator') # Show generators")
print()
print("Advanced Usage:")
print("  # Import plugin-specific components:")
print("  >>> from r2x_{}.models.components import *")
print("  >>> generators = system.get_components(ReEDSGenerator)")
print()
print("To exit: exit() or Ctrl+D")
print("=" * 70)
print()

# Create IPython shell with our namespace
from IPython.terminal.embed import InteractiveShellEmbed

shell = InteractiveShellEmbed(
    user_ns={{
        'system': system,
        'infrasys': infrasys,
        'r2x_core': r2x_core,
        'pl': pl,
        'info': info,
        'show_components': show_components,
        'get_component': get_component,
        'list_types': list_types,
        'export': export,
        'search': search,
    }},
    banner1='',
    exit_msg='\nExiting r2x shell...'
)

# Start the shell
shell()
"#,
        temp_system_path.display(),
        plugin,
        plugin.replace('-', "_")
    );

    // Write startup script to temp file
    let temp_dir = std::env::temp_dir();
    let startup_script_path = temp_dir.join("r2x_ipython_startup.py");
    std::fs::write(&startup_script_path, startup_script)?;

    // Spawn IPython via the Python interpreter
    let status = Command::new(python_exe)
        .arg(&startup_script_path)
        .status()
        .map_err(|e| R2xError::SubprocessError(format!("Failed to start IPython: {}", e)))?;

    // Clean up startup script
    let _ = std::fs::remove_file(&startup_script_path);

    if !status.success() {
        return Err(R2xError::SubprocessError(
            "IPython exited with error".to_string(),
        ));
    }

    Ok(())
}
