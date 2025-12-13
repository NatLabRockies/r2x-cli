//! Integration tests for r2x

use assert_cmd::{cargo::cargo_bin_cmd, Command};
use predicates::prelude::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tempfile::TempDir;
use which::which;

#[cfg(unix)]
const EXECUTABLE_NAME: &str = "r2x";

#[cfg(windows)]
const EXECUTABLE_NAME: &str = "r2x.exe";

fn fixture_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("r2x.toml")
}

fn r2x_cmd() -> Command {
    let mut cmd = cargo_bin_cmd!("r2x");
    cmd.env("R2X_CONFIG", fixture_config_path());
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
fn test_invalid_command() {
    r2x_cmd().arg("invalid").assert().failure();
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
        )));
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

#[test]
fn test_pipeline_reeds_test_runs() {
    let env = PipelineHarness::new().expect("pipeline harness");
    env.command()
        .arg("run")
        .arg(env.reeds_pipeline())
        .arg("reeds-test")
        .assert()
        .success();
}

#[test]
fn test_pipeline_s2p_runs() {
    let env = PipelineHarness::new().expect("pipeline harness");
    env.command()
        .arg("run")
        .arg(env.s2p_pipeline())
        .arg("s2p")
        .assert()
        .success();
}

struct PipelineHarness {
    _home: TempDir,
    config_path: PathBuf,
    site_packages: PathBuf,
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

        let config_path = config_dir.join("r2x.toml");

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
        fs::write(&s2p_pipeline, build_s2p_pipeline(&sienna_data, &s2p_output))?;

        Ok(Self {
            _home: home,
            config_path,
            site_packages,
            reeds_pipeline,
            s2p_pipeline,
        })
    }

    fn command(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("r2x");
        cmd.env("HOME", self.home_path());
        cmd.env("R2X_CONFIG", &self.config_path);
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
    if let Some(uv) = find_tool(&["uv"]) {
        let status = StdCommand::new(uv)
            .arg("venv")
            .arg(venv_path)
            .arg("--python")
            .arg("3.12")
            .status()?;
        if status.success() {
            return Ok(());
        }
    }

    if let Some(py) = find_tool(&["python3", "python"]) {
        let status = StdCommand::new(py)
            .arg("-m")
            .arg("venv")
            .arg(venv_path)
            .status()?;
        if status.success() {
            return Ok(());
        }
    }

    Err(io::Error::new(
        io::ErrorKind::Other,
        "failed to create test venv (uv/python not available)",
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
fn default_site_packages_path(venv_path: &Path) -> PathBuf {
    venv_path
        .join("lib")
        .join("python3.12")
        .join("site-packages")
}

#[cfg(target_os = "windows")]
fn default_site_packages_path(venv_path: &Path) -> PathBuf {
    venv_path.join("Lib").join("site-packages")
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
    r#"[metadata]
version = "1.0"
generated_at = "2024-01-01T00:00:00Z"

[[packages]]
name = "r2x-reeds"
entry_points_dist_info = ""
editable_install = false
install_type = "explicit"
decorator_registrations = []

[[packages.plugins]]
name = "r2x_reeds.upgrader"
kind = "upgrader"
entry = "r2x_reeds.upgrader.data_upgrader:ReEDSUpgrader"

[packages.plugins.invocation]
implementation = "class"
method = "run"
constructor = [{ name = "folder_path", source = "path", optional = false }]
call = []

[packages.plugins.io]
consumes = [{ kind = "store_folder", optional = false }]
produces = [{ kind = "store_folder", optional = false }]

[packages.plugins.resources.store]
required = true
modes = ["folder"]

[[packages.plugins]]
name = "r2x_reeds.parser"
kind = "parser"
entry = "r2x_reeds.parser:ReEDSParser"

[packages.plugins.invocation]
implementation = "class"
method = "build_system"
constructor = [
  { name = "config", source = "config", optional = false },
  { name = "data_store", source = "store", optional = false }
]
call = []

[packages.plugins.io]
consumes = [
  { kind = "store_folder", optional = false },
  { kind = "config_file", optional = true }
]
produces = [{ kind = "system", optional = false }]

[packages.plugins.resources.store]
required = true
modes = ["folder"]

[packages.plugins.resources.config]
model = "r2x_reeds.parser:ReEDSConfig"
required = true

[[packages]]
name = "r2x-sienna"
entry_points_dist_info = ""
editable_install = false
install_type = "explicit"
decorator_registrations = []

[[packages.plugins]]
name = "r2x-sienna.upgrader"
kind = "upgrader"
entry = "r2x_sienna.upgrader:SiennaUpgrader"

[packages.plugins.invocation]
implementation = "class"
method = "run"
constructor = [{ name = "path", source = "path", optional = false }]
call = []

[packages.plugins.io]
consumes = [{ kind = "store_folder", optional = false }]
produces = [{ kind = "store_folder", optional = false }]

[packages.plugins.resources.store]
required = true
modes = ["folder"]

[[packages.plugins]]
name = "r2x-sienna.parser"
kind = "parser"
entry = "r2x_sienna.parser:SiennaParser"

[packages.plugins.invocation]
implementation = "class"
method = "build_system"
constructor = [
  { name = "config", source = "config", optional = true },
  { name = "path", source = "path", optional = true }
]
call = []

[packages.plugins.io]
consumes = [
  { kind = "store_folder", optional = false },
  { kind = "config_file", optional = true }
]
produces = [{ kind = "system", optional = false }]

[packages.plugins.resources.store]
required = true
modes = ["folder"]

[packages.plugins.resources.config]
model = "r2x_sienna.parser:SiennaConfig"
required = false
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
    store_path: "{store}"
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
