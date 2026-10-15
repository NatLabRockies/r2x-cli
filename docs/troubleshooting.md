---
title: Diagnose discovery, execution, and artifact failures
---

Start from the symptom, then repeat the smallest check that separates environment, metadata, configuration, and data failures.
Commands below use [the runnable tutorial fixture](getting-started.md).

## The tutorial plugin is not found

From `docs/examples/first-pipeline`, inspect the CLI's managed environment:

```bash
r2x list r2x-tutorial
```

If `create` and `scale` are missing, install the fixture there:

```bash
r2x install -e .
r2x list r2x-tutorial
```

Installing with `uv add` in a separate project does not install a package into the CLI's environment.
After changing entry points or configuration metadata, refresh discovery and inspect the generated help:

```bash
r2x sync
r2x run scale --help
```

`sync` refreshes metadata; it only upgrades packages when `--upgrade` is supplied.
The tutorial uses explicit entry-point groups: the parser is `r2x.parsers`, and the function transform is `r2x.transforms`.
See [plugin management](plugin-management.md) and [runtime contracts](cli-reference.md#tutorial-plugin-contracts).

## Dry-run worked, but execution fails

A dry run prints the flow without loading inputs or performing the executing path's package/config validation.
Check the actual references with `r2x list`, and inspect resolved values:

```bash
r2x run pipeline.yaml double-values --print
```

The tutorial's `factor` must be positive.
Setting it to zero can still produce a dry-run diagram, but actual execution rejects its typed configuration before running the pipeline steps.
Restore `factor: 2`, execute again, and rerun `r2x read --exec inspect_system.py output/system.json`.
A file left over from a previous successful run does not prove the latest command succeeded.

## Read reports no JSON on stdin

Do not pipe a command's empty stdout after choosing an output file.
Read that file explicitly:

```bash
r2x run pipeline.yaml double-values --output output/system.json
r2x read output/system.json --exec inspect_system.py
```

For a ZIP, add `--zip` to `read`.
See [artifact inspection and transfer](inspect-artifacts.md).

## Read cannot import a component class or find storage

The loading environment needs the package that defines the stored components.
For the tutorial, reinstall the editable package from its source directory with `r2x install -e .`.
For a real workflow, install its model package in the receiving CLI environment.
Keep `system.json` beside `system_time_series/`, or transport the ZIP archive.
Native Sienna/PLEXOS exports are not interchangeable with infrasys-compatible system artifacts.
Use the appropriate model parser for native files.

## The launcher or Python runtime fails

```bash
r2x --version
r2x python show
r2x python path
```

A missing command is a PATH/installation issue; keep `r2x-runtime` beside the launcher.
A Python major.minor ABI mismatch requires a runtime matching the binary, not simply the newest installed Python.
Only recreate the managed environment after selecting the matching runtime, then reinstall its plugins.
The first inspection command may need network access to install IPython and other inspection dependencies.
See [development troubleshooting](development.md#troubleshooting) for source builds and shared libraries.

## A plugin raises an exception

Use `--log-python` for Python diagnostics and the log path reported by execution.
For an interactive post-mortem session, use `--pdb` with a file-backed input so stdin remains available to the debugger.
Correct the failed plugin's input/configuration and verify the resulting artifact again.
See [run options](cli-reference.md#run); the CLI cannot infer whether a model-specific transformation is scientifically valid.
