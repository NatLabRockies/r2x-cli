//! Shared subprocess policy for uv operations.
//!
//! uv writes progress to stderr when it has a terminal. The runner keeps that
//! stderr attached to the user's terminal, forwards uv output to stderr so CLI
//! data output remains available for future machine-readable commands, and
//! adds explicit non-interactive options when animation is not appropriate.

use r2x_logger as logger;
use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputMode {
    Interactive,
    Linear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OutputPolicy {
    mode: OutputMode,
    disable_color: bool,
}

#[derive(Debug)]
pub(crate) struct UvCommandError {
    pub(crate) phase: String,
    pub(crate) target: String,
    pub(crate) command: String,
    pub(crate) status: Option<i32>,
    pub(crate) elapsed: Duration,
    pub(crate) reason: String,
    pub(crate) log_path: String,
}

impl std::fmt::Display for UvCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "uv command failed during {} for '{}' after {}ms (exit {:?}): {}. Command: {}. See log: {}",
            self.phase,
            self.target,
            self.elapsed.as_millis(),
            self.status,
            self.reason,
            self.command,
            self.log_path
        )
    }
}

impl std::error::Error for UvCommandError {}

fn output_policy(
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
    no_color: bool,
    term_dumb: bool,
    ci: bool,
) -> OutputPolicy {
    let interactive = stdout_is_terminal && stderr_is_terminal && !no_color && !term_dumb && !ci;
    OutputPolicy {
        mode: if interactive {
            OutputMode::Interactive
        } else {
            OutputMode::Linear
        },
        disable_color: no_color || term_dumb || ci,
    }
}

fn current_output_policy() -> OutputPolicy {
    output_policy(
        io::stdout().is_terminal(),
        io::stderr().is_terminal(),
        env::var_os("NO_COLOR").is_some(),
        env::var("TERM").ok().as_deref() == Some("dumb"),
        env::var_os("CI").is_some(),
    )
}

fn build_command_args(
    operation_args: &[String],
    policy: OutputPolicy,
    quiet: u8,
    verbose: u8,
) -> Vec<String> {
    let mut args = Vec::with_capacity(operation_args.len() + 5);

    for _ in 0..quiet {
        args.push("--quiet".to_string());
    }
    if quiet == 0 {
        for _ in 0..verbose {
            args.push("--verbose".to_string());
        }
    }

    if policy.mode == OutputMode::Linear || quiet > 0 {
        args.push("--no-progress".to_string());
    }
    if policy.disable_color {
        args.push("--color".to_string());
        args.push("never".to_string());
    }

    args.extend(operation_args.iter().cloned());
    args
}

fn forward_stream<R: Read, W: Write>(mut source: R, mut destination: W) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        let bytes_read = source.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        destination.write_all(&buffer[..bytes_read])?;
        destination.flush()?;
    }
    Ok(())
}

fn flush_logged_lines(
    pending: &mut Vec<u8>,
    phase: &str,
    target: &str,
    stream: &str,
    flush_partial: bool,
) {
    loop {
        let Some(newline) = pending.iter().position(|byte| *byte == b'\n') else {
            break;
        };
        let line: Vec<u8> = pending.drain(..=newline).collect();
        logger::record_command_output(phase, target, stream, &line);
    }

    if flush_partial && !pending.is_empty() {
        logger::record_command_output(phase, target, stream, pending);
        pending.clear();
    }
}

fn forward_logged_stream<R: Read, W: Write>(
    mut source: R,
    mut destination: W,
    phase: &str,
    target: &str,
    stream: &str,
) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    let mut pending = Vec::new();
    loop {
        let bytes_read = source.read(&mut buffer)?;
        if bytes_read == 0 {
            flush_logged_lines(&mut pending, phase, target, stream, true);
            break;
        }
        destination.write_all(&buffer[..bytes_read])?;
        destination.flush()?;
        pending.extend_from_slice(&buffer[..bytes_read]);
        flush_logged_lines(&mut pending, phase, target, stream, false);
    }
    Ok(())
}

fn is_sensitive_option(argument: &str) -> bool {
    let name = argument
        .trim_start_matches('-')
        .split('=')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('_', "-");
    matches!(
        name.as_str(),
        "access-token"
            | "refresh-token"
            | "token"
            | "password"
            | "passwd"
            | "secret"
            | "api-key"
            | "apikey"
            | "private-key"
    )
}

#[cfg(unix)]
fn configure_command(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(windows)]
fn configure_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    // CREATE_NEW_PROCESS_GROUP; taskkill /T below handles descendants.
    command.creation_flags(0x0000_0200);
}

#[cfg(not(any(unix, windows)))]
fn configure_command(_command: &mut Command) {}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let _ = Command::new("kill")
            .args(["-KILL", &process_group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(windows)]
    {
        let process_id = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", &process_id, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let _ = child.kill();
}

fn append_command_argument(command: &mut String, argument: &str) {
    command.push(' ');
    if argument
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "./_:@=-[]".contains(character))
    {
        command.push_str(argument);
    } else {
        command.push_str(&format!("{argument:?}"));
    }
}

fn format_command(uv_path: &str, args: &[String]) -> String {
    let mut command = uv_path.to_string();
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        let argument = logger::redact_sensitive_text(argument);
        if is_sensitive_option(&argument) && !argument.contains('=') && index + 1 < args.len() {
            append_command_argument(&mut command, &format!("{argument} [REDACTED]"));
            index += 2;
        } else {
            append_command_argument(&mut command, &argument);
            index += 1;
        }
    }
    command
}

/// Run one uv operation with inherited stdin and streamed output.
///
/// uv's stderr remains inherited in an interactive terminal so it can render
/// its own progress. In linear mode both streams are drained concurrently,
/// forwarded to stderr, and copied to the sanitized r2x log.
pub(crate) fn run(
    uv_path: &str,
    phase: &str,
    target: &str,
    operation_args: Vec<String>,
) -> Result<Duration, UvCommandError> {
    let target = logger::redact_sensitive_text(target);
    let policy = current_output_policy();
    let args = build_command_args(
        &operation_args,
        policy,
        logger::get_quiet_level(),
        logger::get_verbosity(),
    );
    let command_display = format_command(uv_path, &args);
    let log_path = logger::get_log_path_string();
    let start = Instant::now();

    logger::status(&format!("{phase}: {target}"));
    logger::record_command(phase, &target, &command_display, None, Duration::ZERO);

    let mut command = Command::new(uv_path);
    let capture_output = policy.mode == OutputMode::Linear;
    command
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(if capture_output {
            Stdio::piped()
        } else {
            Stdio::inherit()
        });
    configure_command(&mut command);

    let mut child = command.spawn().map_err(|error| UvCommandError {
        phase: phase.to_string(),
        target: target.clone(),
        command: command_display.clone(),
        status: None,
        elapsed: start.elapsed(),
        reason: format!("failed to start uv: {error}"),
        log_path: log_path.clone(),
    })?;

    let Some(stdout) = child.stdout.take() else {
        terminate_process_tree(&mut child);
        let _ = child.wait();
        return Err(UvCommandError {
            phase: phase.to_string(),
            target: target.clone(),
            command: command_display.clone(),
            status: None,
            elapsed: start.elapsed(),
            reason: "uv stdout was not available for streaming".to_string(),
            log_path: log_path.clone(),
        });
    };

    let (forward_error_sender, forward_error_receiver) = mpsc::channel();
    let phase_for_stdout = phase.to_string();
    let target_for_stdout = target.clone();
    let stdout_error_sender = forward_error_sender.clone();
    let stdout_forwarder = thread::spawn(move || {
        let result = if capture_output {
            forward_logged_stream(
                stdout,
                io::stderr(),
                &phase_for_stdout,
                &target_for_stdout,
                "stdout",
            )
        } else {
            forward_stream(stdout, io::stderr())
        };
        if let Err(error) = &result {
            let _ = stdout_error_sender.send(io::Error::new(error.kind(), error.to_string()));
        }
        result
    });

    let stderr_forwarder = if capture_output {
        let Some(stderr) = child.stderr.take() else {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            let _ = stdout_forwarder.join();
            return Err(UvCommandError {
                phase: phase.to_string(),
                target: target.clone(),
                command: command_display,
                status: None,
                elapsed: start.elapsed(),
                reason: "uv stderr was not available for streaming".to_string(),
                log_path,
            });
        };
        let phase_for_stderr = phase.to_string();
        let target_for_stderr = target.clone();
        let stderr_error_sender = forward_error_sender;
        Some(thread::spawn(move || {
            let result = forward_logged_stream(
                stderr,
                io::stderr(),
                &phase_for_stderr,
                &target_for_stderr,
                "stderr",
            );
            if let Err(error) = &result {
                let _ = stderr_error_sender.send(io::Error::new(error.kind(), error.to_string()));
            }
            result
        }))
    } else {
        drop(forward_error_sender);
        None
    };

    let (process_status, wait_error, forwarding_error) = loop {
        if let Ok(error) = forward_error_receiver.try_recv() {
            terminate_process_tree(&mut child);
            break (child.wait().ok(), None, Some(error));
        }

        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), None, None),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                terminate_process_tree(&mut child);
                break (child.wait().ok(), Some(error), None);
            }
        }
    };

    let elapsed = start.elapsed();
    let forwarding_error = forwarding_error.or_else(|| forward_error_receiver.try_recv().ok());
    let stdout_result = if forwarding_error.is_some() {
        None
    } else {
        Some(match stdout_forwarder.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::Other,
                "uv stdout forwarding thread panicked",
            )),
        })
    };
    let stderr_result = if forwarding_error.is_some() {
        None
    } else {
        stderr_forwarder.map(|forwarder| match forwarder.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::Other,
                "uv stderr forwarding thread panicked",
            )),
        })
    };

    let status = process_status.ok_or_else(|| UvCommandError {
        phase: phase.to_string(),
        target: target.clone(),
        command: command_display.clone(),
        status: None,
        elapsed,
        reason: format!(
            "failed while waiting for uv: {}",
            wait_error.map_or_else(
                || "the process did not report an exit status".to_string(),
                |error| error.to_string()
            )
        ),
        log_path: log_path.clone(),
    })?;
    let exit_code = status.code();
    logger::record_command(phase, &target, &command_display, exit_code, elapsed);

    let forwarding_error = forwarding_error
        .or_else(|| stdout_result.and_then(Result::err))
        .or_else(|| stderr_result.and_then(Result::err));
    if let Some(error) = forwarding_error {
        return Err(UvCommandError {
            phase: phase.to_string(),
            target: target.clone(),
            command: command_display,
            status: exit_code,
            elapsed,
            reason: format!("failed to forward uv output: {error}"),
            log_path,
        });
    }

    if !status.success() {
        return Err(UvCommandError {
            phase: phase.to_string(),
            target: target.clone(),
            command: command_display,
            status: exit_code,
            elapsed,
            reason: "process exited unsuccessfully".to_string(),
            log_path,
        });
    }

    Ok(elapsed)
}

#[cfg(test)]
mod tests {
    use super::{
        build_command_args, format_command, forward_stream, output_policy, OutputMode, OutputPolicy,
    };

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn streamed_stdout_is_forwarded_without_buffering() {
        let mut destination = Vec::new();
        let result = forward_stream("uv output\n".as_bytes(), &mut destination);
        assert!(result.is_ok());
        assert_eq!(destination, b"uv output\n");
    }

    #[test]
    fn interactive_mode_keeps_uv_progress_enabled() {
        let policy = output_policy(true, true, false, false, false);
        assert_eq!(policy.mode, OutputMode::Interactive);
        assert_eq!(
            build_command_args(&args(&["pip", "install"]), policy, 0, 0),
            args(&["pip", "install"])
        );
    }

    #[test]
    fn pipes_and_ci_use_linear_no_progress_output() {
        for policy in [
            output_policy(false, true, false, false, false),
            output_policy(true, false, false, false, false),
            output_policy(true, true, false, false, true),
            output_policy(true, true, false, true, false),
        ] {
            assert_eq!(policy.mode, OutputMode::Linear);
            let command = build_command_args(&args(&["pip", "install"]), policy, 0, 0);
            assert!(command.contains(&"--no-progress".to_string()));
        }
    }

    #[test]
    fn quiet_and_verbose_levels_are_forwarded_to_uv() {
        let interactive = OutputPolicy {
            mode: OutputMode::Interactive,
            disable_color: false,
        };
        assert_eq!(
            build_command_args(&args(&["pip"]), interactive, 2, 0),
            args(&["--quiet", "--quiet", "--no-progress", "pip"])
        );
        assert_eq!(
            build_command_args(&args(&["pip"]), interactive, 0, 2),
            args(&["--verbose", "--verbose", "pip"])
        );
    }

    #[test]
    fn command_display_redacts_credentials() {
        let command = format_command(
            "uv",
            &args(&["pip", "https://user:password@example.com/pkg?token=secret"]),
        );
        assert!(!command.contains("password@example.com"));
        assert!(!command.contains("token=secret"));
        assert!(command.contains("[REDACTED]"));

        let option_command = format_command("uv", &args(&["--token", "secret"]));
        assert!(!option_command.contains("secret"));
        let underscored_option = format_command("uv", &args(&["--api_key", "secret"]));
        assert!(!underscored_option.contains("secret"));
    }

    #[test]
    fn no_color_disables_uv_color() {
        let policy = output_policy(true, true, true, false, false);
        assert_eq!(
            build_command_args(&args(&["pip"]), policy, 0, 0),
            args(&["--no-progress", "--color", "never", "pip"])
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_subprocess_reports_phase_target_and_exit_status() {
        let error = match super::run("false", "Installing", "demo", Vec::new()) {
            Ok(_) => return,
            Err(error) => error,
        };

        assert_eq!(error.phase, "Installing");
        assert_eq!(error.target, "demo");
        assert_eq!(error.status, Some(1));
        assert!(error.elapsed < std::time::Duration::from_secs(5));
    }
}
