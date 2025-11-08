//! Integration tests for r2x

use assert_cmd::{cargo::cargo_bin_cmd, Command};
use predicates::prelude::*;

fn r2x_cmd() -> Command {
    cargo_bin_cmd!("r2x")
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
fn test_invalid_command() {
    r2x_cmd().arg("invalid").assert().failure();
}

#[test]
fn test_plugins_help() {
    r2x_cmd()
        .args(["run", "plugin", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: r2x run plugin"));
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
        .stdout(predicate::str::contains("r2x.toml"));
}
