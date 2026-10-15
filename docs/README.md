---
title: r2x-cli documentation
---

`r2x-cli` installs Python plugins and runs pipelines through the `r2x` executable.
Start with the tutorial, then use the reference to inspect the installed command surface.

## Tutorials

[Getting started](getting-started.md) walks through installation, workspace creation, and pipeline validation.

## How-to guides

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
Use `npm run preview` to inspect the production output locally.
The original Markdown filenames stay available in GitHub; the site supplies redirects for their `.md` links.
