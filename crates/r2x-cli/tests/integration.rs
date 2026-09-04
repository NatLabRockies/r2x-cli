//! Integration tests for r2x

use assert_cmd::{
    cargo::{cargo_bin, cargo_bin_cmd},
    Command,
};
use predicates::prelude::*;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use tempfile::TempDir;
use which::which;

thread_local! {
    /// Retain `TempDir` handles so isolated fixture directories are cleaned
    /// up when the test thread exits, rather than leaking into `/tmp`.
    static ISOLATED_FIXTURE_DIRS: RefCell<Vec<TempDir>> = const { RefCell::new(Vec::new()) };
}

#[cfg(unix)]
const EXECUTABLE_NAME: &str = "r2x";

#[cfg(windows)]
const EXECUTABLE_NAME: &str = "r2x.exe";

fn fixture_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("config.toml")
}

fn isolated_fixture_config_path() -> PathBuf {
    let Ok(dir) = tempfile::tempdir() else {
        return fixture_config_path();
    };
    let config_path = dir.path().join("config.toml");
    if fs::copy(fixture_config_path(), &config_path).is_err() {
        return fixture_config_path();
    }
    ISOLATED_FIXTURE_DIRS.with(|dirs| dirs.borrow_mut().push(dir));
    config_path
}

fn r2x_cmd() -> Command {
    let mut cmd = cargo_bin_cmd!("r2x-runtime");
    cmd.env("R2X_CONFIG", isolated_fixture_config_path());
    cmd
}

fn r2x_cmd_with_config(config_path: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("r2x-runtime");
    cmd.env("R2X_CONFIG", config_path);
    cmd
}

#[test]
fn test_version() {
    r2x_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("r2x"));
}

#[test]
fn test_help() {
    r2x_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("R2X is a CLI tool"));
}

#[test]
fn test_list_plugins_no_plugins() {
    r2x_cmd().arg("list").assert().success();
}

#[test]
fn test_list_plugins_setup_failure_exits_nonzero() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let invalid_venv_path = temp_dir.path().join("not-a-venv");
    if fs::write(&invalid_venv_path, "not a directory").is_err() {
        return;
    }

    let config_path = temp_dir.path().join("invalid-config.toml");
    let cache_path = temp_dir.path().join("cache");
    if fs::write(
        &config_path,
        format!(
            "cache_path = \"{}\"\nvenv_path = \"{}\"\n",
            cache_path.to_string_lossy(),
            invalid_venv_path.to_string_lossy()
        ),
    )
    .is_err()
    {
        return;
    }

    r2x_cmd_with_config(&config_path)
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to setup venv"));
}

#[test]
fn test_invalid_command() {
    r2x_cmd().arg("invalid").assert().failure();
}

#[test]
#[cfg(not(target_os = "windows"))]
fn test_venv_create_creates_missing_default_venv_without_prompt_hint() {
    let Some(uv_path) = find_tool(&["uv"]) else {
        return;
    };
    let python_version = test_python_version();
    if !uv_can_find_python(&uv_path, &python_version) {
        return;
    }

    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let home = temp_dir.path();
    let config_dir = home.join(".config").join("r2x");
    if fs::create_dir_all(&config_dir).is_err() {
        return;
    }

    let config_path = config_dir.join("config.toml");
    if fs::write(
        &config_path,
        format!(
            "uv_path = \"{}\"\npython_version = \"{}\"\n",
            uv_path, python_version
        ),
    )
    .is_err()
    {
        return;
    }

    let venv_path = config_dir.join(".venv");
    let mut cmd = cargo_bin_cmd!("r2x-runtime");
    cmd.env("HOME", home)
        // Isolate XDG dirs so migrate_legacy_venv() cannot escape to the real
        // runner home (e.g. via XDG_CONFIG_HOME set by the CI environment).
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local").join("share"))
        .env("R2X_CONFIG", &config_path)
        .env("NO_COLOR", "1")
        .args(["venv", "create"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Virtual environment ready at"))
        .stderr(predicate::str::contains("Failed to configure venv").not())
        .stdout(predicate::str::contains("Use the `-y/--yes` flag").not());

    assert!(venv_path.join("pyvenv.cfg").is_file());
}

#[test]
fn test_plugins_help() {
    r2x_cmd()
        .args(["run", "plugin", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Usage: {} run plugin",
            EXECUTABLE_NAME
        )))
        .stdout(predicate::str::contains("--input <FILE>"))
        .stdout(predicate::str::contains("--output <FILE>"))
        .stdout(predicate::str::contains("--repeat"))
        .stdout(predicate::str::contains("--benchmark"))
        .stdout(predicate::str::contains("--pdb"));
}

#[test]
fn test_pdb_rejects_noninteractive_invocation() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    env.command()
        .args(["run", "plugin", "r2x_reeds.no_stdin_function", "--pdb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--pdb requires an interactive terminal",
        ));

    env.command()
        .args(["run", &env.reeds_pipeline(), "reeds-test", "--pdb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--pdb requires an interactive terminal",
        ));
}

#[test]
fn test_direct_plugin_output_writes_utf8_json_and_read_accepts_it(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let output_path = env.home_path().join("plugin-output.json");
    env.command()
        .args([
            "run",
            "plugin",
            "r2x-sienna.parser",
            "--output",
            output_path.to_string_lossy().as_ref(),
        ])
        .assert()
        .success();

    let bytes = fs::read(&output_path)?;
    assert!(bytes.starts_with(b"{"));
    let expected = "sienña".as_bytes();
    assert!(bytes
        .windows(expected.len())
        .any(|window| window == expected));
    assert!(!bytes.starts_with(&[0xFF, 0xFE]));
    assert!(!bytes.starts_with(&[0xFE, 0xFF]));

    env.command()
        .env("R2X_READ_NONINTERACTIVE", "1")
        .args([
            "read",
            output_path.to_string_lossy().as_ref(),
            "--no-banner",
        ])
        .assert()
        .success();

    Ok(())
}

#[test]
fn test_direct_plugin_receives_system_from_stdin() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    env.command()
        .args(["run", "r2x_reeds.with_system_function"])
        .write_stdin(r#"{"system":"reeds","status":"input"}"#)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"status\": \"function-with-system\"",
        ));

    Ok(())
}

#[test]
fn test_direct_plugin_reports_missing_system_input() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    env.command()
        .args(["run", "r2x_reeds.with_system_function"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires a System input"));

    Ok(())
}

#[test]
fn test_direct_plugin_rejects_input_for_nonconsumer() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let reeds_path = env.home_path().join("data/reeds-store");
    env.command()
        .args([
            "run",
            "r2x_reeds.parser",
            "--path",
            reeds_path.to_string_lossy().as_ref(),
            "--solve-year",
            "2032",
            "--weather-year",
            "2012",
        ])
        .write_stdin(r#"{"system":"reeds","status":"input"}"#)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "does not accept a System or JSON input",
        ));

    Ok(())
}

#[test]
fn test_direct_plugin_keeps_plugin_diagnostics_off_stdout() -> Result<(), Box<dyn std::error::Error>>
{
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    env.command()
        .args(["run", "r2x_reeds.noisy_json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("plugin diagnostic").not())
        .stdout(predicate::str::contains(r#"{"status": "ok"}"#))
        .stderr(predicate::str::contains("plugin diagnostic"));

    Ok(())
}

#[test]
fn test_direct_upgrader_keeps_plugin_diagnostics_off_stdout(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let folder_path = env.home_path().join("upgrade-source");
    env.command()
        .args([
            "run",
            "r2x_reeds.upgrader",
            "--folder-path",
            folder_path.to_string_lossy().as_ref(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("upgrader diagnostic").not())
        .stdout(predicate::str::contains(r#""upgraded": "reeds""#))
        .stderr(predicate::str::contains("upgrader diagnostic"));

    Ok(())
}

#[test]
fn test_direct_plugin_json_output_creates_parent_directories(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let output_path = env
        .home_path()
        .join("nested")
        .join("results")
        .join("output.json");
    env.command()
        .args([
            "run",
            "r2x_reeds.noisy_json",
            "-o",
            output_path.to_string_lossy().as_ref(),
        ])
        .assert()
        .success();

    let output: serde_json::Value = serde_json::from_slice(&fs::read(output_path)?)?;
    assert_eq!(output, serde_json::json!({"status": "ok"}));
    Ok(())
}

#[test]
fn test_direct_plugin_pipe_preserves_time_series_sidecars() -> Result<(), Box<dyn std::error::Error>>
{
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let mut producer = env
        .std_command()
        .args(["run", "r2x_reeds.system_with_sidecar"])
        .stdout(Stdio::piped())
        .spawn()?;
    let producer_stdout = producer.stdout.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "producer stdout was not captured",
        )
    })?;
    let consumer = env
        .std_command()
        .args(["run", "r2x_reeds.mark_streamed_system"])
        .stdin(Stdio::from(producer_stdout))
        .output()?;

    assert!(producer.wait()?.success());
    assert!(
        consumer.status.success(),
        "consumer stderr: {}",
        String::from_utf8_lossy(&consumer.stderr)
    );

    let output: serde_json::Value = serde_json::from_slice(&consumer.stdout)?;
    assert_eq!(output["status"], "streamed");
    let sidecar = output["time_series"]["directory"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sidecar path"))?;
    let sidecar = Path::new(sidecar);
    assert!(sidecar.is_absolute());
    assert!(sidecar.join("time_series_metadata.db").is_file());

    Ok(())
}

#[test]
fn test_direct_exporter_consumes_system_without_emitting_json(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let mut producer = env
        .std_command()
        .args(["run", "r2x_reeds.system_with_sidecar"])
        .stdout(Stdio::piped())
        .spawn()?;
    let producer_stdout = producer.stdout.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "producer stdout was not captured",
        )
    })?;
    let consumer = env
        .std_command()
        .args(["run", "r2x_reeds.test_exporter"])
        .stdin(Stdio::from(producer_stdout))
        .output()?;

    assert!(producer.wait()?.success());
    assert!(
        consumer.status.success(),
        "exporter stderr: {}",
        String::from_utf8_lossy(&consumer.stderr)
    );
    assert!(consumer.stdout.is_empty());

    Ok(())
}

#[test]
fn test_direct_context_exporter_uses_plugin_context_system(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let mut producer = env
        .std_command()
        .args(["run", "r2x_reeds.system_with_sidecar"])
        .stdout(Stdio::piped())
        .spawn()?;
    let producer_stdout = producer.stdout.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "producer stdout was not captured",
        )
    })?;
    let consumer = env
        .std_command()
        .args(["run", "r2x_reeds.context_exporter"])
        .stdin(Stdio::from(producer_stdout))
        .output()?;

    assert!(producer.wait()?.success());
    assert!(
        consumer.status.success(),
        "context exporter stderr: {}",
        String::from_utf8_lossy(&consumer.stderr)
    );
    assert!(consumer.stdout.is_empty());

    Ok(())
}

#[test]
fn test_direct_plugin_output_materializes_system_sidecars_for_file_input(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let output_path = env
        .home_path()
        .join("nested")
        .join("exports")
        .join("system.json");
    for _ in 0..2 {
        env.command()
            .args([
                "run",
                "r2x_reeds.system_with_sidecar",
                "-o",
                output_path.to_string_lossy().as_ref(),
            ])
            .assert()
            .success();
    }

    assert!(output_path.is_file());
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&output_path)?)?;
    assert_eq!(persisted["time_series"]["directory"], "system_time_series");
    assert!(output_path.parent().is_some_and(|parent| parent
        .join("system_time_series/time_series_metadata.db")
        .is_file()));

    env.command()
        .args([
            "run",
            "r2x_reeds.with_system_function",
            "-i",
            output_path.to_string_lossy().as_ref(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"status\": \"function-with-system\"",
        ));

    Ok(())
}

#[test]
fn test_direct_plugin_rejects_file_input_and_piped_input_together(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let input_path = env.home_path().join("input.json");
    fs::write(&input_path, r#"{"system":"reeds"}"#)?;

    env.command()
        .args([
            "run",
            "r2x_reeds.with_system_function",
            "--input",
            input_path.to_string_lossy().as_ref(),
        ])
        .write_stdin(r#"{"system":"other"}"#)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--input FILE cannot be combined with JSON supplied on stdin",
        ));

    Ok(())
}

#[test]
fn test_plugins_repeat_rejects_zero() {
    r2x_cmd()
        .args(["run", "plugin", "--repeat", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value '0'"));
}

#[test]
fn test_self_update_help() {
    r2x_cmd()
        .args(["self", "update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: r2x self update"))
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn test_self_upgrade_alias() {
    r2x_cmd()
        .args(["self", "upgrade", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: r2x self update"));
}

#[cfg(feature = "self-update")]
#[test]
fn test_self_update_requires_standalone_receipt() {
    r2x_cmd()
        .args(["self", "update", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Self-update is only available for r2x binaries installed via the standalone installation scripts",
        ));
}

#[cfg(not(feature = "self-update"))]
#[test]
fn test_self_update_requires_feature() {
    r2x_cmd()
        .args(["self", "update", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot self-update"));
}

#[test]
fn test_config_show() {
    r2x_cmd()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration:"));
}

#[test]
fn test_config_get() {
    r2x_cmd()
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("config.toml"));
}

#[test]
fn test_config_set_python_version_normalizes_value() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let config_path = temp_dir.path().join("config.toml");

    let version = format!("{}.1", test_python_version());
    r2x_cmd_with_config(&config_path)
        .args(["config", "set", "python-version", &format!(" {version} ")])
        .assert()
        .success();

    let Ok(contents) = fs::read_to_string(config_path) else {
        return;
    };
    assert!(contents.contains(&format!("python_version = \"{version}\"")));
}

#[test]
fn test_python_path_shortcut() {
    r2x_cmd()
        .args(["python", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("python"));
}

#[test]
fn test_log_set_no_stdout() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let config_path = temp_dir.path().join("config.toml");

    r2x_cmd_with_config(&config_path)
        .args(["log", "set", "no-stdout", "true"])
        .assert()
        .success();

    let Ok(contents) = fs::read_to_string(config_path) else {
        return;
    };
    assert!(contents.contains("no_stdout = true"));
}

#[test]
fn test_log_path_override_and_get() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let config_path = temp_dir.path().join("config.toml");
    let custom_log_path = temp_dir.path().join("custom-r2x.log");
    let expected_path = custom_log_path.to_string_lossy().to_string();

    r2x_cmd_with_config(&config_path)
        .args(["log", "path", &expected_path])
        .assert()
        .success()
        .stdout(predicate::str::contains(&expected_path));

    r2x_cmd_with_config(&config_path)
        .args(["log", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&expected_path));
}

#[test]
fn test_log_set_max_size_bytes() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let config_path = temp_dir.path().join("config.toml");

    r2x_cmd_with_config(&config_path)
        .args(["log", "set", "max-size", "26214400"])
        .assert()
        .success();

    let Ok(contents) = fs::read_to_string(config_path) else {
        return;
    };
    assert!(contents.contains("log_max_size = 26214400"));
}

#[test]
fn test_pipeline_reeds_test_runs() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    env.command()
        .arg("run")
        .arg(env.reeds_pipeline())
        .arg("reeds-test")
        .assert()
        .success();
}

#[test]
fn test_direct_plugin_accepts_flag_forms() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    let reeds_path = env
        .home_path()
        .join("data")
        .join("reeds-store")
        .to_string_lossy()
        .to_string();
    let path_key_value = format!("path={}", reeds_path);

    env.command()
        .args([
            "run",
            "plugin",
            "r2x_reeds.parser",
            "--path",
            &reeds_path,
            "--weather-year",
            "2012",
            "--solve-year=2025",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("reeds"));

    env.command()
        .args([
            "run",
            "plugin",
            "r2x_reeds.parser",
            &path_key_value,
            "weather_year=2012",
            "solve_year=2025",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("reeds"));

    env.command()
        .args([
            "run",
            "plugin",
            "r2x_reeds.parser",
            "--set",
            &path_key_value,
            "--weather_year",
            "2012",
            "--solve-year",
            "2025",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("reeds"));
}

#[test]
fn test_direct_plugin_unknown_option_suggests_known_flag() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    let reeds_path = env
        .home_path()
        .join("data")
        .join("reeds-store")
        .to_string_lossy()
        .to_string();

    env.command()
        .args([
            "run",
            "plugin",
            "r2x_reeds.parser",
            "--path",
            &reeds_path,
            "--weathear_year",
            "2012",
            "--solve-year",
            "2025",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown option '--weathear_year'"))
        .stderr(predicate::str::contains("Did you mean '--weather-year'?"));
}

#[test]
fn test_direct_plugin_help_prefers_copy_pasteable_kebab_flags() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    env.command()
        .args(["run", "r2x_reeds.parser", "--show-help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "r2x run r2x_reeds.parser --path <value> --solve-year <value> --weather-year <value>",
        ))
        .stdout(predicate::str::contains("--weather-year"))
        .stdout(predicate::str::contains("Alias: --weather_year"))
        .stdout(predicate::str::contains("Compatibility:"))
        .stdout(predicate::str::contains("weather_year=<value>"));
}

#[test]
fn test_pipeline_options_after_yaml_path_are_preserved() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let pipeline = env.reeds_pipeline();
    env.command()
        .args(["run", &pipeline, "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("reeds-test"));

    Ok(())
}

#[test]
fn test_pipeline_s2p_runs() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    env.command()
        .arg("run")
        .arg(env.s2p_pipeline())
        .arg("s2p")
        .assert()
        .success();
}

#[test]
fn test_pipeline_function_plugin_ignores_stdin_when_not_declared() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let pipeline_path = env
        .home_path()
        .join("pipelines")
        .join("function-no-stdin.yaml");
    let output_dir = env.home_path().join("output").join("function-no-stdin");
    if fs::create_dir_all(&output_dir).is_err() {
        return;
    }

    let pipeline_yaml = format!(
        r#"pipelines:
  function-no-stdin:
    - r2x_reeds.parser
    - r2x_reeds.no_stdin_function

config:
  r2x_reeds.parser:
    weather_year: 2012
    solve_year: 2032

output_folder: "{output}"
"#,
        output = output_dir.to_string_lossy()
    );
    if fs::write(&pipeline_path, pipeline_yaml).is_err() {
        return;
    }

    env.command()
        .arg("run")
        .arg(pipeline_path.to_string_lossy().to_string())
        .arg("function-no-stdin")
        .assert()
        .success()
        .stdout(predicate::str::contains("function-no-stdin"));
}

#[test]
fn test_pipeline_function_plugin_receives_system_when_declared() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let pipeline_path = env
        .home_path()
        .join("pipelines")
        .join("function-with-system.yaml");
    let output_dir = env.home_path().join("output").join("function-with-system");
    if fs::create_dir_all(&output_dir).is_err() {
        return;
    }

    let pipeline_yaml = format!(
        r#"pipelines:
  function-with-system:
    - r2x_reeds.parser
    - r2x_reeds.with_system_function

config:
  r2x_reeds.parser:
    weather_year: 2012
    solve_year: 2032

output_folder: "{output}"
"#,
        output = output_dir.to_string_lossy()
    );
    if fs::write(&pipeline_path, pipeline_yaml).is_err() {
        return;
    }

    env.command()
        .arg("run")
        .arg(pipeline_path.to_string_lossy().to_string())
        .arg("function-with-system")
        .assert()
        .success()
        .stdout(predicate::str::contains("function-with-system"))
        .stdout(predicate::str::contains("reeds"));
}

#[test]
fn test_pipeline_output_retains_system_sidecar_bundle() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    let pipeline_path = env.home_path().join("pipelines").join("sidecar.yaml");
    let output_path = env.home_path().join("exports").join("result.json");
    let pipeline_yaml = "pipelines:
  sidecar:
    - r2x_reeds.system_with_sidecar
";
    if fs::write(&pipeline_path, pipeline_yaml).is_err() {
        return;
    }

    env.command()
        .arg("run")
        .arg(pipeline_path.to_string_lossy().to_string())
        .arg("sidecar")
        .args(["--no-stdout", "--output"])
        .arg(&output_path)
        .assert()
        .success();

    assert!(output_path.is_file());
    assert!(output_path.parent().is_some_and(|parent| parent
        .join("system_time_series/time_series_metadata.db")
        .is_file()));
}

#[test]
fn test_pipeline_sidecar_handoff_pipes_to_read_and_cleans_up() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    let pipeline_path = env
        .home_path()
        .join("pipelines")
        .join("sidecar-handoff.yaml");
    let pipeline_yaml = "pipelines:
  sidecar-handoff:
    - r2x_reeds.system_with_sidecar
";
    if fs::write(&pipeline_path, pipeline_yaml).is_err() {
        return;
    }
    let empty_cwd = env.home_path().join("empty-cwd");
    if fs::create_dir_all(&empty_cwd).is_err() {
        return;
    }

    let mut pipeline = env.std_command();
    pipeline
        .current_dir(&empty_cwd)
        .arg("run")
        .arg(&pipeline_path)
        .arg("sidecar-handoff")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut pipeline_child) = pipeline.spawn() else {
        return;
    };
    let Some(pipeline_stdout) = pipeline_child.stdout.take() else {
        return;
    };

    let read_output = env
        .std_command()
        .current_dir(&empty_cwd)
        .env("R2X_READ_NONINTERACTIVE", "1")
        .args(["read", "--no-banner"])
        .stdin(Stdio::from(pipeline_stdout))
        .output();
    let Ok(read_output) = read_output else {
        return;
    };
    let Ok(pipeline_status) = pipeline_child.wait() else {
        return;
    };

    assert!(pipeline_status.success());
    assert!(read_output.status.success());
    let handoffs = env
        .home_path()
        .join(".cache")
        .join("r2x")
        .join("pipeline-artifacts")
        .join("handoffs");
    assert!(fs::read_dir(handoffs).is_ok_and(|mut entries| entries.next().is_none()));
}

#[test]
fn test_pipeline_artifact_upgrader_preserves_sidecar_bundle() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };
    let pipeline_path = env
        .home_path()
        .join("pipelines")
        .join("artifact-upgrader.yaml");
    let output_path = env.home_path().join("upgraded.json");
    let pipeline_yaml = format!(
        "pipelines:\n  artifact-upgrader:\n    - r2x_reeds.system_with_sidecar\n    - r2x-sienna.upgrader\n\nconfig:\n  r2x-sienna.upgrader:\n    path: \"{}\"\n",
        env.home_path().display()
    );
    if fs::write(&pipeline_path, pipeline_yaml).is_err() {
        return;
    }

    env.command()
        .arg("run")
        .arg(pipeline_path.to_string_lossy().to_string())
        .arg("artifact-upgrader")
        .args(["--no-stdout", "--output"])
        .arg(&output_path)
        .assert()
        .success();

    assert!(fs::read_to_string(&output_path).is_ok_and(|output| {
        output.contains("\"upgraded\": \"sienna\"") && output.contains("pipeline-artifacts")
    }));
    assert!(output_path.parent().is_some_and(|parent| parent
        .join("system_time_series/time_series_metadata.db")
        .is_file()));
}

#[test]
fn test_run_plugin_benchmark_repeat_outputs_summary() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let reeds_path = env
        .home_path()
        .join("data")
        .join("reeds-store")
        .to_string_lossy()
        .to_string();

    let assert = env
        .command()
        .args([
            "run",
            "plugin",
            "r2x_reeds.parser",
            "--repeat",
            "3",
            "--benchmark",
            "--path",
            &reeds_path,
            "--weather-year",
            "2012",
            "solve_year=2032",
        ])
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("Benchmark r2x_reeds.parser: runs=3"));
    assert!(stderr.contains("Benchmark r2x_reeds.parser breakdown:"));

    if let Ok(path) = std::env::var("R2X_BENCHMARK_SUMMARY_PATH") {
        let summary = stderr
            .lines()
            .filter(|line| line.starts_with("Benchmark "))
            .collect::<Vec<_>>()
            .join("\n");
        if !summary.is_empty() {
            let _ = fs::write(path, format!("{summary}\n"));
        }
    }
}

#[test]
fn test_read_file_resolves_time_series_sidecar_relative_to_json_parent() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let system_dir = env.home_path().join("exports").join("system");
    let cwd = env.home_path().join("caller-cwd");
    let sidecar_dir = system_dir.join("system_time_series");
    if fs::create_dir_all(&sidecar_dir).is_err() || fs::create_dir_all(&cwd).is_err() {
        return;
    }
    if fs::write(sidecar_dir.join("time_series_metadata.db"), "stub").is_err() {
        return;
    }
    let system_path = system_dir.join("system.json");
    if fs::write(&system_path, system_json("system_time_series")).is_err() {
        return;
    }

    env.command()
        .current_dir(cwd)
        .env("R2X_READ_NONINTERACTIVE", "1")
        .args([
            "read",
            system_path.to_string_lossy().as_ref(),
            "--no-banner",
        ])
        .assert()
        .success();
}

#[test]
fn test_read_stdin_keeps_relative_time_series_sidecars_resolved_from_cwd() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let cwd = env.home_path().join("stdin-cwd");
    let sidecar_dir = cwd.join("stdin_time_series");
    if fs::create_dir_all(&sidecar_dir).is_err() {
        return;
    }
    if fs::write(sidecar_dir.join("time_series_metadata.db"), "stub").is_err() {
        return;
    }

    env.command()
        .current_dir(cwd)
        .env("R2X_READ_NONINTERACTIVE", "1")
        .write_stdin(system_json("stdin_time_series"))
        .args(["read", "--no-banner"])
        .assert()
        .success();
}

#[test]
fn test_read_missing_time_series_sidecar_directory_reports_required_path() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let system_dir = env.home_path().join("exports").join("missing-sidecar");
    if fs::create_dir_all(&system_dir).is_err() {
        return;
    }
    let missing_sidecar = env.home_path().join("cache-backed-missing-time-series");
    let system_path = system_dir.join("system.json");
    if fs::write(&system_path, system_json(missing_sidecar.to_string_lossy())).is_err() {
        return;
    }
    let missing_sidecar_text = missing_sidecar.to_string_lossy().to_string();

    env.command()
        .env("R2X_READ_NONINTERACTIVE", "1")
        .args([
            "read",
            system_path.to_string_lossy().as_ref(),
            "--no-banner",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Time-series Sidecar Error"))
        .stderr(predicate::str::contains(
            "Missing time-series sidecar directory",
        ))
        .stderr(predicate::str::contains(missing_sidecar_text))
        .stderr(predicate::str::contains(
            "JSON references external time-series sidecar data",
        ))
        .stderr(predicate::str::contains("Verify the JSON structure").not());
}

#[test]
fn test_read_missing_time_series_metadata_db_reports_required_path() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    let system_dir = env.home_path().join("exports").join("missing-metadata-db");
    let sidecar_dir = system_dir.join("system_time_series");
    if fs::create_dir_all(&sidecar_dir).is_err() {
        return;
    }
    let system_path = system_dir.join("system.json");
    if fs::write(&system_path, system_json("system_time_series")).is_err() {
        return;
    }
    let metadata_db = sidecar_dir.join("time_series_metadata.db");
    let metadata_db_text = metadata_db.to_string_lossy().to_string();

    env.command()
        .env("R2X_READ_NONINTERACTIVE", "1")
        .args([
            "read",
            system_path.to_string_lossy().as_ref(),
            "--no-banner",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Time-series Sidecar Error"))
        .stderr(predicate::str::contains(
            "Missing time-series sidecar metadata database",
        ))
        .stderr(predicate::str::contains(metadata_db_text))
        .stderr(predicate::str::contains(
            "JSON references external time-series sidecar data",
        ))
        .stderr(predicate::str::contains("Verify the JSON structure").not());
}

fn system_json(sidecar_directory: impl AsRef<str>) -> String {
    serde_json::json!({
        "components": [],
        "supplemental_attributes": [],
        "time_series": {
            "directory": sidecar_directory.as_ref(),
            "time_series_storage_type": "arrow"
        },
        "uuid": "00000000-0000-0000-0000-000000000001",
        "r2x_core_version": "test"
    })
    .to_string()
}

struct PipelineHarness {
    _home: TempDir,
    config_path: PathBuf,
    site_packages: PathBuf,
    venv_path: PathBuf,
    reeds_pipeline: PathBuf,
    s2p_pipeline: PathBuf,
}

impl PipelineHarness {
    fn new() -> io::Result<Self> {
        let home = TempDir::new()?;
        let home_path = home.path();

        let config_dir = home_path.join(".config").join("r2x");
        fs::create_dir_all(&config_dir)?;
        let cache_dir = home_path.join(".cache").join("r2x");
        fs::create_dir_all(&cache_dir)?;

        let venv_path = config_dir.join(".venv");
        create_real_venv(&venv_path)?;
        let site_packages = default_site_packages_path(&venv_path);
        fs::create_dir_all(&site_packages)?;

        let config_path = config_dir.join("config.toml");

        fs::write(
            &config_path,
            format!(
                "cache_path = \"{}\"\nvenv_path = \"{}\"\n",
                cache_dir.to_string_lossy(),
                venv_path.to_string_lossy()
            ),
        )?;

        let manifest_path = cache_dir.join("manifest.toml");
        fs::write(&manifest_path, stub_manifest_toml())?;

        copy_python_stub("IPython", &site_packages)?;
        copy_python_stub("traitlets", &site_packages)?;
        copy_python_stub("r2x_reeds", &site_packages)?;
        copy_python_stub("r2x_sienna", &site_packages)?;
        copy_python_stub("r2x_core", &site_packages)?;
        fs::create_dir_all(site_packages.join("r2x_reeds-0.0.1.dist-info"))?;
        fs::create_dir_all(site_packages.join("r2x_sienna-0.0.1.dist-info"))?;

        let data_root = home_path.join("data");
        let reeds_data = data_root.join("reeds-store");
        let sienna_data = data_root.join("sienna-store");
        fs::create_dir_all(&reeds_data)?;
        fs::create_dir_all(&sienna_data)?;
        let sienna_system = sienna_data.join("system.json");
        fs::write(&sienna_system, r#"{"system":"sienna","status":"input"}"#)?;

        let output_root = home_path.join("output");
        fs::create_dir_all(&output_root)?;
        let reeds_output = output_root.join("reeds");
        let s2p_output = output_root.join("s2p");
        fs::create_dir_all(&reeds_output)?;
        fs::create_dir_all(&s2p_output)?;

        let pipelines_dir = home_path.join("pipelines");
        fs::create_dir_all(&pipelines_dir)?;
        let reeds_pipeline = pipelines_dir.join("reeds.yaml");
        fs::write(
            &reeds_pipeline,
            build_reeds_pipeline(&reeds_data, &reeds_output),
        )?;
        let s2p_pipeline = pipelines_dir.join("s2p.yaml");
        fs::write(
            &s2p_pipeline,
            build_s2p_pipeline(&sienna_system, &s2p_output),
        )?;

        Ok(Self {
            _home: home,
            config_path,
            site_packages,
            venv_path,
            reeds_pipeline,
            s2p_pipeline,
        })
    }

    fn command(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("r2x-runtime");
        cmd.env("HOME", self.home_path());
        cmd.env("R2X_CONFIG", &self.config_path);
        cmd.env("VIRTUAL_ENV", &self.venv_path);
        cmd.env(
            "PYTHONPATH",
            self.site_packages.to_string_lossy().to_string(),
        );
        cmd
    }

    fn std_command(&self) -> StdCommand {
        let mut cmd = StdCommand::new(cargo_bin("r2x-runtime"));
        cmd.env("HOME", self.home_path());
        cmd.env("R2X_CONFIG", &self.config_path);
        cmd.env("VIRTUAL_ENV", &self.venv_path);
        cmd.env(
            "PYTHONPATH",
            self.site_packages.to_string_lossy().to_string(),
        );
        cmd
    }

    fn home_path(&self) -> &Path {
        self._home.path()
    }

    fn reeds_pipeline(&self) -> String {
        self.reeds_pipeline.to_string_lossy().to_string()
    }

    fn s2p_pipeline(&self) -> String {
        self.s2p_pipeline.to_string_lossy().to_string()
    }
}

fn create_real_venv(venv_path: &Path) -> io::Result<()> {
    if venv_path.exists() {
        fs::remove_dir_all(venv_path)?;
    }

    let Some(uv) = find_tool(&["uv"]) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to create test venv: uv is not available",
        ));
    };
    let python_version = test_python_version();
    let status = StdCommand::new(uv)
        .args([
            "venv",
            "--no-config",
            "--no-project",
            "--managed-python",
            "--python",
            &python_version,
        ])
        .arg(venv_path)
        .status()?;
    if status.success() {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::Other,
        "failed to create test venv with uv",
    ))
}

fn find_tool(candidates: &[&str]) -> Option<String> {
    for name in candidates {
        if let Ok(path) = which(name) {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn uv_can_find_python(uv_path: &str, python_version: &str) -> bool {
    StdCommand::new(uv_path)
        .args(["python", "find", python_version, "--managed-python"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(not(target_os = "windows"))]
fn default_site_packages_path(venv_path: &Path) -> PathBuf {
    venv_path
        .join("lib")
        .join(format!("python{}", test_python_version()))
        .join("site-packages")
}

#[cfg(target_os = "windows")]
fn default_site_packages_path(venv_path: &Path) -> PathBuf {
    venv_path.join("Lib").join("site-packages")
}

fn test_python_version() -> String {
    let Ok(python) = std::env::var("PYO3_PYTHON") else {
        return "3.12".to_string();
    };

    python_minor_version(&python).unwrap_or_else(|| "3.12".to_string())
}

fn python_minor_version(python: &str) -> Option<String> {
    let output = StdCommand::new(python)
        .args([
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?;
    let version = version.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

fn copy_python_stub(package: &str, site_packages: &Path) -> io::Result<()> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("python_plugins")
        .join(package);
    let dst = site_packages.join(package);
    copy_dir_recursive(&src, &dst)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            fs::copy(&entry_path, &dest_path)?;
        }
    }
    Ok(())
}

fn stub_manifest_toml() -> String {
    r#"version = "3.0"
generated_at = "2024-01-01T00:00:00Z"

[[packages]]
name = "r2x-reeds"
version = "0.1.0"
editable_install = false
install_type = "explicit"

[[packages.plugins]]
name = "r2x_reeds.upgrader"
type = "class"
module = "r2x_reeds.upgrader.data_upgrader"
class_name = "ReEDSUpgrader"

[[packages.plugins]]
name = "r2x_reeds.parser"
type = "class"
module = "r2x_reeds.parser"
class_name = "ReEDSParser"
config_class = "ReEDSConfig"
config_module = "r2x_reeds.parser"

[packages.plugins.config_schema.path]
type = "str"
required = true

[packages.plugins.config_schema.solve_year]
type = "int"
required = true

[packages.plugins.config_schema.weather_year]
type = "int"
required = true

[[packages.plugins]]
name = "r2x_reeds.no_stdin_function"
type = "function"
module = "r2x_reeds.parser"
function_name = "no_stdin_function"

[[packages.plugins]]
name = "r2x_reeds.with_system_function"
type = "function"
module = "r2x_reeds.parser"
function_name = "with_system_function"

[[packages.plugins]]
name = "r2x_reeds.system_with_sidecar"
type = "function"
module = "r2x_reeds.parser"
function_name = "system_with_sidecar"

[[packages.plugins]]
name = "r2x_reeds.mark_streamed_system"
type = "function"
module = "r2x_reeds.parser"
function_name = "mark_streamed_system"

[[packages.plugins]]
name = "r2x_reeds.noisy_json"
type = "function"
module = "r2x_reeds.parser"
function_name = "noisy_json"

[[packages.plugins]]
name = "r2x_reeds.test_exporter"
type = "function"
module = "r2x_reeds.parser"
function_name = "test_exporter"

[[packages.plugins]]
name = "r2x_reeds.context_exporter"
type = "class"
module = "r2x_reeds.parser"
class_name = "ContextExporter"
config_class = "ReEDSConfig"
config_module = "r2x_reeds.parser"

[[packages]]
name = "r2x-sienna"
version = "0.1.0"
editable_install = false
install_type = "explicit"

[[packages.plugins]]
name = "r2x-sienna.upgrader"
type = "class"
module = "r2x_sienna.upgrader"
class_name = "SiennaUpgrader"

[[packages.plugins]]
name = "r2x-sienna.parser"
type = "class"
module = "r2x_sienna.parser"
class_name = "SiennaParser"
config_class = "SiennaConfig"
config_module = "r2x_sienna.parser"
"#
    .to_string()
}

fn build_reeds_pipeline(store_path: &Path, output: &Path) -> String {
    format!(
        r#"pipelines:
  reeds-test:
    - r2x_reeds.upgrader
    - r2x_reeds.parser

config:
  r2x_reeds.upgrader:
    folder_path: "{store}"
  r2x_reeds.parser:
    weather_year: 2012
    solve_year: 2032

output_folder: "{output}"
"#,
        store = store_path.to_string_lossy(),
        output = output.to_string_lossy()
    )
}

fn build_s2p_pipeline(system_path: &Path, output: &Path) -> String {
    format!(
        r#"pipelines:
  s2p:
    - r2x-sienna.upgrader
    - r2x-sienna.parser

config:
  r2x-sienna.upgrader:
    path: "{path}"
  r2x-sienna.parser:
    system_name: "stub"

output_folder: "{output}"
"#,
        path = system_path.to_string_lossy(),
        output = output.to_string_lossy()
    )
}
