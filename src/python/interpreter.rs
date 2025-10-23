use crate::{R2xError, Result};
use pyo3::prelude::*;
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
    let site_packages = super::venv::get_site_packages(venv_path)?;

    Python::with_gil(|py| {
        let sys = py.import_bound("sys")?;
        let version: String = sys.getattr("version")?.extract()?;
        if !version.starts_with("3.11") {
            return Err(R2xError::PythonInit(format!(
                "Wrong Python version! Expected 3.11, got: {}",
                version
            )));
        }

        let path = sys.getattr("path")?;
        path.call_method1("insert", (0, site_packages.to_str().unwrap()))?;
        process_pth_files(py, &site_packages)?;

        debug!(
            "Python {}: {} packages in sys.path",
            version.split_whitespace().next().unwrap_or("3.11"),
            path.len()?
        );

        import_r2x_core(py, venv_path)?;

        Ok(())
    })
}

fn import_r2x_core(py: Python, venv_path: &Path) -> Result<()> {
    py.import_bound("r2x_core")
        .map(|module| {
            if let Ok(version) = module.getattr("__version__").and_then(|v| v.extract::<String>()) {
                debug!("r2x-core {} ready", version);
            }
        })
        .map_err(|e| {
            R2xError::PythonInit(format!(
                "Failed to import r2x-core: {}\n\nInstall with: uv pip install r2x-core --python {}",
                e, venv_path.join("bin/python3").display()
            ))
        })
}

fn process_pth_files(py: Python, site_packages: &Path) -> Result<()> {
    use std::fs;
    use std::io::{BufRead, BufReader};

    if let Ok(entries) = fs::read_dir(site_packages) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("pth") {
                if let Ok(file) = fs::File::open(&path) {
                    let reader = BufReader::new(file);
                    for line in reader.lines().flatten() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') || line.starts_with("import ") {
                            continue;
                        }

                        let sys = py.import_bound("sys")?;
                        let sys_path = sys.getattr("path")?;
                        if !line.is_empty() {
                            sys_path.call_method1("append", (line,))?;
                            debug!("Added to sys.path from .pth: {}", line);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
