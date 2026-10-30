//! Integration tests for r2x-cli

use assert_cmd::{
    cargo::{cargo_bin, cargo_bin_cmd},
    Command,
};
use predicates::prelude::*;
use std::cell::RefCell;
use std::ffi::OsString;
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
        .stdout(predicate::str::contains(
            "r2x-cli is the command-line interface",
        ));
}

#[test]
fn test_list_plugins_no_plugins() {
    r2x_cmd().arg("list").assert().success();
}

#[test]
fn test_list_filtered_json_describes_the_selected_package() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    fs::write(
        default_site_packages_path(&env.venv_path)
            .join("r2x_reeds-0.0.1.dist-info")
            .join("METADATA"),
        "Metadata-Version: 2.1\nName: r2x-reeds\nVersion: 0.0.1\n\n",
    )?;
    let output = env
        .command()
        .args(["list", "r2x-reeds", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["packages"][0]["name"], "r2x-reeds");
    assert_eq!(data["packages"][0]["version"], "0.0.1");
    for args in [vec!["list"], vec!["list", "r2x-reeds"]] {
        env.command()
            .args(args)
            .assert()
            .success()
            .stdout(predicate::str::contains("r2x-reeds:v0.0.1"));
    }
    let plugins = data["packages"][0]["plugins"]
        .as_array()
        .ok_or("missing plugins")?;
    assert!(plugins.iter().any(|plugin| {
        plugin["name"] == "r2x_reeds.parser"
            && plugin["module"] == "r2x_reeds.parser"
            && plugin["config_class"] == "ReEDSConfig"
    }));
    Ok(())
}

#[test]
fn test_list_json_filters_plugins_and_reports_missing_packages(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let output = env
        .command()
        .args(["list", "r2x-reeds", "parser", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        data["packages"][0]["plugins"]
            .as_array()
            .ok_or("missing plugins")?
            .len(),
        1
    );
    assert_eq!(
        data["packages"][0]["plugins"][0]["name"],
        "r2x_reeds.parser"
    );

    let output = env
        .command()
        .args(["list", "missing-package", "--json"])
        .output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "PACKAGE_NOT_FOUND");

    let output = env
        .command()
        .args(["list", "r2x-reeds", "missing-module", "--json"])
        .output()?;
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "PLUGIN_NOT_FOUND");
    assert!(error["error"]["candidates"]
        .as_array()
        .ok_or("missing candidates")?
        .iter()
        .any(|candidate| candidate == "r2x_reeds.parser"));

    let output = env
        .command()
        .args(["list", "r2x-reeds", "r2x_reeds.parser", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "candidate did not round-trip: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        data["packages"][0]["plugins"][0]["name"],
        "r2x_reeds.parser"
    );
    env.command()
        .args(["list", "r2x-reeds", "r2x_reeds.parser"])
        .assert()
        .success()
        .stdout(predicate::str::contains("r2x_reeds.parser"));
    Ok(())
}

#[test]
fn test_list_json_distinguishes_empty_registry_from_missing_package(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    fs::write(
        env.home_path().join(".cache/r2x/manifest.toml"),
        "version = \"3.0\"\ngenerated_at = \"2024-01-01T00:00:00Z\"\n",
    )?;
    let output = env.command().args(["list", "--json"]).output()?;
    assert!(output.status.success());
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["packages"], serde_json::json!([]));

    let output = env.command().args(["list", "missing", "--json"]).output()?;
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "PACKAGE_NOT_FOUND");
    Ok(())
}

#[test]
fn test_list_package_points_to_its_declared_mappings() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    env.command()
        .args(["list", "r2x-reeds"])
        .assert()
        .success()
        .stdout(predicate::str::contains("r2x show r2x-reeds"));
    Ok(())
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

fn write_show_mappings(env: &PipelineHarness) -> io::Result<PathBuf> {
    let config_dir = default_site_packages_path(&env.venv_path)
        .join("r2x_reeds")
        .join("config");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"cost_fom","relative_fpath":"inputs/plantcharout.csv"}]"#,
    )?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"generator_rows","source":"capacity_solve_years","operations":[
            {"operation":"join","source":"fom_solve_years","how":"left","on":["technology","year"]},
            {"operation":"filter","column":"capacity","comparison":"greater_than","value":0}
        ],"columns":{"fom_cost":"raw_value","solve_year":"year"}}]"#,
    )?;
    fs::write(config_dir.join("translation_rules.json"), "{}")?;
    Ok(config_dir)
}

#[test]
fn test_show_lists_declared_mappings_from_an_installed_package(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;

    env.command()
        .current_dir(env.home_path())
        .args(["show", "r2x-reeds"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cost_fom"))
        .stdout(predicate::str::contains(
            "generator_rows  [table:generator_rows]",
        ))
        .stdout(predicate::str::contains("Translation rules"))
        .stdout(predicate::str::contains("r2x show r2x-reeds <selector>"));

    Ok(())
}

#[test]
fn test_show_json_exposes_mapping_status_and_exact_selectors(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;

    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["package"], "r2x-reeds");
    assert_eq!(data["configuration"]["origin"], "package_default");
    assert_eq!(data["mappings"]["tables"]["status"], "present");
    assert_eq!(
        data["mappings"]["tables"]["items"][0]["selector"],
        "table:generator_rows"
    );
    assert_eq!(data["mappings"]["rules"]["status"], "empty");
    Ok(())
}

#[test]
fn test_show_json_selectors_round_trip_names_with_spaces() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"load data:west","relative_fpath":"load.csv"}]"#,
    )?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(output.status.success());
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let selector = data["mappings"]["files"]["items"][0]["selector"]
        .as_str()
        .ok_or("missing selector")?;
    assert_eq!(selector, "file:load data:west");
    let output = env
        .command()
        .args(["show", "r2x-reeds", selector, "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(output.status.success());
    let selected: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(selected["item"]["name"], "load data:west");
    Ok(())
}

#[test]
fn test_show_json_returns_a_selected_declaration() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    let output = env
        .command()
        .args([
            "show",
            "r2x-reeds",
            "table:generator_rows",
            "--json",
            "--config-dir",
        ])
        .arg(&config_dir)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["configuration"]["origin"], "override");
    assert_eq!(data["item"]["kind"], "table");
    assert_eq!(data["item"]["selector"], "table:generator_rows");
    assert_eq!(
        data["item"]["declaration"]["operations"][0]["operation"],
        "join"
    );
    assert_eq!(data["item"]["declaration"]["operations"][1]["value"], 0);
    Ok(())
}

#[test]
fn test_show_json_preserves_nested_scalar_values() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"input","info":{"integer":-4,"decimal":1.25,"enabled":true,"missing":null,"values":[1,"x",false]}}]"#,
    )?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "file:input", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        data["item"]["declaration"]["info"],
        serde_json::json!({
            "integer": -4, "decimal": 1.25, "enabled": true, "missing": null,
            "values": [1, "x", false]
        })
    );
    Ok(())
}

#[test]
fn test_show_json_preserves_large_integer_values() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"input","info":{"positive":18446744073709551617,"negative":-9223372036854775809,"metadata":{"$serde_json::private::Number":"10"}}}]"#,
    )?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "file:input", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("18446744073709551617"), "{stdout}");
    assert!(stdout.contains("-9223372036854775809"), "{stdout}");
    assert!(
        stdout.contains(r#""$serde_json::private::Number":"10""#),
        "{stdout}"
    );
    Ok(())
}

#[test]
fn test_show_json_reports_ambiguous_selectors_with_candidates(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"generator_rows","relative_fpath":"input.csv"}]"#,
    )?;
    let output = env
        .command()
        .args([
            "show",
            "r2x-reeds",
            "generator_rows",
            "--json",
            "--config-dir",
        ])
        .arg(&config_dir)
        .output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let data: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["error"]["code"], "AMBIGUOUS_MAPPING");
    assert_eq!(
        data["error"]["candidates"],
        serde_json::json!(["file:generator_rows", "table:generator_rows"])
    );
    Ok(())
}

#[test]
fn test_show_json_keeps_plugin_import_output_off_stdout() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;
    let parser = default_site_packages_path(&env.venv_path)
        .join("r2x_reeds")
        .join("parser.py");
    let source = fs::read_to_string(&parser)?.replace(
        "from __future__ import annotations",
        "from __future__ import annotations\nprint('config import diagnostic')",
    );
    fs::write(parser, source)?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json"])
        .output()?;
    assert!(output.status.success());
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["schema_version"], 1);
    assert!(String::from_utf8(output.stderr)?.contains("config import diagnostic"));

    let output = env
        .command()
        .args(["show", "r2x-reeds", "missing", "--json"])
        .output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("config import diagnostic"));
    let error: serde_json::Value =
        serde_json::from_str(stderr.lines().last().ok_or("missing error")?)?;
    assert_eq!(error["error"]["code"], "MAPPING_NOT_FOUND");
    Ok(())
}

#[test]
fn test_show_json_error_is_structured_after_default_config_discovery(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "missing", "--json"])
        .output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "MAPPING_NOT_FOUND");
    Ok(())
}

#[test]
fn test_show_json_distinguishes_missing_and_empty_assets() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    let config_dir = env.home_path().join("custom-config");
    fs::create_dir_all(&config_dir)?;
    fs::write(config_dir.join("table_mapping.json"), "[]")?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(output.status.success());
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["configuration"]["origin"], "override");
    assert_eq!(data["mappings"]["files"]["status"], "not_provided");
    assert_eq!(data["mappings"]["tables"]["status"], "empty");
    assert_eq!(data["mappings"]["rules"]["status"], "not_provided");
    Ok(())
}

#[test]
fn test_show_json_rejects_duplicate_keys_with_a_machine_error(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"sample","source":"input","operations":[
            {"operation":"filter","column":"year","\u0063olumn":"capacity","comparison":"equals","value":1}
        ]}]"#,
    )?;
    let output = env
        .command()
        .args([
            "show",
            "r2x-reeds",
            "table:sample",
            "--json",
            "--config-dir",
        ])
        .arg(&config_dir)
        .output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "INVALID_JSON");
    assert!(error["error"]["message"]
        .as_str()
        .ok_or("missing error message")?
        .contains("column"));
    Ok(())
}

#[test]
fn test_show_describes_table_operations_in_declared_order() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;

    let output = env
        .command()
        .args(["show", "r2x-reeds", "generator_rows"])
        .output()?;
    assert!(
        output.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("capacity_solve_years"), "{stdout}");
    assert!(stdout.contains("fom_solve_years"), "{stdout}");
    assert!(stdout.contains("technology"), "{stdout}");
    assert!(stdout.contains("capacity"), "{stdout}");
    let join = stdout.find("1. join").ok_or("missing join")?;
    let filter = stdout.find("2. filter").ok_or("missing filter")?;
    assert!(join < filter, "{stdout}");
    Ok(())
}

#[test]
fn test_show_displays_projected_fields_as_readable_lines() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;

    env.command()
        .args(["show", "r2x-reeds", "generator_rows"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "columns:\n    fom_cost: raw_value",
        ))
        .stdout(predicate::str::contains("solve_year: year"));
    Ok(())
}

#[test]
fn test_show_lists_and_describes_translation_rules_without_explicit_names(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("translation_rules.json"),
        r#"[{"source_type":"Generator","target_type":"ThermalUnit","version":1,
            "field_map":{"rating":"capacity"}}]"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generator -> ThermalUnit (v1)"));
    env.command()
        .args(["show", "r2x-reeds", "Generator"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ThermalUnit"))
        .stdout(predicate::str::contains("rating"))
        .stdout(predicate::str::contains("capacity"));
    Ok(())
}

#[test]
fn test_show_json_exposes_usable_selectors_for_rules_with_duplicate_names(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("translation_rules.json"),
        r#"[
            {"name":"shared","source_type":"Alpha","target_type":"First","version":1},
            {"name":"shared","source_type":"Beta","target_type":"Second","version":1}
        ]"#,
    )?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        data["mappings"]["rules"]["items"][0]["selector"],
        "rule:Alpha"
    );
    assert_eq!(
        data["mappings"]["rules"]["items"][1]["selector"],
        "rule:Beta"
    );
    env.command()
        .args(["show", "r2x-reeds", "--config-dir"])
        .arg(&config_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("shared  [rule:Alpha]"))
        .stdout(predicate::str::contains("shared  [rule:Beta]"));

    let output = env
        .command()
        .args(["show", "r2x-reeds", "rule:shared", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "AMBIGUOUS_MAPPING");
    assert_eq!(
        error["error"]["candidates"],
        serde_json::json!(["rule:Alpha", "rule:Beta"])
    );
    for (selector, target) in [("rule:Alpha", "First"), ("rule:Beta", "Second")] {
        let output = env
            .command()
            .args(["show", "r2x-reeds", selector, "--json", "--config-dir"])
            .arg(&config_dir)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let item: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(item["item"]["selector"], selector);
        assert_eq!(item["item"]["declaration"]["target_type"], target);
    }
    Ok(())
}

#[test]
fn test_show_json_does_not_invent_selectors_for_indistinguishable_rules(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("translation_rules.json"),
        r#"[
            {"name":"shared","source_type":"Alpha","target_type":"First","version":1},
            {"name":"shared","source_type":"Alpha","target_type":"Second","version":1}
        ]"#,
    )?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(output.status.success());
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(data["mappings"]["rules"]["items"][0]["selector"].is_null());
    assert!(data["mappings"]["rules"]["items"][1]["selector"].is_null());
    env.command()
        .args(["show", "r2x-reeds", "--config-dir"])
        .arg(&config_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("shared  [no unique selector]"));

    let output = env
        .command()
        .args(["show", "r2x-reeds", "rule:shared", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "AMBIGUOUS_MAPPING");
    assert_eq!(error["error"]["candidates"], serde_json::json!([]));
    Ok(())
}

#[test]
fn test_show_does_not_guess_translation_rule_object_structure(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("translation_rules.json"),
        r#"{"source_type":"Generator","target_type":"ThermalUnit","version":1}"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("translation_rules.json"))
        .stderr(predicate::str::contains("unsupported"));
    Ok(())
}

#[test]
fn test_show_json_requires_a_choice_between_multiple_configuration_directories(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    let manifest = stub_manifest_toml().replace(
        "function_name = \"no_stdin_function\"",
        "function_name = \"no_stdin_function\"\nconfig_class = \"ActiveVenvConfig\"\nconfig_module = \"r2x_reeds.parser\"",
    );
    fs::write(env.home_path().join(".cache/r2x/manifest.toml"), manifest)?;
    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json"])
        .output()?;
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "AMBIGUOUS_CONFIGURATION");
    let choices = error["error"]["candidates"]
        .as_array()
        .ok_or("missing candidates")?;
    assert_eq!(choices.len(), 2);
    assert!(choices
        .iter()
        .any(|choice| choice.as_str() == config_dir.to_str()));

    let output = env
        .command()
        .args(["show", "r2x-reeds", "--json", "--config-dir"])
        .arg(&config_dir)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["configuration"]["origin"], "override");
    Ok(())
}

#[test]
fn test_show_uses_explicit_configuration_directory() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;
    let override_dir = env.home_path().join("custom-config");
    fs::create_dir_all(&override_dir)?;
    fs::write(
        override_dir.join("table_mapping.json"),
        r#"[{"name":"custom_table","source":"input","operations":[]}]"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds", "--config-dir"])
        .arg(&override_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("custom_table"))
        .stdout(predicate::str::contains("generator_rows").not());
    Ok(())
}

#[test]
fn test_show_prefers_an_exact_rule_name_over_another_rules_source_type(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("translation_rules.json"),
        r#"[
            {"name":"Generator","source_type":"Other","target_type":"OtherTarget","version":1},
            {"source_type":"Generator","target_type":"ThermalUnit","version":1}
        ]"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds", "rule:Generator"])
        .assert()
        .success()
        .stdout(predicate::str::contains("OtherTarget"))
        .stdout(predicate::str::contains("ThermalUnit").not());
    Ok(())
}

#[test]
fn test_show_disambiguates_file_and_table_names() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"generator_rows","relative_fpath":"input.csv"}]"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds", "generator_rows"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("file:generator_rows"))
        .stderr(predicate::str::contains("table:generator_rows"));
    env.command()
        .args(["show", "r2x-reeds", "table:generator_rows"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Table: generator_rows"));
    env.command()
        .args(["show", "r2x-reeds", "file:generator_rows"])
        .assert()
        .success()
        .stdout(predicate::str::contains("input.csv"));
    Ok(())
}

#[test]
fn test_show_qualified_table_ignores_unrelated_malformed_mapping(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(config_dir.join("translation_rules.json"), "{")?;

    env.command()
        .args(["show", "r2x-reeds", "table:generator_rows"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Table: generator_rows"));
    env.command()
        .args(["show", "r2x-reeds", "generator_rows"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("translation_rules.json"));
    let output = env
        .command()
        .args(["show", "r2x-reeds", "table:generator_rows", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["item"]["selector"], "table:generator_rows");
    Ok(())
}

#[test]
fn test_show_reports_unknown_items_and_invalid_json() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;

    env.command()
        .args(["show", "r2x-reeds", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Mapping 'missing' not found"))
        .stderr(predicate::str::contains("r2x show r2x-reeds"));
    env.command()
        .args(["show", "r2x-reeds", "missing", "--config-dir"])
        .arg(&config_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("same configuration options"));
    fs::write(config_dir.join("table_mapping.json"), "{")?;
    env.command()
        .args(["show", "r2x-reeds"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid JSON"))
        .stderr(predicate::str::contains("table_mapping.json"));
    Ok(())
}

#[test]
fn test_installed_r2x_core_show_uses_public_config_path() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(env) = real_core_harness()? else {
        return Ok(());
    };
    let config_dir = default_site_packages_path(&env.venv_path)
        .join("r2x_core_integration")
        .join("config");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"input","relative_fpath":"input.csv"}]"#,
    )?;
    env.command()
        .args(["show", "r2x-core-integration", "input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("File: input"))
        .stdout(predicate::str::contains("input.csv"));
    let output = env
        .command()
        .args(["show", "r2x-core-integration", "file:input", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(data["item"]["selector"], "file:input");
    assert_eq!(data["item"]["declaration"]["relative_fpath"], "input.csv");
    Ok(())
}

#[test]
fn test_show_rejects_duplicate_json_keys_inside_operations(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"sample","source":"input","operations":[
            {"operation":"filter","column":"year","column":"capacity","comparison":"equals","value":1}
        ]}]"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds", "table:sample"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("table_mapping.json"))
        .stderr(predicate::str::contains("Duplicate JSON key 'column'"));
    Ok(())
}

#[test]
fn test_show_reports_duplicate_table_names() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"same","operations":[]},{"name":"same","operations":[]}]"#,
    )?;

    env.command()
        .args(["show", "r2x-reeds"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("table_mapping.json"))
        .stderr(predicate::str::contains("same"));
    Ok(())
}

#[test]
fn test_machine_query_reports_invalid_arguments_as_json() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    let output = env.command().args(["show", "--json"]).output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

#[test]
fn test_machine_queries_do_not_create_a_missing_environment(
) -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    let venv_path = home.path().join("missing-venv");
    let config_path = home.path().join("config.toml");
    let uv = find_tool(&["uv"]).ok_or("uv is required")?;
    let python_dir_output = StdCommand::new(&uv).args(["python", "dir"]).output()?;
    assert!(python_dir_output.status.success());
    let python_dir = String::from_utf8(python_dir_output.stdout)?
        .trim()
        .to_string();
    fs::write(
        &config_path,
        format!(
            "uv_path = {}\nvenv_path = {}\npython_version = {}\n",
            serde_json::to_string(&uv)?,
            serde_json::to_string(&venv_path.to_string_lossy())?,
            serde_json::to_string(&test_python_version())?,
        ),
    )?;

    for executable in ["r2x", "r2x-runtime"] {
        for args in [vec!["list", "--json"], vec!["show", "example", "--json"]] {
            let output = StdCommand::new(cargo_bin(executable))
                .args(args)
                .env("HOME", home.path())
                .env("R2X_CONFIG", &config_path)
                .env("UV_PYTHON_INSTALL_DIR", &python_dir)
                .env("UV_OFFLINE", "1")
                .env_remove("VIRTUAL_ENV")
                .stdin(Stdio::null())
                .output()?;
            assert!(
                !output.status.success(),
                "{executable} unexpectedly succeeded"
            );
            assert!(
                output.stdout.is_empty(),
                "unexpected output: {:?}",
                output.stdout
            );
            let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
            assert_eq!(error["error"]["code"], "ENVIRONMENT_UNAVAILABLE");
            assert!(!venv_path.exists(), "query created a virtual environment");
        }
    }
    Ok(())
}

#[test]
fn test_machine_query_reports_invalid_config_consistently() -> Result<(), Box<dyn std::error::Error>>
{
    let home = TempDir::new()?;
    let config_path = home.path().join("invalid-config.toml");
    fs::write(&config_path, "invalid = [")?;
    for executable in ["r2x", "r2x-runtime"] {
        let output = StdCommand::new(cargo_bin(executable))
            .args(["list", "--json"])
            .env("HOME", home.path())
            .env("R2X_CONFIG", &config_path)
            .stdin(Stdio::null())
            .output()?;
        assert!(!output.status.success());
        assert_eq!(output.stdout, Vec::<u8>::new());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
        assert_eq!(
            error["error"]["code"], "CONFIGURATION_ERROR",
            "{executable}"
        );
    }
    Ok(())
}

#[test]
fn test_machine_query_does_not_prompt_when_uv_is_missing() -> Result<(), Box<dyn std::error::Error>>
{
    let home = TempDir::new()?;
    let config_path = home.path().join("config.toml");
    let missing_uv = home.path().join("missing-uv");
    let venv = home.path().join("missing-venv");
    fs::write(
        &config_path,
        format!(
            "uv_path = {}\nvenv_path = {}\n",
            serde_json::to_string(&missing_uv.to_string_lossy())?,
            serde_json::to_string(&venv.to_string_lossy())?,
        ),
    )?;
    for executable in ["r2x", "r2x-runtime"] {
        let output = StdCommand::new(cargo_bin(executable))
            .args(["list", "--json"])
            .env("HOME", home.path())
            .env("R2X_CONFIG", &config_path)
            .stdin(Stdio::null())
            .output()?;
        assert!(!output.status.success());
        assert_eq!(output.stdout, Vec::<u8>::new());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
        assert_eq!(error["error"]["code"], "ENVIRONMENT_UNAVAILABLE");
        assert!(!venv.exists());
    }
    Ok(())
}

#[test]
fn test_machine_queries_work_through_the_public_launcher() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;
    for args in [
        vec!["list", "r2x-reeds", "--json"],
        vec!["show", "r2x-reeds", "table:generator_rows", "--json"],
    ] {
        let output = StdCommand::new(cargo_bin("r2x"))
            .args(args)
            .env("HOME", env.home_path())
            .env("R2X_CONFIG", &env.config_path)
            .env_remove("VIRTUAL_ENV")
            .env_remove("PYTHONPATH")
            .stdin(Stdio::null())
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(data["schema_version"], 1);
    }

    let output = StdCommand::new(cargo_bin("r2x"))
        .args(["show", "r2x-reeds", "missing", "--json"])
        .env("HOME", env.home_path())
        .env("R2X_CONFIG", &env.config_path)
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["error"]["code"], "MAPPING_NOT_FOUND");
    Ok(())
}

#[test]
fn test_machine_query_does_not_write_python_bytecode() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    write_show_mappings(&env)?;
    let site_packages = default_site_packages_path(&env.venv_path);
    fs::write(site_packages.join("sitecustomize.py"), "value = 1\n")?;
    let bytecode = site_packages.join("__pycache__");
    let has_sitecustomize_bytecode = || -> io::Result<bool> {
        if !bytecode.exists() {
            return Ok(false);
        }
        for entry in fs::read_dir(&bytecode)? {
            if entry?
                .file_name()
                .to_string_lossy()
                .starts_with("sitecustomize.")
            {
                return Ok(true);
            }
        }
        Ok(false)
    };
    assert!(!has_sitecustomize_bytecode()?);

    for executable in ["r2x", "r2x-runtime"] {
        let output = StdCommand::new(cargo_bin(executable))
            .args(["show", "r2x-reeds", "--json"])
            .env("HOME", env.home_path())
            .env("R2X_CONFIG", &env.config_path)
            .env_remove("PYTHONDONTWRITEBYTECODE")
            .env_remove("VIRTUAL_ENV")
            .env_remove("PYTHONPATH")
            .output()?;
        assert!(
            output.status.success(),
            "{executable} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !has_sitecustomize_bytecode()?,
            "{executable} wrote Python bytecode"
        );
    }
    Ok(())
}

#[test]
fn test_show_launcher_uses_the_configured_venv_python() -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let alternate_venv = env.home_path().join("alternate-venv");
    create_real_venv(&alternate_venv)?;
    let alternate_python = r2x_config::venv_paths::resolve_python_exe(&alternate_venv)?;
    let config_dir = env.venv_path.join("mapping-assets");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"active_table","source":"input","operations":[]}]"#,
    )?;

    let parser = default_site_packages_path(&env.venv_path)
        .join("r2x_reeds")
        .join("parser.py");
    let source = fs::read_to_string(&parser)?;
    let original = "return Path(os.environ[\"VIRTUAL_ENV\"]) / \"mapping-assets\"";
    assert!(source.contains(original), "missing config fixture");
    let source = source.replace(
        original,
        "from subprocess import check_output\n        return Path(check_output(['python', '-I', '-c', 'import sys; print(sys.prefix)'], text=True).strip()) / 'mapping-assets'",
    );
    fs::write(parser, source)?;
    let manifest = stub_manifest_toml().replace(
        "config_class = \"ReEDSConfig\"",
        "config_class = \"ActiveVenvConfig\"",
    );
    fs::write(env.home_path().join(".cache/r2x/manifest.toml"), manifest)?;

    let output = StdCommand::new(cargo_bin("r2x"))
        .args(["show", "r2x-reeds", "--json"])
        .env("HOME", env.home_path())
        .env("R2X_CONFIG", &env.config_path)
        .env("UV_PYTHON", alternate_python)
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .output()?;
    assert!(
        output.status.success(),
        "launcher failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        data["mappings"]["tables"]["items"][0]["name"],
        "active_table"
    );
    Ok(())
}

#[test]
fn test_show_allows_json_as_an_item_name_after_the_separator(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = PipelineHarness::new()?;
    let config_dir = write_show_mappings(&env)?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"--json","source":"input","operations":[]}]"#,
    )?;
    let output = StdCommand::new(cargo_bin("r2x"))
        .args(["show", "r2x-reeds", "--config-dir"])
        .arg(&config_dir)
        .args(["--", "--json"])
        .env("HOME", env.home_path())
        .env("R2X_CONFIG", &env.config_path)
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)?.contains("Table: --json"));
    Ok(())
}

#[test]
fn test_show_launcher_activates_the_managed_environment() -> Result<(), Box<dyn std::error::Error>>
{
    let env = PipelineHarness::new()?;
    let config_dir = env.venv_path.join("mapping-assets");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("table_mapping.json"),
        r#"[{"name":"active_table","source":"input","operations":[]}]"#,
    )?;
    let manifest = stub_manifest_toml().replace(
        "config_class = \"ReEDSConfig\"",
        "config_class = \"ActiveVenvConfig\"",
    );
    fs::write(env.home_path().join(".cache/r2x/manifest.toml"), manifest)?;

    let output = StdCommand::new(cargo_bin("r2x"))
        .args(["show", "r2x-reeds"])
        .current_dir(env.home_path())
        .env("HOME", env.home_path())
        .env("R2X_CONFIG", &env.config_path)
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .output()?;
    assert!(
        output.status.success(),
        "launcher failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)?.contains("active_table"));
    Ok(())
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
fn test_read_interactive_shows_system_summary_and_installed_plugins(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(env) = PipelineHarness::new() else {
        return Ok(());
    };

    let system_path = env.home_path().join("read-ui-system.json");
    fs::write(&system_path, r#"{"name":"read-ui-test","components":[]}"#)?;

    env.command()
        .args(["read", system_path.to_string_lossy().as_ref()])
        .assert()
        .success()
        .stdout(predicate::str::contains("System summary: read-ui-test"))
        .stdout(predicate::str::contains("\nSource: "))
        .stdout(predicate::str::contains("\nPlugins: "))
        .stdout(predicate::str::contains("\nSystem loaded as `sys`;"))
        .stdout(predicate::str::contains(
            "Plugins: 11 loaded from 2 packages",
        ));

    let script_path = env.home_path().join("inspect-plugin.py");
    fs::write(
        &script_path,
        "import sys\nprint(type(plugins.r2x_reeds.ReEDSParser()).__name__)\n",
    )?;
    env.command()
        .args([
            "read",
            "--exec",
            script_path.to_string_lossy().as_ref(),
            system_path.to_string_lossy().as_ref(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ReEDSParser"))
        .stdout(predicate::str::contains("System summary:").not());

    env.command()
        .args([
            "read",
            "--exec",
            script_path.to_string_lossy().as_ref(),
            "--interactive",
            system_path.to_string_lossy().as_ref(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ReEDSParser"))
        .stdout(predicate::str::contains("System summary: read-ui-test"));

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
fn test_installed_r2x_core_system_reports_missing_required_input(
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(env) = real_core_harness()? else {
        return Ok(());
    };

    let data_dir = env.home_path().join("missing-core-data");
    fs::create_dir_all(&data_dir)?;
    let config_dir = env.home_path().join("missing-core-config");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"input","relative_fpath":"input.csv"}]"#,
    )?;
    let store_arg = format!(
        "store={}",
        serde_json::json!({"path": data_dir.to_string_lossy().to_string()})
    );
    let config_path_arg = format!(
        "config_path_override={}",
        serde_json::to_string(&config_dir.to_string_lossy().to_string())?
    );

    let output = env
        .command()
        .args(["run", "plugin", "r2x_core_integration.parser"])
        .arg(&store_arg)
        .arg(&config_path_arg)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "unexpected success: {stderr}");
    assert!(stderr.contains("Missing required file"), "{stderr}");
    assert!(stderr.contains("input.csv"), "{stderr}");

    Ok(())
}

#[test]
fn test_installed_r2x_core_system_supports_stdout_and_file_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(env) = real_core_harness()? else {
        return Ok(());
    };

    let data_dir = env.home_path().join("real-core-data");
    fs::create_dir_all(&data_dir)?;
    fs::write(data_dir.join("input.csv"), "value\n1\n")?;
    let config_dir = env.home_path().join("real-core-config");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("file_mapping.json"),
        r#"[{"name":"input","relative_fpath":"input.csv"}]"#,
    )?;
    let store_arg = format!(
        "store={}",
        serde_json::json!({"path": data_dir.to_string_lossy().to_string()})
    );
    let config_path_arg = format!(
        "config_path_override={}",
        serde_json::to_string(&config_dir.to_string_lossy().to_string())?
    );

    let mut stdout_command = env.command();
    let stdout = stdout_command
        .args(["run", "plugin", "r2x_core_integration.parser"])
        .arg(&store_arg)
        .arg(&config_path_arg)
        .output()?;
    assert!(
        stdout.status.success(),
        "r2x-core stdout invocation failed: {}",
        String::from_utf8_lossy(&stdout.stderr)
    );
    let stdout_system: serde_json::Value = serde_json::from_slice(&stdout.stdout)?;
    assert_eq!(
        stdout_system["name"],
        "cli-r2x-core-integration",
        "stdout: {}",
        String::from_utf8_lossy(&stdout.stdout)
    );
    let sidecar = stdout_system["time_series"]["directory"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing core sidecar path"))?;
    assert!(Path::new(sidecar).is_dir());

    let output_path = env.home_path().join("real-core-output.json");
    let mut file_command = env.command();
    let file_output = file_command
        .args(["run", "plugin", "r2x_core_integration.parser"])
        .arg(&store_arg)
        .arg(&config_path_arg)
        .args(["--output", output_path.to_string_lossy().as_ref()])
        .output()?;
    assert!(
        file_output.status.success(),
        "r2x-core file invocation failed: {}",
        String::from_utf8_lossy(&file_output.stderr)
    );
    let file_system: serde_json::Value = serde_json::from_slice(&fs::read(output_path)?)?;
    assert_eq!(file_system["name"], "cli-r2x-core-integration");

    Ok(())
}

#[test]
fn test_installed_r2x_core_public_upgrade_step_fallback() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(env) = real_core_harness()? else {
        return Ok(());
    };

    let system_path = env.home_path().join("upgrade-system.json");
    fs::write(&system_path, r#"{"status":"before"}"#)?;
    let output = env
        .command()
        .args([
            "run",
            "plugin",
            "r2x_core_integration.upgrader",
            "--path",
            system_path.to_string_lossy().as_ref(),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "r2x-core upgrader fallback failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let upgraded: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(upgraded["status"], "upgraded-by-r2x-core");
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(system_path)?)?;
    assert_eq!(persisted["status"], "upgraded-by-r2x-core");

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
    assert_eq!(consumer.stdout, Vec::<u8>::new());

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
    assert_eq!(consumer.stdout, Vec::<u8>::new());

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
            "Self-update is only available for r2x-cli binaries installed via the standalone installation scripts",
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
fn test_run_package_selector_suggests_plugin_commands() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    env.command()
        .args(["run", "r2x-reeds", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is a package, not a plugin"))
        .stderr(predicate::str::contains("r2x run r2x_reeds.parser --help"));
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
fn test_direct_plugin_help_groups_plugin_and_global_options() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    env.command()
        .args(["run", "r2x_reeds.parser", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Run the r2x_reeds.parser plugin\n\nUsage: r2x run r2x_reeds.parser [OPTIONS]\n\nPlugin Options:",
        ))
        .stdout(predicate::str::contains("--path <PATH> [required]"))
        .stdout(predicate::str::contains("--solve-year <YEAR> [required]"))
        .stdout(predicate::str::contains("--weather-year <YEAR> [required]"))
        .stdout(predicate::str::contains("Global Options:"))
        .stdout(predicate::str::contains("-i, --input <FILE>"))
        .stdout(predicate::str::contains("-o, --output <FILE>"))
        .stdout(predicate::str::contains(
            "Plugin option names accept kebab-case and snake_case spellings.",
        ));

    env.command()
        .args([
            "run",
            "r2x_reeds.parser",
            "--weather-year",
            "2030",
            "--input",
            "input.json",
            "--help",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Plugin Options:"));
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
fn test_read_empty_stdin_explains_file_and_pipeline_output() {
    let Ok(env) = PipelineHarness::new() else {
        return;
    };

    env.command()
        .args(["read", "--no-banner"])
        .write_stdin("")
        .assert()
        .failure()
        .stderr(predicate::str::contains("No JSON data received from stdin"))
        .stderr(predicate::str::contains("remove `-o` to pipe JSON"));
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
    venv_path: PathBuf,
    reeds_pipeline: PathBuf,
    s2p_pipeline: PathBuf,
    python_path: OsString,
}

impl PipelineHarness {
    fn new() -> io::Result<Self> {
        Self::new_with_core_site_packages(None)
    }

    fn with_real_core(core_site_packages: &Path) -> io::Result<Self> {
        Self::new_with_core_site_packages(Some(core_site_packages))
    }

    fn new_with_core_site_packages(core_site_packages: Option<&Path>) -> io::Result<Self> {
        let core_source_path = if let Some(site_packages) = core_site_packages {
            if site_packages.join("r2x_core").is_dir() {
                None
            } else {
                let editable_path = site_packages.join("r2x_core.pth");
                let source_path = fs::read_to_string(&editable_path)?.trim().to_owned();
                let source_path = PathBuf::from(source_path);
                if !source_path.join("r2x_core").is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("r2x_core package not found via {}", editable_path.display()),
                    ));
                }
                Some(source_path)
            }
        } else {
            None
        };
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
        let manifest = if core_site_packages.is_some() {
            real_core_manifest_toml()
        } else {
            stub_manifest_toml()
        };
        fs::write(&manifest_path, manifest)?;

        copy_python_stub("IPython", &site_packages)?;
        copy_python_stub("traitlets", &site_packages)?;
        copy_python_stub("r2x_reeds", &site_packages)?;
        copy_python_stub("r2x_sienna", &site_packages)?;
        if core_site_packages.is_none() {
            copy_python_stub("r2x_core", &site_packages)?;
        } else {
            copy_python_stub("r2x_core_integration", &site_packages)?;
        }
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

        let mut python_paths = vec![site_packages.as_os_str()];
        if let Some(core_site_packages) = core_site_packages {
            python_paths.push(core_site_packages.as_os_str());
        }
        if let Some(core_source_path) = core_source_path.as_deref() {
            python_paths.push(core_source_path.as_os_str());
        }
        let python_path = std::env::join_paths(python_paths)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

        Ok(Self {
            _home: home,
            config_path,
            venv_path,
            reeds_pipeline,
            s2p_pipeline,
            python_path,
        })
    }

    fn command(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("r2x-runtime");
        cmd.env("HOME", self.home_path());
        cmd.env("R2X_CONFIG", &self.config_path);
        cmd.env("VIRTUAL_ENV", &self.venv_path);
        cmd.env("PYTHONPATH", &self.python_path);
        if let Some((name, path)) = python_library_search_path() {
            cmd.env(name, path);
        }
        cmd
    }

    fn std_command(&self) -> StdCommand {
        let mut cmd = StdCommand::new(cargo_bin("r2x-runtime"));
        cmd.env("HOME", self.home_path());
        cmd.env("R2X_CONFIG", &self.config_path);
        cmd.env("VIRTUAL_ENV", &self.venv_path);
        cmd.env("PYTHONPATH", &self.python_path);
        if let Some((name, path)) = python_library_search_path() {
            cmd.env(name, path);
        }
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

fn real_core_harness() -> io::Result<Option<PipelineHarness>> {
    let Some(core_site_packages) = std::env::var_os("R2X_CORE_SITE_PACKAGES") else {
        eprintln!(
            "Skipping installed-core integration test. Set R2X_CORE_SITE_PACKAGES to an installed r2x_core site-packages directory."
        );
        return Ok(None);
    };
    let core_site_packages = PathBuf::from(core_site_packages);
    PipelineHarness::with_real_core(&core_site_packages).map(Some)
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn python_library_search_path() -> Option<(&'static str, OsString)> {
    let python = PathBuf::from(std::env::var_os("PYO3_PYTHON")?);
    let prefix = python.parent()?.parent()?;
    #[cfg(target_os = "windows")]
    let library_dir = prefix.to_path_buf();
    #[cfg(not(target_os = "windows"))]
    let library_dir = prefix.join("lib");
    #[cfg(target_os = "macos")]
    let variable = "DYLD_LIBRARY_PATH";
    #[cfg(target_os = "linux")]
    let variable = "LD_LIBRARY_PATH";
    #[cfg(target_os = "windows")]
    let variable = "PATH";

    let mut paths = vec![library_dir];
    if let Some(existing) = std::env::var_os(variable) {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths)
        .ok()
        .map(|paths| (variable, paths))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn python_library_search_path() -> Option<(&'static str, OsString)> {
    None
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

fn real_core_manifest_toml() -> String {
    r#"version = "3.0"
generated_at = "2024-01-01T00:00:00Z"

[[packages]]
name = "r2x-core-integration"
version = "0.1.0"
editable_install = false
install_type = "explicit"

[[packages.plugins]]
name = "r2x_core_integration.parser"
type = "class"
module = "r2x_core_integration.plugin"
class_name = "CoreSystemPlugin"
config_class = "CoreConfig"
config_module = "r2x_core_integration.plugin"

[[packages.plugins.parameters]]
name = "store"
type = ["r2x_core.DataStore"]
required = false

[packages.plugins.config_schema.config_path_override]
type = "str"
required = true

[[packages.plugins]]
name = "r2x_core_integration.upgrader"
type = "class"
module = "r2x_core_integration.plugin"
class_name = "CoreUpgrader"
"#
    .to_string()
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
