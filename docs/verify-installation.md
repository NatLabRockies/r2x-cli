---
title: Verify the CLI and first use
---

After [Installation](installation.md) and [Python environment setup](environment.md):

```bash
r2x --version
r2x --help
r2x python show
r2x list
```

Expect the launcher version, available commands, runtime paths, and plugin discovery output.
A fresh environment may have no translator plugins; that is expected before installing a package.
If the launcher cannot find its runtime payload or Python, consult [Troubleshooting](troubleshooting.md).

Continue with [Run and inspect your first two-step pipeline](getting-started.md).
It installs a small real plugin package, doubles values 10 and 20, saves an artifact, and checks the reloaded total of 60.
For an existing translator package, follow [Manage plugins](plugin-management.md) instead.
