use crate::config_manager::Config;
use crate::logger;
use crate::GlobalOpts;
use atty::Stream;
use clap::Parser;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
pub struct ReadCommand {
    /// Path to JSON file to read. If not provided, reads from stdin
    pub file: Option<PathBuf>,
}

pub fn handle_read(cmd: ReadCommand, _opts: GlobalOpts) -> Result<(), Box<dyn std::error::Error>> {
    logger::debug("Starting read command");

    // Load configuration
    let mut config = Config::load()?;
    let venv_path = config.ensure_venv_path()?;
    logger::debug(&format!("Using virtual environment at {}", venv_path));

    // Get Python executable path (ensured via ensure_venv_path)
    let python_exe = config.get_venv_python_path();
    if !Path::new(&python_exe).exists() {
        return Err(format!(
            "Python executable not found at {}. Recreate the venv via `r2x python venv create`.",
            python_exe
        )
        .into());
    }

    logger::debug(&format!("Python executable: {}", python_exe));

    ensure_prerequisites(&mut config, &python_exe)?;

    // Load JSON input
    let json_file_path = match cmd.file {
        Some(file_path) => {
            logger::debug(&format!("Reading JSON from file: {}", file_path.display()));
            file_path
        }
        None => {
            if atty::is(Stream::Stdin) {
                logger::info(
                    "No JSON input detected; please provide --file or pipe JSON via stdin.",
                );
                return Err(
                    "No JSON input provided; either use --file or pipe data into `r2x read`".into(),
                );
            }

            logger::debug("Reading JSON from stdin");
            let mut json_data = String::new();
            std::io::stdin()
                .read_to_string(&mut json_data)
                .map_err(|e| format!("Failed to read from stdin: {}", e))?;

            let cache_dir = config.ensure_cache_path()?;
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let temp_json = PathBuf::from(cache_dir).join(format!("stdin_input_{}.json", unique));
            fs::write(&temp_json, &json_data)
                .map_err(|e| format!("Failed to write temporary JSON file: {}", e))?;

            logger::debug(&format!(
                "Saved stdin to temporary file: {}",
                temp_json.display()
            ));
            temp_json
        }
    };

    // Generate Python initialization code
    let file_path_str = json_file_path
        .to_str()
        .ok_or("Invalid file path")?
        .replace('\\', "\\\\");

    let python_code = format!(
        r#"
import json
import os
import sys as py_sys
import traceback
from IPython.terminal.embed import InteractiveShellEmbed
from traitlets.config import Config
from r2x_core.system import System

JSON_PATH = r'''{}'''

try:
    with open(JSON_PATH, 'r') as handle:
        data = json.load(handle)
    cwd = os.getcwd()
    system = System.from_dict(data, cwd)
except Exception:
    traceback.print_exc()
    py_sys.exit(1)

cfg = Config()
cfg.TerminalInteractiveShell.confirm_exit = False
cfg.TerminalInteractiveShell.display_banner = False
cfg.TerminalInteractiveShell.banner1 = ""
cfg.TerminalInteractiveShell.banner2 = ""
cfg.TerminalInteractiveShell.colors = "Linux"

force_simple_env = os.environ.get("R2X_FORCE_SIMPLE_PROMPT")
if force_simple_env is None:
    simple_prompt = not (py_sys.stdin.isatty() and py_sys.stdout.isatty())
else:
    simple_prompt = force_simple_env.lower() in ("1", "true", "yes", "on")

cfg.TerminalInteractiveShell.simple_prompt = simple_prompt

if os.environ.get("R2X_READ_NONINTERACTIVE") == "1":
    print("System available as `sys`. Run sys.info() for details.")
    py_sys.exit(0)

context = {{"sys": system}}
InteractiveShellEmbed(config=cfg, banner1="", exit_msg="")(
    header="System available as `sys` (use sys.info())",
    local_ns=context,
    global_ns=context,
)
"#,
        file_path_str
    );

    logger::debug("Generated Python initialization code");

    logger::debug("Launching interactive IPython session...");

    let ipython_dir = ensure_ipython_dir();
    let stdin_is_tty = atty::is(Stream::Stdin);
    let stdout_is_tty = atty::is(Stream::Stdout);
    let interactive_prompt = stdin_is_tty && stdout_is_tty;
    let (_tty_attached, stdin_tty, stdout_tty, stderr_tty) = acquire_tty_stdio();

    // Spawn IPython bootstrap script with interactive embed
    let mut command = Command::new(&python_exe);
    command
        .arg("-c")
        .arg(&python_code)
        .env("PYTHONUNBUFFERED", "1");

    command
        .stdin(stdin_tty)
        .stdout(stdout_tty)
        .stderr(stderr_tty);

    if interactive_prompt {
        command
            .env("PY_COLORS", "1")
            .env("CLICOLOR_FORCE", "1")
            .env("R2X_FORCE_SIMPLE_PROMPT", "0");
    } else {
        command.env("R2X_FORCE_SIMPLE_PROMPT", "1");
    }

    if std::env::var_os("TERM").is_none() {
        command.env("TERM", "xterm-256color");
    }

    if let Some(dir) = &ipython_dir {
        command.env("IPYTHONDIR", dir);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn IPython process: {}", e))?;

    logger::debug("IPython process spawned, waiting for completion");

    // Wait for IPython to finish
    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for IPython process: {}", e))?;

    if !status.success() {
        let exit_code = status.code().unwrap_or(-1);
        logger::debug(&format!("IPython exited with code: {}", exit_code));
        return Err(format!("IPython exited with code {}", exit_code).into());
    }

    logger::debug("IPython session completed successfully");
    Ok(())
}

fn ensure_prerequisites(
    config: &mut Config,
    python_exe: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_module_installed(config, python_exe, "IPython", "IPython", "IPython")?;
    let r2x_core_spec = config.get_r2x_core_package_spec();
    ensure_module_installed(
        config,
        python_exe,
        "r2x_core.system",
        &r2x_core_spec,
        "r2x-core",
    )?;
    Ok(())
}

#[cfg(unix)]
fn acquire_tty_stdio() -> (bool, Stdio, Stdio, Stdio) {
    let (stdin_attached, stdin) = match std::fs::File::open("/dev/tty") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stdout_attached, stdout) = match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stderr_attached, stderr) = match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    (
        stdin_attached || stdout_attached || stderr_attached,
        stdin,
        stdout,
        stderr,
    )
}

#[cfg(windows)]
fn acquire_tty_stdio() -> (bool, Stdio, Stdio, Stdio) {
    let (stdin_attached, stdin) = match std::fs::OpenOptions::new().read(true).open("CONIN$") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stdout_attached, stdout) = match std::fs::OpenOptions::new().write(true).open("CONOUT$") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    let (stderr_attached, stderr) = match std::fs::OpenOptions::new().write(true).open("CONOUT$") {
        Ok(handle) => (true, Stdio::from(handle)),
        Err(_) => (false, Stdio::inherit()),
    };

    (
        stdin_attached || stdout_attached || stderr_attached,
        stdin,
        stdout,
        stderr,
    )
}

fn ensure_module_installed(
    config: &mut Config,
    python_exe: &str,
    module_name: &str,
    package_spec: &str,
    display_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if module_exists(python_exe, module_name) {
        logger::debug(&format!("{} already available in venv", display_name));
        return Ok(());
    }

    logger::info(&format!(
        "{} not found in venv; installing via uv pip install",
        display_name
    ));

    install_package_with_spinner(config, python_exe, package_spec, display_name)?;
    Ok(())
}

fn module_exists(python_exe: &str, module_name: &str) -> bool {
    Command::new(python_exe)
        .arg("-c")
        .arg(&format!("import {}", module_name))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn install_package_with_spinner(
    config: &mut Config,
    python_exe: &str,
    package_spec: &str,
    display_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let uv_path = config.ensure_uv_path()?;
    let mut install_cmd = Command::new(&uv_path);
    install_cmd
        .arg("pip")
        .arg("install")
        .arg("--python")
        .arg(python_exe)
        .arg(package_spec);

    logger::debug(&format!("Running: {:?}", install_cmd));
    logger::spinner_start(&format!("Installing {} into venv...", display_name));

    let output = install_cmd.output().map_err(|e| {
        logger::spinner_error(&format!("Failed to install {} into venv", display_name));
        format!("Failed to run uv pip install: {}", e)
    })?;

    logger::capture_output(&format!("uv pip install {}", package_spec), &output);

    if !output.status.success() {
        logger::spinner_error(&format!("Failed to install {} into venv", display_name));
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("uv pip install {} failed: {}", package_spec, stderr).into());
    }

    logger::spinner_success(&format!("Installed {} into venv", display_name));
    Ok(())
}

fn ensure_ipython_dir() -> Option<PathBuf> {
    let config_path = Config::path();
    if let Some(dir) = config_path.parent() {
        let ipython_dir = dir.join("ipython");
        if let Err(err) = fs::create_dir_all(&ipython_dir) {
            logger::debug(&format!(
                "Failed to create IPython dir {}: {}",
                ipython_dir.display(),
                err
            ));
            None
        } else {
            Some(ipython_dir)
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_command_creation() {
        let cmd = ReadCommand { file: None };
        assert!(cmd.file.is_none());
    }

    #[test]
    fn test_read_command_with_file() {
        let cmd = ReadCommand {
            file: Some(PathBuf::from("test.json")),
        };
        assert!(cmd.file.is_some());
    }
}
