//! Public launcher regressions for UV-managed Python startup.

#![cfg(unix)]

use assert_cmd::cargo::cargo_bin;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use which::which;

#[test]
fn launcher_uses_uv_without_a_python_executable_on_path() {
    let Some(uv) = which("uv").ok() else {
        return;
    };
    let python_version = r2x_config::default_python_version();
    if !Command::new(&uv)
        .args(["python", "find", python_version, "--managed-python"])
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return;
    }

    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let path = temp_dir.path().join("path-without-python");
    if fs::create_dir_all(&path).is_err() || !add_uv_runtime_tool(&path) {
        return;
    }

    let config_path = temp_dir.path().join("config.toml");
    let venv_path = temp_dir.path().join(".venv");
    if fs::write(
        &config_path,
        format!(
            "uv_path = \"{}\"\npython_version = \"{}\"\nvenv_path = \"{}\"\n",
            uv.display(),
            python_version,
            venv_path.display(),
        ),
    )
    .is_err()
    {
        return;
    }

    let output = Command::new(cargo_bin("r2x"))
        .arg("--version")
        .env("PATH", &path)
        .env("R2X_CONFIG", &config_path)
        .env("PYTHONHOME", "poison")
        .env("PYTHONPATH", "poison")
        .output();

    assert!(
        output.as_ref().is_ok_and(|output| output.status.success()),
        "launcher failed: {output:?}"
    );
    assert!(venv_path.join("pyvenv.cfg").is_file());
}

fn add_uv_runtime_tool(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let install_name_tool = Path::new("/usr/bin/install_name_tool");
        if !install_name_tool.is_file() {
            return false;
        }
        std::os::unix::fs::symlink(install_name_tool, path.join("install_name_tool")).is_ok()
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        true
    }
}
