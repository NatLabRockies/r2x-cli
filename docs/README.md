---
title: r2x-cli documentation
---

`r2x-cli` is a Rust-based CLI. Its `r2x` executable installs Python plugins and runs pipelines.
Invoke the CLI as `r2x`; Python code in these guides implements plugins or inspects system data.
Start with [Installation](installation.md), [Python environment setup](environment.md), and [Verify the CLI](verify-installation.md), then follow the tutorial.

## Tutorials

[Run your first pipeline](getting-started.md) installs a real two-step fixture, executes it, and checks the saved values with `r2x read`.

## How-to guides

- [Inspect and transfer artifacts](inspect-artifacts.md): direct file boundaries, ZIP archives, and repeatable component checks.
- [Manage plugins](plugin-management.md): install, inspect, refresh, and remove packages.
- [Development](development.md): build the launcher and runtime and run repository checks.

## Reference/API

[CLI reference](cli-reference.md) covers commands, shared flags, and artifact inspection.
Plugin arguments belong to each installed plugin; inspect them with `r2x run PLUGIN_REF --help`.

## Explanation

[Architecture](architecture.md) explains discovery, managed Python, and artifact boundaries.

## Troubleshooting

[Diagnose failures](troubleshooting.md) separates launcher, runtime, discovery, and pipeline errors.

## Changelog

[Release history and versions](changelog.md) links to canonical releases and tagged source documentation.

## Contributors

[Development](development.md) covers source builds and repository checks.

## Build this site

From the repository root, use Node.js 22.12 or newer and npm:

```bash
cd docs
npm ci
npm run build
```

The build writes `dist/`, including the Pagefind search index.
Mermaid diagrams describe the actual tutorial, invocation, and artifact flow.
The `examples/first-pipeline/` package supplies the executable tutorial fixture.
Use `npm run preview` to inspect the production output locally.
The original Markdown filenames stay available in GitHub; the site supplies redirects for their `.md` links.
