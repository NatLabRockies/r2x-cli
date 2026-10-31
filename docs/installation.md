---
title: Installation
---

Install a published binary release of `r2x-cli`; Rust is not required.
Use the installer for your platform:

Install the latest CLI release on macOS/Linux:

```bash
curl -fsSL https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.sh | sh
```

Windows users can use the official PowerShell installer:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.ps1 | iex"
```

Restart your shell if the installer added a directory to `PATH`.

```bash
r2x --version
```

Expect a version string. For platform archives and release details, see the [official releases](https://github.com/NatLabRockies/r2x-cli/releases).
Keep `r2x` and its `r2x-runtime` payload together when relocating an installation.
Continue with [Python environment setup](environment.md), then [Verify the CLI](verify-installation.md).

To modify the CLI or build for another Python ABI, use [Development](development.md).
