pub fn detect_build_python_version() -> Result<Option<String>, String> {
    let requested_version = requested_build_python_version();
    let requested_abi = requested_version
        .as_deref()
        .map(|value| requested_python_abi_version("R2X_PYTHON_VERSION", value))
        .transpose()?;

    let Some(python) = std::env::var("PYO3_PYTHON")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let version = detect_python_version(&python)
        .map_err(|error| format!("PYO3_PYTHON={python:?} is not usable: {error}"))?;
    ensure_selected_python_matches_request("PYO3_PYTHON", &version, requested_abi.as_deref())?;
    Ok(Some(version))
}

pub fn detect_python_version(python: &str) -> Result<String, String> {
    let output = std::process::Command::new(python)
        .args([
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .output()
        .map_err(|error| format!("failed to execute interpreter: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "interpreter exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let version = String::from_utf8(output.stdout)
        .map_err(|error| format!("interpreter printed non-UTF-8 output: {error}"))?;
    let version = version.trim();
    if version.is_empty() {
        return Err("interpreter did not print a Python version".to_string());
    }

    ensure_supported_python_version(version)?;
    Ok(version.to_string())
}

fn requested_build_python_version() -> Option<String> {
    std::env::var("R2X_PYTHON_VERSION")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn ensure_selected_python_matches_request(
    label: &str,
    selected_version: &str,
    requested_abi: Option<&str>,
) -> Result<(), String> {
    let Some(requested_abi) = requested_abi else {
        return Ok(());
    };
    let selected_abi = requested_python_abi_version(label, selected_version)?;
    if selected_abi != requested_abi {
        return Err(format!(
            "{label} resolves to Python {selected_abi} but R2X_PYTHON_VERSION requests {requested_abi}"
        ));
    }
    Ok(())
}

fn ensure_supported_python_version(version: &str) -> Result<(), String> {
    requested_python_abi_version("Python", version).map(|_| ())
}

pub fn requested_python_abi_version(label: &str, version: &str) -> Result<String, String> {
    let version = version.trim();
    let mut parts = version.split('.');
    let major = parts
        .next()
        .ok_or_else(|| format!("{label} must be a Python version like 3.12 or 3.12.1"))?
        .parse::<u16>()
        .map_err(|_| {
            format!("{label} must be a Python version like 3.12 or 3.12.1, got {version:?}")
        })?;
    let minor = parts
        .next()
        .ok_or_else(|| {
            format!("{label} must be a Python version like 3.12 or 3.12.1, got {version:?}")
        })?
        .parse::<u16>()
        .map_err(|_| {
            format!("{label} must be a Python version like 3.12 or 3.12.1, got {version:?}")
        })?;

    match parts.next() {
        Some(patch) if patch.parse::<u16>().is_ok() && parts.next().is_none() => {}
        Some(_) => {
            return Err(format!(
                "{label} must be a Python version like 3.12 or 3.12.1, got {version:?}"
            ));
        }
        None => {}
    }

    if major != 3 || minor < 11 {
        return Err(format!(
            "{label}={version} is not supported; r2x requires Python 3.11 or newer"
        ));
    }

    Ok(format!("3.{minor}"))
}
