//! AST-based plugin discovery using ast-grep
//!
//! This module provides static analysis based plugin discovery by:
//! 1. Using ast-grep to parse Python source code without runtime (Phase 1)
//! 2. Extracting plugin definitions from the register_plugin() function
//! 3. Resolving class/function references to extract metadata (Phase 2)
//! 4. Associating decorator registrations with plugins (Phase 3)
//!
//! This approach is significantly faster than Python-based discovery and requires
//! no Python interpreter startup.

pub mod discovery_types;
pub mod entry_points;
pub mod naming;
pub mod package_cache;
pub mod schema_extractor;

use crate::discovery_types::{ConfigField, ConfigSpec, EntryPointInfo};
use crate::entry_points::{parser as entry_parser, pyproject as entry_pyproject};
// Re-export for tests
#[cfg(test)]
use crate::entry_points::parser::is_r2x_section;
use crate::naming::{camel_to_kebab, find_matching_paren, snake_to_kebab};
use crate::package_cache::PackageAstCache;
use crate::schema_extractor::TypeResolver;
use anyhow::{anyhow, Result};
use ast_grep_language::Python;
use r2x_logger as logger;
use r2x_manifest::types::{Parameter, Plugin, PluginType};
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

type PythonAst = ast_grep_core::AstGrep<ast_grep_core::source::StrDoc<Python>>;

struct CachedFile {
    content: String,
    ast: PythonAst,
}

/// Adapter that implements TypeResolver using a pre-built class content map.
///
/// This allows the SchemaExtractor to recursively resolve nested config
/// class definitions without holding a reference to the package cache.
struct ClassContentResolver {
    /// Map from class name to file content containing that class
    class_content: HashMap<String, String>,
}

impl ClassContentResolver {
    /// Build a resolver from a PackageAstCache by extracting all class content.
    fn from_package_cache(cache: &PackageAstCache) -> Self {
        let mut class_content = HashMap::new();

        // Iterate over all files and index class content
        for file in cache.files().values() {
            for class in &file.classes {
                class_content.insert(class.name.clone(), file.content.clone());
            }
        }

        ClassContentResolver { class_content }
    }
}

impl TypeResolver for ClassContentResolver {
    fn resolve_class_content(&self, class_name: &str) -> Option<String> {
        self.class_content.get(class_name).cloned()
    }
}

#[derive(Debug, Clone)]
struct ParsedArgument {
    name: String,
    annotation: Option<String>,
    default: Option<String>,
    required: bool,
}

/// AST-based plugin discovery orchestrator
pub struct AstDiscovery;

impl AstDiscovery {
    /// Discover plugins from a Python package using AST parsing
    ///
    /// Uses an entry-points-first approach: parses entry_points.txt directly
    /// to find all plugin entry points, then uses AST to extract metadata.
    ///
    /// Performance optimizations:
    /// - Single directory walk via PackageAstCache
    /// - Single AST parse per file
    /// - Kind-based decorator matching
    ///
    /// # Arguments
    /// * `package_path` - Path to the installed package (e.g., site-packages/r2x_reeds)
    /// * `package_name_full` - Full package name (e.g., "r2x-reeds")
    /// * `venv_path` - Optional path to virtual environment for entry_points.txt lookup
    /// * `_package_version` - Optional package version string
    /// * `dist_info_path` - Optional pre-resolved dist-info directory path from PackageLocator
    ///
    /// # Returns
    /// Vector of discovered plugins
    pub fn discover_plugins(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
        _package_version: Option<&str>,
        dist_info_path: Option<&Path>,
    ) -> Result<Vec<Plugin>> {
        let total_start = Instant::now();
        logger::debug(&format!("AST discovery started for: {}", package_name_full));

        let discovery_root = Self::resolve_discovery_root(package_path, package_name_full);
        let is_site_packages = Self::is_site_packages_root(&discovery_root, venv_path);
        logger::debug(&format!(
            "AST discovery root: {} (site-packages: {})",
            discovery_root.display(),
            is_site_packages
        ));

        // Step 1: Check explicit entry points (entry_points.txt or pyproject.toml)
        let entry_start = Instant::now();
        let pyproject_entries =
            Self::find_pyproject_entry_points(package_path, &discovery_root, package_name_full);
        let entry_point_entries = if pyproject_entries.is_empty() {
            // If caller provided a pre-resolved dist-info path, read entry_points.txt directly
            // Otherwise fall back to find_entry_points_txt which only checks the package directory
            let entry_points_result = if let Some(dist_info) = dist_info_path {
                let entry_points_txt = dist_info.join("entry_points.txt");
                if entry_points_txt.exists() {
                    Ok(entry_points_txt)
                } else {
                    Err(anyhow!(
                        "No entry_points.txt in dist-info: {}",
                        dist_info.display()
                    ))
                }
            } else {
                Self::find_entry_points_txt(package_path, package_name_full)
            };

            match entry_points_result {
                Ok(entry_points_path) => {
                    logger::debug(&format!(
                        "Found entry_points.txt at: {}",
                        entry_points_path.display()
                    ));

                    let content = std::fs::read_to_string(&entry_points_path)
                        .map_err(|e| anyhow!("Failed to read entry_points.txt: {}", e))?;

                    entry_parser::parse_all_entry_points(&content)
                }
                Err(e) => {
                    logger::debug(&format!(
                        "No entry_points.txt found for '{}': {}",
                        package_name_full, e
                    ));
                    Vec::new()
                }
            }
        } else {
            pyproject_entries
        };

        logger::debug(&format!(
            "Entry points parsing: {} entries in {:.2}ms",
            entry_point_entries.len(),
            entry_start.elapsed().as_secs_f64() * 1000.0
        ));

        if entry_point_entries.is_empty() && Self::is_site_packages_root(&discovery_root, venv_path)
        {
            logger::warn(&format!(
                "Skipping AST discovery for '{}' because discovery root is site-packages: {}",
                package_name_full,
                discovery_root.display()
            ));
            return Ok(Vec::new());
        }

        // Step 2: Package-wide AST discovery only when entry points are missing
        let mut package_cache: Option<PackageAstCache> = None;
        let mut class_entries = Vec::new();
        let mut function_entries = Vec::new();

        if entry_point_entries.is_empty() {
            let ast_start = Instant::now();
            let cache = PackageAstCache::build(&discovery_root);
            class_entries =
                Self::discover_plugins_with_ast_grep(&cache, &discovery_root, package_name_full);
            function_entries =
                Self::discover_expose_plugin_functions(&cache, &discovery_root, package_name_full);

            logger::debug(&format!(
                "AST-grep discovery: {} classes, {} functions in {:.2}ms ({} files)",
                class_entries.len(),
                function_entries.len(),
                ast_start.elapsed().as_secs_f64() * 1000.0,
                cache.file_count()
            ));

            package_cache = Some(cache);
        }

        // Step 3: Merge and deduplicate all discoveries
        // Priority: entry_points.txt > ast-grep (entry_points.txt has explicit registrations)
        let mut all_entries = Vec::new();
        all_entries.extend(entry_point_entries); // Entry points first (higher priority)
        all_entries.extend(class_entries);
        all_entries.extend(function_entries);

        let all_entries = Self::deduplicate_entries(all_entries);

        if all_entries.is_empty() {
            logger::debug(&format!("No plugins found for '{}'", package_name_full));
            return Ok(Vec::new());
        }

        logger::debug(&format!(
            "Total unique entry points: {} entries",
            all_entries.len()
        ));

        // Step 4: Discover plugins from each entry point using targeted file parsing
        // Only parse files that are actually needed (not the whole package)
        let discover_start = Instant::now();
        let mut plugins = Vec::new();
        let mut file_cache: HashMap<PathBuf, Arc<CachedFile>> = HashMap::new();

        for entry in &all_entries {
            match Self::discover_direct_entry_point(
                package_path,
                &discovery_root,
                entry,
                package_name_full,
                &mut file_cache,
                &mut package_cache,
                is_site_packages,
            ) {
                Ok(plugin) => {
                    logger::debug(&format!(
                        "Discovered plugin: {} ({:?})",
                        plugin.name, plugin.plugin_type
                    ));
                    plugins.push(plugin);
                }
                Err(e) => {
                    logger::debug(&format!(
                        "Failed to discover plugin from entry point '{}': {}",
                        entry.name, e
                    ));
                }
            }
        }
        logger::debug(&format!(
            "Plugin discovery: {} plugins in {:.2}ms (parsed {} files)",
            plugins.len(),
            discover_start.elapsed().as_secs_f64() * 1000.0,
            file_cache.len()
        ));

        logger::info(&format!(
            "AST discovery total: {} plugins in {:.2}ms for {}",
            plugins.len(),
            total_start.elapsed().as_secs_f64() * 1000.0,
            package_name_full
        ));

        Ok(plugins)
    }

    /// Resolve the most likely package root for AST discovery.
    ///
    /// Prefers the actual module directory over a project root to avoid
    /// traversing unrelated directories like .git or build artifacts.
    fn resolve_discovery_root(package_path: &Path, package_name_full: &str) -> PathBuf {
        let normalized_name = package_name_full.replace('-', "_");

        let direct_candidate = package_path.join(&normalized_name);
        if direct_candidate.is_dir() {
            return direct_candidate;
        }

        let src_candidate = package_path.join("src").join(&normalized_name);
        if src_candidate.is_dir() {
            return src_candidate;
        }

        let path_name_matches = package_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == normalized_name);

        if path_name_matches || package_path.join("__init__.py").exists() {
            return package_path.to_path_buf();
        }

        package_path.to_path_buf()
    }

    fn is_site_packages_root(discovery_root: &Path, venv_path: Option<&str>) -> bool {
        let Some(venv_path) = venv_path else {
            return false;
        };

        let Ok(site_packages) = r2x_config::venv_paths::resolve_site_packages(Path::new(venv_path))
        else {
            return false;
        };

        Self::paths_equivalent(discovery_root, &site_packages)
    }

    fn paths_equivalent(left: &Path, right: &Path) -> bool {
        match (left.canonicalize(), right.canonicalize()) {
            (Ok(left), Ok(right)) => left == right,
            _ => left == right,
        }
    }

    /// Find entry_points.txt for the package
    ///
    /// Only checks for entry_points.txt directly in the package path (for source/editable installs).
    /// For installed packages, the caller should provide `dist_info_path` to `discover_plugins`
    /// which is resolved by PackageLocator using cached directory entries.
    fn find_entry_points_txt(
        package_path: &Path,
        package_name_full: &str,
    ) -> Result<std::path::PathBuf> {
        // Look for entry_points.txt directly in package_path (for source packages)
        // This handles editable installs where package_path is the source directory
        let direct_path = package_path.join("entry_points.txt");
        if direct_path.exists() {
            return Ok(direct_path);
        }

        Err(anyhow!(
            "Package '{}' has no entry_points.txt",
            package_name_full
        ))
    }

    /// Find entry points in pyproject.toml (PEP 621)
    fn find_pyproject_entry_points(
        package_path: &Path,
        discovery_root: &Path,
        package_name_full: &str,
    ) -> Vec<EntryPointInfo> {
        let pyproject_path =
            match entry_pyproject::find_pyproject_toml_path(package_path, discovery_root) {
                Some(path) => path,
                None => return Vec::new(),
            };

        let content = match std::fs::read_to_string(&pyproject_path) {
            Ok(content) => content,
            Err(e) => {
                logger::debug(&format!(
                    "Failed to read pyproject.toml at {}: {}",
                    pyproject_path.display(),
                    e
                ));
                return Vec::new();
            }
        };

        let entries = entry_pyproject::parse_pyproject_entry_points(&content);
        if !entries.is_empty() {
            logger::debug(&format!(
                "Parsed {} entry points from pyproject.toml for '{}'",
                entries.len(),
                package_name_full
            ));
        }

        entries
    }

    /// Resolve a module path to a source file location
    ///
    /// Handles both regular and editable installs:
    /// - Regular: site-packages/r2x_reeds/sysmod/break_gens.py
    /// - Editable: src/r2x_reeds/sysmod/break_gens.py
    ///
    /// The function tries multiple path patterns to locate the source file.
    fn resolve_source_file(
        package_path: &Path,
        module: &str,
        package_name: &str,
    ) -> Option<std::path::PathBuf> {
        // Convert module path to relative file path
        // e.g., "r2x_reeds.sysmod.pcm_defaults" -> "r2x_reeds/sysmod/pcm_defaults.py"
        let relative_path = module.replace('.', "/") + ".py";

        // Also prepare a path without the package prefix
        // e.g., "sysmod/pcm_defaults.py" for use within package_path
        let normalized_package = package_name.replace('-', "_");
        let relative_without_prefix = module
            .strip_prefix(&normalized_package)
            .map(|s| s.trim_start_matches('.'))
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('.', "/") + ".py");

        // Try various path patterns
        let candidates: Vec<std::path::PathBuf> = [
            // Direct path from package_path (for editable installs where package_path is the source)
            relative_without_prefix
                .as_ref()
                .map(|p| package_path.join(p)),
            // Full relative path from package_path
            Some(package_path.join(&relative_path)),
            // One level up (in case package_path is the package root)
            package_path.parent().map(|p| p.join(&relative_path)),
            // src/ subdirectory (common in editable installs)
            Some(package_path.join("src").join(&relative_path)),
            // Two levels up with src/ (common project structure)
            package_path
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join(&relative_path)),
        ]
        .into_iter()
        .flatten()
        .collect();

        for candidate in &candidates {
            if candidate.exists() {
                logger::debug(&format!(
                    "Resolved source file for module '{}': {}",
                    module,
                    candidate.display()
                ));
                return Some(candidate.clone());
            }
        }

        // Also check for __init__.py if the module might be a package
        let init_path = module.replace('.', "/") + "/__init__.py";
        let init_candidates: Vec<std::path::PathBuf> = [
            Some(package_path.join(&init_path)),
            package_path.parent().map(|p| p.join(&init_path)),
            // Also try src/ subdirectory
            Some(package_path.join("src").join(&init_path)),
        ]
        .into_iter()
        .flatten()
        .collect();

        for candidate in &init_candidates {
            if candidate.exists() {
                logger::debug(&format!(
                    "Resolved source file for module '{}' (as package): {}",
                    module,
                    candidate.display()
                ));
                return Some(candidate.clone());
            }
        }

        logger::debug(&format!(
            "Could not resolve source file for module '{}'. Tried: {:?}",
            module, candidates
        ));
        None
    }

    // =========================================================================
    // AST-GREP LIBRARY-BASED DISCOVERY - Fast discovery without subprocess
    // =========================================================================

    /// Discover plugins using ast-grep library to search for Plugin class patterns
    ///
    /// Used when explicit entry points are missing to find classes that inherit from Plugin[Config].
    ///
    /// Uses ast-grep-core library directly for fast discovery without subprocess overhead.
    fn discover_plugins_with_ast_grep(
        package_cache: &PackageAstCache,
        discovery_root: &Path,
        package_name: &str,
    ) -> Vec<EntryPointInfo> {
        let start = Instant::now();
        let mut entries = Vec::new();
        let normalized_package = package_name.replace('-', "_");

        for (path, file) in package_cache.files() {
            let path_str = match path.to_str() {
                Some(path_str) => path_str,
                None => continue,
            };
            let module =
                Self::infer_module_from_file_path(path_str, discovery_root, &normalized_package);

            for class in &file.classes {
                if class.generic_param.is_none() {
                    continue;
                }

                entries.push(EntryPointInfo {
                    name: camel_to_kebab(&class.name),
                    module: module.clone(),
                    symbol: class.name.clone(),
                    section: "r2x_plugin".to_string(),
                });
            }
        }

        logger::debug(&format!(
            "ast-grep discovery: found {} Plugin classes in {:.2}ms",
            entries.len(),
            start.elapsed().as_secs_f64() * 1000.0
        ));

        entries
    }

    /// Discover @expose_plugin decorated functions using ast-grep library
    ///
    /// Searches for functions decorated with @expose_plugin in the package.
    /// These are registered in the r2x.transforms section.
    fn discover_expose_plugin_functions(
        package_cache: &PackageAstCache,
        discovery_root: &Path,
        package_name: &str,
    ) -> Vec<EntryPointInfo> {
        let start = Instant::now();
        let mut entries = Vec::new();
        let normalized_package = package_name.replace('-', "_");

        for (path, func) in package_cache.get_all_decorated_functions() {
            let path_str = match path.to_str() {
                Some(path_str) => path_str,
                None => continue,
            };
            let module =
                Self::infer_module_from_file_path(path_str, discovery_root, &normalized_package);
            let func_name = func.function_name.as_str();

            entries.push(EntryPointInfo {
                name: snake_to_kebab(func_name),
                module,
                symbol: func_name.to_string(),
                section: "r2x.transforms".to_string(),
            });
        }

        logger::debug(&format!(
            "ast-grep discovery: found {} @expose_plugin functions in {:.2}ms",
            entries.len(),
            start.elapsed().as_secs_f64() * 1000.0
        ));

        entries
    }

    /// Extract function name from decorated function text
    #[cfg(test)]
    fn extract_function_name_from_text(text: &str) -> Option<String> {
        // Look for "def function_name(" pattern
        let def_idx = text.find("def ")?;
        let after_def = &text[def_idx + 4..];
        let paren_idx = after_def.find('(')?;
        let name = after_def[..paren_idx].trim();

        if name.is_empty() {
            return None;
        }

        Some(name.to_string())
    }

    /// Extract class name from ast-grep matched text like "class MyParser(Plugin[Config]): ..."
    #[cfg(test)]
    fn extract_class_name_from_match(text: &str) -> Option<String> {
        // Pattern: "class ClassName(..."
        let text = text.trim();
        if !text.starts_with("class ") {
            return None;
        }
        let after_class = &text[6..];
        let end = after_class.find('(')?;
        let name = after_class[..end].trim();
        if name.is_empty() {
            return None;
        }
        Some(name.to_string())
    }
    /// Deduplicate entry points by symbol name
    ///
    /// When merging entry_points.txt with ast-grep discoveries, we may have
    /// duplicates (same class/function discovered via different paths).
    /// This function deduplicates by symbol, preserving the first occurrence
    /// (entry_points.txt entries have priority since they're added first).
    fn deduplicate_entries(entries: Vec<EntryPointInfo>) -> Vec<EntryPointInfo> {
        use std::collections::HashSet;

        let mut seen_symbols: HashSet<String> = HashSet::new();
        let mut result = Vec::new();

        for entry in entries {
            // Deduplicate by symbol name only
            // This prevents the same class (e.g., ReEDSParser) from being
            // registered twice with different inferred module paths
            if seen_symbols.insert(entry.symbol.clone()) {
                result.push(entry);
            }
        }

        result
    }

    /// Infer module path from file path
    fn infer_module_from_file_path(
        file_path: &str,
        package_path: &Path,
        package_name: &str,
    ) -> String {
        let path = std::path::Path::new(file_path);
        // Normalize package name: dashes to underscores for Python module names
        let normalized_package = package_name.replace('-', "_");

        // Try to get relative path from package_path
        if let Ok(rel) = path.strip_prefix(package_path) {
            let parts: Vec<&str> = rel
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect();

            if !parts.is_empty() {
                let module_parts: Vec<&str> = parts
                    .into_iter()
                    .map(|p| p.strip_suffix(".py").unwrap_or(p))
                    .filter(|p| *p != "__init__")
                    .collect();

                if module_parts.is_empty() {
                    return normalized_package;
                }

                // Check if the first part already matches the package name
                // (happens with editable installs where package_path is source root)
                if module_parts[0] == normalized_package {
                    return module_parts.join(".");
                }

                return format!("{}.{}", normalized_package, module_parts.join("."));
            }
        }

        // Fallback: just use normalized package name
        normalized_package
    }

    // =========================================================================
    // DIRECT FILE PARSING - Parse only the files needed for each entry point
    // =========================================================================

    /// Discover a plugin from a direct entry point using targeted file parsing
    ///
    /// This approach only parses the specific file(s) needed for each entry point,
    /// avoiding the overhead of parsing all files in the package upfront.
    ///
    /// When `is_site_packages` is true, we skip expensive fallback operations like
    /// building a full PackageAstCache, since non-editable installs don't have
    /// accessible source files and scanning site-packages would be catastrophically slow.
    fn discover_direct_entry_point(
        package_path: &Path,
        discovery_root: &Path,
        entry: &EntryPointInfo,
        package_name: &str,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        package_cache: &mut Option<PackageAstCache>,
        is_site_packages: bool,
    ) -> Result<Plugin> {
        logger::debug(&format!(
            "Discovering direct entry point: {} = {}:{}",
            entry.name, entry.module, entry.symbol
        ));

        // Resolve the source file
        let mut source_file = Self::resolve_source_file(package_path, &entry.module, package_name);
        let mut cached = source_file
            .as_ref()
            .and_then(|source_path| Self::read_file_cached(file_cache, source_path));

        // Determine plugin type based on symbol naming convention
        let plugin_type = if entry.is_class() {
            PluginType::Class
        } else {
            PluginType::Function
        };

        if matches!(plugin_type, PluginType::Class) {
            let mut has_class = cached
                .as_ref()
                .is_some_and(|cached| Self::ast_has_class(&cached.ast, &entry.symbol));

            if !has_class {
                if let Some(cached_file) = cached.as_ref() {
                    if let Some(resolved_module) = Self::resolve_reexported_symbol(
                        &cached_file.content,
                        &entry.module,
                        &entry.symbol,
                    ) {
                        if let Some(path) =
                            Self::resolve_source_file(package_path, &resolved_module, package_name)
                        {
                            source_file = Some(path.clone());
                            cached = Self::read_file_cached(file_cache, &path);
                            has_class = cached.as_ref().is_some_and(|cached| {
                                Self::ast_has_class(&cached.ast, &entry.symbol)
                            });
                        }
                    }
                }
            }

            // Only attempt full package AST scan if NOT in site-packages.
            // For non-editable installs, discovery_root points to site-packages and
            // scanning all of site-packages would parse thousands of unrelated files.
            if !has_class && !is_site_packages {
                if package_cache.is_none() {
                    *package_cache = Some(PackageAstCache::build(discovery_root));
                }

                if let Some(cache) = package_cache.as_ref() {
                    if let Some((path, _class)) = cache.find_class_with_path(&entry.symbol) {
                        source_file = Some(path.clone());
                        cached = Self::read_file_cached(file_cache, path);
                    }
                }
            }
        } else {
            // For functions, also check if it's re-exported and follow the import
            let has_function = cached
                .as_ref()
                .is_some_and(|cached| Self::ast_has_function(&cached.ast, &entry.symbol));

            if !has_function {
                // Function not defined here, try to resolve from imports
                if let Some(cached_file) = cached.as_ref() {
                    if let Some(resolved_module) = Self::resolve_reexported_symbol(
                        &cached_file.content,
                        &entry.module,
                        &entry.symbol,
                    ) {
                        if let Some(path) =
                            Self::resolve_source_file(package_path, &resolved_module, package_name)
                        {
                            source_file = Some(path.clone());
                            cached = Self::read_file_cached(file_cache, &path);
                        }
                    }
                }
            }
        }

        // Extract constructor/call arguments and config using direct file parsing
        let (call_args, config) =
            if let (Some(source_path), Some(cached)) = (source_file.as_ref(), cached.as_ref()) {
                Self::extract_entry_metadata(
                    discovery_root,
                    source_path,
                    cached.as_ref(),
                    entry,
                    plugin_type,
                    file_cache,
                    package_cache,
                    package_name,
                )
            } else {
                (Vec::new(), None)
            };

        Ok(Self::build_manifest_plugin(
            entry,
            plugin_type,
            config.as_ref(),
            &call_args,
        ))
    }

    /// Read file content with caching to avoid re-reading the same file
    fn read_file_cached(
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        path: &Path,
    ) -> Option<Arc<CachedFile>> {
        if let Some(cached) = file_cache.get(path) {
            return Some(Arc::clone(cached));
        }

        let content = std::fs::read_to_string(path).ok()?;
        let ast = PythonAst::new(&content, Python);
        let cached = Arc::new(CachedFile { content, ast });
        file_cache.insert(path.to_path_buf(), Arc::clone(&cached));
        Some(cached)
    }

    /// Extract entry metadata using direct file parsing
    #[allow(clippy::too_many_arguments)]
    fn extract_entry_metadata(
        discovery_root: &Path,
        source_path: &Path,
        cached: &CachedFile,
        entry: &EntryPointInfo,
        plugin_type: PluginType,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        package_cache: &mut Option<PackageAstCache>,
        package_name: &str,
    ) -> (Vec<ParsedArgument>, Option<ConfigSpec>) {
        let call_args = if matches!(plugin_type, PluginType::Function) {
            Self::extract_function_params(&cached.content, &entry.symbol)
        } else {
            Vec::new()
        };

        // Try to extract config class from type hints or class generic
        let mut config = Self::extract_config_with_fields(
            discovery_root,
            source_path,
            cached,
            entry,
            plugin_type,
            file_cache,
            package_cache,
            package_name,
        );

        // For functions without a config class, convert call_args to ConfigField format
        if config.is_none() && matches!(plugin_type, PluginType::Function) && !call_args.is_empty()
        {
            use r2x_manifest::types::SchemaFields;

            // For functions without config, use function parameters as config fields
            let fields = call_args
                .iter()
                .map(Self::argument_to_config_field)
                .collect();

            config = Some(ConfigSpec {
                module: entry.module.clone(),
                name: format!("{}Params", entry.symbol),
                fields,
                config_schema: SchemaFields::default(),
            });
        }

        (call_args, config)
    }

    fn build_manifest_plugin(
        entry: &EntryPointInfo,
        plugin_type: PluginType,
        config: Option<&ConfigSpec>,
        call_args: &[ParsedArgument],
    ) -> Plugin {
        use crate::schema_extractor::parse_union_types_from_annotation;

        let module = Arc::from(entry.module.as_str());
        let (class_name, function_name) = match plugin_type {
            PluginType::Class => (Some(Arc::from(entry.symbol.as_str())), None),
            PluginType::Function => (None, Some(Arc::from(entry.symbol.as_str()))),
        };

        let (config_class, config_module, config_fields) = config.map_or_else(
            || (None, None, &[] as &[ConfigField]),
            |spec| {
                (
                    Some(Arc::from(spec.name.as_str())),
                    Some(Arc::from(spec.module.as_str())),
                    spec.fields.as_slice(),
                )
            },
        );

        let mut parameters: SmallVec<[Parameter; 4]> = config_fields
            .iter()
            .map(|field| Parameter {
                name: Arc::from(field.name.as_str()),
                types: field.types.iter().map(|t| Arc::from(t.as_str())).collect(),
                module: None,
                required: field.required,
                default: field.default.as_ref().map(|d| Arc::from(d.as_str())),
                description: field.description.as_ref().map(|d| Arc::from(d.as_str())),
            })
            .collect();

        let existing_names: HashSet<String> =
            parameters.iter().map(|p| p.name.to_string()).collect();

        // Runtime-injected params that shouldn't appear in user-facing config
        const RUNTIME_PARAMS: &[&str] = &[
            "self", "system", "config", "store", "stdin", "ctx", "context",
        ];

        for arg in call_args {
            let name = arg.name.as_str();
            if RUNTIME_PARAMS.contains(&name) || existing_names.contains(name) {
                continue;
            }

            let types: SmallVec<[Arc<str>; 2]> = arg.annotation.as_deref().map_or_else(
                || SmallVec::from_elem(Arc::from("Any"), 1),
                |ann| {
                    parse_union_types_from_annotation(ann)
                        .into_iter()
                        .map(|t| Arc::from(t.as_str()))
                        .collect()
                },
            );

            parameters.push(Parameter {
                name: Arc::from(name),
                types,
                module: None,
                required: arg.required,
                default: arg.default.as_ref().map(|d| Arc::from(d.as_str())),
                description: None,
            });
        }

        // Use config_schema from ConfigSpec if available
        let config_schema = config
            .map(|spec| spec.config_schema.clone())
            .unwrap_or_default();

        Plugin {
            name: Arc::from(entry.name.as_str()),
            plugin_type,
            module,
            class_name,
            function_name,
            config_class,
            config_module,
            hooks: SmallVec::new(),
            parameters,
            config_schema,
            content_hash: 0,
        }
    }

    /// Extract config class and fields using direct file parsing
    #[allow(clippy::too_many_arguments)]
    fn extract_config_with_fields(
        discovery_root: &Path,
        source_path: &Path,
        cached: &CachedFile,
        entry: &EntryPointInfo,
        plugin_type: PluginType,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        package_cache: &mut Option<PackageAstCache>,
        package_name: &str,
    ) -> Option<ConfigSpec> {
        use crate::schema_extractor::SchemaExtractor;
        use r2x_manifest::types::SchemaFields;

        // First, find the config class name from the current file
        let config_name =
            Self::find_config_class_name(&cached.ast, &cached.content, &entry.symbol, plugin_type)?;

        // Track where we found the config class to determine the correct module
        let mut config_file_path: Option<PathBuf> = None;
        let mut config_content: Option<String> = None;

        // Try to extract fields from the same file first
        let mut fields = Self::extract_config_fields_from_ast(&cached.ast, &config_name);
        if !fields.is_empty() {
            // Config class is in the same file as the function
            config_file_path = Some(source_path.to_path_buf());
            config_content = Some(cached.content.clone());
        }

        // If no fields found in the same file, search the package using ast-grep
        if fields.is_empty() {
            if package_cache.is_none() {
                *package_cache = Some(PackageAstCache::build(discovery_root));
            }

            if let Some(cache) = package_cache.as_ref() {
                if let Some((path, content)) = cache.find_config_class_content(&config_name) {
                    fields = Self::extract_config_fields(content, &config_name);
                    if !fields.is_empty() {
                        config_file_path = Some(path.clone());
                        config_content = Some(content.to_string());
                    }
                }
            }
        }

        // If still no fields, try looking in common locations relative to source file
        if fields.is_empty() {
            if let Some((path, content)) = Self::find_config_in_common_locations_with_path(
                source_path,
                &config_name,
                file_cache,
            ) {
                fields = Self::extract_config_fields(&content, &config_name);
                if !fields.is_empty() {
                    config_file_path = Some(path);
                    config_content = Some(content);
                }
            }
        }

        // Determine the config module path
        let config_module = if let Some(ref path) = config_file_path {
            // Use the file path where we found the config class to infer its module
            let path_str = path.to_string_lossy();
            Self::infer_module_from_file_path(&path_str, discovery_root, package_name)
        } else {
            // Fallback: try to resolve from imports in the source file
            Self::resolve_config_module_from_imports(&cached.content, &config_name, &entry.module)
                .unwrap_or_else(|| entry.module.clone())
        };

        // Extract schema with nested type resolution if we have the content and package cache
        let config_schema = if let (Some(content), Some(cache)) =
            (config_content.as_ref(), package_cache.as_ref())
        {
            let resolver = ClassContentResolver::from_package_cache(cache);
            let extractor = SchemaExtractor::with_resolver(HashMap::new(), Arc::new(resolver));
            extractor
                .extract_with_nesting(content, &config_name)
                .unwrap_or_default()
        } else {
            SchemaFields::default()
        };

        Some(ConfigSpec {
            module: config_module,
            name: config_name,
            fields,
            config_schema,
        })
    }

    /// Find config class in common locations and return the path where it was found
    fn find_config_in_common_locations_with_path(
        source_path: &Path,
        config_name: &str,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
    ) -> Option<(PathBuf, String)> {
        let parent = source_path.parent()?;
        let class_with_base = format!("class {}(", config_name);
        let class_no_base = format!("class {}:", config_name);

        let candidates = [
            "__init__.py",
            "config.py",
            "configs.py",
            "types.py",
            "plugin_config.py",
        ];

        for filename in candidates {
            let candidate = parent.join(filename);
            if let Some(cached) = Self::read_file_cached(file_cache, &candidate) {
                if cached.content.contains(&class_with_base)
                    || cached.content.contains(&class_no_base)
                {
                    return Some((candidate, cached.content.clone()));
                }
            }
        }

        None
    }

    /// Resolve config module from import statements in the source file
    fn resolve_config_module_from_imports(
        content: &str,
        config_name: &str,
        current_module: &str,
    ) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();

            // Match: from X import Y or from X import Y, Z, ...
            if let Some(rest) = trimmed.strip_prefix("from ") {
                if let Some(import_idx) = rest.find(" import ") {
                    let from_part = rest[..import_idx].trim();
                    let import_part = rest[import_idx + 8..].trim();

                    // Check if our config class is in the import list
                    let imports: Vec<&str> = import_part
                        .split(',')
                        .map(|s| s.trim().split(" as ").next().unwrap_or(s.trim()).trim())
                        .collect();

                    if imports.contains(&config_name) {
                        // Resolve relative imports
                        if from_part.starts_with('.') {
                            let dot_count = from_part.chars().take_while(|c| *c == '.').count();
                            let relative_path = &from_part[dot_count..];

                            let mut parts: Vec<&str> = current_module.split('.').collect();
                            // Go up dot_count levels (one dot means same package)
                            for _ in 0..dot_count {
                                parts.pop();
                            }

                            if !relative_path.is_empty() {
                                parts.push(relative_path);
                            }

                            return Some(parts.join("."));
                        }
                        return Some(from_part.to_string());
                    }
                }
            }
        }
        None
    }

    // =========================================================================
    // HELPER METHODS - Used by cached discovery methods
    // =========================================================================

    /// Find config class name from content (function param or class generic) using AST
    fn find_config_class_name(
        ast: &PythonAst,
        content: &str,
        symbol: &str,
        plugin_type: PluginType,
    ) -> Option<String> {
        if matches!(plugin_type, PluginType::Function) {
            return Self::extract_config_type_from_function_text(content, symbol);
        }

        let class_pattern = format!("class {}($$$BASES): $$$BODY", symbol);

        for class_match in ast.root().find_all(class_pattern.as_str()) {
            // ast-grep's $$$BASES doesn't capture correctly for Python class bases,
            // so we find the argument_list child which contains "(Plugin[Config], ...)"
            let config = class_match
                .children()
                .find(|c| c.kind() == "argument_list")
                .and_then(|arg_list| Self::extract_plugin_generic(&arg_list.text()));

            if config.is_some() {
                return config;
            }
        }

        None
    }

    /// Extract config name from "Plugin[ConfigName]" in a base class list
    fn extract_plugin_generic(bases_text: &str) -> Option<String> {
        let start = bases_text.find("Plugin[")?;
        let rest = &bases_text[start + 7..];
        let end = rest.find(']')?;
        let config_name = rest[..end].trim();

        if config_name.is_empty() {
            None
        } else {
            Some(config_name.to_string())
        }
    }

    /// Extract config type from function definition in content
    /// Finds "def symbol(..., config: ConfigClass, ...)" and extracts "ConfigClass"
    fn extract_config_type_from_function_text(content: &str, symbol: &str) -> Option<String> {
        // Find "def symbol(" in content
        let search = format!("def {}(", symbol);
        let start = content.find(&search)?;
        let after_start = &content[start + search.len()..];

        // Find closing paren (handle nested parens)
        let paren_end = find_matching_paren(after_start)?;
        let params_text = &after_start[..paren_end];

        // Look for "config:" pattern
        let config_start = params_text.find("config:")?;
        let rest = &params_text[config_start + 7..];

        // Find end of type annotation (comma or end of string)
        let type_end = rest.find(|c| [',', ')'].contains(&c)).unwrap_or(rest.len());
        let config_type = rest[..type_end].trim();

        // Verify it ends with "Config"
        if config_type.ends_with("Config") {
            Some(config_type.to_string())
        } else {
            None
        }
    }

    fn ast_has_class(ast: &PythonAst, class_name: &str) -> bool {
        let pattern = format!("class {}($$$BASES): $$$BODY", class_name);
        let found = ast.root().find_all(pattern.as_str()).next().is_some();
        found
    }

    fn ast_has_function(ast: &PythonAst, function_name: &str) -> bool {
        let pattern = format!("def {}($$$PARAMS): $$$BODY", function_name);
        let found = ast.root().find_all(pattern.as_str()).next().is_some();
        found
    }

    fn resolve_reexported_symbol(content: &str, base_module: &str, symbol: &str) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with("from ") {
                continue;
            }

            let Some(rest) = trimmed.strip_prefix("from ") else {
                continue;
            };
            let Some((from_part, import_part)) = rest.split_once(" import ") else {
                continue;
            };
            let from_part = from_part.trim();

            let import_part = import_part
                .split('#')
                .next()
                .unwrap_or(import_part)
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')');

            for item in import_part.split(',') {
                let name = item.trim();
                if name.is_empty() {
                    continue;
                }
                let name = name.split_whitespace().next().unwrap_or(name);
                if name != symbol {
                    continue;
                }

                if from_part.starts_with('.') {
                    let dot_count = from_part.chars().take_while(|c| *c == '.').count();
                    let levels_up = dot_count.saturating_sub(1);
                    let rel = &from_part[dot_count..];
                    let mut base_parts: Vec<&str> = base_module.split('.').collect();
                    if levels_up > 0 && base_parts.len() >= levels_up {
                        base_parts.truncate(base_parts.len() - levels_up);
                    }
                    let base_prefix = base_parts.join(".");
                    if rel.is_empty() {
                        return Some(base_prefix);
                    }
                    if base_prefix.is_empty() {
                        return Some(rel.to_string());
                    }
                    return Some(format!("{}.{}", base_prefix, rel));
                }

                return Some(from_part.to_string());
            }
        }

        None
    }

    /// Extract parameters from a function definition using text-based parsing
    fn extract_function_params(content: &str, function_name: &str) -> Vec<ParsedArgument> {
        // Find "def function_name(" in content
        let search = format!("def {}(", function_name);
        let Some(start) = content.find(&search) else {
            return Vec::new();
        };
        let after_start = &content[start + search.len()..];

        // Find closing paren
        let Some(paren_end) = find_matching_paren(after_start) else {
            return Vec::new();
        };
        let params_text = &after_start[..paren_end];

        let mut params = Vec::new();

        // Split by comma, respecting nested brackets
        let mut depth = 0;
        let mut param_start = 0;

        for (i, ch) in params_text.char_indices() {
            match ch {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    if let Some(arg) = Self::parse_function_param(&params_text[param_start..i]) {
                        params.push(arg);
                    }
                    param_start = i + 1;
                }
                _ => {}
            }
        }

        // Don't forget the last parameter
        if param_start < params_text.len() {
            if let Some(arg) = Self::parse_function_param(&params_text[param_start..]) {
                params.push(arg);
            }
        }

        params
    }

    /// Check if a parameter name should be skipped
    fn is_skipped_param(name: &str) -> bool {
        name.is_empty()
            || name == "self"
            || name == "system"
            || name == "kwargs"
            || name.starts_with('_')
    }

    /// Parse a single function parameter string into ParsedArgument
    fn parse_function_param(param_str: &str) -> Option<ParsedArgument> {
        let param_str = param_str.trim();

        // Skip empty, self, *args, **kwargs
        if param_str.is_empty() || param_str == "self" || param_str.starts_with('*') {
            return None;
        }

        let (name, annotation, default) = Self::parse_param_text(param_str);

        if Self::is_skipped_param(&name) {
            return None;
        }

        Some(ParsedArgument {
            name,
            annotation,
            required: default.is_none(),
            default,
        })
    }

    /// Parse a parameter text into name, annotation, and default
    fn parse_param_text(text: &str) -> (String, Option<String>, Option<String>) {
        let text = text.trim();

        // Handle: name: type = default
        if let Some(eq_idx) = text.find('=') {
            let before_eq = text[..eq_idx].trim();
            let default = text[eq_idx + 1..].trim().to_string();

            if let Some(colon_idx) = before_eq.find(':') {
                let name = before_eq[..colon_idx].trim().to_string();
                let annotation = before_eq[colon_idx + 1..].trim().to_string();
                return (name, Some(annotation), Some(default));
            }
            return (before_eq.to_string(), None, Some(default));
        }

        // Handle: name: type
        if let Some(colon_idx) = text.find(':') {
            let name = text[..colon_idx].trim().to_string();
            let annotation = text[colon_idx + 1..].trim().to_string();
            return (name, Some(annotation), None);
        }

        // Handle: just name
        (text.to_string(), None, None)
    }

    /// Extract fields from a config class definition
    fn extract_config_fields(content: &str, class_name: &str) -> Vec<ConfigField> {
        let ast = PythonAst::new(content, Python);
        let root = ast.root();

        Self::extract_config_fields_from_root(&root, class_name)
    }

    fn extract_config_fields_from_ast(ast: &PythonAst, class_name: &str) -> Vec<ConfigField> {
        let root = ast.root();
        Self::extract_config_fields_from_root(&root, class_name)
    }

    fn extract_config_fields_from_root(
        root: &ast_grep_core::Node<'_, ast_grep_core::source::StrDoc<Python>>,
        class_name: &str,
    ) -> Vec<ConfigField> {
        use schema_extractor::{extract_description_from_field, parse_union_types_from_annotation};

        // Find the class definition
        let class_pattern = format!("class {}($$$): $$$BODY", class_name);
        let Some(class_match) = root.find_all(class_pattern.as_str()).next() else {
            return Vec::new();
        };

        // Pattern for annotated assignment: name: Type = value or name: Type
        let matches: Vec<_> = class_match.find_all("$NAME: $TYPE").collect();

        let mut fields = Vec::new();
        for m in matches {
            let full_text = m.text();

            // Parse name and type from the match text
            let Some(colon_pos) = full_text.find(':') else {
                continue;
            };

            let name = full_text[..colon_pos].trim().to_string();

            // Skip private/magic attributes and non-identifier names
            if name.starts_with('_') || !name.chars().next().is_some_and(|c| c.is_alphabetic()) {
                continue;
            }

            // Extract type annotation - everything after the colon until = or end
            let after_colon = &full_text[colon_pos + 1..];
            let type_text = if let Some(eq_pos) = after_colon.rfind(" = ") {
                after_colon[..eq_pos].trim().to_string()
            } else {
                after_colon.trim().to_string()
            };

            // Parse union types from annotation (handles Annotated[] and union types)
            let types = parse_union_types_from_annotation(&type_text);

            // Extract description from Field(description="...")
            let description = extract_description_from_field(&full_text);

            // Check if there's a default value
            let has_default = full_text.contains(" = ") || full_text.contains("default=");

            // Determine if required
            let required =
                !has_default && !type_text.contains("None") && !type_text.starts_with("Optional");

            // Try to extract default value
            let default = if has_default {
                Self::extract_default_value(&full_text)
            } else {
                None
            };

            fields.push(ConfigField {
                name,
                types,
                default,
                required,
                description,
            });
        }

        fields
    }

    /// Extract default value from field definition text
    fn extract_default_value(text: &str) -> Option<String> {
        // Look for = value after the type annotation
        if let Some(eq_pos) = text.rfind(" = ") {
            let value_part = text[eq_pos + 3..].trim();
            // Skip if it's a Field() call - we'll extract default from there
            if value_part.starts_with("Field(") {
                // Try to extract from Field(default=...)
                if let Some(start) = value_part.find("default=") {
                    let rest = &value_part[start + 8..];
                    // Find the end of the default value
                    let mut depth = 0;
                    let mut end = 0;
                    for (i, ch) in rest.char_indices() {
                        match ch {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => {
                                if depth == 0 {
                                    end = i;
                                    break;
                                }
                                depth -= 1;
                            }
                            ',' if depth == 0 => {
                                end = i;
                                break;
                            }
                            _ => {}
                        }
                    }
                    if end > 0 {
                        let default_str = rest[..end].trim();
                        return Some(Self::clean_default_value(default_str));
                    }
                }
                return None;
            }
            return Some(Self::clean_default_value(value_part));
        }
        None
    }

    /// Clean up default value string (remove surrounding quotes if present)
    fn clean_default_value(value: &str) -> String {
        let value = value.trim();
        // Remove surrounding quotes for string values
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            value[1..value.len() - 1].to_string()
        } else {
            value.to_string()
        }
    }

    /// Convert a parsed argument to a ConfigField for function parameter extraction
    fn argument_to_config_field(arg: &ParsedArgument) -> ConfigField {
        use schema_extractor::parse_union_types_from_annotation;

        let types = arg.annotation.as_deref().map_or_else(
            || vec!["Any".to_string()],
            parse_union_types_from_annotation,
        );

        let default = arg.default.as_deref().map(Self::clean_default_value);
        let required = arg.required && !types.iter().any(|t| t == "None");

        ConfigField {
            name: arg.name.clone(),
            types,
            default,
            required,
            description: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_all_entry_points_r2x_plugin() {
        let content = r"[r2x_plugin]
reeds = r2x_reeds:ReEDSParser
";
        let entries = entry_parser::parse_all_entry_points(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "reeds");
        assert_eq!(entries[0].module, "r2x_reeds");
        assert_eq!(entries[0].symbol, "ReEDSParser");
        assert_eq!(entries[0].section, "r2x_plugin");
    }

    #[test]
    fn test_parse_all_entry_points_multiple_sections() {
        let content = r"[r2x_plugin]
reeds = r2x_reeds:ReEDSParser

[r2x.transforms]
add-pcm-defaults = r2x_reeds.sysmod.pcm_defaults:add_pcm_defaults
add-emission-cap = r2x_reeds.sysmod.emission_cap:add_emission_cap

[console_scripts]
some-cli = some_module:main
";
        let entries = entry_parser::parse_all_entry_points(content);
        assert_eq!(entries.len(), 3);

        // Check r2x_plugin entry
        assert_eq!(entries[0].name, "reeds");
        assert_eq!(entries[0].section, "r2x_plugin");

        // Check r2x.transforms entries
        assert_eq!(entries[1].name, "add-pcm-defaults");
        assert_eq!(entries[1].section, "r2x.transforms");
        assert_eq!(entries[1].module, "r2x_reeds.sysmod.pcm_defaults");
        assert_eq!(entries[1].symbol, "add_pcm_defaults");

        assert_eq!(entries[2].name, "add-emission-cap");
        assert_eq!(entries[2].section, "r2x.transforms");
    }

    #[test]
    fn test_parse_all_entry_points_ignores_non_r2x_sections() {
        let content = r"[console_scripts]
some-cli = some_module:main

[gui_scripts]
some-gui = some_module:gui_main
";
        let entries = entry_parser::parse_all_entry_points(content);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_is_r2x_section() {
        assert!(is_r2x_section("r2x_plugin"));
        assert!(is_r2x_section("r2x.transforms"));
        assert!(is_r2x_section("r2x.parsers"));
        assert!(is_r2x_section("r2x.exporters"));
        assert!(!is_r2x_section("console_scripts"));
        assert!(!is_r2x_section("gui_scripts"));
    }

    #[test]
    fn test_entry_point_info_is_class() {
        let class_entry = EntryPointInfo {
            name: "reeds".to_string(),
            module: "r2x_reeds".to_string(),
            symbol: "ReEDSParser".to_string(),
            section: "r2x_plugin".to_string(),
        };
        assert!(class_entry.is_class());

        let func_entry = EntryPointInfo {
            name: "add-pcm-defaults".to_string(),
            module: "r2x_reeds.sysmod".to_string(),
            symbol: "add_pcm_defaults".to_string(),
            section: "r2x.transforms".to_string(),
        };
        assert!(!func_entry.is_class());
    }

    #[test]
    fn test_resolve_reexported_symbol() {
        let content = r"
from .parser import ReEDSParser
from .plugin_config import ReEDSConfig
";
        let resolved = AstDiscovery::resolve_reexported_symbol(content, "r2x_reeds", "ReEDSParser");
        assert_eq!(resolved, Some("r2x_reeds.parser".to_string()));
    }

    #[test]
    fn test_parse_param_text() {
        // Test name: type = default
        let (name, ann, def) = AstDiscovery::parse_param_text("config: MyConfig = None");
        assert_eq!(name, "config");
        assert_eq!(ann, Some("MyConfig".to_string()));
        assert_eq!(def, Some("None".to_string()));

        // Test name: type
        let (name, ann, def) = AstDiscovery::parse_param_text("system: System");
        assert_eq!(name, "system");
        assert_eq!(ann, Some("System".to_string()));
        assert!(def.is_none());

        // Test just name
        let (name, ann, def) = AstDiscovery::parse_param_text("data");
        assert_eq!(name, "data");
        assert!(ann.is_none());
        assert!(def.is_none());
    }

    #[test]
    fn test_find_config_class_name_from_function() {
        // Test single-line function with return type
        let source_single = r"
def add_pcm_defaults(system: System, config: PCMDefaultsConfig) -> Result[System, str]:
    pass
";
        let config_name =
            AstDiscovery::extract_config_type_from_function_text(source_single, "add_pcm_defaults");
        assert_eq!(config_name, Some("PCMDefaultsConfig".to_string()));

        // Test multi-line function with return type
        let source_multi = r"
def add_pcm_defaults(
    system: System,
    config: PCMDefaultsConfig,
) -> Result[System, str]:
    pass
";
        let config_name_multi =
            AstDiscovery::extract_config_type_from_function_text(source_multi, "add_pcm_defaults");
        assert_eq!(config_name_multi, Some("PCMDefaultsConfig".to_string()));

        // Test function without return type
        let source_no_ret = r"
def add_pcm_defaults(system: System, config: PCMDefaultsConfig):
    pass
";
        let config_name_no_ret =
            AstDiscovery::extract_config_type_from_function_text(source_no_ret, "add_pcm_defaults");
        assert_eq!(config_name_no_ret, Some("PCMDefaultsConfig".to_string()));
    }

    #[test]
    fn test_extract_function_params_direct() {
        let source = r"
def add_pcm_defaults(
    system: System,
    pcm_defaults_fpath: str | None = None,
    pcm_defaults_dict: dict | None = None,
    pcm_defaults_override: bool = False,
) -> System:
    pass
";
        let params = AstDiscovery::extract_function_params(source, "add_pcm_defaults");
        // system should be filtered out
        assert!(!params.iter().any(|p| p.name == "system"));
        // Should have the 3 params
        assert_eq!(params.len(), 3);

        let fpath = params.iter().find(|p| p.name == "pcm_defaults_fpath");
        assert!(
            fpath.is_some_and(|f| f.annotation.as_deref() == Some("str | None")
                && f.default.as_deref() == Some("None")
                && !f.required)
        );

        let override_param = params.iter().find(|p| p.name == "pcm_defaults_override");
        assert!(override_param
            .is_some_and(|p| p.annotation.as_deref() == Some("bool")
                && p.default.as_deref() == Some("False")));
    }

    #[test]
    fn test_argument_to_config_field() {
        let arg = ParsedArgument {
            name: "pcm_defaults_fpath".to_string(),
            annotation: Some("str | None".to_string()),
            default: Some("None".to_string()),
            required: false,
        };
        let field = AstDiscovery::argument_to_config_field(&arg);
        assert_eq!(field.name, "pcm_defaults_fpath");
        assert_eq!(field.types, vec!["str".to_string(), "None".to_string()]);
        assert_eq!(field.default, Some("None".to_string()));
        assert!(!field.required);
    }

    #[test]
    fn test_camel_to_kebab() {
        // Simple CamelCase
        assert_eq!(camel_to_kebab("MyParser"), "my-parser");
        assert_eq!(camel_to_kebab("SimplePlugin"), "simple-plugin");
        assert_eq!(camel_to_kebab("MyExporter"), "my-exporter");

        // Acronyms (consecutive uppercase)
        assert_eq!(camel_to_kebab("ReEDSParser"), "reeds-parser");
        assert_eq!(camel_to_kebab("XMLParser"), "xml-parser");
        assert_eq!(camel_to_kebab("HTTPClient"), "http-client");

        // Single word
        assert_eq!(camel_to_kebab("Parser"), "parser");
        assert_eq!(camel_to_kebab("Reeds"), "reeds");

        // All uppercase acronym
        assert_eq!(camel_to_kebab("HTTP"), "http");
        assert_eq!(camel_to_kebab("API"), "api");
    }

    #[test]
    fn test_snake_to_kebab() {
        assert_eq!(snake_to_kebab("add_pcm_defaults"), "add-pcm-defaults");
        assert_eq!(snake_to_kebab("break_gens"), "break-gens");
        assert_eq!(snake_to_kebab("simple"), "simple");
        assert_eq!(snake_to_kebab("a_b_c"), "a-b-c");
    }

    #[test]
    fn test_deduplicate_entries() {
        let entries = vec![
            EntryPointInfo {
                name: "reeds".to_string(),
                module: "r2x_reeds".to_string(),
                symbol: "ReEDSParser".to_string(),
                section: "r2x_plugin".to_string(),
            },
            EntryPointInfo {
                name: "reeds-parser".to_string(), // Different name and module, but same symbol
                module: "r2x_reeds.parser".to_string(),
                symbol: "ReEDSParser".to_string(),
                section: "r2x_plugin".to_string(),
            },
            EntryPointInfo {
                name: "add-pcm-defaults".to_string(),
                module: "r2x_reeds.sysmod".to_string(),
                symbol: "add_pcm_defaults".to_string(),
                section: "r2x.transforms".to_string(),
            },
        ];

        let deduped = AstDiscovery::deduplicate_entries(entries);
        assert_eq!(deduped.len(), 2);
        // First occurrence should be preserved (entry_points.txt has priority)
        assert_eq!(deduped[0].name, "reeds");
        assert_eq!(deduped[0].module, "r2x_reeds"); // Original module preserved
        assert_eq!(deduped[1].name, "add-pcm-defaults");
    }

    #[test]
    fn test_extract_function_name_from_text() {
        // Simple function
        let text = "@expose_plugin\ndef add_pcm_defaults(system: System): pass";
        assert_eq!(
            AstDiscovery::extract_function_name_from_text(text),
            Some("add_pcm_defaults".to_string())
        );

        // Multi-line function
        let text_multi = r"@expose_plugin
def break_gens(
    system: System,
    config: BreakGensConfig,
) -> System:
    pass";
        assert_eq!(
            AstDiscovery::extract_function_name_from_text(text_multi),
            Some("break_gens".to_string())
        );

        // No def keyword
        let text_no_def = "@expose_plugin\nclass MyClass: pass";
        assert_eq!(
            AstDiscovery::extract_function_name_from_text(text_no_def),
            None
        );
    }

    #[test]
    fn test_extract_class_name_from_match() {
        assert_eq!(
            AstDiscovery::extract_class_name_from_match("class MyParser(Plugin[Config]): pass"),
            Some("MyParser".to_string())
        );
        assert_eq!(
            AstDiscovery::extract_class_name_from_match(
                "class ReEDSParser(Plugin[ReEDSConfig]): pass"
            ),
            Some("ReEDSParser".to_string())
        );
        assert_eq!(
            AstDiscovery::extract_class_name_from_match("def my_func(): pass"),
            None
        );
    }

    #[test]
    fn test_find_config_class_name_from_class() {
        // Test class with generic Plugin[Config] - simple single line
        let source_simple = r"class MyParser(Plugin[MyConfig]):
    pass
";
        let ast = PythonAst::new(source_simple, Python);
        let config_name = AstDiscovery::find_config_class_name(
            &ast,
            source_simple,
            "MyParser",
            PluginType::Class,
        );
        assert_eq!(config_name, Some("MyConfig".to_string()));

        // Test class with generic Plugin[Config] - realistic example
        let source_reeds = r#"from r2x.api import Plugin

class ReEDSParser(Plugin[ReEDSConfig]):
    """ReEDS parser implementation."""

    def run(self):
        pass
"#;
        let ast_reeds = PythonAst::new(source_reeds, Python);
        let config_name_reeds = AstDiscovery::find_config_class_name(
            &ast_reeds,
            source_reeds,
            "ReEDSParser",
            PluginType::Class,
        );
        assert_eq!(config_name_reeds, Some("ReEDSConfig".to_string()));

        // Test class with multiple bases - Plugin[Config] should still be found
        let source_multi_base = r"class MyExporter(SomeMixin, Plugin[ExporterConfig], AnotherMixin):
    pass
";
        let ast_multi = PythonAst::new(source_multi_base, Python);
        let config_name_multi = AstDiscovery::find_config_class_name(
            &ast_multi,
            source_multi_base,
            "MyExporter",
            PluginType::Class,
        );
        assert_eq!(config_name_multi, Some("ExporterConfig".to_string()));

        // Test class without Plugin generic - should return None
        let source_no_plugin = r"class RegularClass(BaseClass):
    pass
";
        let ast_no_plugin = PythonAst::new(source_no_plugin, Python);
        let config_name_none = AstDiscovery::find_config_class_name(
            &ast_no_plugin,
            source_no_plugin,
            "RegularClass",
            PluginType::Class,
        );
        assert_eq!(config_name_none, None);
    }

    #[test]
    fn test_discover_plugins_skips_site_packages_root() {
        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };
        let venv_path = temp_dir.path().join("venv");

        #[cfg(windows)]
        let site_packages = {
            let site_packages = venv_path
                .join(r2x_config::venv_paths::PYTHON_LIB_DIR)
                .join("site-packages");
            if let Err(err) = fs::create_dir_all(&site_packages) {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create site-packages dir: {err}"
                );
                return;
            }
            site_packages
        };

        #[cfg(not(windows))]
        let site_packages = {
            let site_packages = venv_path
                .join(r2x_config::venv_paths::PYTHON_LIB_DIR)
                .join("python3.12")
                .join("site-packages");
            if let Err(err) = fs::create_dir_all(&site_packages) {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create site-packages dir: {err}"
                );
                return;
            }
            site_packages
        };

        let other_pkg = site_packages.join("other_pkg");
        if let Err(err) = fs::create_dir_all(&other_pkg) {
            assert!(
                err.to_string().is_empty(),
                "Failed to create package dir: {err}"
            );
            return;
        }
        if let Err(err) = fs::write(
            other_pkg.join("plugin.py"),
            r"
from r2x_core import Plugin

class MyPlugin(Plugin[MyConfig]):
    pass
",
        ) {
            assert!(
                err.to_string().is_empty(),
                "Failed to write plugin file: {err}"
            );
            return;
        }

        let venv_str = venv_path.to_str().unwrap_or_default();
        assert!(
            !venv_str.is_empty(),
            "Failed to convert venv path to string"
        );

        let plugins = match AstDiscovery::discover_plugins(
            &site_packages,
            "r2x-reeds",
            Some(venv_str),
            None,
            None,
        ) {
            Ok(plugins) => plugins,
            Err(err) => {
                assert!(err.to_string().is_empty(), "Discovery failed: {err}");
                return;
            }
        };

        assert!(plugins.is_empty());
    }

    #[test]
    fn test_nested_config_discovery_integration() {
        use crate::package_cache::PackageAstCache;
        use crate::schema_extractor::SchemaExtractor;

        let temp_dir = match TempDir::new() {
            Ok(dir) => dir,
            Err(err) => {
                assert!(
                    err.to_string().is_empty(),
                    "Failed to create temp dir: {err}"
                );
                return;
            }
        };

        // Create config.py with nested configs matching the real use case
        let config_content = r"
from pydantic import BaseModel

class TransmissionConfig(BaseModel):
    capacity: float = 1000.0
    voltage: int = 345

class NodalConfig(BaseModel):
    transmission: TransmissionConfig
    excluded_techs: list[str] | None = None
    solve_year: int = 2030

class ZonalToNodalConfig(BaseModel):
    config: NodalConfig
    name: str
";

        if let Err(err) = fs::write(temp_dir.path().join("config.py"), config_content) {
            assert!(
                err.to_string().is_empty(),
                "Failed to write config file: {err}"
            );
            return;
        }

        let cache = PackageAstCache::build(temp_dir.path());

        // Build resolver from cache
        let resolver = ClassContentResolver::from_package_cache(&cache);
        let extractor = SchemaExtractor::with_resolver(
            std::collections::HashMap::new(),
            std::sync::Arc::new(resolver),
        );

        // Find ZonalToNodalConfig content
        let Some((_, content)) = cache.find_config_class_content("ZonalToNodalConfig") else {
            // Simulate the pattern used in other tests
            let err = "ZonalToNodalConfig not found";
            assert!(err.is_empty(), "Config class not found: {err}");
            return;
        };

        // Extract with nesting
        let Ok(fields) = extractor.extract_with_nesting(content, "ZonalToNodalConfig") else {
            let err = "extraction failed";
            assert!(err.is_empty(), "extraction failed");
            return;
        };

        // Verify top-level structure
        assert!(fields.get("name").is_some(), "name field missing");

        // Verify nested NodalConfig
        assert!(fields.get("config").is_some(), "config field missing");
        let config_field = fields.get("config");
        assert!(
            config_field.is_some_and(|f| f.field_type == r2x_manifest::types::FieldType::Object)
        );

        // Get nodal properties
        let nodal_props = config_field.and_then(|f| f.properties.as_ref());
        assert!(nodal_props.is_some(), "NodalConfig properties missing");

        // Verify NodalConfig has solve_year (the field that was causing the runtime error)
        let nodal = nodal_props;
        assert!(
            nodal.is_some_and(|p| p.get("solve_year").is_some()),
            "solve_year field missing in NodalConfig"
        );
        assert!(nodal.is_some_and(|p| p
            .get("solve_year")
            .is_some_and(|f| f.field_type == r2x_manifest::types::FieldType::Int)));

        // Verify nested TransmissionConfig
        assert!(
            nodal.is_some_and(|p| p.get("transmission").is_some()),
            "transmission field missing"
        );

        let tx_props = nodal
            .and_then(|p| p.get("transmission"))
            .and_then(|f| f.properties.as_ref());
        assert!(tx_props.is_some(), "TransmissionConfig properties missing");

        assert!(
            tx_props.is_some_and(|p| p.get("capacity").is_some()),
            "capacity field missing"
        );
        assert!(
            tx_props.is_some_and(|p| p.get("voltage").is_some()),
            "voltage field missing"
        );
    }
}
