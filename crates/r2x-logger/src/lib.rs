use colored::Colorize;
use indicatif::ProgressBar;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
static VERBOSITY: Mutex<u8> = Mutex::new(0);
static LOG_PYTHON: Mutex<bool> = Mutex::new(false);
static NO_STDOUT: Mutex<bool> = Mutex::new(false);
static CURRENT_PLUGIN: Mutex<Option<String>> = Mutex::new(None);
static SPINNER: Mutex<Option<ProgressBar>> = Mutex::new(None);

/// Get the current verbosity level for use by other modules (e.g., Python bridge)
pub fn get_verbosity() -> u8 {
    VERBOSITY.lock().ok().map(|v| *v).unwrap_or(0)
}

/// Get whether Python logging to console is enabled
pub fn get_log_python() -> bool {
    LOG_PYTHON.lock().ok().map(|v| *v).unwrap_or(false)
}

/// Set whether Python logging to console is enabled
pub fn set_log_python(enabled: bool) {
    if let Ok(mut v) = LOG_PYTHON.lock() {
        *v = enabled;
    }
}

/// Get whether stdout logging is disabled
pub fn get_no_stdout() -> bool {
    NO_STDOUT.lock().ok().map(|v| *v).unwrap_or(false)
}

/// Set whether stdout logging is disabled
pub fn set_no_stdout(disabled: bool) {
    if let Ok(mut v) = NO_STDOUT.lock() {
        *v = disabled;
    }
}

/// Get the current plugin name being executed
pub fn get_current_plugin() -> Option<String> {
    CURRENT_PLUGIN.lock().ok().and_then(|guard| guard.clone())
}

/// Set the current plugin name being executed
pub fn set_current_plugin(plugin_name: Option<String>) {
    if let Ok(mut v) = CURRENT_PLUGIN.lock() {
        *v = plugin_name;
    }
}

/// Convert verbosity level to loguru log level string
/// 0 = warn only, 1 = debug (-v), 2 = trace (-vv)
pub fn verbosity_to_loguru_level() -> String {
    match get_verbosity() {
        0 => "WARNING".to_string(),
        1 => "DEBUG".to_string(),
        _ => "TRACE".to_string(),
    }
}

/// Initialize the logger with a log file path and verbosity level
pub fn init_with_verbosity(verbosity: u8, log_python: bool, no_stdout: bool) -> Result<(), String> {
    // Set verbosity level
    if let Ok(mut v) = VERBOSITY.lock() {
        *v = verbosity;
    }

    // Set log_python flag
    set_log_python(log_python);

    // Set no_stdout flag
    set_no_stdout(no_stdout);

    init()
}

/// Initialize the logger with a log file path (internal)
fn init() -> Result<(), String> {
    let config_dir = get_config_dir()?;
    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    let log_file = config_dir.join("r2x.log");

    // Truncate log file on each run (overwrite instead of append)
    if log_file.exists() {
        let _ = fs::remove_file(&log_file);
    }

    let mut log_file_guard = LOG_FILE.lock().unwrap();
    *log_file_guard = Some(log_file);

    Ok(())
}

/// Get the config directory path
fn get_config_dir() -> Result<PathBuf, String> {
    #[cfg(not(target_os = "windows"))]
    let config_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".config")
        .join("r2x");

    #[cfg(target_os = "windows")]
    let config_dir = dirs::config_dir()
        .ok_or("Could not determine config directory")?
        .join("r2x");

    Ok(config_dir)
}

/// Write to log file
fn write_to_log(message: &str) {
    write_to_log_with_source(message, "RUST")
}

/// Write to log file with custom source tag
fn write_to_log_with_source(message: &str, source: &str) {
    if let Ok(log_file_guard) = LOG_FILE.lock() {
        if let Some(ref log_path) = *log_file_guard {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
                let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(file, "[{}] [{}] {}", timestamp, source, message);
            }
        }
    }
}

/// Log an informational message (to console if verbose >= 1, always to file)
pub fn info(message: &str) {
    write_to_log(&format!("INFO {}", message));
    if get_verbosity() >= 1 {
        eprintln!("{}", message);
    }
}

/// Log a debug message (to console if verbose >= 1, always to file)
pub fn debug(message: &str) {
    write_to_log(&format!("DEBUG {}", message));
    if get_verbosity() >= 1 {
        eprintln!("{} {}", "DEBUG:".blue().bold(), message);
    }
}

/// Log a debug message to console only (not to file)
pub fn debug_console_only(message: &str) {
    if get_verbosity() >= 1 {
        eprintln!("{} {}", "DEBUG:".blue().bold(), message);
    }
}

/// Log a warning message (to both file and console)
pub fn warn(message: &str) {
    write_to_log(&format!("WARN {}", message));
    eprintln!("{} {}", "warning:".yellow().bold(), message);
}

/// Log an error message (to both file and console)
pub fn error(message: &str) {
    write_to_log(&format!("ERROR {}", message));
    eprintln!("{} {}", "Error:".red().bold(), message);
}

/// Log a success message (to console only for user feedback)
pub fn success(message: &str) {
    write_to_log(&format!("SUCCESS {}", message));
    let check = "\u{2714}".green().bold(); // 🗸 HEAVY CHECK MARK
    eprintln!("{} {}", check, message);
}

/// Log a step message (important user-facing step)
pub fn step(message: &str) {
    if get_verbosity() >= 2 {
        eprintln!("TRACE: {}", message);
    }
    write_to_log(&format!("STEP: {}", message));
}

/// Capture command output and log it
pub fn capture_output(command_name: &str, output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    write_to_log(&format!(
        "COMMAND: {} (exit code: {:?})",
        command_name,
        output.status.code()
    ));

    if !stdout.is_empty() {
        write_to_log(&format!("  STDOUT:\n{}", stdout));
    }

    if !stderr.is_empty() {
        write_to_log(&format!("  STDERR:\n{}", stderr));
    }
}

/// Get the log file path for display
pub fn get_log_path() -> Option<PathBuf> {
    LOG_FILE.lock().ok().and_then(|guard| guard.clone())
}

/// Get the log file path as a string for Python configuration
pub fn get_log_path_string() -> String {
    if let Some(path) = get_log_path() {
        path.to_string_lossy().to_string()
    } else if let Ok(config_dir) = get_config_dir() {
        config_dir.join("r2x.log").to_string_lossy().to_string()
    } else {
        String::new()
    }
}

/// Print the log file path to the user
pub fn show_log_path() {
    if let Some(path) = get_log_path() {
        eprintln!("Log file: {}", path.display());
    } else if let Ok(config_dir) = get_config_dir() {
        eprintln!("Log file: {}", config_dir.join("r2x.log").display());
    } else {
        eprintln!("Log file location not available");
    }
}

/// Start a spinner with the given message (only if not verbose)
pub fn spinner_start(message: &str) {
    // Don't show spinner in verbose mode
    if get_verbosity() > 0 {
        return;
    }

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        indicatif::ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));
    spinner.set_message(message.to_string());

    if let Ok(mut spinner_guard) = SPINNER.lock() {
        *spinner_guard = Some(spinner);
    }
}

/// Complete the spinner with a success message
pub fn spinner_success(message: &str) {
    if let Ok(mut spinner_guard) = SPINNER.lock() {
        if let Some(spinner) = spinner_guard.take() {
            spinner.finish_and_clear();
        }
    }
    // Show success message with checkmark
    eprintln!("{} {}", "✔".green().bold(), message);
}

/// Stop the spinner with an error message
pub fn spinner_error(message: &str) {
    if let Ok(mut spinner_guard) = SPINNER.lock() {
        if let Some(spinner) = spinner_guard.take() {
            spinner.finish_and_clear();
        }
    }
    // Show error message with cross
    eprintln!("  {} {}", "✗".red().bold(), message);
}

/// Stop the spinner without any message
pub fn spinner_stop() {
    if let Ok(mut spinner_guard) = SPINNER.lock() {
        if let Some(spinner) = spinner_guard.take() {
            spinner.finish_and_clear();
        }
    }
}
