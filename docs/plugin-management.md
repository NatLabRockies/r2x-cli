---
title: r2x-cli Plugin Management
---

# r2x-cli Plugin Management

`r2x-cli` installs Python plugins into its managed virtual environment and records
plugin entry points in a local manifest. Plugin discovery uses static analysis,
so listing and syncing plugins does not import plugin modules.

## Install a Plugin

```bash
r2x install <plugin-name>
```

Install from a specific branch, tag, or commit:

```bash
r2x install "git+https://github.com/NatLabRockies/r2x-plexos.git@<branch-name>"
r2x install "git+https://github.com/NatLabRockies/<repo-name>.git@<branch-name>#subdirectory=<path>"
```

Use the `gh:owner/repo` shorthand with `--branch`, `--tag`, or `--commit`.
These options require a Git URL or `gh:` shorthand and do not apply to plain
PyPI package names:

```bash
r2x install --branch <branch-name> gh:NatLabRockies/<repo-name>
r2x install --tag <tag-name> gh:NatLabRockies/<repo-name>
r2x install --commit <hash> gh:NatLabRockies/<repo-name>
r2x install -e <plugin-name>          # editable/development mode
r2x install --no-cache <plugin-name>  # skip cached plugin metadata
```

## Inspect Installed Plugins

```bash
r2x list
```

Inspect the plugins discovered in a package, optionally filtered by module:

```bash
r2x list <plugin-name>
```

## Refresh the Plugin Manifest

Re-run plugin discovery for all installed packages:

```bash
r2x sync
```

Upgrade installed packages and sync metadata:

```bash
r2x sync --upgrade
```

## Remove a Plugin

```bash
r2x remove <plugin-name>
```

> [!NOTE]
> To update a plugin to a specific branch, tag, or git version, use
> `--no-cache` to bypass cached metadata and force the new ref to be resolved:
>
> ```bash
> r2x install --no-cache "git+https://github.com/NatLabRockies/<repo-name>.git@<branch-name>"
> ```

## Remove All Plugins and Clean Cache

```bash
r2x clean
r2x clean --yes   # skip confirmation prompt
```
