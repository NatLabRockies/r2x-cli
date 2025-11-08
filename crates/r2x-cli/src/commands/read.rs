use crate::logger;
use crate::GlobalOpts;
use clap::Parser;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
pub struct ReadCommand {
    /// Path to JSON file to read. If not provided, reads from stdin
    pub file: Option<PathBuf>,
}

pub fn handle_read(cmd: ReadCommand, _opts: GlobalOpts) -> Result<(), Box<dyn std::error::Error>> {
    logger::debug("Starting read command");

    // Load configuration
    let config = crate::config_manager::Config::load()?;
    let venv_path = config.get_venv_path();

    // Get Python executable path
    #[cfg(unix)]
    let python_exe = format!("{}/bin/python", venv_path);
    #[cfg(windows)]
    let python_exe = format!("{}\\Scripts\\python.exe", venv_path);

    logger::debug(&format!("Python executable: {}", python_exe));

    // Load JSON input
    let json_file_path = if let Some(file_path) = cmd.file {
        logger::debug(&format!("Reading JSON from file: {}", file_path.display()));
        file_path
    } else {
        logger::debug("Reading JSON from stdin");
        let mut json_data = String::new();
        std::io::stdin()
            .read_to_string(&mut json_data)
            .map_err(|e| format!("Failed to read from stdin: {}", e))?;

        let temp_json = std::env::temp_dir().join("r2x_input.json");
        fs::write(&temp_json, &json_data)
            .map_err(|e| format!("Failed to write temporary JSON file: {}", e))?;

        logger::debug(&format!(
            "Saved stdin to temporary file: {}",
            temp_json.display()
        ));
        temp_json
    };

    // Generate Python initialization code
    let file_path_str = json_file_path
        .to_str()
        .ok_or("Invalid file path")?
        .replace('\\', "\\\\");

    let python_code = format!(
        r#"import json, os
from r2x_core.system import System
try:
    with open(r'{}', 'r') as f:
        data = json.load(f)
    cwd = os.getcwd()
    system = System.from_dict(data, cwd)
    print('✔ System deserialized successfully')
    print('System object available as system')
except Exception as e:
    print(f'Error loading system: {{e}}')
    import traceback
    traceback.print_exc()
    system = None
"#,
        file_path_str
    );

    logger::debug("Generated Python initialization code");

    logger::success("Launching interactive IPython session...");

    // Open /dev/tty to get a real terminal for interactive input
    // (stdin was already consumed by reading JSON, so we need a fresh connection)
    #[cfg(unix)]
    let stdin_source = fs::File::open("/dev/tty")
        .map(Stdio::from)
        .unwrap_or_else(|_| {
            logger::debug("Could not open /dev/tty, using inherited stdin");
            Stdio::inherit()
        });

    #[cfg(windows)]
    let stdin_source = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CON")
        .map(Stdio::from)
        .unwrap_or_else(|_| {
            logger::debug("Could not open CON, using inherited stdin");
            Stdio::inherit()
        });

    // Spawn IPython with flags for interactive mode
    let mut child = Command::new(&python_exe)
        .arg("-m")
        .arg("IPython")
        .arg("-i") // Force interactive mode
        .arg("--simple-prompt") // Simplified prompt
        .arg("--quick") // Skip startup scripts
        .arg("-c")
        .arg(&python_code)
        .stdin(stdin_source)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
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
