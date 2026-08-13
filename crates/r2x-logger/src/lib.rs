use colored::Colorize;
use indicatif::ProgressBar;
use std::fs::{self, OpenOptions};
use std::io::IsTerminal;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
static VERBOSITY: Mutex<u8> = Mutex::new(0);
static QUIET: Mutex<u8> = Mutex::new(0);
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

/// Get the number of quiet flags supplied by the user.
pub fn get_quiet_level() -> u8 {
    QUIET.lock().ok().map_or(0, |v| *v)
}

/// Get whether Python logging to console is enabled
pub fn get_log_python() -> bool {
    LOG_PYTHON.lock().ok().is_some_and(|v| *v)
}

/// Set whether Python logging to console is enabled
fn set_log_python(enabled: bool) {
    if let Ok(mut v) = LOG_PYTHON.lock() {
        *v = enabled;
    }
}

/// Get whether stdout logging is disabled
pub fn get_no_stdout() -> bool {
    NO_STDOUT.lock().ok().is_some_and(|v| *v)
}

/// Set whether stdout logging is disabled
fn set_no_stdout(disabled: bool) {
    if let Ok(mut v) = NO_STDOUT.lock() {
        *v = disabled;
    }
}

/// Set the current plugin name being executed
pub fn set_current_plugin(plugin_name: Option<String>) {
    if let Ok(mut v) = CURRENT_PLUGIN.lock() {
        *v = plugin_name;
    }
}

/// Initialize logger with optional path override, file level, and max file size.
pub fn init_with_config(
    verbosity: u8,
    quiet: u8,
    log_python: bool,
    no_stdout: bool,
    log_path: Option<&str>,
    max_log_bytes: Option<u64>,
) -> Result<(), String> {
    // Set verbosity level
    if let Ok(mut v) = VERBOSITY.lock() {
        *v = verbosity;
    }

    if let Ok(mut q) = QUIET.lock() {
        *q = quiet;
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
    write_log_line(message, source);
}

/// Write a subprocess transcript regardless of the configured log level.
fn write_to_log_unfiltered(message: &str) {
    write_log_line(message, "RUST");
}

fn write_log_line(message: &str, source: &str) {
    let message = redact_sensitive_text(&sanitize_log_message(message));
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

/// Redact credentials from command summaries and persisted log messages.
pub fn redact_sensitive_text(message: &str) -> String {
    let mut redacted = redact_url_userinfo(message);
    for key in [
        "access_token",
        "refresh_token",
        "token",
        "password",
        "passwd",
        "secret",
        "api_key",
        "api-key",
        "apikey",
        "private_key",
        "private-key",
    ] {
        redacted = redact_key_value(&redacted, key);
    }
    redacted
}

fn redact_url_userinfo(message: &str) -> String {
    let mut result = String::with_capacity(message.len());
    let mut cursor = 0;

    while let Some(relative_scheme_end) = message[cursor..].find("://") {
        let scheme_end = cursor + relative_scheme_end;
        let authority_start = scheme_end + 3;
        let authority_end = message[authority_start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '/' | '?' | '#' | ')' | ']')
            })
            .map_or(message.len(), |offset| authority_start + offset);
        let authority = &message[authority_start..authority_end];

        let Some(userinfo_end) = authority.find('@') else {
            result.push_str(&message[cursor..authority_end]);
            cursor = authority_end;
            continue;
        };
        let userinfo = &authority[..userinfo_end];
        if userinfo == "git" {
            result.push_str(&message[cursor..authority_end]);
        } else {
            result.push_str(&message[cursor..authority_start]);
            result.push_str("[REDACTED]@");
            result.push_str(&authority[userinfo_end + 1..]);
        }
        cursor = authority_end;
    }

    result.push_str(&message[cursor..]);
    result
}

fn redact_key_value(message: &str, key: &str) -> String {
    let lower_message = message.to_ascii_lowercase();
    let mut result = String::with_capacity(message.len());
    let mut cursor = 0;

    while let Some(relative_key_start) = lower_message[cursor..].find(key) {
        let key_start = cursor + relative_key_start;
        let key_end = key_start + key.len();
        let boundary_before = key_start == 0
            || !message[..key_start]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        let boundary_after = key_end == message.len()
            || !message[key_end..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');

        if !boundary_before || !boundary_after {
            result.push_str(&message[cursor..key_end]);
            cursor = key_end;
            continue;
        }

        let key_syntax_end = if message[key_end..].starts_with('"') {
            key_end + 1
        } else {
            key_end
        };
        let separator_start = message[key_syntax_end..]
            .find(|character: char| !character.is_ascii_whitespace())
            .map_or(message.len(), |offset| key_syntax_end + offset);
        let has_separator = separator_start < message.len()
            && matches!(message.as_bytes()[separator_start], b'=' | b':');
        let is_option = message[..key_start].ends_with("--");
        if !has_separator && !is_option {
            result.push_str(&message[cursor..key_end]);
            cursor = key_end;
            continue;
        }

        let value_start = if has_separator {
            message[separator_start + 1..]
                .find(|character: char| !character.is_ascii_whitespace())
                .map_or(message.len(), |offset| separator_start + 1 + offset)
        } else {
            separator_start
        };
        if value_start == message.len() {
            result.push_str(&message[cursor..key_end]);
            cursor = key_end;
            continue;
        }

        let quoted = matches!(message.as_bytes()[value_start], b'"' | b'\'');
        let (value_content_start, value_end, closing_quote) = if quoted {
            let quote = message.as_bytes()[value_start];
            let content_start = value_start + 1;
            let content_end = message[content_start..]
                .find(|character: char| character as u8 == quote)
                .map_or(message.len(), |offset| content_start + offset);
            (
                content_start,
                content_end,
                (content_end < message.len()).then_some(quote),
            )
        } else {
            let content_end = message[value_start..]
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '&' | ',' | ')' | ']' | '"' | '\'')
                })
                .map_or(message.len(), |offset| value_start + offset);
            (value_start, content_end, None)
        };

        result.push_str(&message[cursor..value_content_start]);
        if value_content_start < value_end {
            result.push_str("[REDACTED]");
        }
        if let Some(quote) = closing_quote {
            result.push(quote as char);
            cursor = value_end + 1;
        } else {
            cursor = value_end;
        }
    }

    result.push_str(&message[cursor..]);
    result
}

fn sanitize_log_message(message: &str) -> String {
    let mut sanitized = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            // Drop ANSI CSI sequences, including carriage movement and color.
            if chars.peek() == Some(&'[') {
                chars.next();
                for sequence_character in chars.by_ref() {
                    if ('@'..='~').contains(&sequence_character) {
                        break;
                    }
                }
            }
            continue;
        }

        if character == '\r' || (character.is_control() && character != '\n' && character != '\t') {
            continue;
        }
        sanitized.push(character);
    }

    sanitized
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

/// Emit a user-facing status line on stderr unless quiet output was requested.
///
/// Status lines are deliberately separate from [`info`]: normal commands need
/// to show progress, while informational diagnostics remain controlled by
/// `-v`.
pub fn status(message: &str) {
    let safe_message = redact_sensitive_text(&sanitize_log_message(message));
    write_to_log(LogLevel::Info, &format!("STATUS {}", safe_message));
    if get_quiet_level() == 0 {
        eprintln!("{}", safe_message);
    }
}

/// Record a subprocess invocation without copying interactive terminal output into the log.
pub fn record_command(
    phase: &str,
    target: &str,
    command: &str,
    status: Option<i32>,
    elapsed: std::time::Duration,
) {
    write_to_log(
        LogLevel::Info,
        &format!(
            "COMMAND phase={phase:?} target={target:?} command={command:?} exit={status:?} elapsed_ms={}",
            elapsed.as_millis()
        ),
    );
}

/// Record a non-interactive subprocess output chunk after sanitizing it for the log.
pub fn record_command_output(phase: &str, target: &str, stream: &str, output: &[u8]) {
    let output = String::from_utf8_lossy(output);
    if output.is_empty() {
        return;
    }
    write_to_log_unfiltered(&format!(
        "COMMAND_OUTPUT phase={phase:?} target={target:?} stream={stream:?}:\n{output}"
    ));
}

/// Log a debug message (to console if verbose >= 1, always to file)
pub fn debug(message: &str) {
    write_to_log(LogLevel::Debug, &format!("DEBUG {}", message));
    if get_verbosity() >= 1 {
        eprintln!("{} {}", "DEBUG:".blue().bold(), message);
    }
}

/// Return whether a debug message would be emitted anywhere.
pub fn debug_enabled() -> bool {
    get_verbosity() >= 1
        || FILE_LOG_LEVEL
            .lock()
            .ok()
            .is_some_and(|level| *level >= LogLevel::Debug)
}

/// Log a lazily-built debug message.
pub fn debug_lazy(message: impl FnOnce() -> String) {
    if debug_enabled() {
        debug(&message());
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
    if get_quiet_level() == 0 {
        let check = "\u{2714}".green().bold(); // 🗸 HEAVY CHECK MARK
        eprintln!("{} {}", check, message);
    }
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

/// Start a spinner with the given message (only if not verbose)
pub fn spinner_start(message: &str) {
    // Don't show spinner in verbose mode
    if get_verbosity() > 0 {
        return;
    }

    // Skip spinner for non-TTY output (CI, pipes, redirects)
    if !std::io::stderr().is_terminal() {
        return;
    }

    // Respect NO_COLOR and TERM=dumb for accessibility and automation
    if std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").ok().as_deref() == Some("dumb")
    {
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
    // Show success message with checkmark unless quiet output was requested
    if get_quiet_level() == 0 {
        eprintln!("{} {}", "✔".green().bold(), message);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn debug_lazy_skips_message_construction_when_debug_is_disabled() {
        let Ok(_guard) = TEST_LOCK.lock() else {
            return;
        };
        {
            let Ok(mut verbosity) = VERBOSITY.lock() else {
                return;
            };
            let Ok(mut file_level) = FILE_LOG_LEVEL.lock() else {
                return;
            };
            *verbosity = 0;
            *file_level = LogLevel::Info;
        }

        let called = Cell::new(false);
        debug_lazy(|| {
            called.set(true);
            "expensive debug message".to_string()
        });

        assert!(!called.get());
    }

    #[test]
    fn sanitize_log_message_removes_terminal_control_sequences() {
        assert_eq!(
            sanitize_log_message("\u{1b}[2K\r\u{1b}[32mready\u{1b}[0m"),
            "ready"
        );
    }

    #[test]
    fn redact_sensitive_text_removes_url_and_key_value_credentials() {
        let message = "git+https://user:secret@example.com/pkg?token=abc123";
        assert_eq!(
            redact_sensitive_text(message),
            "git+https://[REDACTED]@example.com/pkg?token=[REDACTED]"
        );
        assert_eq!(
            redact_sensitive_text("git@github.com:org/repo.git"),
            "git@github.com:org/repo.git"
        );
        assert_eq!(
            redact_sensitive_text(r#"{"token": "abc"} --token secret"#),
            r#"{"token": "[REDACTED]"} --token [REDACTED]"#
        );
        assert_eq!(
            redact_sensitive_text("--api-key=secret --private-key secret"),
            "--api-key=[REDACTED] --private-key [REDACTED]"
        );
    }

    #[test]
    fn command_output_is_logged_even_when_file_level_is_error() {
        let Ok(_guard) = TEST_LOCK.lock() else {
            return;
        };
        let log_path = std::env::temp_dir().join(format!(
            "r2x-logger-test-{}-{}.log",
            std::process::id(),
            "command-output"
        ));
        let path_string = log_path.to_string_lossy().to_string();
        if init_with_config(0, 0, false, false, Some(&path_string), None).is_err() {
            return;
        }
        if let Ok(mut file_level) = FILE_LOG_LEVEL.lock() {
            *file_level = LogLevel::Error;
        }

        record_command_output(
            "Installing",
            "demo",
            "stderr",
            b"\x1b[2K\rpassword=secret\n",
        );

        let contents = match fs::read_to_string(&log_path) {
            Ok(contents) => contents,
            Err(_) => return,
        };
        assert!(contents.contains("COMMAND_OUTPUT"));
        assert!(contents.contains("password=[REDACTED]"));
        assert!(!contents.contains('\u{1b}'));
        let _ = fs::remove_file(log_path);

        if let Ok(mut file_level) = FILE_LOG_LEVEL.lock() {
            *file_level = LogLevel::Info;
        }
    }

    #[test]
    fn debug_lazy_builds_message_when_debug_is_enabled_for_file() {
        let Ok(_guard) = TEST_LOCK.lock() else {
            return;
        };
        {
            let Ok(mut verbosity) = VERBOSITY.lock() else {
                return;
            };
            let Ok(mut file_level) = FILE_LOG_LEVEL.lock() else {
                return;
            };
            *verbosity = 0;
            *file_level = LogLevel::Debug;
        }

        let called = Cell::new(false);
        debug_lazy(|| {
            called.set(true);
            "expensive debug message".to_string()
        });

        assert!(called.get());
    }
}
