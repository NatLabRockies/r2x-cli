use anyhow::Result;
use clap::{Args, Subcommand};
#[cfg(feature = "self-update")]
use colored::Colorize;

use crate::install_source::InstallSource;

#[derive(Debug, Args)]
pub struct SelfNamespace {
    #[command(subcommand)]
    pub command: SelfCommand,
}

#[derive(Debug, Subcommand)]
pub enum SelfCommand {
    /// Update r2x.
    #[command(alias = "upgrade")]
    Update(SelfUpdateArgs),
}

#[derive(Debug, Args)]
pub struct SelfUpdateArgs {
    /// Update to the specified version. If not provided, r2x will update to the latest version.
    pub target_version: Option<String>,

    /// A GitHub token for authentication.
    /// A token is not required but can be used to reduce the chance of encountering rate limits.
    #[arg(long, env = "GITHUB_TOKEN")]
    pub token: Option<String>,

    /// Run without performing the update.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn handle_self_command(args: SelfNamespace) -> Result<i32> {
    match args.command {
        SelfCommand::Update(SelfUpdateArgs {
            target_version,
            token,
            dry_run,
        }) => handle_self_update(target_version, token, dry_run),
    }
}

#[cfg(feature = "self-update")]
fn handle_self_update(
    version: Option<String>,
    token: Option<String>,
    dry_run: bool,
) -> Result<i32> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(self_update(version, token, dry_run));
    runtime.shutdown_background();
    result
}

#[cfg(not(feature = "self-update"))]
fn handle_self_update(
    _version: Option<String>,
    _token: Option<String>,
    _dry_run: bool,
) -> Result<i32> {
    let message = InstallSource::detect()
        .map(|source| {
            format!(
                "r2x was installed via {} and cannot self-update. To update, run `{}`",
                source.description(),
                source.update_instructions()
            )
        })
        .unwrap_or_else(|| {
            "r2x was installed via an external package manager and cannot self-update. \
             Please use your package manager to update r2x."
                .to_string()
        });

    anyhow::bail!("{message}");
}

#[cfg(feature = "self-update")]
fn format_install_hint() -> String {
    match InstallSource::detect() {
        Some(source) => format!(
            "{}{} You installed r2x via {}. To update, run `{}`",
            "hint".cyan().bold(),
            ":".bold(),
            source.description(),
            source.update_instructions()
        ),
        None => format!(
            "{}{} If you installed r2x with cargo, pipx, brew, or another package manager, update r2x with `cargo install --locked r2x`, `pipx upgrade`, `brew upgrade`, or similar.",
            "hint".cyan().bold(),
            ":".bold()
        ),
    }
}

#[cfg(feature = "self-update")]
async fn self_update(version: Option<String>, token: Option<String>, dry_run: bool) -> Result<i32> {
    use axoupdater::{AxoUpdater, AxoupdateError, UpdateRequest};
    use tracing::{debug, enabled};

    let mut updater = AxoUpdater::new_for("r2x");
    if enabled!(tracing::Level::DEBUG) {
        std::env::set_var("INSTALLER_PRINT_VERBOSE", "1");
        updater.enable_installer_output();
    } else {
        updater.disable_installer_output();
    }

    if let Some(ref token) = token {
        updater.set_github_token(token);
    }

    // Load the "install receipt" for the current binary. If the receipt is not found, then
    // r2x was likely installed via a package manager.
    let Ok(updater) = updater.load_receipt() else {
        debug!("no receipt found; assuming r2x was installed via a package manager");
        eprintln!(
            "{}{} Self-update is only available for r2x binaries installed via the standalone installation scripts.",
            "error".red().bold(),
            ":".bold(),
        );
        eprintln!("{}", format_install_hint());
        return Ok(1);
    };

    // If we know what our version is, ignore whatever the receipt thinks it is.
    if let Ok(version) = env!("CARGO_PKG_VERSION").parse() {
        let _ = updater.set_current_version(version);
    }

    // Ensure the receipt is for the current binary. If it's not, then the user likely has multiple
    // r2x binaries installed, and the current binary was _not_ installed via the standalone
    // installation scripts.
    if !updater.check_receipt_is_for_this_executable()? {
        debug!(
            "receipt is not for this executable; assuming r2x was installed via a package manager"
        );
        eprintln!(
            "{}{} Self-update is only available for r2x binaries installed via the standalone installation scripts.",
            "error".red().bold(),
            ":".bold(),
        );
        eprintln!("{}", format_install_hint());
        return Ok(1);
    }

    eprintln!(
        "{}",
        format_args!(
            "{}{} Checking for updates...",
            "info".cyan().bold(),
            ":".bold()
        )
    );

    let update_request = if let Some(version) = version {
        UpdateRequest::SpecificTag(version)
    } else {
        UpdateRequest::Latest
    };

    updater.configure_version_specifier(update_request.clone());

    if dry_run {
        if updater.is_update_needed().await? {
            let version = match update_request {
                UpdateRequest::Latest | UpdateRequest::LatestMaybePrerelease => {
                    "the latest version".to_string()
                }
                UpdateRequest::SpecificTag(version) | UpdateRequest::SpecificVersion(version) => {
                    format!("v{version}")
                }
            };
            eprintln!(
                "Would update r2x from {} to {}",
                format!("v{}", env!("CARGO_PKG_VERSION")).bold().white(),
                version.bold().white(),
            );
        } else {
            eprintln!(
                "{}",
                format_args!(
                    "{}{} You're on the latest version of r2x ({})",
                    "success".green().bold(),
                    ":".bold(),
                    format!("v{}", env!("CARGO_PKG_VERSION")).bold().white()
                )
            );
        }
        return Ok(0);
    }

    match updater.run().await {
        Ok(Some(result)) => {
            let direction = if result
                .old_version
                .as_ref()
                .is_some_and(|old_version| *old_version > result.new_version)
            {
                "Downgraded"
            } else {
                "Upgraded"
            };

            let version_information = if let Some(old_version) = result.old_version {
                format!(
                    "from {} to {}",
                    format!("v{old_version}").bold().white(),
                    format!("v{}", result.new_version).bold().white(),
                )
            } else {
                format!("to {}", format!("v{}", result.new_version).bold().white())
            };

            eprintln!(
                "{}",
                format_args!(
                    "{}{} {direction} r2x {}! {}",
                    "success".green().bold(),
                    ":".bold(),
                    version_information,
                    format!(
                        "https://github.com/NatLabRockies/r2x-cli/releases/tag/{}",
                        result.new_version_tag
                    )
                    .cyan()
                )
            );
        }
        Ok(None) => {
            eprintln!(
                "{}",
                format_args!(
                    "{}{} You're on the latest version of r2x ({})",
                    "success".green().bold(),
                    ":".bold(),
                    format!("v{}", env!("CARGO_PKG_VERSION")).bold().white()
                )
            );
        }
        Err(err) => {
            return if let AxoupdateError::Reqwest(err) = err {
                if err.status() == Some(http::StatusCode::FORBIDDEN) && token.is_none() {
                    eprintln!(
                        "{}",
                        format_args!(
                            "{}{} GitHub API rate limit exceeded. Please provide a GitHub token via the {} option.",
                            "error".red().bold(),
                            ":".bold(),
                            "`--token`".green().bold()
                        )
                    );
                    Ok(1)
                } else {
                    Err(err.into())
                }
            } else {
                Err(err.into())
            };
        }
    }

    Ok(0)
}

#[cfg(all(test, feature = "self-update"))]
mod tests {
    use super::*;

    #[test]
    fn install_hint_mentions_known_source() {
        let hint = format_install_hint();
        assert!(hint.contains("r2x"));
    }
}
