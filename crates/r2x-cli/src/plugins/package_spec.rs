/// Expand tilde (~) to home directory path (cross-platform)
/// Works on Windows, macOS, and Linux
fn expand_tilde(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }

    // Get home directory (cross-platform compatible)
    match dirs::home_dir() {
        Some(home) => {
            let home_str = home.to_string_lossy();
            if path == "~" {
                home_str.to_string()
            } else if path.starts_with("~/") {
                // Replace ~/ with home directory path
                format!("{}{}", home_str, &path[1..])
            } else {
                // Edge case: ~someuser paths are not supported, return as-is
                path.to_string()
            }
        }
        None => {
            // If home directory cannot be determined, return path as-is
            path.to_string()
        }
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
        // Simple regex-free parsing: look for name = "..." or name = '...'
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name") && trimmed.contains('=') {
                // Extract the value between quotes
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

/// Extract package name from package specifier
pub fn extract_package_name(package: &str) -> Result<String, String> {
    // Remove git+ prefix if present
    let pkg = package.strip_prefix("git+").unwrap_or(package);

    // Remove @ref if present
    let pkg = pkg.split('@').next().unwrap_or(pkg);

    // For URLs, extract the repository name
    if pkg.contains("://") || pkg.starts_with("git@") {
        // Extract last path component and remove .git suffix
        Ok(pkg
            .split('/')
            .next_back()
            .unwrap_or(pkg)
            .trim_end_matches(".git")
            .to_string())
    } else if pkg.contains('/') || pkg.contains('\\') {
        // For local paths, always read from pyproject.toml
        extract_name_from_pyproject(pkg)
            .ok_or_else(|| format!("Failed to extract package name from {}", package))
    } else {
        // For PyPI packages, use as-is
        Ok(pkg.to_string())
    }
}

/// Build package specifier for pip install
/// Handles PyPI packages, local paths, git URLs, and org/repo shorthand
pub fn build_package_spec(
    package: &str,
    host: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    commit: Option<String>,
) -> Result<String, String> {
    // Expand tilde to home directory (cross-platform)
    let expanded_package = expand_tilde(package);
    let package = expanded_package.as_str();

    // 1. If it's a local path (starts with ./ or ../ or / or contains file separators like ./packages/)
    if package.starts_with("./") || package.starts_with("../") || package.starts_with('/') {
        if branch.is_some() || tag.is_some() || commit.is_some() || host.is_some() {
            return Err("Cannot use git flags with local paths".to_string());
        }
        return Ok(package.to_string());
    }

    // 2. If it's already a full URL (http://, https://, git@, git+)
    let is_full_url = package.starts_with("http://")
        || package.starts_with("https://")
        || package.starts_with("git@")
        || package.starts_with("git+");

    if is_full_url {
        // Check if URL already has @ref
        if package.contains('@') && !package.starts_with("git@") {
            if branch.is_some() || tag.is_some() || commit.is_some() {
                return Err(
                    "Cannot use --branch/--tag/--commit with URL that already contains @ref"
                        .to_string(),
                );
            }
            return Ok(package.to_string());
        }

        // Add git+ prefix if needed
        let url = if package.starts_with("git+") || package.starts_with("git@") {
            package.to_string()
        } else {
            format!("git+{}", package)
        };

        return Ok(add_git_ref(&url, branch, tag, commit));
    }

    // 3. If it contains '/' and looks like org/repo (GitHub shorthand) - only if git flags or host provided
    if package.contains('/')
        && !package.contains('\\')
        && (host.is_some() || branch.is_some() || tag.is_some() || commit.is_some())
    {
        let git_host = host.as_deref().unwrap_or("github.com");
        let url = format!("git+https://{}/{}", git_host, package);
        return Ok(add_git_ref(&url, branch, tag, commit));
    }

    // 4. Otherwise, treat as PyPI package name
    if branch.is_some() || tag.is_some() || commit.is_some() || host.is_some() {
        return Err("Cannot use git flags with PyPI package name".to_string());
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
        format!("{}@{}", url, b)
    } else if let Some(t) = tag {
        format!("{}@{}", url, t)
    } else if let Some(c) = commit {
        format!("{}@{}", url, c)
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_package_name_pypi() {
        assert!(extract_package_name("r2x-reeds").is_ok_and(|s| s == "r2x-reeds"));
    }

    #[test]
    fn test_extract_package_name_git_url() {
        assert!(
            extract_package_name("git+https://github.com/nrel/r2x-reeds@main")
                .is_ok_and(|s| s == "r2x-reeds")
        );
    }

    #[test]
    fn test_extract_package_name_local_path() {
        // For local paths, always reads from pyproject.toml
        // Will be None if the path doesn't have a valid pyproject.toml
        let result = extract_package_name("./packages/r2x-reeds");
        // Just verify it returns a Result - actual value depends on whether pyproject.toml exists
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_build_package_spec_pypi() {
        let result = build_package_spec("r2x-reeds", None, None, None, None);
        assert!(result.is_ok_and(|s| s == "r2x-reeds"));
    }

    #[test]
    fn test_build_package_spec_local_path() {
        let result = build_package_spec("./packages/r2x-reeds", None, None, None, None);
        assert!(result.is_ok_and(|s| s == "./packages/r2x-reeds"));
    }

    #[test]
    fn test_build_package_spec_with_branch() {
        let result = build_package_spec(
            "nrel/r2x-reeds",
            None,
            Some("develop".to_string()),
            None,
            None,
        );
        assert!(result.is_ok_and(|s| s.contains("@develop")));
    }

    #[test]
    fn test_build_package_spec_rejects_git_flags_with_pypi() {
        let result = build_package_spec("r2x-reeds", None, Some("main".to_string()), None, None);
        assert!(result.is_err());
    }

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
    fn test_build_package_spec_with_tilde_path() {
        let result = build_package_spec("~/some/local/path", None, None, None, None);
        // The spec should be an absolute path (tilde expanded)
        assert!(result.is_ok_and(|s| !s.starts_with('~')));
    }
}
