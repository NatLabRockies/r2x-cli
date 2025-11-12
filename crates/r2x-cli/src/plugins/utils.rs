//! Shared utility functions for plugin discovery

/// Check if a dependency looks like an r2x plugin (exclude the shared runtime)
pub fn looks_like_r2x_plugin(dep: &str) -> bool {
    dep.starts_with("r2x-") && dep != "r2x-core"
}
