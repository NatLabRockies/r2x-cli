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

Write-Host "Archive URL: $ArchiveUrl"
Write-Host "Checksum URL: $($ArchiveUrl).sha256"

# Download the archive and checksum
Invoke-WebRequest -Uri $ArchiveUrl -OutFile "$TempDir\archive.zip"
$ShaUrl = "$ArchiveUrl.sha256"
Invoke-WebRequest -Uri $ShaUrl -OutFile "$TempDir\archive.zip.sha256"

# Verify checksum
$expectedHash = (Get-Content "$TempDir\archive.zip.sha256").Trim().Split()[0].ToLower()
$computedHash = (Get-FileHash "$TempDir\archive.zip" -Algorithm SHA256).Hash.ToLower()
if ($computedHash -ne $expectedHash) {
    Write-Host "Checksum verification failed!" -ForegroundColor Red
    exit 1
}
Write-Host "Checksum verified."

# Extract the archive
Expand-Archive -Path "$TempDir\archive.zip" -DestinationPath $TempDir

# Ensure install dir exists
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Find source directory (handle potential subdirectory in archive)
$srcDir = $TempDir
$subdirs = Get-ChildItem $TempDir -Directory
if ($subdirs.Count -gt 0) {
    $srcDir = $subdirs[0].FullName
    Write-Host "Archive has subdirectory: $($subdirs[0].Name)"
}

# Copy only r2x.exe and python DLLs
Get-ChildItem $srcDir -File | Where-Object {
    $_.Name -eq 'r2x.exe' -or ($_.Name -like 'python*.dll')
} | ForEach-Object {
    Copy-Item $_.FullName $InstallDir
}

# Optional: Set executable permissions (PowerShell handles .exe)
Write-Host "Installation complete! Run 'r2x' (ensure ~/.cargo/bin is in your PATH)."
