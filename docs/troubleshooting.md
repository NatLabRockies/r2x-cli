---
title: Diagnose discovery, execution, and artifact failures
---

Start with the first failing boundary: launcher, Python environment, discovery, configuration, plugin execution, or artifact reload.
Keep the exit status and complete error. A previous output file does not prove the latest command succeeded.
Concrete checks below use the [two-step tutorial package](getting-started.md); use names returned by `r2x list` for your own package.

## `r2x` is missing, the runtime payload cannot start, or Python cannot load

```bash
r2x --version
r2x python show
r2x python path
```

If the shell cannot find `r2x`, verify the installer's PATH instructions and restart the shell; follow [Installation](installation.md).
If the launcher starts but cannot find its payload, keep `r2x-runtime` beside `r2x` from the same release archive.
If Python fails, compare the configured interpreter and paths reported by `show`/`path` with the binary's Python major.minor ABI.
Published binaries currently target Python 3.12; core's broader Python support does not make a CLI binary compatible with every interpreter.
The shared-library limitation is tracked in [issue #68](https://github.com/NatLabRockies/r2x-cli/issues/68).

Install the matching configured runtime with `r2x python install`, then repeat all three checks.
Do not select the newest Python as a generic fix for an ABI mismatch.
For source builds, match `R2X_PYTHON_VERSION` and `PYO3_PYTHON` as described in [Development](development.md).
If the error specifically says “Python executable not found at …”, confirm the reported path really is absent before using the supported `r2x venv create --yes`; reinstall the required plugins afterward and verify with `r2x list`.
Do not recreate a working environment before diagnosing which path or ABI failed.

<a id="the-tutorial-plugin-is-not-found"></a>

## “Plugin '…' not found in manifest” or `r2x list` omits the tutorial plugins

From `docs/examples/first-pipeline`:

```bash
r2x list r2x-tutorial
```

Expect `create` and `scale`. If the package is absent, install it into the CLI environment:

```bash
r2x install -e .
r2x list r2x-tutorial
```

`uv add` in another project does not install into this environment.
If the package is present but discovery is stale after metadata edits:

```bash
r2x sync
r2x run scale --help
```

Help must expose the installed transform's options, including its factor.
`sync` refreshes metadata; it only upgrades packages with `--upgrade`.
Check the package's actual `pyproject.toml`: the tutorial parser is registered in `r2x.parsers`, the function transform in `r2x.transforms`.
Use the installed entry-point reference, not an assumed Python class name.
If import fails after discovery, read that traceback and correct the package dependency/API mismatch; a manifest entry alone does not prove the plugin can import.
See [plugin management](plugin-management.md) and [runtime contracts](cli-reference.md#tutorial-plugin-contracts).

## Dry-run succeeds but the real pipeline rejects configuration

Dry-run prints the flow; it does not perform execution's package/configuration validation or load the model inputs.
Inspect actual references and resolved values:

```bash
r2x list r2x-tutorial
r2x run pipeline.yaml double-values --print
r2x run scale --help
```

For the tutorial, `factor` must be positive. `factor: 0` can pass dry-run yet fail typed configuration validation during execution.
Restore `factor: 2` in the tutorial YAML, then verify execution and independent reload:

```bash
r2x run pipeline.yaml double-values --output output/system.json
r2x read output/system.json --exec inspect_system.py
```

Both commands must succeed; the inspection prints `2 items; total=60.0`.
For another plugin, follow its own schema and supported values rather than borrowing tutorial settings.
If execution fails, do not inspect a stale output and call the run successful.
See [pipeline configuration](getting-started.md#read-the-real-pipeline).

## “No JSON data received from stdin” or conflicting input sources

This happens when a producer writes a file and its empty stdout is piped to a reader, or when no producer supplied JSON.
Read a file explicitly instead:

```bash
r2x run pipeline.yaml double-values --output output/system.json
r2x read output/system.json --exec inspect_system.py
```

Expect the tutorial's 60 total. For a ZIP, use `r2x read --zip ARCHIVE_PATH` with an actual saved archive.
“--zip requires a ZIP file path; it cannot be used with stdin” means the reader needs the archive path, not piped ZIP bytes.

“--input FILE cannot be combined with JSON supplied on stdin” means a direct plugin invocation received two input sources.
Choose either `--input FILE` or piped JSON; remove the other source and repeat the direct command using [the artifact handoff example](inspect-artifacts.md).
A native exporter may intentionally produce no system JSON; “produced no JSON output; use its plugin-specific output option instead” directs you to its generated help and model-specific output option.
Do not assume every successful plugin writes JSON to stdout.

## A file cannot be parsed, or a plugin raises a model validation error

First identify the input boundary. `r2x read` loads system artifacts; it is not a generic CSV, PLEXOS or Sienna parser.
For the tutorial's input artifact, this Python standard-library check inspects JSON syntax only; it does not invoke the CLI:

```bash
uv run python -m json.tool output/system.json > /dev/null
```

A zero exit status establishes JSON syntax only, not a valid or portable system.
For model source data, use that model's parser and check its reported path, required columns, units and time index against the package's documentation.
The CLI does not define model calendars, unit bases or CSV layouts; reinstalling the launcher cannot repair malformed source records.

Use `--log-python` on the failing run to retain Python diagnostics and read the log location reported by execution.
Correct the first concrete configuration/data error, rerun the smallest failing plugin, then verify the expected model/artifact invariants.
The tutorial's `inspect_system.py` checks its own two items and total; use model-specific checks for real translation work.
See [core troubleshooting](https://natlabrockies.github.io/r2x-core/troubleshooting/) for DataStore, unit and lifecycle failures.
Unavailable licensed/model inputs prevent reproducing that data path; report the missing prerequisite rather than claiming a recovery.

## Reload cannot import a component class or cannot find companion storage

Check package discovery in the receiving environment and inspect the artifact's neighboring files:

```bash
r2x list r2x-tutorial
```

For the tutorial, install its component-defining package from the fixture directory with `r2x install -e .` if missing.
For a real artifact, install the package that defined its component classes; core alone does not supply them.
An editable package also needs its source directory to remain available.

Keep `system.json` beside `system_time_series/` for the tutorial's file export, or transport the complete ZIP bundle using [artifact transfer](inspect-artifacts.md).
A missing sidecar requires restoring/regenerating the complete export. Do not patch stored class names or storage paths by hand.
Then run `r2x read output/system.json --exec inspect_system.py` in the receiving environment; a successful load and the expected assertions verify this boundary.
Native Sienna/PLEXOS exports need their corresponding parser and are not interchangeable with system JSON.

## `--pdb` fails in a pipe or CI

“--pdb requires an interactive terminal on stdin and stderr” is an intentional guard.
Use file-backed `--input FILE` in an interactive terminal so stdin is available for debugger commands, or remove `--pdb` in CI and collect `--log-python` diagnostics instead.
Repeat the original failing command in the supported mode; entering a debugger is not proof of a successful pipeline.
After fixing the underlying exception, rerun without the debugger and validate the artifact.
See [run options](cli-reference.md#run).

See the verified [read input guards](https://github.com/NatLabRockies/r2x-cli/blob/f7bdab75ce3cf494a3592b504625cd3c70877c09/crates/r2x-cli/src/commands/read.rs#L37-L93) and [direct-plugin input handling](https://github.com/NatLabRockies/r2x-cli/blob/f7bdab75ce3cf494a3592b504625cd3c70877c09/crates/r2x-cli/src/commands/run/plugin.rs#L298-L319) when reducing an orchestration failure.

## Report a reproducible bug

Include `r2x --version`, operating system/architecture, `r2x python show`, the relevant `r2x list PACKAGE` output, exact command and working directory, full error/traceback and reported log.
Provide minimal pipeline YAML, plugin package/version or editable revision, expected result and observed exit status.
For file transfer, state JSON versus ZIP, whether sidecars were included, and the component-defining package versions on both ends.
For model data, include a small synthetic failing record with headers, units and time index; identify unavailable licensed inputs explicitly.
Remove credentials and proprietary records from logs/configuration before sharing.
[Open a CLI issue](https://github.com/NatLabRockies/r2x-cli/issues) for launcher/orchestration failures, or the model package's issue for its data/schema constraints.
