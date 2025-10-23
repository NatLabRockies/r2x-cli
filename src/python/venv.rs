use crate::{R2xError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

pub fn ensure_venv(uv_path: &Path) -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir().ok_or(R2xError::NoCacheDir)?;
    let venv_path = cache_dir.join("r2x").join("venv");

    if venv_path.exists() {
        debug!("Using existing venv: {:?}", venv_path);
        return Ok(venv_path);
    }

    create_venv(uv_path, &venv_path)?;
    install_r2x_core(uv_path, &venv_path)?;

    Ok(venv_path)
}

fn create_venv(uv_path: &Path, venv_path: &Path) -> Result<()> {
    info!("Creating Python virtual environment...");

    let status = Command::new(uv_path)
        .args(["venv", venv_path.to_str().unwrap(), "--python", "3.11"])
        .status()
        .map_err(|e| R2xError::VenvError(format!("Failed to create venv: {}", e)))?;

    if !status.success() {
        return Err(R2xError::VenvError(format!(
            "Venv creation failed with exit code: {}",
            status.code().unwrap_or(-1)
        )));
    }

    Ok(())
}

fn install_r2x_core(uv_path: &Path, venv_path: &Path) -> Result<()> {
    info!("Installing r2x-core...");

    let python_exe = if cfg!(windows) {
        venv_path.join("Scripts/python.exe")
    } else {
        venv_path.join("bin/python3")
    };

    let status = Command::new(uv_path)
        .args([
            "pip",
            "install",
            "r2x-core",
            "--python",
            python_exe.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| R2xError::VenvError(format!("Failed to install r2x-core: {}", e)))?;

    if !status.success() {
        return Err(R2xError::VenvError(format!(
            "r2x-core installation failed with exit code: {}",
            status.code().unwrap_or(-1)
        )));
    }

    Ok(())
}

pub fn get_site_packages(venv_path: &Path) -> Result<PathBuf> {
    let site_packages = if cfg!(windows) {
        venv_path.join("Lib/site-packages")
    } else {
        venv_path.join("lib/python3.11/site-packages")
    };

    if !site_packages.exists() {
        return Err(R2xError::VenvError(format!(
            "Site-packages not found: {:?}",
            site_packages
        )));
    }

    Ok(site_packages)
}

pub fn get_venv_path() -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir().ok_or(R2xError::NoCacheDir)?;
    Ok(cache_dir.join("r2x").join("venv"))
}

pub fn get_venv_python(venv_path: &Path) -> Result<PathBuf> {
    let python_exe = if cfg!(windows) {
        venv_path.join("Scripts/python.exe")
    } else {
        venv_path.join("bin/python3")
    };

    if !python_exe.exists() {
        return Err(R2xError::VenvError(format!(
            "Python executable not found at: {}",
            python_exe.display()
        )));
    }

    Ok(python_exe)
}
