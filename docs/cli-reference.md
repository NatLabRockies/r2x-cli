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
| Run a target | `r2x run <target>` |
| Inspect a system | `r2x read <file>` |
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

See [Plugin management](plugin-management.md) for package sources and plugin
lifecycle guidance.

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
| `-n`, `--dry-run` | Validate without executing. |
| `-o`, `--output <file>` | Write output to a file instead of stdout. |
| `--zip` | Save file-mode output as an infrasys ZIP archive. |
| `--pdb` | Open Python post-mortem debugging after an uncaught plugin failure. |

Direct plugin options include:

```bash
r2x run plugin <plugin-ref> --show-help
r2x run plugin <plugin-ref> --input <file>
r2x run plugin <plugin-ref> --output <file>
r2x run plugin <plugin-ref> --repeat <N> --benchmark
r2x run plugin <plugin-ref> --pdb --input <file>
```

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

The runtime commands manage the Python interpreter and virtual environment
used by plugins:

```bash
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
