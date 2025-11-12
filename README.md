# r2x-cli

A comprehensive framework plugin manager for the r2x power systems modeling ecosystem. Simplifies discovery, installation, and management of r2x framework plugins.

## 🚧 Ongoing Refactoring TODO

We're currently refactoring the plugin discovery and manifest system to be more dynamic and maintainable:

### Completed ✅
- [x] Simplified `DiscoveryPlugin` structure - no hardcoded fields, uses `constructor_args` + `resolved_references`
- [x] Added dependency tracking to `Package` (`install_type`, `installed_by`, `dependencies`)
- [x] Implemented AST parser Phase 1 - extracts all constructor args dynamically
- [x] Implemented AST parser Phase 2 methods - resolves class/function references
- [x] Refactored `AstDiscovery` - returns structs instead of JSON
- [x] Reorganized manifest structure:
  - `types.rs` - pure type definitions
  - `manifest.rs` - all manifest operations (load, save, business logic)
  - `manifest_writer.rs` - custom path utilities for testing
- [x] Removed transitional `Plugin` struct

### In Progress 🔄
- [ ] Refactor `plugins/discovery.rs` to use new struct-based flow (no JSON parsing)
- [ ] Implement AST parser Phase 3 - associate decorators with plugins
- [ ] Update Python bridge (`plugin_invoker.rs`) to construct Python objects from `DiscoveryPlugin.constructor_args`
- [ ] Update `commands/run.rs` to work directly with `DiscoveryPlugin`
- [ ] Update `help.rs` to work directly with `DiscoveryPlugin`
- [ ] Remove `plugin_parser.rs` (parse_plugin_json, parse_decorator_registrations) - no longer needed
- [ ] Remove `plugin_manifest.rs` entirely
- [ ] Update `plugin_cache.rs` to work with `DiscoveryPlugin`

### Architecture Goals
- **No intermediate representations**: Work directly with manifest data
- **Python bridge constructs objects**: From `constructor_args` + `resolved_references`
- **Manifest is source of truth**: No conversions, no hardcoded field assumptions
- **Dependency tracking**: Proper cleanup when packages are removed

## Features

- Easy plugin management
- Built-in package resolution
- Python integration support

## Python Package Management

This project uses [uv](https://github.com/astral-sh/uv) for handling Python packages. With uv, you don't need to worry about Python or plugin versioning—uv automatically handles dependency resolution and environment isolation for you.

## License

BSD-3-Clause License. See [LICENSE.txt](LICENSE.txt) for details.

## Commands

### Getting Started

```bash
# Initialize a new pipeline file in the current directory
r2x init

# Initialize with custom filename
r2x init my-pipeline.yaml
```

The `r2x init` command creates a template pipeline file with:
- Example variables for substitution
- Multiple pipeline examples
- Plugin configuration templates
- Comments explaining all features

You can then edit the file to configure your own pipelines.

### Python & Virtual Environment Management

```bash
# Install or update Python version
r2x python install 3.13

# Get the Python executable path (useful for scripting)
r2x python path

# Show current Python configuration
r2x python show

# Manage virtual environment
r2x venv              # Recreate venv (prompts for confirmation)
r2x venv -y/--yes     # Skip confirmation
r2x venv path         # Show venv path

# Get or set custom venv path
r2x venv path /path/to/custom/venv

# Automated venv recreation
R2X_VENV_YES=1 r2x venv  # Skip confirmation with environment variable
```

**Install packages in your managed venv:**
```bash
uv pip install <package> --python $(r2x python path)
```

### Pipeline Management

```bash
# List available pipelines
r2x run pipeline.yaml --list

# Execute a pipeline
r2x run pipeline.yaml my-pipeline

# Show pipeline flow without executing (--dry-run)
r2x run pipeline.yaml my-pipeline --dry-run

# Save pipeline output to file
r2x run pipeline.yaml my-pipeline -o output.json
```

The `--dry-run` flag displays which plugins produce/consume stdout, helping you understand data flow between pipeline stages before execution.

### Plugin Management

```bash
# List installed plugins
r2x list

# Install a plugin
r2x install <package>

# Run a plugin directly
r2x run plugin my-plugin [args...]

# Show plugin help
r2x run plugin my-plugin --show-help
```

### System Integration

```bash
# Load and inspect a JSON system file
r2x read system.json

# Load from stdin and open IPython with system available
cat system.json | r2x read
```
