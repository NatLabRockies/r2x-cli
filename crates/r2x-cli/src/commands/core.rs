use r2x_config::Config;
use r2x_logger as logger;
use r2x_python::python_bridge::is_r2x_core_api_compatible;
use std::path::Path;
use std::process::{Command, Stdio};

// Pin the published release commit. Unreleased commits can share its version
// while exposing a different API.
const R2X_CORE_PACKAGE_SPEC: &str =
    "r2x-core @ https://github.com/NatLabRockies/r2x-core/archive/8a3240dc051f4632121e89dcab8fdeead665b7f3.zip";

pub(crate) fn ensure_r2x_core_installed_for_cli() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    config.ensure_venv_path()?;
    let python_exe = config.get_venv_python_path();
    if !Path::new(&python_exe).exists() {
        return Err(format!(
            "Python executable not found at {}. Run `r2x venv create`.",
            python_exe
        )
        .into());
    }

    ensure_r2x_core_installed(&mut config, &python_exe)
}

pub(super) fn ensure_r2x_core_installed(
    config: &mut Config,
    python_exe: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let installed = module_spec_exists(python_exe, "r2x_core.system");
    if installed && is_r2x_core_api_compatible(Path::new(python_exe))? {
        logger::debug("Python runtime is compatible with this r2x-cli version");
        return Ok(());
    }

    if installed {
        logger::info("Python runtime is incompatible; repairing it automatically");
    } else {
        logger::info("A required Python runtime component is missing; installing it automatically");
    }

    install_package(config, python_exe, R2X_CORE_PACKAGE_SPEC, "r2x-core", true)?;
    if !is_r2x_core_api_compatible(Path::new(python_exe))? {
        return Err(
            "This r2x-cli release could not prepare a compatible Python runtime. Update or reinstall r2x-cli, then retry."
                .into(),
        );
    }

    Ok(())
}

pub(super) fn module_spec_exists(python_exe: &str, module_name: &str) -> bool {
    Command::new(python_exe)
        .arg("-c")
        .arg("import importlib.util, sys; sys.exit(importlib.util.find_spec(sys.argv[1]) is None)")
        .arg(module_name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn install_package(
    config: &mut Config,
    python_exe: &str,
    package_spec: &str,
    display_name: &str,
    reinstall: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let uv_path = config.ensure_uv_path()?;
    let mut install_args = vec!["pip".to_string(), "install".to_string()];
    if reinstall {
        install_args.extend(["--reinstall-package".to_string(), display_name.to_string()]);
    }
    install_args.extend([
        "--python".to_string(),
        python_exe.to_string(),
        package_spec.to_string(),
    ]);

    crate::uv::run(
        &uv_path,
        "Installing runtime dependency",
        display_name,
        install_args,
    )?;

    let action = if reinstall {
        "Reinstalled"
    } else {
        "Installed"
    };
    logger::success(&format!("{action} {display_name} into venv"));
    Ok(())
}
