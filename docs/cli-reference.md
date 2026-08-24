# CLI Reference

## Running a Translation

### Initialize a new pipeline

```bash
r2x init
```

### Run a pipeline

```bash
r2x run <file-name>.yaml <pipeline-name>
```

The `<pipeline-name>` is the key under the `pipelines` entry in the YAML file.

**Common run flags:**

| Flag | Description |
|------|-------------|
| `--log-python` | Stream Python logs to the console |
| `-n` / `--dry-run` | Validate the pipeline without executing it |
| `--list` | List all pipelines defined in the YAML file |
| `--print` | Print the resolved pipeline config without running |
| `-o <file>` | Write output to a file instead of stdout |
| `--pdb` | Enter post-mortem PDB on an uncaught plugin exception (interactive terminals only) |

### Run a single plugin directly

```bash
r2x run plugin <plugin-name>               # run with interactive config
r2x run plugin <plugin-name> --show-help   # show plugin-specific help
r2x run plugin <plugin-name> --pdb --input input.json  # debug an uncaught exception
r2x run plugin <plugin-name> --benchmark   # print benchmark summary
r2x run plugin <plugin-name> --repeat <N>  # run N times
```

## Post-mortem debugging

Pass `--pdb` to a direct plugin or pipeline run to enter Python's
post-mortem debugger for an uncaught plugin exception. In a pipeline, the
failure is debugged in the failing plugin and the pipeline stops there.
Continue or quit PDB to return the original failure and nonzero exit status.

`--pdb` requires an interactive stdin and stderr and fails immediately in
piped or CI execution. Debugger prompts are written to stderr. For direct
plugin debugging, prefer `--input FILE` so stdin remains available for PDB
commands. Exceptions caught by plugin code do not start a debugger.

## Pipeline YAML Structure

A pipeline file has four top-level sections:

```yaml
variables:
  # reusable values

pipelines:
  # named plugin step lists

config:
  # per-plugin configuration

output_folder: ${some_variable}
```

`r2x-cli` supports variable substitution in strings using both `${var}` and `$(var)`.

### Naming convention for plugin steps

In `pipelines`, each step uses the fully-qualified plugin identifier:

`<package-name>.<plugin-name>`

Examples:

- `r2x-reeds.reeds-parser`
- `r2x-sienna.sienna-parser`
- `r2x-sienna.sienna-exporter`
- `r2x-plexos.plexos-parser`
- `r2x-plexos.plexos-exporter`

The same identifier must be used as the key under `config`.

When no other installed plugin shares the same name, the short form (plugin name only) is equivalent:

```yaml
pipelines:
  r2s:
    - r2x-reeds.reeds-parser
    - break-gens        # equivalent to r2x-reeds.break-gens when unambiguous
```

## Configuration

```bash
r2x config
r2x config show
```

Set a configuration value:

```bash
r2x config set <key> <value>
# Example:
r2x config set python-version 3.13
```

Show or set the config file path:

```bash
r2x config path
r2x config path <new-path>
```

Reset to defaults:

```bash
r2x config reset
r2x config reset --yes   # skip confirmation prompt
```

## Log Management

```bash
r2x log
r2x log show
```

Show or set the log file path:

```bash
r2x log path
r2x log path <new-path>
```

Update logging settings:

```bash
r2x log set log-python true       # enable Python logs on console by default
r2x log set log-python false
r2x log set no-stdout true        # capture plugin stdout in logs by default
r2x log set max-size <bytes>      # e.g., 26214400 for 25 MiB
```

## Cache

```bash
r2x cache clean
r2x cache path
r2x cache path <new-path>
```

## Reading a System

Load a Sienna system from a JSON file and open an interactive IPython session:

```bash
r2x read <system.json>
r2x read                             # read from stdin
r2x read --no-banner <system.json>   # suppress the startup banner
```

Execute a Python script against the loaded system:

```bash
r2x read --exec <script.py> <system.json>
```

Drop into an interactive IPython session after running a script:

```bash
r2x read -i --exec <script.py> <system.json>
```
