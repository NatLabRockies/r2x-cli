use crate::config_manager::Config;
use crate::logger;
use crate::GlobalOpts;
use atty::Stream;
use clap::Parser;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
pub struct ReadCommand {
    /// Path to JSON file to read. If not provided, reads from stdin
    pub file: Option<PathBuf>,

    /// Suppress the startup banner
    #[arg(long = "no-banner")]
    pub no_banner: bool,

    /// Execute a Python script against the loaded system
    #[arg(long = "exec", value_name = "SCRIPT")]
    pub exec: Option<PathBuf>,

    /// Drop into interactive IPython session after script execution (use with --exec)
    #[arg(short = 'i', long = "interactive")]
    pub interactive: bool,
}

pub fn handle_read(cmd: ReadCommand, opts: GlobalOpts) -> Result<(), Box<dyn std::error::Error>> {
    logger::debug("Starting read command");

    // Load configuration
    let mut config = Config::load()?;
    let venv_path = config.ensure_venv_path()?;
    logger::debug(&format!("Using virtual environment at {}", venv_path));

    // Get Python executable path (ensured via ensure_venv_path)
    let python_exe = config.get_venv_python_path();
    if !Path::new(&python_exe).exists() {
        return Err(format!(
            "Python executable not found at {}. Recreate the venv via `r2x python venv create`.",
            python_exe
        )
        .into());
    }

    logger::debug(&format!("Python executable: {}", python_exe));

    ensure_prerequisites(&mut config, &python_exe)?;

    // Determine if input is from stdin (for banner display)
    let is_stdin = cmd.file.is_none();

    // Load JSON input
    let json_file_path = match cmd.file {
        Some(file_path) => {
            logger::debug(&format!("Reading JSON from file: {}", file_path.display()));
            file_path
        }
        None => {
            if atty::is(Stream::Stdin) {
                logger::info(
                    "No JSON input detected; please provide --file or pipe JSON via stdin.",
                );
                return Err(
                    "No JSON input provided; either use --file or pipe data into `r2x read`".into(),
                );
            }

            logger::debug("Reading JSON from stdin");
            let mut json_data = String::new();
            std::io::stdin()
                .read_to_string(&mut json_data)
                .map_err(|e| format!("Failed to read from stdin: {}", e))?;

            let cache_dir = config.ensure_cache_path()?;
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let temp_json = PathBuf::from(cache_dir).join(format!("stdin_input_{}.json", unique));
            fs::write(&temp_json, &json_data)
                .map_err(|e| format!("Failed to write temporary JSON file: {}", e))?;

            logger::debug(&format!(
                "Saved stdin to temporary file: {}",
                temp_json.display()
            ));
            temp_json
        }
    };

    // Determine the display source for the banner
    let display_source = if is_stdin {
        "[piped from stdin]".to_string()
    } else {
        json_file_path.display().to_string()
    };

    // Generate Python initialization code
    let file_path_str = json_file_path
        .to_str()
        .ok_or("Invalid file path")?
        .replace('\\', "\\\\");

    let display_source_str = display_source.replace('\\', "\\\\").replace('\'', "\\'");

    let python_code = format!(
        r#"
import json
import os
import sys as py_sys
import traceback
from pathlib import Path
from IPython.terminal.embed import InteractiveShellEmbed
from IPython.terminal.prompts import Prompts, Token
from traitlets.config import Config
from r2x_core.system import System


class R2XPrompts(Prompts):
    """Custom prompts showing 'r2x [N]:' instead of 'In [N]:'"""

    def in_prompt_tokens(self):
        return [
            (Token.Prompt, "r2x ["),
            (Token.PromptNum, str(self.shell.execution_count)),
            (Token.Prompt, "]: "),
        ]

    def out_prompt_tokens(self):
        return [
            (Token.OutPrompt, "out ["),
            (Token.OutPromptNum, str(self.shell.execution_count)),
            (Token.OutPrompt, "]: "),
        ]


def format_import_error(name, module_path, error):
    """Format an import error with helpful suggestions including pip install instructions."""
    msg_lines = []
    msg_lines.append(f"\n  Plugin Import Error: '{{name}}' from '{{module_path}}'")
    msg_lines.append("  " + "-" * 50)

    error_str = str(error)
    error_type = type(error).__name__

    # Handle ModuleNotFoundError specifically
    if isinstance(error, ModuleNotFoundError):
        # Try to extract the missing module name
        missing_module = None
        if hasattr(error, 'name') and error.name:
            missing_module = error.name
        elif "No module named" in error_str:
            import re
            match = re.search(r"No module named ['\"]?([^'\"]+)['\"]?", error_str)
            if match:
                missing_module = match.group(1)

        msg_lines.append(f"  {{error_type}}: {{error}}")
        msg_lines.append("")
        if missing_module:
            # Suggest pip install for the base package
            base_pkg = missing_module.split('.')[0]
            msg_lines.append("  To fix this, install the missing dependency:")
            msg_lines.append(f"    pip install {{base_pkg}}")
            msg_lines.append("")
            msg_lines.append("  Or install into the r2x virtual environment:")
            msg_lines.append(f"    r2x python pip install {{base_pkg}}")
        else:
            msg_lines.append("  Install the required dependency with pip.")
    elif isinstance(error, ImportError):
        msg_lines.append(f"  {{error_type}}: {{error}}")
        msg_lines.append("")
        msg_lines.append("  Possible causes:")
        msg_lines.append("  - The plugin module is not installed")
        msg_lines.append("  - A dependency of the plugin is missing")
        msg_lines.append("  - The module path in manifest.toml is incorrect")
    else:
        msg_lines.append(f"  {{error_type}}: {{error}}")
        msg_lines.append("")
        msg_lines.append("  This plugin failed to load. Check the plugin configuration.")

    msg_lines.append("")
    return "\n".join(msg_lines)


class PluginProxy:
    """Lazy-loading proxy for a single plugin that imports on first access."""

    def __init__(self, name, module_path, plugin_type, info):
        self._name = name
        self._module_path = module_path
        self._plugin_type = plugin_type  # 'function' or 'class'
        self._info = info
        self._loaded = None
        self._error = None
        self._warning_shown = False

    def _load(self):
        """Import and return the actual plugin."""
        if self._loaded is not None:
            return self._loaded
        if self._error is not None:
            # Show warning only once per session
            if not self._warning_shown:
                print(format_import_error(self._name, self._module_path, self._error), file=py_sys.stderr)
                self._warning_shown = True
            raise self._error

        try:
            import importlib
            module = importlib.import_module(self._module_path)
            if self._plugin_type == "function":
                func_name = self._info.get("function_name", self._name)
                self._loaded = getattr(module, func_name)
            else:
                self._loaded = getattr(module, self._name)
            return self._loaded
        except Exception as e:
            self._error = e
            # Show the formatted error message immediately
            print(format_import_error(self._name, self._module_path, e), file=py_sys.stderr)
            self._warning_shown = True
            # Wrap in ImportError for consistent error type
            if not isinstance(e, ImportError):
                wrapped = ImportError(f"Failed to import plugin '{{self._name}}' from '{{self._module_path}}': {{e}}")
                wrapped.__cause__ = e
                self._error = wrapped
            raise self._error

    def __call__(self, *args, **kwargs):
        """Allow calling function plugins directly."""
        return self._load()(*args, **kwargs)

    def __getattr__(self, name):
        """Forward attribute access to the loaded plugin."""
        if name.startswith("_"):
            raise AttributeError(name)
        return getattr(self._load(), name)

    def __repr__(self):
        status = "loaded" if self._loaded else ("error" if self._error else "not loaded")
        return f"<PluginProxy: {{self._name}} ({{self._plugin_type}}, {{status}})>"


class PackageProxy:
    """Lazy-loading proxy for a package containing multiple plugins."""

    def __init__(self, package_name, plugins_info):
        self._package_name = package_name
        self._plugins_info = plugins_info  # dict of plugin_name -> (module_path, plugin_type, info)
        self._proxies = {{}}


# Global registry to track lazy imports
_lazy_import_registry = {{}}


class LazyModuleProxy:
    """Lazy-loading proxy for heavy libraries like pandas, numpy, matplotlib.

    The module is imported on first use with a brief message shown.
    """

    def __init__(self, module_name, display_name, pip_package=None, import_statement=None):
        # Use object.__setattr__ to avoid triggering __setattr__
        object.__setattr__(self, "_module_name", module_name)
        object.__setattr__(self, "_display_name", display_name)
        object.__setattr__(self, "_pip_package", pip_package or module_name.split('.')[0])
        object.__setattr__(self, "_import_statement", import_statement or f"import {{module_name}}")
        object.__setattr__(self, "_module", None)
        object.__setattr__(self, "_imported", False)
        # Register in global registry
        _lazy_import_registry[display_name] = self

    def _load(self):
        """Import and return the actual module."""
        if object.__getattribute__(self, "_imported"):
            return object.__getattribute__(self, "_module")

        module_name = object.__getattribute__(self, "_module_name")
        display_name = object.__getattribute__(self, "_display_name")
        pip_package = object.__getattribute__(self, "_pip_package")

        print(f"Importing {{display_name}}...")
        try:
            import importlib
            module = importlib.import_module(module_name)
            object.__setattr__(self, "_module", module)
            object.__setattr__(self, "_imported", True)
            return module
        except ModuleNotFoundError as e:
            msg_lines = [
                f"\n  Module Not Found: {{display_name}}",
                "  " + "-" * 50,
                f"  {{e}}",
                "",
                "  To install this dependency:",
                f"    pip install {{pip_package}}",
                "",
                "  Or install into the r2x virtual environment:",
                f"    r2x python pip install {{pip_package}}",
                "",
            ]
            raise ModuleNotFoundError("\n".join(msg_lines)) from e
        except ImportError as e:
            msg_lines = [
                f"\n  Import Error: {{display_name}}",
                "  " + "-" * 50,
                f"  {{e}}",
                "",
                "  The module may be installed but has missing dependencies.",
                "  Try reinstalling:",
                f"    pip install --upgrade {{pip_package}}",
                "",
            ]
            raise ImportError("\n".join(msg_lines)) from e

    def __getattr__(self, name):
        """Forward attribute access to the loaded module."""
        return getattr(self._load(), name)

    def __setattr__(self, name, value):
        """Forward attribute setting to the loaded module."""
        setattr(self._load(), name, value)

    def __call__(self, *args, **kwargs):
        """Allow calling the module (e.g., for matplotlib.pyplot.figure())."""
        return self._load()(*args, **kwargs)

    def __repr__(self):
        imported = object.__getattribute__(self, "_imported")
        display_name = object.__getattribute__(self, "_display_name")
        if imported:
            module = object.__getattribute__(self, "_module")
            return repr(module)
        return f"<LazyModuleProxy: {{display_name}} (not yet imported)>"

    def __dir__(self):
        """Return module attributes for tab completion."""
        return dir(self._load())

    def _get_proxy(self, name):
        """Get or create a PluginProxy for the given plugin name."""
        if name not in self._proxies:
            if name not in self._plugins_info:
                raise AttributeError(f"Plugin '{{name}}' not found in package '{{self._package_name}}'")
            module_path, plugin_type, info = self._plugins_info[name]
            self._proxies[name] = PluginProxy(name, module_path, plugin_type, info)
        return self._proxies[name]

    def __getattr__(self, name):
        """Return PluginProxy for the requested plugin."""
        if name.startswith("_"):
            raise AttributeError(name)
        return self._get_proxy(name)

    def __dir__(self):
        """Return plugin names for tab completion."""
        return list(self._plugins_info.keys())

    def __repr__(self):
        count = len(self._plugins_info)
        return f"<PackageProxy: {{self._package_name}} ({{count}} plugins)>"


class R2XMagics:
    """r2x custom magic commands for IPython."""

    # Command registry: name -> (brief_description, detailed_help, usage_examples)
    COMMANDS = {{
        "r2x_help": (
            "Show help for r2x commands",
            "Display help information for r2x magic commands. Without arguments, lists all available commands. With a command name, shows detailed help for that command.",
            [
                "%r2x_help              # List all r2x commands",
                "%r2x_help plugins      # Show help for %plugins command",
            ],
        ),
        "plugins": (
            "List and manage installed plugins",
            "Display and search installed r2x plugins. Supports listing all plugins, searching by name/description, showing detailed info for a specific plugin, and reloading the plugin list from manifest.",
            [
                "%plugins               # List all installed plugins",
                "%plugins search parser # Search for plugins containing 'parser'",
                "%plugins info Parser   # Show detailed info for 'Parser' plugin",
                "%plugins reload        # Reload plugin list from manifest",
            ],
        ),
        "components": (
            "Get system components as a list",
            "Get components from the loaded system. Without arguments, directs to sys.info(). With a type name, stores components in '_components' variable for programmatic access.",
            [
                "%components              # Show help",
                "%components Generator    # Store generators in '_components'",
            ],
        ),
        "export": (
            "Export system or components to files",
            "Export the current system or specific component types to JSON or CSV files. Supports pretty-printing JSON and filtering by component type.",
            [
                "%export output.json                   # Export full system to JSON",
                "%export output.json --pretty          # Export with indentation",
                "%export generators.csv --type Generator  # Export generators as CSV",
            ],
        ),
        "reload": (
            "Reload system from disk",
            "Reload the system from its original file or load a different file. Useful after making external changes to the source file.",
            [
                "%reload                # Reload from original file",
                "%reload other.json     # Load a different file",
            ],
        ),
        "run_plugin": (
            "Run a plugin on the current system",
            "Execute a plugin against the current system. The result is stored in the 'result' variable. Optionally pass configuration as JSON or replace the current system with the plugin output.",
            [
                "%run_plugin r2x_reeds.Parser                    # Run plugin, store in 'result'",
                "%run_plugin r2x_reeds.Parser --config '{{...}}'   # Pass JSON config",
                "%run_plugin r2x_reeds.Parser --replace          # Replace 'sys' with output",
            ],
        ),
        "imports": (
            "Show currently imported modules",
            "Display which heavy libraries (pandas, numpy, matplotlib) have been imported in the current session.",
            [
                "%imports               # Show import status",
            ],
        ),
    }}

    def __init__(self, shell, plugins_ns, system_ns, json_path, display_source):
        self.shell = shell
        self._plugins = plugins_ns
        self._system = system_ns
        self._json_path = json_path
        self._display_source = display_source
        self._is_stdin = display_source == "[piped from stdin]"

    def r2x_help(self, line):
        """Show help for r2x magic commands.

        Usage:
            %r2x_help           - List all available commands
            %r2x_help <command> - Show detailed help for a command
        """
        line = line.strip()

        if not line:
            # List all commands
            print()
            print("  r2x Magic Commands")
            print("  " + "=" * 40)
            print()
            for cmd_name, (brief, _, _) in sorted(self.COMMANDS.items()):
                print(f"  %{{cmd_name:<15}} {{brief}}")
            print()
            print("  Type '%r2x_help <command>' for detailed help on a specific command.")
            print()
            return

        # Show help for specific command
        cmd_name = line.lstrip("%")  # Allow both %r2x_help plugins and %r2x_help %plugins
        if cmd_name not in self.COMMANDS:
            print(f"  Unknown command: %{{cmd_name}}")
            print(f"  Type '%r2x_help' to see available commands.")
            return

        brief, detailed, examples = self.COMMANDS[cmd_name]
        print()
        print(f"  %{{cmd_name}} - {{brief}}")
        print("  " + "-" * 40)
        print()
        print(f"  {{detailed}}")
        print()
        print("  Examples:")
        for ex in examples:
            print(f"    {{ex}}")
        print()

    def plugins(self, line):
        """List and manage installed plugins.

        Usage:
            %plugins              - List all installed plugins
            %plugins search <q>   - Search for plugins by name/description
            %plugins info <name>  - Show detailed info for a plugin
            %plugins reload       - Reload plugin list from manifest
        """
        args = line.strip().split()

        if not args:
            # %plugins - list all plugins
            self._plugins.list()
            return

        subcommand = args[0].lower()

        if subcommand == "search":
            # %plugins search <query>
            if len(args) < 2:
                print("  Usage: %plugins search <query>")
                print("  Example: %plugins search parser")
                return
            query = " ".join(args[1:])
            self._plugins.search(query)
            return

        if subcommand == "info":
            # %plugins info <name>
            if len(args) < 2:
                print("  Usage: %plugins info <plugin_name>")
                print("  Example: %plugins info Parser")
                return
            plugin_name = args[1]
            self._show_plugin_info(plugin_name)
            return

        if subcommand == "reload":
            # %plugins reload
            self._plugins._load_manifest()
            print(f"  Reloaded plugins: {{self._plugins._plugin_count}} plugins from {{len(self._plugins._packages)}} packages")
            return

        # Unknown subcommand - treat as search query
        print(f"  Unknown subcommand: '{{subcommand}}'")
        print("  Available subcommands: search, info, reload")
        print("  Or run '%plugins' without arguments to list all plugins.")

    def _show_plugin_info(self, plugin_name):
        """Show detailed information for a specific plugin."""
        # Search for the plugin across all packages
        found = []
        for pkg_name, plugins_info in self._plugins._packages.items():
            if plugin_name in plugins_info:
                module_path, plugin_type, info = plugins_info[plugin_name]
                found.append((pkg_name, plugin_name, module_path, plugin_type, info))
            else:
                # Also check case-insensitive
                for name, (mod_path, p_type, p_info) in plugins_info.items():
                    if name.lower() == plugin_name.lower():
                        found.append((pkg_name, name, mod_path, p_type, p_info))

        if not found:
            print(f"  Plugin '{{plugin_name}}' not found.")
            print(f"  Run '%plugins' to see all available plugins.")
            return

        for pkg_name, name, module_path, plugin_type, info in found:
            print()
            print(f"  Plugin: {{name}}")
            print("  " + "-" * 40)
            print(f"  Package:     {{pkg_name}}")
            print(f"  Type:        {{plugin_type}}")
            print(f"  Module:      {{module_path}}")
            if info.get("description"):
                print(f"  Description: {{info['description']}}")
            if info.get("function_name"):
                print(f"  Function:    {{info['function_name']}}")

            # Show any additional info fields
            skip_keys = {{"module", "description", "function_name"}}
            extra = {{k: v for k, v in info.items() if k not in skip_keys}}
            if extra:
                print("  Extra info:")
                for k, v in extra.items():
                    print(f"    {{k}}: {{v}}")
            print()

    def components(self, line):
        """List and filter system components.

        Usage:
            %components           - Show component types (directs to sys.info())
            %components <Type>    - Get components of that type as a list
        """
        line = line.strip()
        if not line:
            print()
            print("  Run 'sys.info()' to see component types and counts.")
            print("  Use '%components <Type>' to get components as a list.")
            print()
            return

        # Find matching component type (case-insensitive)
        type_name = line.split()[0]
        matching_class = None
        for comp_type in self._system.get_component_types():
            if comp_type.__name__.lower() == type_name.lower():
                matching_class = comp_type
                type_name = comp_type.__name__
                break

        if matching_class is None:
            available = sorted(ct.__name__ for ct in self._system.get_component_types())
            print(f"  Component type '{{type_name}}' not found.")
            print(f"  Available types: {{', '.join(available)}}")
            return

        # Return components as list and store in shell namespace
        components = list(self._system.get_components(matching_class))
        self.shell.user_ns["_components"] = components
        print(f"  {{len(components)}} {{type_name}} components stored in '_components'")
        print(f"  Access via: _components[0], len(_components), etc.")

    def export(self, line):
        """Export system or components to files.

        Usage:
            %export output.json              - Export full system to JSON
            %export output.json --pretty     - Export with indentation
            %export gen.csv --type Generator - Export specific type as CSV
            %export output.json --force      - Overwrite without confirmation
        """
        import shlex
        import csv

        line = line.strip()
        if not line:
            print("  Usage: %export <filename> [--pretty] [--type <Type>] [--force]")
            print("  Example: %export output.json --pretty")
            print("  Example: %export generators.csv --type Generator")
            return

        try:
            args = shlex.split(line)
        except ValueError as e:
            print(f"  Error parsing arguments: {{e}}")
            return

        if not args:
            print("  Usage: %export <filename> [--pretty] [--type <Type>] [--force]")
            return

        filename = args[0]
        filepath = Path(filename)

        # Parse flags
        pretty = "--pretty" in args
        force = "--force" in args
        comp_type = None
        for i, arg in enumerate(args):
            if arg == "--type" and i + 1 < len(args):
                comp_type = args[i + 1]
                break

        # Check for overwrite confirmation
        if filepath.exists() and not force:
            response = input(f"  File '{{filename}}' exists. Overwrite? [y/N]: ").strip().lower()
            if response not in ("y", "yes"):
                print("  Export cancelled.")
                return

        ext = filepath.suffix.lower()

        if ext == ".csv":
            if comp_type is None:
                print("  CSV export requires --type flag to specify component type.")
                print("  Example: %export generators.csv --type Generator")
                return

            # Find matching component type (case-insensitive)
            matching_class = None
            matching_type = comp_type
            for ct in self._system.get_component_types():
                if ct.__name__.lower() == comp_type.lower():
                    matching_class = ct
                    matching_type = ct.__name__
                    break

            if matching_class is None:
                available = sorted(ct.__name__ for ct in self._system.get_component_types())
                print(f"  Component type '{{comp_type}}' not found.")
                print(f"  Available types: {{', '.join(available)}}")
                return

            components = list(self._system.get_components(matching_class))
            if not components:
                print(f"  No components of type '{{matching_type}}' to export.")
                return

            # Convert components to dicts and collect all keys
            all_keys = set()
            comp_dicts = []
            for comp in components:
                comp_dict = {{
                    attr: getattr(comp, attr)
                    for attr in dir(comp)
                    if not attr.startswith("_") and not callable(getattr(comp, attr, None))
                }}
                comp_dict["uuid"] = str(comp_dict.get("uuid", ""))
                all_keys.update(comp_dict.keys())
                comp_dicts.append(comp_dict)

            # Sort keys with priority fields first
            priority = ["name", "uuid", "id", "bus", "type"]
            sorted_keys = [k for k in priority if k in all_keys]
            sorted_keys.extend(sorted(all_keys - set(sorted_keys)))

            try:
                with open(filepath, "w", newline="", encoding="utf-8") as f:
                    writer = csv.DictWriter(f, fieldnames=sorted_keys, extrasaction='ignore')
                    writer.writeheader()
                    writer.writerows(comp_dicts)
                print(f"  Exported {{len(components)}} {{matching_type}} components to '{{filename}}'")
            except Exception as e:
                print(f"  Error writing CSV: {{e}}")

        elif ext == ".json":
            if comp_type is not None:
                print("  Note: Component-specific JSON export exports the full system.")
                print("  Use CSV export for component-specific data: %export file.csv --type <Type>")

            try:
                self._system.to_json(fname=filepath, overwrite=True, indent=2 if pretty else None)
                print(f"  Exported full system to '{{filename}}'")
            except Exception as e:
                print(f"  Error writing JSON: {{e}}")

        else:
            print(f"  Unsupported file format: {{ext}}")
            print("  Supported formats: .json, .csv")

    def reload(self, line):
        """Reload system from disk.

        Usage:
            %reload              - Reload from original file
            %reload other.json   - Load a different file
        """
        line = line.strip()

        # Determine file path to load
        if not line:
            # Reload from original file
            if self._is_stdin:
                print("  Warning: Original source was stdin (piped input).")
                print("  Cannot reload - no file path available.")
                print("  Use '%reload <filename>' to load a specific file.")
                return

            file_path = Path(self._json_path)
        else:
            # Load a different file
            file_path = Path(line)

        # Check if file exists
        if not file_path.exists():
            print(f"  Error: File not found: {{file_path}}")
            return

        if not file_path.is_file():
            print(f"  Error: Not a file: {{file_path}}")
            return

        # Load and parse the JSON file
        try:
            with open(file_path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except json.JSONDecodeError as e:
            print(format_json_error(str(file_path), e))
            return
        except Exception as e:
            print(f"  Error reading file: {{e}}")
            return

        # Create new System object
        try:
            cwd = os.getcwd()
            new_system = System.from_dict(data, cwd)
        except Exception as e:
            print(format_system_error(e))
            return

        # Update the system reference and shell namespace
        self._system = new_system
        self.shell.user_ns["sys"] = new_system

        # Update tracked path if loading a new file
        if line:
            self._json_path = str(file_path.resolve())
            self._display_source = str(file_path)
            self._is_stdin = False

        # Print confirmation
        print()
        print(f"  Reloaded system from: {{file_path}}")
        print("  Run 'sys.info()' for component details.")
        print()

    def run_plugin(self, line):
        """Run a plugin against the current system.

        Usage:
            %run_plugin pkg.PluginName              - Run plugin, store result in 'result'
            %run_plugin pkg.PluginName --config '{{...}}' - Pass JSON config
            %run_plugin pkg.PluginName --replace    - Replace 'sys' with output
        """
        import shlex

        line = line.strip()
        if not line:
            print("  Usage: %run_plugin <package.PluginName> [--config '{{...}}'] [--replace]")
            print("  Example: %run_plugin r2x_reeds.Parser")
            return

        try:
            args = shlex.split(line)
        except ValueError as e:
            print(f"  Error parsing arguments: {{e}}")
            return

        if not args:
            print("  Usage: %run_plugin <package.PluginName> [--config '{{...}}'] [--replace]")
            return

        plugin_ref = args[0]
        replace = "--replace" in args

        # Extract --config value
        config_json = None
        for i, arg in enumerate(args):
            if arg == "--config" and i + 1 < len(args):
                config_json = args[i + 1]
                break

        # Parse plugin reference (package.PluginName)
        if "." not in plugin_ref:
            print(f"  Invalid plugin reference: '{{plugin_ref}}'")
            print("  Use format: <package>.<PluginName> (e.g., r2x_reeds.Parser)")
            return

        package_name, plugin_name = plugin_ref.split(".", 1)

        # Find the plugin
        if package_name not in self._plugins._packages:
            print(f"  Package '{{package_name}}' not found.")
            print(f"  Available packages: {{', '.join(sorted(self._plugins._packages.keys()))}}")
            return

        package_plugins = self._plugins._packages[package_name]
        if plugin_name not in package_plugins:
            print(f"  Plugin '{{plugin_name}}' not found in package '{{package_name}}'.")
            print(f"  Available plugins: {{', '.join(sorted(package_plugins.keys()))}}")
            return

        module_path, plugin_type, info = package_plugins[plugin_name]

        # Import the plugin
        try:
            import importlib
            module = importlib.import_module(module_path)
            attr_name = info.get("function_name", plugin_name) if plugin_type == "function" else plugin_name
            plugin = getattr(module, attr_name)
        except Exception:
            print(f"  Error importing plugin '{{plugin_ref}}':")
            traceback.print_exc()
            return

        # Parse config
        config = {{}}
        if config_json:
            try:
                config = json.loads(config_json)
            except json.JSONDecodeError as e:
                print(f"  Error parsing config JSON: {{e}}")
                return

        # Run the plugin
        print(f"  Running {{plugin_ref}}...")
        try:
            if plugin_type == "function":
                result = plugin(self._system, **config) if config else plugin(self._system)
            else:
                instance = plugin(**config) if config else plugin()
                # Find callable method on instance
                for method_name in ["__call__", "run", "process", "parse"]:
                    if method_name == "__call__" and callable(instance):
                        result = instance(self._system)
                        break
                    elif hasattr(instance, method_name):
                        result = getattr(instance, method_name)(self._system)
                        break
                else:
                    print(f"  Error: Plugin class '{{plugin_name}}' has no run(), process(), parse() method")
                    return
        except Exception:
            print(f"  Error running plugin '{{plugin_ref}}':")
            traceback.print_exc()
            return

        # Store result
        self.shell.user_ns["result"] = result
        print(f"  Plugin completed. Result stored in 'result' variable.")

        if replace:
            if result is None:
                print("  Warning: Plugin returned None. 'sys' not updated.")
            else:
                self._system = result
                self.shell.user_ns["sys"] = result
                print("  System updated: 'sys' now contains the plugin output.")

        # Show result summary
        if result is not None:
            result_type = type(result).__name__
            if hasattr(result, "info"):
                print(f"  Result type: {{result_type}} (run result.info() for details)")
            elif isinstance(result, (dict, list)):
                print(f"  Result type: {{result_type}} ({{len(result)}} {{'keys' if isinstance(result, dict) else 'items'}})")
            else:
                print(f"  Result type: {{result_type}}")

    def imports(self, line):
        """Show currently imported modules.

        Usage:
            %imports  - Show which heavy libraries have been imported
        """
        print()
        print("  Lazy Import Status")
        print("  " + "=" * 40)
        print()

        if not _lazy_import_registry:
            print("  No lazy imports registered.")
            print()
            return

        # Get status for each registered lazy import
        for display_name, proxy in sorted(_lazy_import_registry.items()):
            imported = object.__getattribute__(proxy, "_imported")
            module_name = object.__getattribute__(proxy, "_module_name")
            status = "imported" if imported else "not imported"
            print(f"  {{display_name:<10}} ({{module_name:<20}}) - {{status}}")

        print()
        print("  Libraries are imported on first use to speed up startup.")
        print("  Access pd, np, or plt to trigger import.")
        print()


class R2XPlugins:
    """Namespace for accessing installed r2x plugins with tab completion."""

    def __init__(self):
        self._manifest = {{}}
        self._packages = {{}}  # package_name -> dict of plugin_name -> (module_path, plugin_type, info)
        self._package_proxies = {{}}
        self._plugin_count = 0
        self._load_manifest()

    def _load_manifest(self):
        """Load plugin manifest from ~/.r2x/manifest.toml."""
        manifest_path = Path.home() / ".r2x" / "manifest.toml"
        if not manifest_path.exists():
            return

        try:
            # Use tomllib (Python 3.11+) or tomli as fallback
            try:
                import tomllib
            except ImportError:
                try:
                    import tomli as tomllib
                except ImportError:
                    # No TOML parser available, manifest won't be loaded
                    return

            with open(manifest_path, "rb") as f:
                self._manifest = tomllib.load(f)

            # Build package -> plugins mapping
            packages = {{}}
            plugin_count = 0

            # Process function plugins
            functions = self._manifest.get("functions", {{}})
            for name, info in functions.items():
                module = info.get("module", "")
                if module:
                    pkg = module.split(".")[0]
                    if pkg not in packages:
                        packages[pkg] = {{}}
                    packages[pkg][name] = (module, "function", info)
                    plugin_count += 1

            # Process class plugins
            classes = self._manifest.get("classes", {{}})
            for name, info in classes.items():
                module = info.get("module", "")
                if module:
                    pkg = module.split(".")[0]
                    if pkg not in packages:
                        packages[pkg] = {{}}
                    packages[pkg][name] = (module, "class", info)
                    plugin_count += 1

            self._packages = packages
            self._plugin_count = plugin_count

        except Exception:
            # Handle any errors gracefully - just leave empty
            pass

    def __getattr__(self, name):
        """Return PackageProxy for the requested package."""
        if name.startswith("_"):
            raise AttributeError(name)
        if name not in self._packages:
            raise AttributeError(f"Package '{{name}}' not found in plugins")

        if name not in self._package_proxies:
            self._package_proxies[name] = PackageProxy(name, self._packages[name])
        return self._package_proxies[name]

    def __dir__(self):
        """Return package names for tab completion."""
        return list(self._packages.keys()) + ["list", "search"]

    def __repr__(self):
        """Return string representation showing plugin/package counts."""
        pkg_count = len(self._packages)
        return f"<R2XPlugins: {{self._plugin_count}} plugins from {{pkg_count}} packages>"

    def list(self):
        """List all installed plugins with their details.

        Returns a list of dicts with keys: package, name, type, module, description
        """
        result = []
        for pkg_name, plugins_info in sorted(self._packages.items()):
            for plugin_name, (module_path, plugin_type, info) in sorted(plugins_info.items()):
                entry = {{
                    "package": pkg_name,
                    "name": plugin_name,
                    "type": plugin_type,
                    "module": module_path,
                    "description": info.get("description", ""),
                }}
                result.append(entry)

        # Print formatted table
        if not result:
            print("No plugins installed. Run 'r2x install' to add plugins.")
            return result

        # Calculate column widths
        pkg_width = max(len("Package"), max(len(p["package"]) for p in result))
        name_width = max(len("Name"), max(len(p["name"]) for p in result))
        type_width = max(len("Type"), max(len(p["type"]) for p in result))

        # Print header
        header = f"  {{'Package':<{{pkg_width}}}}  {{'Name':<{{name_width}}}}  {{'Type':<{{type_width}}}}  Description"
        print(header)
        print("  " + "-" * (len(header) - 2))

        # Print rows
        for p in result:
            desc = p["description"][:50] + "..." if len(p["description"]) > 50 else p["description"]
            print(f"  {{p['package']:<{{pkg_width}}}}  {{p['name']:<{{name_width}}}}  {{p['type']:<{{type_width}}}}  {{desc}}")

        print(f"\n  Total: {{len(result)}} plugins from {{len(self._packages)}} packages")
        return result

    def search(self, query):
        """Search plugins by name (case-insensitive).

        Args:
            query: String to search for in plugin names

        Returns a list of matching plugin dicts with keys: package, name, type, module, description
        """
        query_lower = query.lower()
        result = []
        for pkg_name, plugins_info in sorted(self._packages.items()):
            for plugin_name, (module_path, plugin_type, info) in sorted(plugins_info.items()):
                # Search in name and description
                if query_lower in plugin_name.lower() or query_lower in info.get("description", "").lower():
                    entry = {{
                        "package": pkg_name,
                        "name": plugin_name,
                        "type": plugin_type,
                        "module": module_path,
                        "description": info.get("description", ""),
                    }}
                    result.append(entry)

        # Print formatted table
        if not result:
            print(f"No plugins found matching '{{query}}'")
            return result

        # Calculate column widths
        pkg_width = max(len("Package"), max(len(p["package"]) for p in result))
        name_width = max(len("Name"), max(len(p["name"]) for p in result))
        type_width = max(len("Type"), max(len(p["type"]) for p in result))

        # Print header
        header = f"  {{'Package':<{{pkg_width}}}}  {{'Name':<{{name_width}}}}  {{'Type':<{{type_width}}}}  Description"
        print(header)
        print("  " + "-" * (len(header) - 2))

        # Print rows
        for p in result:
            desc = p["description"][:50] + "..." if len(p["description"]) > 50 else p["description"]
            print(f"  {{p['package']:<{{pkg_width}}}}  {{p['name']:<{{name_width}}}}  {{p['type']:<{{type_width}}}}  {{desc}}")

        print(f"\n  Found: {{len(result)}} plugins matching '{{query}}'")
        return result


def print_startup_banner(display_source, plugins):
    """Print startup banner with source and plugin info."""
    quiet = os.environ.get("R2X_READ_QUIET") == "1"
    no_banner = os.environ.get("R2X_READ_NO_BANNER") == "1"

    if quiet or no_banner:
        return

    plugin_count = plugins._plugin_count
    package_count = len(plugins._packages)

    print()
    print(f"  Source: {{display_source}}")
    if plugin_count > 0:
        print(f"  Plugins: {{plugin_count}} loaded from {{package_count}} packages")
    else:
        print("  Plugins: none (run r2x install to add)")
    print()
    print("  Type 'sys.info()' for system details, %r2x_help for commands")
    print()


DISPLAY_SOURCE = r'''{}'''
JSON_PATH = r'''{}'''


def format_json_error(json_path, error):
    """Format a JSON parse error with line/column and context snippet."""
    msg_lines = []
    msg_lines.append(f"\n  JSON Parse Error in: {{json_path}}")
    msg_lines.append("  " + "-" * 50)

    # Extract line/column from JSONDecodeError
    if hasattr(error, 'lineno') and hasattr(error, 'colno'):
        msg_lines.append(f"  Line {{error.lineno}}, Column {{error.colno}}: {{error.msg}}")

        # Try to show context snippet
        try:
            with open(json_path, 'r', encoding='utf-8') as f:
                lines = f.readlines()

            line_idx = error.lineno - 1
            start = max(0, line_idx - 2)
            end = min(len(lines), line_idx + 3)

            msg_lines.append("")
            msg_lines.append("  Context:")
            for i in range(start, end):
                prefix = "  >>> " if i == line_idx else "      "
                line_text = lines[i].rstrip()
                if len(line_text) > 80:
                    line_text = line_text[:77] + "..."
                msg_lines.append(f"  {{prefix}}{{i+1:4d}} | {{line_text}}")
                # Show caret at error position
                if i == line_idx and hasattr(error, 'colno'):
                    caret_line = "  " + " " * 6 + " " * 4 + " | " + " " * (error.colno - 1) + "^"
                    msg_lines.append(caret_line)
        except Exception:
            pass
    else:
        msg_lines.append(f"  {{error}}")

    msg_lines.append("")
    msg_lines.append("  Common fixes:")
    msg_lines.append("  - Check for trailing commas in arrays/objects")
    msg_lines.append("  - Ensure all strings are properly quoted")
    msg_lines.append("  - Verify brackets/braces are balanced")
    msg_lines.append("")
    return "\n".join(msg_lines)


def format_system_error(error):
    """Format a System.from_dict error with suggestions for common fixes."""
    msg_lines = []
    msg_lines.append("\n  System Loading Error")
    msg_lines.append("  " + "-" * 50)
    msg_lines.append(f"  {{error}}")
    msg_lines.append("")

    error_str = str(error).lower()

    # Provide contextual suggestions based on error message
    if "key" in error_str or "missing" in error_str or "required" in error_str:
        msg_lines.append("  Suggestions:")
        msg_lines.append("  - Check that all required fields are present in the JSON")
        msg_lines.append("  - Verify field names match the expected schema")
        msg_lines.append("  - Use 'r2x schema' to view the expected JSON structure")
    elif "type" in error_str or "invalid" in error_str or "expected" in error_str:
        msg_lines.append("  Suggestions:")
        msg_lines.append("  - Ensure values have the correct data types (string, number, array)")
        msg_lines.append("  - Check that numeric fields don't contain strings")
        msg_lines.append("  - Verify boolean values are 'true' or 'false' (not quoted)")
    elif "uuid" in error_str:
        msg_lines.append("  Suggestions:")
        msg_lines.append("  - Ensure UUIDs are valid format (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)")
        msg_lines.append("  - Check that UUIDs are unique across all components")
    elif "component" in error_str:
        msg_lines.append("  Suggestions:")
        msg_lines.append("  - Verify component references point to existing components")
        msg_lines.append("  - Check that component types are supported")
    else:
        msg_lines.append("  Suggestions:")
        msg_lines.append("  - Verify the JSON structure matches the r2x schema")
        msg_lines.append("  - Check for typos in field names")
        msg_lines.append("  - Ensure all references to other components are valid")

    msg_lines.append("")
    return "\n".join(msg_lines)


try:
    with open(JSON_PATH, 'r', encoding='utf-8') as handle:
        try:
            data = json.load(handle)
        except json.JSONDecodeError as e:
            print(format_json_error(JSON_PATH, e), file=py_sys.stderr)
            py_sys.exit(1)
    cwd = os.getcwd()
    try:
        system = System.from_dict(data, cwd)
    except Exception as e:
        print(format_system_error(e), file=py_sys.stderr)
        py_sys.exit(1)
except FileNotFoundError:
    print(f"\n  Error: File not found: {{JSON_PATH}}", file=py_sys.stderr)
    py_sys.exit(1)
except PermissionError:
    print(f"\n  Error: Permission denied reading: {{JSON_PATH}}", file=py_sys.stderr)
    py_sys.exit(1)
except Exception:
    traceback.print_exc()
    py_sys.exit(1)

cfg = Config()
cfg.TerminalInteractiveShell.confirm_exit = False
cfg.TerminalInteractiveShell.display_banner = False
cfg.TerminalInteractiveShell.banner1 = ""
cfg.TerminalInteractiveShell.banner2 = ""
cfg.TerminalInteractiveShell.highlighting_style = "monokai"
cfg.TerminalInteractiveShell.colors = "Linux"

# Configure persistent history isolated to r2x
r2x_ipython_dir = Path.home() / ".r2x" / "ipython"
r2x_ipython_dir.mkdir(parents=True, exist_ok=True)
history_file = r2x_ipython_dir / "history.sqlite"
cfg.HistoryManager.hist_file = str(history_file)

force_simple_env = os.environ.get("R2X_FORCE_SIMPLE_PROMPT")
if force_simple_env is None:
    simple_prompt = not (py_sys.stdin.isatty() and py_sys.stdout.isatty())
else:
    simple_prompt = force_simple_env.lower() in ("1", "true", "yes", "on")

cfg.TerminalInteractiveShell.simple_prompt = simple_prompt

if os.environ.get("R2X_READ_NONINTERACTIVE") == "1":
    print("System available as `sys`. Run sys.info() for details.")
    py_sys.exit(0)

# Initialize plugins namespace
plugins = R2XPlugins()

# Print startup banner (after plugins are loaded to show plugin count)
print_startup_banner(DISPLAY_SOURCE, plugins)

# Create lazy imports for heavy libraries (not imported at startup)
pd = LazyModuleProxy("pandas", "pandas")
np = LazyModuleProxy("numpy", "numpy")
plt = LazyModuleProxy("matplotlib.pyplot", "matplotlib")

# Handle --exec script execution
exec_script = os.environ.get("R2X_READ_EXEC")
interactive_after_exec = os.environ.get("R2X_READ_INTERACTIVE") == "1"

if exec_script:
    # Execute the provided script with sys and plugins in namespace
    exec_path = Path(exec_script)

    if not exec_path.exists():
        print(f"Error: Script not found: {{exec_path}}", file=py_sys.stderr)
        py_sys.exit(1)

    if not exec_path.is_file():
        print(f"Error: Not a file: {{exec_path}}", file=py_sys.stderr)
        py_sys.exit(1)

    # Build execution namespace with all r2x objects
    exec_namespace = {{
        "sys": system,
        "plugins": plugins,
        "pd": pd,
        "np": np,
        "plt": plt,
        "__name__": "__main__",
        "__file__": str(exec_path.resolve()),
    }}

    try:
        # Read and execute the script
        script_source = exec_path.read_text(encoding="utf-8")
        compiled = compile(script_source, str(exec_path), "exec")
        exec(compiled, exec_namespace, exec_namespace)
    except SystemExit:
        # Allow scripts to call sys.exit()
        raise
    except Exception:
        # Show full traceback on errors
        print(f"\nError executing script: {{exec_path}}\n", file=py_sys.stderr)
        traceback.print_exc()
        py_sys.exit(1)

    # If --interactive not set, exit after script completes
    if not interactive_after_exec:
        py_sys.exit(0)

    # Update system reference in case script modified it
    if "sys" in exec_namespace and exec_namespace["sys"] is not system:
        system = exec_namespace["sys"]

context = {{"sys": system, "plugins": plugins, "pd": pd, "np": np, "plt": plt}}
shell = InteractiveShellEmbed(config=cfg, banner1="", exit_msg="")
shell.prompts = R2XPrompts(shell)

# Register r2x magic commands
r2x_magics = R2XMagics(shell, plugins, system, JSON_PATH, DISPLAY_SOURCE)
shell.register_magic_function(r2x_magics.r2x_help, magic_kind="line", magic_name="r2x_help")
shell.register_magic_function(r2x_magics.plugins, magic_kind="line", magic_name="plugins")
shell.register_magic_function(r2x_magics.components, magic_kind="line", magic_name="components")
shell.register_magic_function(r2x_magics.export, magic_kind="line", magic_name="export")
shell.register_magic_function(r2x_magics.reload, magic_kind="line", magic_name="reload")
shell.register_magic_function(r2x_magics.run_plugin, magic_kind="line", magic_name="run_plugin")
shell.register_magic_function(r2x_magics.imports, magic_kind="line", magic_name="imports")

shell(
    header="",
    local_ns=context,
    global_ns=context,
)
"#,
        display_source_str, file_path_str
    );

    logger::debug("Generated Python initialization code");

    logger::debug("Launching interactive IPython session...");

    let ipython_dir = ensure_ipython_dir();
    let stdin_is_tty = atty::is(Stream::Stdin);
    let stdout_is_tty = atty::is(Stream::Stdout);
    let interactive_prompt = stdin_is_tty && stdout_is_tty;
    let (_tty_attached, stdin_tty, stdout_tty, stderr_tty) = acquire_tty_stdio();

    // Spawn IPython bootstrap script with interactive embed
    let mut command = Command::new(&python_exe);
    command
        .arg("-c")
        .arg(&python_code)
        .env("PYTHONUNBUFFERED", "1");

    command
        .stdin(stdin_tty)
        .stdout(stdout_tty)
        .stderr(stderr_tty);

    // Pass quiet and no-banner flags as environment variables
    if opts.quiet > 0 {
        command.env("R2X_READ_QUIET", "1");
    }
    if cmd.no_banner {
        command.env("R2X_READ_NO_BANNER", "1");
    }

    // Pass exec script path and interactive flag as environment variables
    if let Some(exec_path) = &cmd.exec {
        let exec_path_str = exec_path.to_str().ok_or("Invalid exec script path")?;
        command.env("R2X_READ_EXEC", exec_path_str);
    }
    if cmd.interactive {
        command.env("R2X_READ_INTERACTIVE", "1");
    }

    if interactive_prompt {
        command
            .env("PY_COLORS", "1")
            .env("CLICOLOR_FORCE", "1")
            .env("R2X_FORCE_SIMPLE_PROMPT", "0");
    } else {
        command.env("R2X_FORCE_SIMPLE_PROMPT", "1");
    }

    if std::env::var_os("TERM").is_none() {
        command.env("TERM", "xterm-256color");
    }

    if let Some(dir) = &ipython_dir {
        command.env("IPYTHONDIR", dir);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn IPython process: {}", e))?;

    logger::debug("IPython process spawned, waiting for completion");

    // Wait for IPython to finish
    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for IPython process: {}", e))?;

    if !status.success() {
        let exit_code = status.code().unwrap_or(-1);
        logger::debug(&format!("IPython exited with code: {}", exit_code));
        return Err(format!("IPython exited with code {}", exit_code).into());
    }

    logger::debug("IPython session completed successfully");
    Ok(())
}

fn ensure_prerequisites(
    config: &mut Config,
    python_exe: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_module_installed(config, python_exe, "IPython", "IPython", "IPython")?;
    let r2x_core_spec = config.get_r2x_core_package_spec();
    ensure_module_installed(
        config,
        python_exe,
        "r2x_core.system",
        &r2x_core_spec,
        "r2x-core",
    )?;
    Ok(())
}

#[cfg(unix)]
fn acquire_tty_stdio() -> (bool, Stdio, Stdio, Stdio) {
    let (stdin_attached, stdin) = match std::fs::File::open("/dev/tty") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stdout_attached, stdout) = match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stderr_attached, stderr) = match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    (
        stdin_attached || stdout_attached || stderr_attached,
        stdin,
        stdout,
        stderr,
    )
}

#[cfg(windows)]
fn acquire_tty_stdio() -> (bool, Stdio, Stdio, Stdio) {
    let (stdin_attached, stdin) = match std::fs::OpenOptions::new().read(true).open("CONIN$") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stdout_attached, stdout) = match std::fs::OpenOptions::new().write(true).open("CONOUT$") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stderr_attached, stderr) = match std::fs::OpenOptions::new().write(true).open("CONOUT$") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    (
        stdin_attached || stdout_attached || stderr_attached,
        stdin,
        stdout,
        stderr,
    )
}

fn ensure_module_installed(
    config: &mut Config,
    python_exe: &str,
    module_name: &str,
    package_spec: &str,
    display_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if module_exists(python_exe, module_name) {
        logger::debug(&format!("{} already available in venv", display_name));
        return Ok(());
    }

    logger::info(&format!(
        "{} not found in venv; installing via uv pip install",
        display_name
    ));

    install_package_with_spinner(config, python_exe, package_spec, display_name)?;
    Ok(())
}

fn module_exists(python_exe: &str, module_name: &str) -> bool {
    Command::new(python_exe)
        .arg("-c")
        .arg(format!("import {}", module_name))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn install_package_with_spinner(
    config: &mut Config,
    python_exe: &str,
    package_spec: &str,
    display_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let uv_path = config.ensure_uv_path()?;
    let mut install_cmd = Command::new(&uv_path);
    install_cmd
        .arg("pip")
        .arg("install")
        .arg("--python")
        .arg(python_exe)
        .arg("--no-progress")
        .arg(package_spec);

    logger::debug(&format!("Running: {:?}", install_cmd));
    // Print status without spinner since we need interactive terminal for SSH prompts
    logger::info(&format!("Installing {} into venv...", display_name));

    // Use inherited stdio to allow interactive prompts (e.g., SSH key passphrases)
    let status = install_cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| {
            logger::error(&format!("Failed to install {} into venv", display_name));
            format!("Failed to run uv pip install: {}", e)
        })?;

    if !status.success() {
        logger::error(&format!("Failed to install {} into venv", display_name));
        return Err(format!(
            "uv pip install {} failed: exit code {}",
            package_spec,
            status.code().unwrap_or(-1)
        )
        .into());
    }

    logger::success(&format!("Installed {} into venv", display_name));
    Ok(())
}

fn ensure_ipython_dir() -> Option<PathBuf> {
    let config_path = Config::path();
    if let Some(dir) = config_path.parent() {
        let ipython_dir = dir.join("ipython");
        if let Err(err) = fs::create_dir_all(&ipython_dir) {
            logger::debug(&format!(
                "Failed to create IPython dir {}: {}",
                ipython_dir.display(),
                err
            ));
            None
        } else {
            Some(ipython_dir)
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_command_creation() {
        let cmd = ReadCommand {
            file: None,
            no_banner: false,
            exec: None,
            interactive: false,
        };
        assert!(cmd.file.is_none());
        assert!(!cmd.no_banner);
        assert!(cmd.exec.is_none());
        assert!(!cmd.interactive);
    }

    #[test]
    fn test_read_command_with_file() {
        let cmd = ReadCommand {
            file: Some(PathBuf::from("test.json")),
            no_banner: false,
            exec: None,
            interactive: false,
        };
        assert!(cmd.file.is_some());
    }

    #[test]
    fn test_read_command_with_no_banner_flag() {
        let cmd = ReadCommand {
            file: None,
            no_banner: true,
            exec: None,
            interactive: false,
        };
        assert!(cmd.no_banner);
    }

    #[test]
    fn test_read_command_with_exec_flag() {
        let cmd = ReadCommand {
            file: Some(PathBuf::from("system.json")),
            no_banner: false,
            exec: Some(PathBuf::from("script.py")),
            interactive: false,
        };
        assert!(cmd.exec.is_some_and(|e| e == PathBuf::from("script.py")));
        assert!(!cmd.interactive);
    }

    #[test]
    fn test_read_command_with_interactive_flag() {
        let cmd = ReadCommand {
            file: None,
            no_banner: false,
            exec: Some(PathBuf::from("script.py")),
            interactive: true,
        };
        assert!(cmd.exec.is_some());
        assert!(cmd.interactive);
    }
}
