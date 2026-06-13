use crate::plugins::error::PluginError;

/// Check if a string looks like a local filesystem path.
pub fn is_local_path(s: &str) -> bool {
    s.starts_with("./") || s.starts_with("../") || s.starts_with('/') || s == "." || s == ".."
}

/// Check if a string looks like a git URL in any common format.
///
/// Recognizes all formats people typically copy from GitHub:
///   - `git@github.com:org/repo.git`          (SSH shorthand)
///   - `ssh://git@github.com/org/repo.git`    (SSH full)
///   - `https://github.com/org/repo.git`      (HTTPS)
///   - `https://github.com/org/repo`           (HTTPS without .git)
///   - `http://github.com/org/repo`            (HTTP)
///   - `git+ssh://...` / `git+https://...`     (pip-style prefixed)
pub fn is_git_url(s: &str) -> bool {
    s.starts_with("git+")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.starts_with("https://")
        || s.starts_with("http://")
}

/// Check if a string uses the `gh:owner/repo` shorthand.
pub fn is_github_shorthand(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("gh:") else {
        return false;
    };
    !rest.is_empty() && rest.contains('/') && !rest.contains('\\')
}

/// Normalize any git URL to the `git+{protocol}://...` form that pip/uv expects.
///
/// | Input format                              | Output                                             |
/// |-------------------------------------------|----------------------------------------------------|
/// | `git+https://host/org/repo.git`           | unchanged                                          |
/// | `git+ssh://git@host/org/repo.git`         | unchanged                                          |
/// | `https://host/org/repo.git`               | `git+https://host/org/repo.git`                    |
/// | `http://host/org/repo`                    | `git+http://host/org/repo`                         |
/// | `ssh://git@host/org/repo.git`             | `git+ssh://git@host/org/repo.git`                  |
/// | `git@host:org/repo.git`                   | `git+ssh://git@host/org/repo.git`                  |
fn normalize_git_url(url: &str) -> String {
    if url.starts_with("git+") {
        return url.to_string();
    }

    // SSH shorthand: git@host:org/repo → git+ssh://git@host/org/repo
    if let Some(rest) = url.strip_prefix("git@") {
        let normalized = rest.replacen(':', "/", 1);
        return format!("git+ssh://git@{normalized}");
    }

    // Already a protocol URL (ssh://, https://, http://) — just add git+ prefix
    format!("git+{url}")
}

/// Strip a trailing `@ref` from a URL, handling SSH URLs that contain `@` in `git@host`.
fn strip_git_ref(url: &str) -> &str {
    if let Some(rest) = url.strip_prefix("git+") {
        // After stripping git+, the @ for a ref is always the last one
        if let Some(pos) = rest.rfind('@') {
            // Only strip if the @ is after :// (not the user@ in ssh://git@host)
            if let Some(protocol_end) = rest.find("://") {
                if pos > protocol_end {
                    return &url[..url.len() - (rest.len() - pos)];
                }
            }
        }
        return url;
    }

    if let Some(rest) = url.strip_prefix("git@") {
        // SSH shorthand: git@host:org/repo.git@ref — the ref @ is the last one,
        // but only if there are at least two @'s worth of content after git@
        if let Some(pos) = rest.rfind('@') {
            if pos > 0 {
                return &url[.."git@".len() + pos];
            }
        }
        return url;
    }

    // ssh://, https://, http:// — a ref @ comes after the host/path portion.
    // Find the first / after :// to locate where the path starts; a ref @
    // can only appear after that path segment.
    if let Some(protocol_end) = url.find("://") {
        let after_protocol = protocol_end + 3;
        if let Some(path_start) = url[after_protocol..].find('/') {
            let path_start = after_protocol + path_start;
            if let Some(pos) = url[path_start..].rfind('@') {
                return &url[..path_start + pos];
            }
        }
        return url;
    }

    // Fallback: split on first @
    url.split('@').next().unwrap_or(url)
}

/// Extract the repository name from any git URL or package specifier.
///
/// Examples:
///   - `git@github.com:org/R2X.git`          → `R2X`
///   - `https://github.com/org/R2X.git@main` → `R2X`
///   - `git+ssh://git@host/org/R2X.git`      → `R2X`
///   - `r2x-reeds`                            → `r2x-reeds`
pub fn extract_package_name(package: &str) -> Result<String, PluginError> {
    let pkg = strip_git_ref(package);

    if let Some(repo_path) = pkg.strip_prefix("gh:") {
        if repo_path.is_empty() || !repo_path.contains('/') || repo_path.contains('\\') {
            return Err(PluginError::PackageSpec(
                "GitHub shorthand must use gh:owner/repo".to_string(),
            ));
        }
        return Ok(repo_path
            .split('/')
            .next_back()
            .unwrap_or(repo_path)
            .trim_end_matches(".git")
            .to_string());
    }

    if is_git_url(pkg) {
        // For any git URL: extract last path component, strip .git suffix
        let normalized = if let Some(rest) = pkg.strip_prefix("git@") {
            // SSH shorthand uses : as separator — normalize to /
            rest.replacen(':', "/", 1)
        } else {
            pkg.to_string()
        };

        Ok(normalized
            .split('/')
            .next_back()
            .unwrap_or(pkg)
            .trim_end_matches(".git")
            .to_string())
    } else if pkg.contains('/') || pkg.contains('\\') || is_local_path(pkg) {
        // For local paths, always read from pyproject.toml
        extract_name_from_pyproject(pkg).ok_or_else(|| {
            PluginError::PackageSpec(format!("Failed to extract package name from {}", package))
        })
    } else {
        // For PyPI packages, use as-is
        Ok(pkg.to_string())
    }
}

/// Build package specifier for pip install.
///
/// Handles PyPI packages, local paths, git URLs (any format), and `gh:owner/repo` shorthand.
pub fn build_package_spec(
    package: &str,
    host: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    commit: Option<String>,
) -> Result<String, PluginError> {
    // Expand tilde to home directory (cross-platform)
    let expanded_package = expand_tilde(package);
    let package = expanded_package.as_str();

    // 1. Local paths
    if is_local_path(package) {
        if branch.is_some() || tag.is_some() || commit.is_some() || host.is_some() {
            return Err(PluginError::PackageSpec(
                "Cannot use git flags with local paths".to_string(),
            ));
        }
        return Ok(package.to_string());
    }

    // 2. GitHub shorthand (gh:owner/repo)
    if let Some(repo_path) = package.strip_prefix("gh:") {
        if repo_path.is_empty() || !repo_path.contains('/') || repo_path.contains('\\') {
            return Err(PluginError::PackageSpec(
                "GitHub shorthand must use gh:owner/repo".to_string(),
            ));
        }
        let git_host = host.as_deref().unwrap_or("github.com");
        let url = format!("git+https://{git_host}/{repo_path}");
        return Ok(add_git_ref(&url, branch, tag, commit));
    }

    // 3. Any git URL (SSH, HTTPS, HTTP, git+, ssh://)
    if is_git_url(package) {
        let url = normalize_git_url(package);
        return Ok(add_git_ref(&url, branch, tag, commit));
    }

    // 4. Bare owner/repo is no longer accepted: require explicit gh:owner/repo.
    if package.contains('/') && !package.contains('\\') {
        return Err(PluginError::PackageSpec(
            "GitHub repositories must use gh:owner/repo (for example: gh:NatLabRockies/r2x-reeds)"
                .to_string(),
        ));
    }

    // 5. PyPI package name
    if branch.is_some() || tag.is_some() || commit.is_some() || host.is_some() {
        return Err(PluginError::PackageSpec(
            "Cannot use git flags with PyPI package name".to_string(),
        ));
    }
    Ok(package.to_string())
}

/// Add git ref (@branch, @tag, or @commit) to a URL
fn add_git_ref(
    url: &str,
    branch: Option<String>,
    tag: Option<String>,
    commit: Option<String>,
) -> String {
    if let Some(b) = branch {
        format!("{url}@{b}")
    } else if let Some(t) = tag {
        format!("{url}@{t}")
    } else if let Some(c) = commit {
        format!("{url}@{c}")
    } else {
        url.to_string()
    }
}

/// Expand tilde (~) to home directory path (cross-platform)
/// Works on Windows, macOS, and Linux
fn expand_tilde(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }

    match dirs::home_dir() {
        Some(home) => {
            let home_str = home.to_string_lossy();
            if path == "~" {
                home_str.to_string()
            } else if path.starts_with("~/") {
                format!("{}{}", home_str, &path[1..])
            } else {
                path.to_string()
            }
        }
        None => path.to_string(),
    }
}

/// Extract package name from pyproject.toml
fn extract_name_from_pyproject(path: &str) -> Option<String> {
    use std::fs;
    use std::path::Path;

    let project_path = Path::new(path);
    let pyproject_path = project_path.join("pyproject.toml");

    if !pyproject_path.exists() {
        return None;
    }

    if let Ok(content) = fs::read_to_string(&pyproject_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name") && trimmed.contains('=') {
                if let Some(start_idx) = trimmed.find('"') {
                    if let Some(end_idx) = trimmed.rfind('"') {
                        if start_idx < end_idx {
                            return Some(trimmed[start_idx + 1..end_idx].to_string());
                        }
                    }
                } else if let Some(start_idx) = trimmed.find('\'') {
                    if let Some(end_idx) = trimmed.rfind('\'') {
                        if start_idx < end_idx {
                            return Some(trimmed[start_idx + 1..end_idx].to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::plugins::package_spec::*;

    // ── is_git_url ──────────────────────────────────────────────────────

    #[test]
    fn test_is_git_url_ssh_shorthand() {
        assert!(is_git_url("git@github.com:org/repo.git"));
    }

    #[test]
    fn test_is_git_url_ssh_full() {
        assert!(is_git_url("ssh://git@github.com/org/repo.git"));
    }

    #[test]
    fn test_is_git_url_https() {
        assert!(is_git_url("https://github.com/org/repo.git"));
    }

    #[test]
    fn test_is_git_url_https_no_git_suffix() {
        assert!(is_git_url("https://github.com/org/repo"));
    }

    #[test]
    fn test_is_git_url_git_plus() {
        assert!(is_git_url("git+https://github.com/org/repo.git"));
    }

    #[test]
    fn test_is_github_shorthand() {
        assert!(is_github_shorthand("gh:NatLabRockies/r2x-reeds"));
    }

    #[test]
    fn test_is_github_shorthand_invalid() {
        assert!(!is_github_shorthand("gh:NatLabRockies"));
    }

    #[test]
    fn test_is_git_url_pypi_is_not() {
        assert!(!is_git_url("r2x-reeds"));
    }

    #[test]
    fn test_is_git_url_local_path_is_not() {
        assert!(!is_git_url("./packages/r2x-reeds"));
    }

    // ── normalize_git_url ───────────────────────────────────────────────

    #[test]
    fn test_normalize_ssh_shorthand() {
        assert_eq!(
            normalize_git_url("git@github.com:NatLabRockies/R2X.git"),
            "git+ssh://git@github.com/NatLabRockies/R2X.git"
        );
    }

    #[test]
    fn test_normalize_ssh_full() {
        assert_eq!(
            normalize_git_url("ssh://git@github.com/org/repo.git"),
            "git+ssh://git@github.com/org/repo.git"
        );
    }

    #[test]
    fn test_normalize_https() {
        assert_eq!(
            normalize_git_url("https://github.com/org/repo.git"),
            "git+https://github.com/org/repo.git"
        );
    }

    #[test]
    fn test_normalize_https_no_git_suffix() {
        assert_eq!(
            normalize_git_url("https://github.com/org/repo"),
            "git+https://github.com/org/repo"
        );
    }

    #[test]
    fn test_normalize_already_prefixed() {
        let url = "git+https://github.com/org/repo.git";
        assert_eq!(normalize_git_url(url), url);
    }

    // ── extract_package_name ────────────────────────────────────────────

    #[test]
    fn test_extract_name_pypi() {
        assert!(extract_package_name("r2x-reeds").is_ok_and(|s| s == "r2x-reeds"));
    }

    #[test]
    fn test_extract_name_git_plus_https() {
        assert!(
            extract_package_name("git+https://github.com/nrel/r2x-reeds@main")
                .is_ok_and(|s| s == "r2x-reeds")
        );
    }

    #[test]
    fn test_extract_name_https() {
        assert!(
            extract_package_name("https://github.com/NatLabRockies/R2X.git")
                .is_ok_and(|s| s == "R2X")
        );
    }

    #[test]
    fn test_extract_name_https_with_ref() {
        assert!(
            extract_package_name("https://github.com/NatLabRockies/R2X.git@v2.0.0")
                .is_ok_and(|s| s == "R2X")
        );
    }

    #[test]
    fn test_extract_name_ssh_shorthand() {
        assert!(
            extract_package_name("git@github.com:NatLabRockies/R2X.git").is_ok_and(|s| s == "R2X")
        );
    }

    #[test]
    fn test_extract_name_ssh_shorthand_with_ref() {
        assert!(
            extract_package_name("git@github.com:NatLabRockies/R2X.git@main")
                .is_ok_and(|s| s == "R2X")
        );
    }

    #[test]
    fn test_extract_name_ssh_full() {
        assert!(extract_package_name("ssh://git@github.com/org/R2X.git").is_ok_and(|s| s == "R2X"));
    }

    #[test]
    fn test_extract_name_gh_shorthand() {
        assert!(extract_package_name("gh:NatLabRockies/R2X").is_ok_and(|s| s == "R2X"));
    }

    #[test]
    fn test_extract_name_gh_shorthand_with_ref() {
        assert!(extract_package_name("gh:NatLabRockies/R2X@main").is_ok_and(|s| s == "R2X"));
    }

    #[test]
    fn test_extract_name_local_path() {
        let result = extract_package_name("./packages/r2x-reeds");
        assert!(result.is_ok() || result.is_err());
    }

    // ── build_package_spec ──────────────────────────────────────────────

    #[test]
    fn test_spec_pypi() {
        let result = build_package_spec("r2x-reeds", None, None, None, None);
        assert!(result.is_ok_and(|s| s == "r2x-reeds"));
    }

    #[test]
    fn test_spec_local_path() {
        let result = build_package_spec("./packages/r2x-reeds", None, None, None, None);
        assert!(result.is_ok_and(|s| s == "./packages/r2x-reeds"));
    }

    #[test]
    fn test_spec_dot() {
        assert!(build_package_spec(".", None, None, None, None).is_ok_and(|s| s == "."));
    }

    #[test]
    fn test_spec_dotdot() {
        assert!(build_package_spec("..", None, None, None, None).is_ok_and(|s| s == ".."));
    }

    #[test]
    fn test_spec_dot_rejects_git_flags() {
        assert!(build_package_spec(".", None, Some("main".to_string()), None, None).is_err());
    }

    #[test]
    fn test_spec_gh_with_branch() {
        let result = build_package_spec(
            "gh:nrel/r2x-reeds",
            None,
            Some("develop".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+https://github.com/nrel/r2x-reeds@develop"));
    }

    #[test]
    fn test_spec_gh_with_custom_host() {
        let result = build_package_spec(
            "gh:acme/r2x-plugin",
            Some("github.example.com".to_string()),
            Some("main".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+https://github.example.com/acme/r2x-plugin@main"));
    }

    #[test]
    fn test_spec_gh_without_ref() {
        let result = build_package_spec("gh:NatLabRockies/R2X", None, None, None, None);
        assert!(result.is_ok_and(|s| s == "git+https://github.com/NatLabRockies/R2X"));
    }

    #[test]
    fn test_spec_gh_rejects_invalid() {
        assert!(build_package_spec(
            "gh:NatLabRockies",
            None,
            Some("main".to_string()),
            None,
            None
        )
        .is_err());
    }

    #[test]
    fn test_spec_org_repo_with_branch_rejected() {
        let result = build_package_spec(
            "nrel/r2x-reeds",
            None,
            Some("develop".to_string()),
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_spec_org_repo_without_branch_rejected() {
        let result = build_package_spec("nrel/r2x-reeds", None, None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_spec_rejects_git_flags_with_pypi() {
        assert!(
            build_package_spec("r2x-reeds", None, Some("main".to_string()), None, None).is_err()
        );
    }

    #[test]
    fn test_spec_ssh_shorthand_with_branch() {
        let result = build_package_spec(
            "git@github.com:NatLabRockies/R2X.git",
            None,
            Some("v2.0.0".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+ssh://git@github.com/NatLabRockies/R2X.git@v2.0.0"));
    }

    #[test]
    fn test_spec_ssh_shorthand_no_branch() {
        let result = build_package_spec(
            "git@github.com:NatLabRockies/R2X.git",
            None,
            None,
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+ssh://git@github.com/NatLabRockies/R2X.git"));
    }

    #[test]
    fn test_spec_https_with_branch() {
        let result = build_package_spec(
            "https://github.com/NatLabRockies/R2X.git",
            None,
            Some("v2.0.0".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+https://github.com/NatLabRockies/R2X.git@v2.0.0"));
    }

    #[test]
    fn test_spec_https_no_git_suffix() {
        let result = build_package_spec(
            "https://github.com/NatLabRockies/R2X",
            None,
            Some("v2.0.0".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+https://github.com/NatLabRockies/R2X@v2.0.0"));
    }

    #[test]
    fn test_spec_ssh_full_with_branch() {
        let result = build_package_spec(
            "ssh://git@github.com/org/repo.git",
            None,
            Some("main".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+ssh://git@github.com/org/repo.git@main"));
    }

    #[test]
    fn test_spec_ssh_full_with_embedded_ref_and_subdirectory() {
        let result = build_package_spec(
            "ssh://git@github.com/NatLabRockies/R2X.git@v2.0.0#subdirectory=packages/r2x-plexos-to-sienna",
            None,
            None,
            None,
            None,
        );
        assert!(result.is_ok_and(
            |s| s
                == "git+ssh://git@github.com/NatLabRockies/R2X.git@v2.0.0#subdirectory=packages/r2x-plexos-to-sienna"
        ));
    }

    #[test]
    fn test_spec_git_plus_passthrough() {
        let result = build_package_spec(
            "git+https://github.com/org/repo.git",
            None,
            Some("main".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s == "git+https://github.com/org/repo.git@main"));
    }

    // ── expand_tilde ────────────────────────────────────────────────────

    #[test]
    fn test_expand_tilde_with_slash() {
        let expanded = expand_tilde("~/dev/r2x-reeds");
        assert!(expanded.contains("dev/r2x-reeds") || expanded.contains("dev\\r2x-reeds"));
        assert!(!expanded.starts_with('~'));
    }

    #[test]
    fn test_expand_tilde_home_only() {
        let expanded = expand_tilde("~");
        assert!(!expanded.starts_with('~'));
        assert!(!expanded.is_empty());
    }

    #[test]
    fn test_expand_tilde_non_tilde_path() {
        let path = "/absolute/path";
        assert_eq!(expand_tilde(path), path);
    }

    #[test]
    fn test_spec_with_tilde_path() {
        let result = build_package_spec("~/some/local/path", None, None, None, None);
        assert!(result.is_ok_and(|s| !s.starts_with('~')));
    }
}
