//! Launch the PyO3 runtime through R2X's UV-managed virtual environment.

use anyhow::{bail, ensure, Context, Result};
use r2x_config::Config;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const PYTHON_RUNTIME_PATHS: &str =
    "import sys, sysconfig\nprint(sysconfig.get_config_var('LIBDIR') or '')\nprint(sys.base_prefix)";

struct Runtime {
    uv: PathBuf,
    venv: PathBuf,
}

impl Runtime {
    fn load() -> Result<Self> {
        let mut config = Config::load().context("failed to load R2X configuration")?;
        let uv = PathBuf::from(
            config
                .ensure_uv_path()
                .context("failed to find the UV executable")?,
        );
        let venv = PathBuf::from(
            config
                .reconcile_venv_path()
                .context("failed to create the UV-managed R2X virtual environment")?,
        );

        Ok(Self { uv, venv })
    }
}

fn main() -> Result<()> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    launch(&args)
}

fn launch(args: &[OsString]) -> Result<()> {
    let runtime = Runtime::load()?;
    let library_dir = python_library_dir(&runtime)?;
    let payload = payload_path()?;

    let mut command = uv_run_in_venv(&runtime);
    command.arg(&payload).args(args);
    configure_python_loader(&mut command, &library_dir)?;

    let status = command.status().context("failed to start R2X through uv")?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => bail!("R2X exited with status {code}"),
        None => bail!("R2X terminated without an exit code"),
    }
}

fn payload_path() -> Result<PathBuf> {
    let launcher = env::current_exe().context("failed to locate the R2X launcher")?;
    let payload = payload_path_next_to(&launcher)?;
    ensure!(
        payload.is_file(),
        "R2X runtime not found: {}",
        payload.display()
    );
    Ok(payload)
}

fn payload_path_next_to(launcher: &Path) -> Result<PathBuf> {
    let directory = launcher
        .parent()
        .context("R2X launcher does not have a parent directory")?;
    Ok(directory.join(payload_name()))
}

#[cfg(target_os = "windows")]
const fn payload_name() -> &'static str {
    "r2x-runtime.exe"
}

#[cfg(not(target_os = "windows"))]
const fn payload_name() -> &'static str {
    "r2x-runtime"
}

fn uv_run_in_venv(runtime: &Runtime) -> Command {
    let mut command = Command::new(&runtime.uv);
    command
        .args(["run", "--no-config", "--no-project", "--active", "--"])
        .env("VIRTUAL_ENV", &runtime.venv)
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH");
    command
}

fn python_library_dir(runtime: &Runtime) -> Result<PathBuf> {
    let output = uv_run_in_venv(runtime)
        .args(["python", "-I", "-S", "-c", PYTHON_RUNTIME_PATHS])
        .output()
        .with_context(|| {
            format!(
                "failed to probe the UV-managed Python in {}",
                runtime.venv.display()
            )
        })?;
    ensure!(
        output.status.success(),
        "Python probe failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let output =
        String::from_utf8(output.stdout).context("Python probe returned non-UTF-8 output")?;
    let mut paths = output.lines();

    #[cfg(target_os = "windows")]
    let library_dir = {
        let _ = paths.next().context("Python probe did not return LIBDIR")?;
        PathBuf::from(
            paths
                .next()
                .context("Python probe did not return the base prefix")?,
        )
    };
    #[cfg(not(target_os = "windows"))]
    let library_dir = PathBuf::from(paths.next().context("Python probe did not return LIBDIR")?);

    ensure!(
        library_dir.is_dir(),
        "Python library directory does not exist: {}",
        library_dir.display()
    );
    Ok(library_dir)
}

fn configure_python_loader(command: &mut Command, library_dir: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    command.env(
        "DYLD_LIBRARY_PATH",
        prepend_path(library_dir, env::var_os("DYLD_LIBRARY_PATH"))?,
    );

    #[cfg(all(unix, not(target_os = "macos")))]
    command.env(
        "LD_LIBRARY_PATH",
        prepend_path(library_dir, env::var_os("LD_LIBRARY_PATH"))?,
    );

    #[cfg(target_os = "windows")]
    command.env("PATH", prepend_path(library_dir, env::var_os("PATH"))?);

    #[cfg(not(any(unix, target_os = "windows")))]
    bail!("R2X supports macOS, Linux, and Windows only");

    Ok(())
}

fn prepend_path(prefix: &Path, existing: Option<OsString>) -> Result<OsString> {
    let mut paths = vec![prefix.to_path_buf()];
    if let Some(existing) = existing {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths).context("could not construct a process search path")
}

#[cfg(test)]
mod tests {
    use super::{payload_name, payload_path_next_to, prepend_path};
    use anyhow::Result;
    use std::env;
    use std::path::{Path, PathBuf};

    #[test]
    fn places_the_runtime_next_to_the_launcher() -> Result<()> {
        let payload = payload_path_next_to(Path::new("bin/r2x"))?;
        assert_eq!(payload, Path::new("bin").join(payload_name()));
        Ok(())
    }

    #[test]
    fn preserves_existing_search_paths() -> Result<()> {
        let paths = prepend_path(
            Path::new("/runtime/python/lib"),
            Some(env::join_paths(["/usr/local/bin", "/usr/bin"])?),
        )?;

        assert_eq!(
            env::split_paths(&paths).collect::<Vec<_>>(),
            [
                PathBuf::from("/runtime/python/lib"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/bin"),
            ]
        );
        Ok(())
    }
}
