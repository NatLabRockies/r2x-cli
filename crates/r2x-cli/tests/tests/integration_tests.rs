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

// NOTE: test_config_set is disabled because it writes to the user's actual config file
// which can corrupt it during test runs. A proper fix would require using temp directories
// via environment variables or dependency injection.
//
// #[test]
// fn test_config_set() {
//     let mut cmd = cargo_bin_cmd!("r2x");
//     cmd.arg("config").arg("set").arg("cache-path").arg("test-value");
//     cmd.assert()
//         .success()
//         .stdout(predicate::str::contains("Set cache-path = test-value"));
// }

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
