use crate::{R2xError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

pub fn ensure_uv() -> Result<PathBuf> {
    if let Ok(output) = Command::new("which").arg("uv").output() {
        if output.status.success() {
            let system_uv = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !system_uv.is_empty() {
                let uv_path = PathBuf::from(system_uv);
                info!("Using system UV: {:?}", uv_path);
                return Ok(uv_path);
            }
        }
    }

    let cache_dir = dirs::cache_dir().ok_or(R2xError::NoCacheDir)?.join("r2x");

    let bin_dir = cache_dir.join("bin");
    let uv_path = bin_dir.join(if cfg!(windows) { "uv.exe" } else { "uv" });

    if uv_path.exists() {
        debug!("UV found at: {:?}", uv_path);
        return Ok(uv_path);
    }

    info!("UV not found, downloading...");
    download_uv(&bin_dir)?;

    Ok(uv_path)
}

fn download_uv(bin_dir: &Path) -> Result<()> {
    fs::create_dir_all(bin_dir)?;

    let platform = get_platform_string()?;
    let url = format!(
        "https://github.com/astral-sh/uv/releases/latest/download/uv-{}.tar.gz",
        platform
    );

    info!("Downloading UV from: {}", url);

    let response = reqwest::blocking::get(&url)?;
    let bytes = response.bytes()?;

    let tar = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(tar);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;

        if path.file_name() == Some(std::ffi::OsStr::new("uv"))
            || path.file_name() == Some(std::ffi::OsStr::new("uv.exe"))
        {
            let dest = bin_dir.join(path.file_name().unwrap());
            entry.unpack(&dest)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&dest)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&dest, perms)?;
            }

            info!("UV installed to: {:?}", dest);
            return Ok(());
        }
    }

    Err(R2xError::UvDownload(
        "UV binary not found in archive".to_string(),
    ))
}

fn get_platform_string() -> Result<String> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err(R2xError::UnsupportedPlatform),
    };

    Ok(platform.to_string())
}

pub fn ensure_python(uv_path: &Path, version: &str) -> Result<()> {
    let output = Command::new(uv_path)
        .args(["python", "list"])
        .output()
        .map_err(|e| R2xError::VenvError(format!("Failed to list Python versions: {}", e)))?;

    let list_str = String::from_utf8_lossy(&output.stdout);

    if list_str.contains(&format!("cpython-{}", version)) {
        debug!("Python {} already installed", version);
        return Ok(());
    }

    info!("Installing Python {}...", version);
    let status = Command::new(uv_path)
        .args(["python", "install", version])
        .status()
        .map_err(|e| R2xError::VenvError(format!("Failed to install Python: {}", e)))?;

    if !status.success() {
        return Err(R2xError::VenvError(format!(
            "Python installation failed with exit code: {}",
            status.code().unwrap_or(-1)
        )));
    }

    Ok(())
}

pub fn get_python_path(uv_path: &Path, version: &str) -> Result<PathBuf> {
    let output = Command::new(uv_path)
        .args(["python", "find", version])
        .output()?;

    if !output.status.success() {
        return Err(R2xError::PythonInstall(format!(
            "Python {} not found",
            version
        )));
    }

    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(path_str))
}
