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
static FILE_LOG_LEVEL: Mutex<LogLevel> = Mutex::new(LogLevel::Info);
static MAX_LOG_BYTES: Mutex<Option<u64>> = Mutex::new(None);
static CURRENT_PLUGIN: Mutex<Option<String>> = Mutex::new(None);
static SPINNER: Mutex<Option<ProgressBar>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// Get the current verbosity level for use by other modules (e.g., Python bridge)
pub fn get_verbosity() -> u8 {
    VERBOSITY.lock().ok().map_or(0, |v| *v)
}

/// Get whether Python logging to console is enabled
pub fn get_log_python() -> bool {
    LOG_PYTHON.lock().ok().is_some_and(|v| *v)
}

/// Set whether Python logging to console is enabled
pub fn set_log_python(enabled: bool) {
    if let Ok(mut v) = LOG_PYTHON.lock() {
        *v = enabled;
    }
}

/// Get whether stdout logging is disabled
pub fn get_no_stdout() -> bool {
    NO_STDOUT.lock().ok().is_some_and(|v| *v)
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
    init_with_config(verbosity, log_python, no_stdout, None, None)
}

/// Initialize logger with optional path override, file level, and max file size.
pub fn init_with_config(
    verbosity: u8,
    log_python: bool,
    no_stdout: bool,
    log_path: Option<&str>,
    max_log_bytes: Option<u64>,
) -> Result<(), String> {
    // Set verbosity level
    if let Ok(mut v) = VERBOSITY.lock() {
        *v = verbosity;
    }

    // Set log_python flag
    set_log_python(log_python);

    // Set no_stdout flag
    set_no_stdout(no_stdout);

    if let Ok(mut max_size) = MAX_LOG_BYTES.lock() {
        *max_size = max_log_bytes;
    }

    if let Ok(mut file_level) = FILE_LOG_LEVEL.lock() {
        *file_level = match verbosity {
            0 => LogLevel::Info,
            1 => LogLevel::Debug,
            _ => LogLevel::Trace,
        };
    }

    init(log_path)
}

/// Initialize the logger with a log file path (internal)
fn init(log_path_override: Option<&str>) -> Result<(), String> {
    let log_file = if let Some(path) = log_path_override {
        PathBuf::from(path)
    } else {
        let config_dir = get_config_dir()?;
        config_dir.join("r2x.log")
    };

    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create log directory: {}", e))?;
    }

    // Ensure log file exists so commands like `r2x log path` always reference
    // a readable file, and preserve prior command history by appending.
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| format!("Failed to initialize log file: {}", e))?;

    let mut log_file_guard = LOG_FILE
        .lock()
        .map_err(|e| format!("Failed to lock log file mutex: {e}"))?;
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
fn write_to_log(level: LogLevel, message: &str) {
    write_to_log_with_source(level, message, "RUST");
}

/// Write to log file with custom source tag
fn write_to_log_with_source(level: LogLevel, message: &str, source: &str) {
    let allowed_level = FILE_LOG_LEVEL.lock().ok().map_or(LogLevel::Info, |v| *v);
    if level > allowed_level {
        return;
    }

    if let Ok(log_file_guard) = LOG_FILE.lock() {
        if let Some(ref log_path) = *log_file_guard {
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let line = format!("[{}] [{}] {}", timestamp, source, message);
            maybe_rotate_log_file(log_path, line.len() as u64 + 1);

            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
                let _ = writeln!(file, "{}", line);
            }
        }
    }
}

fn maybe_rotate_log_file(log_path: &PathBuf, incoming_bytes: u64) {
    let Some(max_bytes) = MAX_LOG_BYTES.lock().ok().and_then(|v| *v) else {
        return;
    };

    if max_bytes == 0 {
        return;
    }

    let current_len = fs::metadata(log_path).map_or(0, |m| m.len());
    if current_len.saturating_add(incoming_bytes) <= max_bytes {
        return;
    }

    let backup = log_path.with_file_name(format!(
        "{}.1",
        log_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("r2x.log")
    ));
    let _ = fs::remove_file(&backup);
    let _ = fs::rename(log_path, backup);
}

/// Log an informational message (to console if verbose >= 1, always to file)
pub fn info(message: &str) {
    write_to_log(LogLevel::Info, &format!("INFO {}", message));
    if get_verbosity() >= 1 {
        eprintln!("{}", message);
    }
}

/// Log a debug message (to console if verbose >= 1, always to file)
pub fn debug(message: &str) {
    write_to_log(LogLevel::Debug, &format!("DEBUG {}", message));
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
    write_to_log(LogLevel::Warn, &format!("WARN {}", message));
    eprintln!("{} {}", "warning:".yellow().bold(), message);
}

/// Log an error message (to both file and console)
pub fn error(message: &str) {
    write_to_log(LogLevel::Error, &format!("ERROR {}", message));
    eprintln!("{} {}", "Error:".red().bold(), message);
}

/// Log a success message (to console only for user feedback)
pub fn success(message: &str) {
    write_to_log(LogLevel::Info, &format!("SUCCESS {}", message));
    let check = "\u{2714}".green().bold(); // 🗸 HEAVY CHECK MARK
    eprintln!("{} {}", check, message);
}

/// Log a step message (important user-facing step)
pub fn step(message: &str) {
    if get_verbosity() >= 2 {
        eprintln!("TRACE: {}", message);
    }
    write_to_log(LogLevel::Info, &format!("STEP: {}", message));
}

/// Capture command output and log it
pub fn capture_output(command_name: &str, output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    write_to_log(
        LogLevel::Debug,
        &format!(
            "COMMAND: {} (exit code: {:?})",
            command_name,
            output.status.code()
        ),
    );

    if !stdout.is_empty() {
        write_to_log(LogLevel::Debug, &format!("  STDOUT:\n{}", stdout));
    }

    if !stderr.is_empty() {
        write_to_log(LogLevel::Debug, &format!("  STDERR:\n{}", stderr));
    }
}

/// Capture command output and always persist it to log file at info level.
///
/// This is useful for noisy subprocesses where console output is suppressed
/// by default but full output should remain available in logs.
pub fn capture_output_always(command_name: &str, output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    write_to_log(
        LogLevel::Info,
        &format!(
            "COMMAND: {} (exit code: {:?})",
            command_name,
            output.status.code()
        ),
    );

    if !stdout.is_empty() {
        write_to_log(LogLevel::Info, &format!("  STDOUT:\n{}", stdout));
    }

    if !stderr.is_empty() {
        write_to_log(LogLevel::Info, &format!("  STDERR:\n{}", stderr));
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
    let style = indicatif::ProgressStyle::default_spinner()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
        .template("{spinner:.cyan} {msg}");
    if let Ok(s) = style {
        spinner.set_style(s);
    }
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
