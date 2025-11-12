use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

#[test]
fn test_config_show() {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.arg("config").arg("show");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Configuration"));
}

#[test]
fn test_plugins_list() {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.arg("plugins").arg("list");
    cmd.assert().success();
}

#[test]
fn test_plugins_install() {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.arg("plugins").arg("install").arg("test-plugin");
    cmd.assert().success();
}

#[test]
fn test_plugins_remove() {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.arg("plugins").arg("remove").arg("test-plugin");
    cmd.assert().success();
}

#[test]
fn test_config_no_action() {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.arg("config");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn test_plugins_no_action() {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.arg("plugins");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}
