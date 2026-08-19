# Plugin Management

## Dependency Versions

Keep packages up to date by checking their GitHub repositories.

## Install a Plugin

```bash
r2x install <plugin-name>
```

Install from a specific branch, tag, or commit:

```bash
r2x install "git+https://github.com/NatLabRockies/r2x-plexos.git@<branch-name>"
r2x install "git+https://github.com/NatLabRockies/R2X.git@<branch-name>#subdirectory=packages/r2x-reeds-to-plexos"
```

Use the `gh:owner/repo` shorthand with `--branch`, `--tag`, or `--commit` flags (requires a git URL or `gh:` shorthand — does **not** work with plain PyPI package names):

```bash
r2x install --branch <branch-name> gh:NatLabRockies/<repo-name>
r2x install --tag <tag-name> gh:NatLabRockies/<repo-name>
r2x install --commit <hash> gh:NatLabRockies/<repo-name>
r2x install -e <plugin-name>          # editable/development mode
r2x install --no-cache <plugin-name>  # skip cached plugin metadata
```

## List Installed Plugins

```bash
r2x list
```

Inspect a specific plugin's config entries:

```bash
r2x list <plugin-name>
```

## Sync Plugin Manifest

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

> **Note:** To update a plugin to a specific branch, tag, or git version, you must **remove it first** and then reinstall. Simply re-running `r2x install` on an already-installed plugin will not update it.
>
> ```bash
> r2x remove <plugin-name>
> r2x install "git+https://github.com/NatLabRockies/<repo-name>.git@<branch-name>"
> ```

## Remove All Plugins and Clean Cache

```bash
r2x clean
r2x clean --yes   # skip confirmation prompt
```
