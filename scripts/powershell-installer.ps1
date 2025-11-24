#!/usr/bin/env powershell
# Custom PowerShell installer for r2x-cli
param([string]$ArchiveUrl)

# Install uv if not present (Windows)
if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    Write-Host "Installing uv..."
    try {
        # Try Astral's default PowerShell script
        irm https://astral.sh/uv/install.ps1 | iex
    } catch {
        try {
            # Fallback to winget
            winget install --id astral-sh.uv -e
        } catch {
            try {
                # Fallback to choco
                choco install uv -y
            } catch {
                Write-Host "Error: uv installation failed. Please install manually from https://docs.astral.sh/uv/getting-started/installation/" -ForegroundColor Red
                exit 1
            }
        }
    }
} else {
    Write-Host "uv already installed, skipping installation."
}

# Verify uv
if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    Write-Host "Error: uv not found after installation." -ForegroundColor Red
    exit 1
}

$InstallDir = "$env:USERPROFILE\.cargo\bin"
$TempDir = New-TemporaryDirectory

Write-Host "Installing r2x-cli to $InstallDir..."

# Download and extract the archive
Invoke-WebRequest -Uri $ArchiveUrl -OutFile "$TempDir\archive.zip"
Expand-Archive -Path "$TempDir\archive.zip" -DestinationPath $TempDir

# Ensure install dir exists
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Copy ALL extracted files (binary + included libs) to install dir
Copy-Item -Path "$TempDir\*" -Destination $InstallDir -Recurse

# Optional: Set executable permissions (PowerShell handles .exe)
Write-Host "Installation complete! Run 'r2x' (ensure ~/.cargo/bin is in your PATH)."
