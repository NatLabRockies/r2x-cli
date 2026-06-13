use crate::commands::plugins::context::PluginContext;
use crate::commands::plugins::utils::short_commit;
use crate::plugins::error::PluginError;
use crate::plugins::package_spec::{build_package_spec, is_git_url};
use colored::Colorize;
use r2x_ast::AstDiscovery;
use r2x_logger as logger;
use r2x_manifest::types::{InstallType, Package, PackageSource};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

#[derive(Debug, Clone)]
struct SyncPackage {
    name: String,
    manifest_version: String,
    editable_install: bool,
    source_uri: Option<String>,
    install_type: InstallType,
}

#[derive(Debug, Clone, Default)]
struct PackageState {
    version: Option<String>,
    commit_id: Option<String>,
}

#[derive(Debug, Clone)]
struct UpgradeCandidate {
    normalized_target: String,
    commit_samples: Vec<Option<String>>,
}

/// Sync plugin metadata and optionally upgrade installed plugin packages first.
///
/// With `upgrade = true`, explicit packages are upgraded using:
/// `uv pip install --upgrade --python <venv-python> <target>`.
pub fn sync_manifest(ctx: &mut PluginContext, upgrade: bool) -> Result<(), PluginError> {
    let total_start = std::time::Instant::now();

    if ctx.manifest.is_empty() {
        logger::warn("No plugins installed. Nothing to sync.");
        return Ok(());
    }

    let packages_to_sync = collect_packages_to_sync(&ctx.manifest.packages);
    if packages_to_sync.is_empty() {
        logger::warn("No packages with plugin entries found. Nothing to sync.");
        return Ok(());
    }

    let mut baseline: Option<HashMap<String, PackageState>> = None;
    let mut upgraded_count = 0usize;

    if upgrade {
        baseline = Some(capture_package_states(&packages_to_sync, &ctx.locator));
        if let Some(state_before_upgrade) = baseline.as_ref() {
            upgraded_count = upgrade_packages(&packages_to_sync, state_before_upgrade, ctx)?;
        }
        if upgraded_count > 0 {
            ctx.refresh_locator()?;
        }
    }

    let num_packages = packages_to_sync.len();
    logger::step(&format!("Syncing {} package(s)...", num_packages));

    // Partition packages into "unchanged" (skip AST) and "stale" (need rediscovery).
    //
    // For non-editable installs: if the installed version matches the manifest version,
    // the plugins are identical. Skip the entire AST parse.
    // For editable installs: always rediscover because source can change without a version bump.
    let mut unchanged_count = 0usize;
    let mut unchanged_plugins = 0usize;
    let mut needs_discovery: Vec<_> = Vec::new();

    for package in &packages_to_sync {
        let installed_version = ctx.locator.read_version(&package.name);
        let version_matches = installed_version
            .as_deref()
            .is_some_and(|v| v == package.manifest_version && !package.manifest_version.is_empty());

        if !package.editable_install && version_matches {
            // Version unchanged, skip expensive AST discovery.
            let existing_plugins = ctx
                .manifest
                .get_package(&package.name)
                .map_or(0, |p| p.plugins.len());
            unchanged_plugins += existing_plugins;
            unchanged_count += 1;
            logger::debug(&format!(
                "Skipping unchanged package '{}' v{}",
                package.name, package.manifest_version
            ));
            continue;
        }

        let package_path = match resolve_package_path(&ctx.locator, package) {
            Ok(path) => path,
            Err(e) => {
                logger::warn(&format!(
                    "Failed to locate package '{}' during sync: {}",
                    package.name, e
                ));
                continue;
            }
        };
        let version = installed_version.unwrap_or_else(|| package.manifest_version.clone());
        let dist_info = ctx.locator.find_dist_info_path(&package.name);
        let source_path = local_source_path(package);
        let source_kind = ctx
            .locator
            .detect_package_source(&package.name, source_path);
        let source_uri = resolve_source_uri(package, source_kind, source_path, &ctx.locator);
        needs_discovery.push((
            package,
            package_path,
            version,
            dist_info,
            source_kind,
            source_uri,
        ));
    }

    // Run AST discovery in parallel only for packages that actually changed.
    let mut total_plugins = unchanged_plugins;
    let mut synced_packages = unchanged_count;

    if !needs_discovery.is_empty() {
        let venv_path = ctx.venv_path.as_str();
        let results: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = needs_discovery
                .iter()
                .map(
                    |(package, package_path, version, dist_info, source_kind, source_uri)| {
                        s.spawn(move || {
                            let ast_plugins = AstDiscovery::discover_plugins(
                                package_path,
                                &package.name,
                                Some(venv_path),
                                Some(version.as_str()),
                                dist_info.as_deref(),
                            );
                            (package, version, *source_kind, source_uri, ast_plugins)
                        })
                    },
                )
                .collect();

            handles.into_iter().map(|h| h.join()).collect()
        });

        for result in results {
            let Ok((package, version, source_kind, source_uri, ast_result)) = result else {
                logger::warn("A discovery thread panicked");
                continue;
            };

            let ast_plugins = match ast_result {
                Ok(plugins) => plugins,
                Err(e) => {
                    logger::warn(&format!(
                        "Failed to discover plugins for '{}': {}",
                        package.name, e
                    ));
                    continue;
                }
            };

            let plugin_count = ast_plugins.len();
            if plugin_count == 0 {
                logger::debug(&format!("No plugins found in package '{}'", package.name));
                continue;
            }

            {
                let pkg = ctx.manifest.get_or_create_package(&package.name);
                pkg.plugins = ast_plugins;
                pkg.editable_install = package.editable_install;
                pkg.version = Arc::from(version.as_str());
                pkg.source_kind = source_kind;
                pkg.source_uri = source_uri.as_ref().map(|u| Arc::from(u.as_str()));
                pkg.install_type = package.install_type;
            }

            total_plugins += plugin_count;
            synced_packages += 1;
            logger::debug(&format!(
                "Synced {} plugin(s) from '{}'",
                plugin_count, package.name
            ));
        }
    }

    ctx.manifest.save()?;

    if upgrade && upgraded_count > 0 {
        let after = capture_package_states(&packages_to_sync, &ctx.locator);
        if let Some(before) = baseline.as_ref() {
            print_upgrade_changes(&packages_to_sync, before, &after, &ctx.locator);
        }
    }

    let elapsed_ms = total_start.elapsed().as_millis();
    println!(
        "{}",
        format!(
            "Synced {} package(s), {} plugin(s) in {}ms",
            synced_packages, total_plugins, elapsed_ms
        )
        .dimmed()
    );

    Ok(())
}

fn collect_packages_to_sync(packages: &[Package]) -> Vec<SyncPackage> {
    packages
        .iter()
        .filter(|pkg| !pkg.plugins.is_empty())
        .map(|pkg| SyncPackage {
            name: pkg.name.to_string(),
            manifest_version: pkg.version.to_string(),
            editable_install: pkg.editable_install,
            source_uri: pkg.source_uri.as_deref().map(ToString::to_string),
            install_type: pkg.install_type,
        })
        .collect()
}

/// Capture package versions and commit IDs using only filesystem reads.
///
/// Reads version from `.dist-info/METADATA` and commit from `direct_url.json`.
/// Zero subprocess overhead (previously spawned N `uv pip show` calls at ~300ms each).
fn capture_package_states(
    packages: &[SyncPackage],
    locator: &r2x_manifest::package_discovery::PackageLocator,
) -> HashMap<String, PackageState> {
    packages
        .iter()
        .map(|pkg| {
            let version = locator.read_version(&pkg.name).or_else(|| {
                let v = pkg.manifest_version.as_str();
                if v.is_empty() || v == "unknown" || v == "0.0.0" {
                    None
                } else {
                    Some(v.to_string())
                }
            });

            let commit_id = locator.direct_url_commit_id(&pkg.name);
            (pkg.name.clone(), PackageState { version, commit_id })
        })
        .collect()
}

fn upgrade_packages(
    packages: &[SyncPackage],
    baseline: &HashMap<String, PackageState>,
    ctx: &PluginContext,
) -> Result<usize, PluginError> {
    let mut candidates: HashMap<String, UpgradeCandidate> = HashMap::new();
    for pkg in packages {
        if let Some(target) = upgrade_target(pkg, &ctx.locator) {
            let normalized_target = build_package_spec(&target, None, None, None, None)?;
            let commit_sample = baseline
                .get(&pkg.name)
                .and_then(|state| state.commit_id.clone());
            candidates
                .entry(normalized_target.clone())
                .and_modify(|existing| existing.commit_samples.push(commit_sample.clone()))
                .or_insert_with(|| UpgradeCandidate {
                    normalized_target,
                    commit_samples: vec![commit_sample],
                });
        }
    }

    if candidates.is_empty() {
        return Ok(0);
    }

    // Cache git ls-remote results by (repo_url, reference) to avoid duplicate
    // network round-trips for packages from the same repo (e.g. monorepo subdirectories).
    let mut ls_remote_cache: HashMap<(String, Option<String>), Option<String>> = HashMap::new();

    let mut targets: Vec<String> = candidates
        .into_values()
        .filter_map(|candidate| {
            if is_git_url(&candidate.normalized_target) {
                let local_commit = common_git_commit(&candidate.commit_samples);
                if should_skip_git_upgrade_cached(
                    &candidate.normalized_target,
                    local_commit.as_deref(),
                    &mut ls_remote_cache,
                ) {
                    return None;
                }
            }

            Some(candidate.normalized_target)
        })
        .collect();

    if targets.is_empty() {
        return Ok(0);
    }

    targets.sort();

    let target_count = targets.len();
    logger::step(&format!("Upgrading {} package(s)...", target_count));

    for target in &targets {
        logger::info(&format!("Upgrading: {}", target));
    }

    // Batch all targets into a single uv pip install call.
    // Single resolution pass, parallel downloads, one install transaction.
    run_upgrade_batch(&ctx.uv_path, &ctx.python_path, &targets)?;

    Ok(target_count)
}

fn common_git_commit(samples: &[Option<String>]) -> Option<String> {
    let first = samples.first()?.as_deref()?;
    for sample in samples.iter().skip(1) {
        match sample.as_deref() {
            Some(commit) if commit == first => {}
            _ => return None,
        }
    }
    Some(first.to_string())
}

/// Like `should_skip_git_upgrade` but caches `git ls-remote` results by (repo_url, reference).
///
/// Monorepo subdirectory packages (e.g. 4 packages from the same R2X.git@v2.0.0) all resolve
/// to the same remote commit, so we only need one network call instead of four.
fn should_skip_git_upgrade_cached(
    target: &str,
    local_commit: Option<&str>,
    cache: &mut HashMap<(String, Option<String>), Option<String>>,
) -> bool {
    let Some(local_commit) = local_commit else {
        return false;
    };

    let Some((repo_url, reference)) = split_git_target_for_remote(target) else {
        return false;
    };

    let cache_key = (repo_url.clone(), reference.clone());
    let remote_commit = cache
        .entry(cache_key)
        .or_insert_with(|| git_ls_remote_commit(&repo_url, reference.as_deref()));

    match remote_commit {
        Some(commit) => commit == local_commit,
        None => false,
    }
}

fn split_git_target_for_remote(target: &str) -> Option<(String, Option<String>)> {
    let without_prefix = target.strip_prefix("git+")?;
    let no_fragment = without_prefix.split('#').next().unwrap_or(without_prefix);

    let protocol_end = no_fragment.find("://")?;
    let authority_start = protocol_end + 3;
    let path_start = authority_start + no_fragment[authority_start..].find('/')?;
    let ref_pos = no_fragment[path_start..]
        .rfind('@')
        .map(|pos| path_start + pos);

    if let Some(ref_pos) = ref_pos {
        let repo = no_fragment[..ref_pos].to_string();
        let reference = no_fragment[ref_pos + 1..].to_string();
        if reference.is_empty() {
            return Some((repo, None));
        }
        return Some((repo, Some(reference)));
    }

    Some((no_fragment.to_string(), None))
}

fn git_ls_remote_commit(repo_url: &str, reference: Option<&str>) -> Option<String> {
    let refspec = reference.unwrap_or("HEAD");
    let output = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", repo_url, refspec])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_ls_remote_commit(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ls_remote_commit(stdout: &str) -> Option<String> {
    let mut first: Option<String> = None;
    let mut peeled: Option<String> = None;

    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next().unwrap_or_default();

        if first.is_none() {
            first = Some(hash.to_string());
        }
        if name.ends_with("^{}") {
            peeled = Some(hash.to_string());
        }
    }

    peeled.or(first)
}

fn upgrade_target(
    package: &SyncPackage,
    locator: &r2x_manifest::package_discovery::PackageLocator,
) -> Option<String> {
    if package.editable_install && local_source_path(package).is_some() {
        return None;
    }

    let mut git_target: Option<String> = None;
    if let Some(uri) = package.source_uri.as_deref() {
        if is_git_url(uri) {
            git_target = Some(uri.to_string());
        } else if Path::new(uri).exists() {
            return None;
        }
    }

    if git_target.is_none() {
        if let Some(origin) = locator.direct_url_origin(&package.name) {
            if is_git_url(&origin) {
                git_target = Some(origin);
            }
        }
    }

    if let Some(target) = git_target {
        return Some(target);
    }

    if package.install_type != InstallType::Explicit {
        return None;
    }

    Some(package.name.clone())
}

/// Install/upgrade all targets in a single `uv pip install --upgrade` call.
///
/// uv handles multiple packages natively: single resolution pass, parallel downloads,
/// one installation transaction. This is dramatically faster than N sequential calls.
fn run_upgrade_batch(
    uv_path: &str,
    python_path: &str,
    targets: &[String],
) -> Result<(), PluginError> {
    let normalized: Vec<String> = targets
        .iter()
        .map(|t| build_package_spec(t, None, None, None, None))
        .collect::<Result<Vec<_>, _>>()?;

    logger::debug(&format!(
        "Running: {} pip install --upgrade --python {} {}",
        uv_path,
        python_path,
        normalized.join(" ")
    ));

    let mut cmd = Command::new(uv_path);
    cmd.args([
        "pip",
        "install",
        "--python",
        python_path,
        "--upgrade",
        "--prerelease=allow",
        "--no-progress",
    ]);
    for target in &normalized {
        cmd.arg(target.as_str());
    }

    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(PluginError::Io)?;

    if !status.success() {
        return Err(PluginError::CommandFailed {
            command: format!("{uv_path} pip install --upgrade {}", normalized.join(" ")),
            status: status.code(),
        });
    }

    Ok(())
}

fn resolve_package_path(
    locator: &r2x_manifest::package_discovery::PackageLocator,
    package: &SyncPackage,
) -> Result<PathBuf, PluginError> {
    if let Some(path) = local_source_path(package) {
        return Ok(PathBuf::from(path));
    }

    locator.find_package_path(&package.name).map_err(|e| {
        PluginError::Locator(format!(
            "Failed to resolve package '{}': {}",
            package.name, e
        ))
    })
}

fn local_source_path(package: &SyncPackage) -> Option<&str> {
    let uri = package.source_uri.as_deref()?;
    if is_git_url(uri) || !Path::new(uri).exists() {
        return None;
    }
    Some(uri)
}

fn resolve_source_uri(
    package: &SyncPackage,
    source_kind: PackageSource,
    source_path: Option<&str>,
    locator: &r2x_manifest::package_discovery::PackageLocator,
) -> Option<String> {
    if let Some(path) = source_path {
        return Some(path.to_string());
    }

    if matches!(source_kind, PackageSource::Github | PackageSource::Git) {
        if let Some(origin) = locator.direct_url_origin(&package.name) {
            return Some(origin);
        }
    }

    package.source_uri.clone()
}

fn print_upgrade_changes(
    packages: &[SyncPackage],
    before: &HashMap<String, PackageState>,
    after: &HashMap<String, PackageState>,
    locator: &r2x_manifest::package_discovery::PackageLocator,
) {
    let mut changed = 0usize;
    for package in packages {
        let source_kind = locator.detect_package_source(&package.name, local_source_path(package));
        let previous = before.get(&package.name).cloned().unwrap_or_default();
        let current = after.get(&package.name).cloned().unwrap_or_default();

        let version_delta = match (&previous.version, &current.version) {
            (Some(old), Some(new)) if old != new => Some(format!("version {} -> {}", old, new)),
            (None, Some(new)) => Some(format!("version <unknown> -> {}", new)),
            _ => None,
        };

        let commit_delta = match (&previous.commit_id, &current.commit_id) {
            (Some(old), Some(new)) if old != new => Some(format!(
                "commit {} -> {}",
                short_commit(old),
                short_commit(new)
            )),
            (None, Some(new))
                if matches!(source_kind, PackageSource::Github | PackageSource::Git) =>
            {
                Some(format!("commit <unknown> -> {}", short_commit(new)))
            }
            _ => None,
        };

        let change_line = match (version_delta, commit_delta) {
            (Some(v), Some(c)) => Some(format!("{v}; {c}")),
            (Some(v), None) => Some(v),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };

        if let Some(line) = change_line {
            changed += 1;
            println!(
                " {} {} {}",
                "~".bold().yellow(),
                package.name.bold(),
                line.dimmed()
            );
        }
    }

    let _ = changed;
}

#[cfg(test)]
mod tests {
    use super::*;
    use r2x_manifest::package_discovery::PackageLocator;
    use tempfile::TempDir;

    fn sample_package(
        name: &str,
        source_uri: Option<&str>,
        editable_install: bool,
        install_type: InstallType,
    ) -> SyncPackage {
        SyncPackage {
            name: name.to_string(),
            manifest_version: "0.1.0".to_string(),
            editable_install,
            source_uri: source_uri.map(ToString::to_string),
            install_type,
        }
    }

    fn empty_locator() -> Option<(TempDir, PackageLocator)> {
        let temp = match TempDir::new() {
            Ok(temp) => temp,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return None;
            }
        };
        let locator = match PackageLocator::new(temp.path().to_path_buf(), None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create package locator: {err}"
                );
                return None;
            }
        };
        Some((temp, locator))
    }

    #[test]
    fn upgrade_target_uses_package_name_for_pypi() {
        let Some((_tmp, locator)) = empty_locator() else {
            return;
        };
        let pkg = sample_package("r2x-reeds", None, false, InstallType::Explicit);
        assert_eq!(
            upgrade_target(&pkg, &locator),
            Some("r2x-reeds".to_string())
        );
    }

    #[test]
    fn upgrade_target_uses_git_uri_when_present() {
        let Some((_tmp, locator)) = empty_locator() else {
            return;
        };
        let pkg = sample_package(
            "r2x-reeds",
            Some("git+https://github.com/NREL/r2x-reeds.git@main"),
            false,
            InstallType::Explicit,
        );
        assert_eq!(
            upgrade_target(&pkg, &locator),
            Some("git+https://github.com/NREL/r2x-reeds.git@main".to_string())
        );
    }

    #[test]
    fn upgrade_target_skips_local_editable_paths() {
        let Some((tmp, locator)) = empty_locator() else {
            return;
        };
        let local = tmp.path().join("r2x-local");
        if let Err(err) = std::fs::create_dir_all(&local) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create local package dir: {err}"
            );
            return;
        }
        let pkg = sample_package(
            "r2x-local",
            Some(local.to_string_lossy().as_ref()),
            true,
            InstallType::Explicit,
        );
        assert_eq!(upgrade_target(&pkg, &locator), None);
    }

    #[test]
    fn upgrade_target_falls_back_to_direct_url_origin() {
        let tmp = match TempDir::new() {
            Ok(tmp) => tmp,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let site_packages = tmp.path().to_path_buf();
        let dist_info = site_packages.join("r2x_reeds-0.1.0.dist-info");
        if let Err(err) = std::fs::create_dir_all(&dist_info) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create dist-info dir: {err}"
            );
            return;
        }
        if let Err(err) = std::fs::write(
            dist_info.join("direct_url.json"),
            r#"{
  "url": "git+https://github.com/NREL/r2x-reeds.git",
  "vcs_info": { "vcs": "git", "requested_revision": "main", "commit_id": "abc123" }
}"#,
        ) {
            assert!(
                err.to_string().is_empty(),
                "Failed to write direct_url: {err}"
            );
            return;
        }
        let locator = match PackageLocator::new(site_packages, None) {
            Ok(locator) => locator,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create package locator: {err}"
                );
                return;
            }
        };

        let pkg = sample_package("r2x-reeds", None, false, InstallType::Explicit);
        assert_eq!(
            upgrade_target(&pkg, &locator),
            Some("git+https://github.com/NREL/r2x-reeds.git@main".to_string())
        );
    }

    #[test]
    fn upgrade_target_skips_dependency_packages() {
        let Some((_tmp, locator)) = empty_locator() else {
            return;
        };
        let pkg = sample_package("r2x-reeds", None, false, InstallType::Dependency);
        assert_eq!(upgrade_target(&pkg, &locator), None);
    }

    #[test]
    fn upgrade_target_includes_git_dependencies() {
        let Some((_tmp, locator)) = empty_locator() else {
            return;
        };
        let pkg = sample_package(
            "r2x-reeds-to-sienna",
            Some("git+https://github.com/NatLabRockies/R2X.git@v2.0.0#subdirectory=packages/r2x-reeds-to-sienna"),
            false,
            InstallType::Dependency,
        );
        assert_eq!(
            upgrade_target(&pkg, &locator),
            Some("git+https://github.com/NatLabRockies/R2X.git@v2.0.0#subdirectory=packages/r2x-reeds-to-sienna".to_string())
        );
    }

    #[test]
    fn split_git_target_for_remote_handles_ssh_ref_and_fragment() {
        let target = "git+ssh://git@github.com/NatLabRockies/R2X.git@v2.0.0#subdirectory=packages/r2x-reeds-to-sienna";
        assert_eq!(
            split_git_target_for_remote(target),
            Some((
                "ssh://git@github.com/NatLabRockies/R2X.git".to_string(),
                Some("v2.0.0".to_string())
            ))
        );
    }

    #[test]
    fn split_git_target_for_remote_handles_no_ref() {
        let target = "git+https://github.com/NREL/r2x-reeds.git";
        assert_eq!(
            split_git_target_for_remote(target),
            Some(("https://github.com/NREL/r2x-reeds.git".to_string(), None))
        );
    }

    #[test]
    fn parse_ls_remote_commit_prefers_peeled_tag_commit() {
        let stdout = "1111111111111111111111111111111111111111\trefs/tags/v1.0.0\n2222222222222222222222222222222222222222\trefs/tags/v1.0.0^{}\n";
        assert_eq!(
            parse_ls_remote_commit(stdout),
            Some("2222222222222222222222222222222222222222".to_string())
        );
    }
}
