use crate::{R2xError, Result};
use std::path::Path;
use tracing::debug;

pub fn setup_environment(venv_path: &Path) -> Result<()> {
    let python_exe = get_venv_python_exe(venv_path)?;
    let python_home = resolve_python_home(&python_exe)?;

    std::env::set_var("PYTHONHOME", python_home);
    std::env::set_var("VIRTUAL_ENV", venv_path);

    debug!("Python environment configured: venv={:?}", venv_path);
    Ok(())
}

fn get_venv_python_exe(venv_path: &Path) -> Result<std::path::PathBuf> {
    let python_exe = if cfg!(windows) {
        venv_path.join("Scripts/python.exe")
    } else {
        venv_path.join("bin/python3")
    };

    if !python_exe.exists() {
        return Err(R2xError::PythonInit(format!(
            "Python executable not found at: {}",
            python_exe.display()
        )));
    }

    Ok(python_exe)
}

fn resolve_python_home(python_exe: &Path) -> Result<std::path::PathBuf> {
    let real_python = std::fs::canonicalize(python_exe)?;
    let python_home = real_python
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            R2xError::PythonInit(format!(
                "Cannot determine Python home from: {}",
                python_exe.display()
            ))
        })?;

    Ok(python_home.to_path_buf())
}

pub fn verify_python(venv_path: &Path) -> Result<()> {
    use std::process::Command;

    let python_exe = get_venv_python_exe(venv_path)?;

    // Check Python version
    let output = Command::new(&python_exe)
        .arg("-c")
        .arg("import sys; print(sys.version)")
        .output()
        .map_err(|e| R2xError::PythonInit(format!("Failed to run Python: {}", e)))?;

    if !output.status.success() {
        return Err(R2xError::PythonInit(
            "Python version check failed".to_string(),
        ));
    }

    let version = String::from_utf8(output.stdout)
        .map_err(|e| R2xError::PythonInit(format!("Invalid UTF-8 in version output: {}", e)))?
        .trim()
        .to_string();

    if !version.starts_with("3.11") {
        return Err(R2xError::PythonInit(format!(
            "Wrong Python version! Expected 3.11, got: {}",
            version
        )));
    }

    debug!(
        "Python version verified: {}",
        version.split_whitespace().next().unwrap_or("3.11")
    );

    // Check r2x_core import
    let output = Command::new(&python_exe)
        .arg("-c")
        .arg("import r2x_core; print(r2x_core.__version__)")
        .output()
        .map_err(|e| {
            R2xError::PythonInit(format!("Failed to run Python for r2x_core import: {}", e))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(R2xError::PythonInit(format!(
            "Failed to import r2x-core: {}\n\nInstall with: uv pip install r2x-core --python {}",
            stderr,
            python_exe.display()
        )));
    }

    let version_str = String::from_utf8(output.stdout)
        .map_err(|e| R2xError::PythonInit(format!("Invalid UTF-8 in r2x_core version: {}", e)))?
        .trim()
        .to_string();

    debug!("r2x-core {} ready", version_str);

    Ok(())
}
