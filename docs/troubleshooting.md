---
title: Diagnose CLI failures
---

## Check the launcher and runtime

```bash
r2x --version
r2x python show
r2x python path
```

If the launcher is missing, check the installer directory on `PATH`.
Keep `r2x-runtime` beside `r2x` when moving an installation.
The configured Python major.minor version must match the binary's ABI.
After installing the matching Python version, recreate the managed environment with `r2x venv create --yes`.
See [development troubleshooting](development.md#troubleshooting) for source builds and shared-library failures.

## A plugin is missing or its options changed

```bash
r2x list
r2x sync
r2x list
```

`sync` refreshes discovery metadata; it does not upgrade packages unless `--upgrade` is passed.
Use the reference returned by `list` in `r2x run PLUGIN_REF --help`.
Confirm the plugin is installed in the CLI's managed environment rather than only in a separate project environment.

## A pipeline fails

```bash
r2x run pipeline.yaml --list
r2x run pipeline.yaml my-pipeline --print
r2x run pipeline.yaml my-pipeline --dry-run
```

Replace `my-pipeline` with a name returned by `--list`.
Inspect the resolved configuration before executing plugins.
A dry run checks the planned execution; it does not prove that input data is readable or translation succeeds.
Use `--log-python` to expose Python diagnostics when reproducing the failure.
For interactive post-mortem debugging, see [the run reference](cli-reference.md#run).
