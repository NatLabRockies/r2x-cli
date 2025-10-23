//! Integration tests for r2x

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_version() {
    Command::cargo_bin("r2x")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("r2x"));
}

#[test]
fn test_help() {
    Command::cargo_bin("r2x")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Energy model data converter"));
}

#[test]
fn test_list_plugins_no_plugins() {
    Command::cargo_bin("r2x")
        .unwrap()
        .args(["plugin", "list"])
        .assert()
        .success();
}

#[test]
fn test_invalid_command() {
    Command::cargo_bin("r2x")
        .unwrap()
        .arg("invalid")
        .assert()
        .failure();
}

#[test]
fn test_plugins_help() {
    Command::cargo_bin("r2x")
        .unwrap()
        .args(["plugin", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Manage plugins"));
}

#[test]
fn test_config_show() {
    Command::cargo_bin("r2x")
        .unwrap()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[python]"));
}

#[test]
fn test_config_get() {
    Command::cargo_bin("r2x")
        .unwrap()
        .args(["config", "get", "python.version"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3.11"));
}
