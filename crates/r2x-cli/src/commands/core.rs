use clap::Subcommand;
use r2x_config::Config;
use r2x_logger as logger;
use r2x_python::python_bridge::is_r2x_core_api_compatible;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Subcommand, Debug, Clone)]
pub enum CoreCommand {
    /// Reinstall the configured r2x-core package when it is missing or incompatible.
    Repair {
        /// Clear the configured version and use r2x-cli's default package source.
        #[arg(long)]
        use_default: bool,
    },
}

pub fn handle_repair(use_default: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    config.ensure_venv_path()?;
    if use_default && config.r2x_core_version.take().is_some() {
        config.save()?;
        logger::info(
            "Cleared the configured r2x-core version and selected the default package source",
        );
    }
    let python_exe = config.get_venv_python_path();
    if !Path::new(&python_exe).exists() {
        return Err(format!(
            "Python executable not found at {}. Run `r2x venv create`.",
            python_exe
        )
        .into());
    }

    ensure_r2x_core_installed(&mut config, &python_exe)?;
    logger::success("Configured r2x-core package is compatible");
    Ok(())
}

pub(super) fn ensure_r2x_core_installed(
    config: &mut Config,
    python_exe: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let package_spec = config.get_r2x_core_package_spec();
    let installed = module_spec_exists(python_exe, "r2x_core.system");
    if installed && is_r2x_core_api_compatible(Path::new(python_exe))? {
        logger::debug("r2x-core API is compatible with this r2x-cli version");
        return Ok(());
    }

    if installed
        && config
            .r2x_core_version
            .as_deref()
            .is_some_and(is_exact_r2x_core_version)
    {
        return Err(incompatible_core_message(config).into());
    }

    if installed {
        logger::info("Installed r2x-core is incompatible; reinstalling configured package via uv");
    } else {
        logger::info("r2x-core not found in venv; installing via uv pip install");
    }

    install_package(config, python_exe, &package_spec, "r2x-core", installed)?;
    if !is_r2x_core_api_compatible(Path::new(python_exe))? {
        return Err(incompatible_core_message(config).into());
    }

    Ok(())
}

fn is_exact_r2x_core_version(version: &str) -> bool {
    !version.trim().is_empty()
        && !version.contains(".*")
        && !["<=", ">=", "~=", "!=", "<", ">", ","]
            .iter()
            .any(|operator| version.contains(operator))
}

fn incompatible_core_message(config: &Config) -> String {
    if let Some(version) = config
        .r2x_core_version
        .as_deref()
        .filter(|version| is_exact_r2x_core_version(version))
    {
        return format!(
            "The configured r2x-core version `{version}` is incompatible with this r2x-cli version. Run `r2x core repair --use-default` to use the validated default package source, or set a compatible version with `r2x config set r2x-core-version <version>`."
        );
    }

    "The configured r2x-core package is still incompatible after reinstall. Set a compatible version with `r2x config set r2x-core-version <version>`.".to_string()
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
