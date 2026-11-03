---
title: r2x-cli CLI Reference
---

# r2x-cli CLI Reference

This page is a compact overview of the `r2x-cli` command surface. Run
`r2x --help` for the complete command tree and `r2x <command> --help` for
command-specific options.

Workflow-specific plugin options, parser settings, pipeline configuration, and
application configuration are intentionally not documented here. See the
relevant plugin or workflow documentation for those details.

## Common commands

| Goal | Command |
| --- | --- |
| Show the installed version | `r2x --version` |
| Initialize a workspace file | `r2x init` |
| Install a plugin | `r2x install <package>` |
| List installed plugins | `r2x list` |
| Show declared mappings | `r2x show <package>` |
| Run a target | `r2x run <target>` |
| Inspect a system | `r2x read <file>` |
| Repair the configured r2x-core package | `r2x core repair` |
| Update a standalone installation | `r2x self update` |

## Plugins

The common plugin commands are:

```bash
r2x install <package>
r2x install -e <path>
r2x list
r2x remove <package>
r2x sync
r2x sync --upgrade
r2x clean --yes
```

`r2x list <package>` describes its discovered plugin entry points.
Add a full plugin name or its final name component to select matching entry points.
Use `r2x show <package>` for the package's mapping declarations.

See [Plugin management](plugin-management.md) for package sources and plugin
lifecycle guidance.

## Show

`r2x show` lists declared file mappings, table mappings, and translation rules for an installed package.
Use a name from `r2x list` as the package argument:

```bash
r2x show <package>
r2x show <package> <item>
r2x show <package> table:<item>
r2x show <package> --config-dir <directory>
```

The inventory shows a qualified selector for each item when one is available.
Use `file:`, `table:`, or `rule:` to distinguish items with the same name, and quote selectors containing spaces.
A qualified item reads only its corresponding mapping file.
An unqualified item requires all three mapping files to be readable or absent, so the name can be disambiguated.
File and table mappings must be arrays of records with nonempty `name` fields.
Translation rules must be arrays of records with names or source and target types, or an empty object.
The output describes the selected JSON files, not computed data or application-owned transformations.
It rejects duplicate JSON keys but does not validate the complete mapping schemas.
Missing mapping files and empty mapping collections are reported separately.
An item without a unique selector cannot be shown individually, so read its source mapping file to distinguish it.

By default, the command imports discovered configuration classes and uses their public package-default configuration path.
Python module initialization may have side effects, but the command does not invoke the plugin or run a pipeline.
Use `--config-dir` to inspect a different configuration directory without importing those classes, or to choose between multiple configuration directories.

## Machine-readable output

Use `--json` with either command to return versioned JSON:

```bash
r2x list --json
r2x list <package> --json
r2x show <package> --json
r2x show <package> table:<item> --json
```

On success, the CLI writes one JSON object to stdout with `schema_version` set to `1`.
`list` returns a `packages` array containing package and plugin metadata.
`show` returns the configuration origin and path, plus either a mapping inventory or a selected `item` and its declaration.
Inventory assets use `present`, `empty`, or `not_provided` status.
Each inventory item includes a qualified `selector` for the next `show` call, or `null` if the declarations do not provide a unique selector.

On failure, the CLI exits nonzero and emits a JSON error object to stderr with `schema_version`, `error.code`, `error.message`, and `error.candidates`.
Common codes include `PACKAGE_NOT_FOUND`, `PLUGIN_NOT_FOUND`, `MAPPING_NOT_FOUND`, `AMBIGUOUS_MAPPING`, `AMBIGUOUS_CONFIGURATION`, `INVALID_JSON`, and `ENVIRONMENT_UNAVAILABLE`.
Candidates contain usable plugin names, mapping selectors, or configuration paths when available.
Plugin import and verbose diagnostics may also appear on stderr.
Python-level stdout from configuration class imports is redirected to stderr.
Python startup customization runs before that redirection and can cause startup errors or corrupt the JSON stream.
Native stdout writes from plugin initialization can also corrupt the JSON stream.
The `--json` mode requires an existing `uv` executable and compatible virtual environment.
It does not prompt, download dependencies, or create or repair the environment.
Default `show` discovery still imports plugin configuration classes, whose initialization can have side effects.
Treat mapping declarations, paths, and plugin metadata in the output as potentially sensitive.

## Run

`r2x run` accepts either a direct plugin reference or a file and target name.
Plugin-specific arguments are passed through after the target.

```bash
# Direct plugin mode
r2x run <plugin-ref> [PLUGIN_OPTIONS...]

# File and named-target mode
r2x run <file.yaml> <name>
```

Common file and named-target options:

| Option | Purpose |
| --- | --- |
| `--list` | List available targets in the file. |
| `--print` | Print the resolved target without executing it. |
| `-n`, `--dry-run` | Print the named pipeline flow without executing or validating plugin packages/configs. |
| `-o`, `--output <file>` | Write output to a file instead of stdout. |
| `--zip` | Save file-mode output as an infrasys ZIP archive. |
| `--pdb` | Open Python post-mortem debugging after an uncaught plugin failure. |

Dry-run still resolves each step against the discovery manifest; unknown or ambiguous references fail before execution.

Direct plugin options include:

```bash
r2x run <plugin-ref> --help
r2x run <plugin-ref> --<option> <value>
r2x run <plugin-ref> --input <file>
r2x run <plugin-ref> --output <file>
r2x run <plugin-ref> --repeat <N> --benchmark
r2x run <plugin-ref> --pdb --input <file>
```

Plugin help marks mandatory options as `[required]` and separates them from shared run options.

Pass plugin option values as separate arguments.
Option names accept both kebab-case and snake_case spellings.

`--pdb` requires an interactive terminal. Use `--input <file>` when debugging
a plugin so stdin remains available for debugger commands.

## Read

In interactive mode, `r2x read` prints a system overview before opening IPython for the system artifact:

```bash
r2x read <file.json>
r2x read <file.zip> --zip
r2x read --exec <script.py> <file.json>
r2x read --exec <script.py> --interactive <file.json>
```

Use `sys.info()` in the session to print the overview again.
When no file is provided, JSON can be read from stdin:

```bash
cat system.json | r2x read
```

To pipe plugin output into `r2x read`, replace `PLUGIN_REF` with an installed plugin reference and omit `-o` so JSON is written to stdout:

```bash
r2x run PLUGIN_REF | r2x read
```

Use `-o <file>` to save output to a file, then pass that file to `r2x read`.

Use `--no-banner` to suppress the interactive startup banner.

## Runtime commands

Runtime commands manage the Python interpreter and virtual environment used
by plugins.
`r2x core repair` checks the configured package and reinstalls it when missing
or incompatible, without requiring a system artifact.

```bash
r2x core repair
r2x python show
r2x python install
r2x python install <version>
r2x python path

r2x venv create --yes
r2x venv path
r2x venv path <new-path>
```

## Update

Standalone installer users can update in place:

```bash
r2x self update
r2x self update --dry-run
r2x self update <version>
```

The `upgrade` alias is also available:

```bash
r2x self upgrade
```

Installations managed by Cargo, Homebrew, or another package manager should be
updated with that package manager.

## Global options

These options can be used with commands that support the shared CLI options:

| Option | Purpose |
| --- | --- |
| `-q`, `--quiet` | Reduce informational output. Repeat as `-qq` to suppress plugin stdout. |
| `-v`, `--verbose` | Increase diagnostic output. Repeat as `-vv` for trace output. |
| `--log-python` | Show Python logs on the console. |
| `--no-stdout` | Do not write captured plugin stdout to the log. |

## Help

Use the built-in help for the complete surface, including configuration and
logging commands that are intentionally outside this compact reference:

```bash
r2x --help
r2x <command> --help
```

## Source contracts

The command contracts in this guide were checked against current main.
Inspect the pinned [root commands](https://github.com/NatLabRockies/r2x-cli/blob/c3fada787247ff671f3366e72fdf51b867c41ed3/crates/r2x-cli/src/main.rs) and [run argument definitions](https://github.com/NatLabRockies/r2x-cli/blob/c3fada787247ff671f3366e72fdf51b867c41ed3/crates/r2x-cli/src/commands/run/mod.rs) for exact parser types and defaults.
Use the installed binary's help when working with a different release.
`--repeat` accepts a positive integer and defaults to `1`.
Direct plugin execution reads JSON from stdin unless `--input` is supplied and writes its result to stdout unless `--output` is supplied.
Plugin-specific required values, types, and defaults come from the discovered plugin contract.
Invalid arguments, discovery failures, and plugin failures report diagnostics and exit unsuccessfully.

## Pipeline configuration

The [tested fixture](https://github.com/NatLabRockies/r2x-cli/blob/main/docs/examples/first-pipeline/pipeline.yaml) uses these top-level fields:

| Field | Meaning |
| --- | --- |
| `pipelines` | Map of pipeline names to ordered plugin-reference lists. |
| `config` | Map from plugin references to per-plugin configuration values. |
| `variables` | Optional substitutions resolved in configuration values. |
| `output_folder` | Optional default output folder passed into plugin configuration handling. |

`--list` reads names and ordered steps from the YAML.
`--print` resolves and displays the selected pipeline's settings.
`--dry-run` displays flow only; it does not run the package/configuration validation used by actual execution.
The executing path verifies plugins, validates their configurations, and then invokes steps in order.
See [the source dispatcher](https://github.com/NatLabRockies/r2x-cli/blob/c3fada787247ff671f3366e72fdf51b867c41ed3/crates/r2x-cli/src/commands/run/pipeline/mod.rs) and [configuration schema](https://github.com/NatLabRockies/r2x-cli/blob/c3fada787247ff671f3366e72fdf51b867c41ed3/crates/r2x-cli/src/commands/run/pipeline/config.rs).

## Tutorial plugin contracts

These are real APIs in the shipped `r2x-tutorial` fixture, not application-specific model APIs:

These lines are signature notation, not executable calls.

```python
CreateDemo.from_context(ctx: PluginContext[CreateConfig]) -> Plugin[CreateConfig]
scale_values(system: System, config: ScaleConfig) -> Result[System, str]
```

| Configuration | Field | Type/default | Behavior |
| --- | --- | --- | --- |
| `CreateConfig` | `name` | `str = "demo"` | Name of the newly built system. |
| `ScaleConfig` | `factor` | `float = 2`, strictly positive | Multiplies the values of existing `Item` components. |

`create` returns a context containing a new system with two components, initially valued 10 and 20.
`scale_values` mutates that system and returns it in `Ok`; its configuration rejects zero or negative factors.
The transform does not perform a model-schema translation and does not add electrical meaning to the fixture's numeric values.
The entry-point group determines the runtime role; explicit `r2x.parsers` and `r2x.transforms` groups avoid legacy name-based role inference.
See [the fixture implementation](https://github.com/NatLabRockies/r2x-cli/blob/main/docs/examples/first-pipeline/src/r2x_tutorial/plugin.py) and [runtime role mapping](https://github.com/NatLabRockies/r2x-cli/blob/c3fada787247ff671f3366e72fdf51b867c41ed3/crates/r2x-manifest/src/runtime.rs).
