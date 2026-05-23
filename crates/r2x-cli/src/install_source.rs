use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Represents how r2x was installed on the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallSource {
    Homebrew,
    UvTool,
    Pipx,
    Cargo,
    #[cfg(feature = "self-update")]
    StandaloneInstaller,
}

impl InstallSource {
    /// Detect the install source from a given path.
    fn from_path(path: &Path) -> Option<Self> {
        // Resolve symlinks so e.g. ~/.local/bin/r2x -> .../uv/tools/r2x/bin/r2x is detected.
        let canonical = path.canonicalize().unwrap_or_else(|_| PathBuf::from(path));
        let components: Vec<_> = canonical.components().map(Component::as_os_str).collect();

        /// Check whether `components` contains a contiguous subsequence matching `pattern`.
        fn contains_sequence(components: &[&OsStr], pattern: &[&OsStr]) -> bool {
            components.windows(pattern.len()).any(|w| w == pattern)
        }

        let r2x = OsStr::new("r2x");

        // Homebrew: .../Cellar/r2x/...
        if contains_sequence(&components, &[OsStr::new("Cellar"), r2x]) {
            return Some(Self::Homebrew);
        }
        // uv tool: .../uv/tools/r2x/...
        if contains_sequence(&components, &[OsStr::new("uv"), OsStr::new("tools"), r2x]) {
            return Some(Self::UvTool);
        }
        // pipx: .../pipx/venvs/r2x/...
        if contains_sequence(&components, &[OsStr::new("pipx"), OsStr::new("venvs"), r2x]) {
            return Some(Self::Pipx);
        }
        // cargo install: .../.cargo/bin/r2x
        if components
            .windows(2)
            .any(|w| w == [OsStr::new(".cargo"), OsStr::new("bin")])
        {
            return Some(Self::Cargo);
        }

        None
    }

    #[cfg(feature = "self-update")]
    fn is_standalone_installer() -> anyhow::Result<bool> {
        use axoupdater::AxoUpdater;

        let mut updater = AxoUpdater::new_for("r2x");
        let updater = updater.load_receipt()?;
        Ok(updater.check_receipt_is_for_this_executable()?)
    }

    /// Detect the install source from the current executable path.
    pub(crate) fn detect() -> Option<Self> {
        #[cfg(feature = "self-update")]
        match Self::is_standalone_installer() {
            Ok(true) => return Some(Self::StandaloneInstaller),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to check for standalone installer: {e}"),
        }

        Self::from_path(&std::env::current_exe().ok()?)
    }

    /// Returns a human-readable description of the install source.
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Homebrew => "Homebrew",
            Self::UvTool => "uv tool",
            Self::Pipx => "pipx",
            Self::Cargo => "cargo",
            #[cfg(feature = "self-update")]
            Self::StandaloneInstaller => "the standalone installer",
        }
    }

    /// Returns the command to update r2x for this install source.
    pub(crate) fn update_instructions(self) -> &'static str {
        match self {
            Self::Homebrew => "brew update && brew upgrade r2x",
            Self::UvTool => "uv tool upgrade r2x",
            Self::Pipx => "pipx upgrade r2x",
            Self::Cargo => "cargo install --locked r2x",
            #[cfg(feature = "self-update")]
            Self::StandaloneInstaller => "r2x self update",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_homebrew_cellar_arm() {
        assert_eq!(
            InstallSource::from_path(Path::new("/opt/homebrew/Cellar/r2x/0.1.0/bin/r2x")),
            Some(InstallSource::Homebrew)
        );
    }

    #[test]
    fn detects_uv_tool() {
        assert_eq!(
            InstallSource::from_path(Path::new("/Users/me/.local/share/uv/tools/r2x/bin/r2x")),
            Some(InstallSource::UvTool)
        );
    }

    #[test]
    fn detects_pipx() {
        assert_eq!(
            InstallSource::from_path(Path::new("/Users/me/.local/pipx/venvs/r2x/bin/r2x")),
            Some(InstallSource::Pipx)
        );
    }

    #[test]
    fn detects_cargo_bin() {
        assert_eq!(
            InstallSource::from_path(Path::new("/home/me/.cargo/bin/r2x")),
            Some(InstallSource::Cargo)
        );
    }

    #[test]
    fn returns_none_for_unknown_unix_path() {
        assert_eq!(
            InstallSource::from_path(Path::new("/usr/local/bin/r2x")),
            None
        );
    }
}
