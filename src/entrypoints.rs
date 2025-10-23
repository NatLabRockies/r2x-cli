//! Entry point wrapper management
//!
//! Creates and manages wrapper executables for plugin entry points.
//! - Unix/macOS/Linux: Symlinks
//! - Windows: Batch files (.bat)

use crate::{R2xError, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct EntryPoint {
    pub name: String,
    pub kind: EntryPointKind,
}

#[derive(Debug, Clone)]
pub enum EntryPointKind {
    Parser,
    Exporter,
    Modifier,
}

impl EntryPoint {
    pub fn from_command_name(command: &str) -> Self {
        if let Some(name) = command.strip_suffix("_parser") {
            EntryPoint {
                name: name.to_string(),
                kind: EntryPointKind::Parser,
            }
        } else if let Some(name) = command.strip_suffix("_exporter") {
            EntryPoint {
                name: name.to_string(),
                kind: EntryPointKind::Exporter,
            }
        } else {
            EntryPoint {
                name: command.to_string(),
                kind: EntryPointKind::Modifier,
            }
        }
    }

    pub fn wrapper_name(&self) -> String {
        match self.kind {
            EntryPointKind::Parser => format!("{}_parser", self.name),
            EntryPointKind::Exporter => format!("{}_exporter", self.name),
            EntryPointKind::Modifier => self.name.clone(),
        }
    }

    pub fn subcommand(&self) -> &'static str {
        match self.kind {
            EntryPointKind::Parser => "read",
            EntryPointKind::Exporter => "write",
            EntryPointKind::Modifier => "run",
        }
    }
}

pub fn discover_all_entry_points() -> Result<Vec<EntryPoint>> {
    crate::python::init()?;
    let registry = crate::python::plugin::discover_plugins()?;

    let mut entry_points = Vec::new();

    for (name, _) in registry.parsers {
        entry_points.push(EntryPoint {
            name,
            kind: EntryPointKind::Parser,
        });
    }

    for (name, _) in registry.exporters {
        entry_points.push(EntryPoint {
            name,
            kind: EntryPointKind::Exporter,
        });
    }

    for (name, _) in registry.modifiers {
        entry_points.push(EntryPoint {
            name,
            kind: EntryPointKind::Modifier,
        });
    }

    Ok(entry_points)
}

fn get_bin_directory() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()
        .map_err(|e| R2xError::ConfigError(format!("Failed to get current exe path: {}", e)))?;

    let bin_dir = exe_path.parent().ok_or_else(|| {
        R2xError::ConfigError(format!(
            "Failed to get parent directory of exe: {:?}",
            exe_path
        ))
    })?;

    Ok(bin_dir.to_path_buf())
}

pub fn create_wrapper(entry_point: &EntryPoint) -> Result<PathBuf> {
    let bin_dir = get_bin_directory()?;
    let r2x_exe = std::env::current_exe()
        .map_err(|e| R2xError::ConfigError(format!("Failed to get current exe: {}", e)))?;

    let wrapper_name = entry_point.wrapper_name();

    #[cfg(unix)]
    {
        create_symlink(&bin_dir, &wrapper_name, &r2x_exe)
    }

    #[cfg(windows)]
    {
        create_batch_file(&bin_dir, &wrapper_name, &r2x_exe, entry_point)
    }
}

#[cfg(unix)]
fn create_symlink(bin_dir: &Path, wrapper_name: &str, r2x_exe: &Path) -> Result<PathBuf> {
    let link_path = bin_dir.join(wrapper_name);

    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        debug!("Removing existing wrapper: {:?}", link_path);
        std::fs::remove_file(&link_path).ok();
    }

    let r2x_filename = r2x_exe
        .file_name()
        .ok_or_else(|| R2xError::ConfigError("Invalid r2x exe path".to_string()))?;

    std::os::unix::fs::symlink(r2x_filename, &link_path).map_err(|e| {
        R2xError::ConfigError(format!("Failed to create symlink {:?}: {}", link_path, e))
    })?;

    Ok(link_path)
}

#[cfg(windows)]
fn create_batch_file(
    bin_dir: &Path,
    wrapper_name: &str,
    r2x_exe: &Path,
    entry_point: &EntryPoint,
) -> Result<PathBuf> {
    let bat_path = bin_dir.join(format!("{}.bat", wrapper_name));

    let content = format!(
        "@echo off\r\n\"{}\" {} {} %*\r\n",
        r2x_exe.display(),
        entry_point.subcommand(),
        entry_point.name
    );

    std::fs::write(&bat_path, content).map_err(|e| {
        R2xError::ConfigError(format!("Failed to create batch file {:?}: {}", bat_path, e))
    })?;

    Ok(bat_path)
}

pub fn remove_wrapper(entry_point: &EntryPoint) -> Result<()> {
    let bin_dir = get_bin_directory()?;
    let wrapper_name = entry_point.wrapper_name();

    #[cfg(unix)]
    let wrapper_path = bin_dir.join(&wrapper_name);

    #[cfg(windows)]
    let wrapper_path = bin_dir.join(format!("{}.bat", wrapper_name));

    if wrapper_path.exists() || wrapper_path.symlink_metadata().is_ok() {
        std::fs::remove_file(&wrapper_path).map_err(|e| {
            R2xError::ConfigError(format!("Failed to remove wrapper {:?}: {}", wrapper_path, e))
        })?;
    }

    Ok(())
}

pub fn create_all_wrappers() -> Result<Vec<String>> {
    let entry_points = discover_all_entry_points()?;
    let mut created = Vec::new();

    for entry_point in entry_points {
        match create_wrapper(&entry_point) {
            Ok(_) => {
                created.push(entry_point.wrapper_name());
            }
            Err(e) => {
                warn!(
                    "Failed to create wrapper for {}: {}",
                    entry_point.wrapper_name(),
                    e
                );
            }
        }
    }

    Ok(created)
}

pub fn remove_all_wrappers() -> Result<Vec<String>> {
    let entry_points = discover_all_entry_points()?;
    let mut removed = Vec::new();

    for entry_point in entry_points {
        match remove_wrapper(&entry_point) {
            Ok(_) => {
                removed.push(entry_point.wrapper_name());
            }
            Err(e) => {
                warn!(
                    "Failed to remove wrapper for {}: {}",
                    entry_point.wrapper_name(),
                    e
                );
            }
        }
    }

    Ok(removed)
}
