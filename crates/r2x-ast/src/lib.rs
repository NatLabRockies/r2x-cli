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

pub mod decorator_scanner;
pub mod discovery_types;
pub mod extractor;
pub mod package_cache;
pub mod schema_extractor;

use crate::package_cache::PackageAstCache;
use anyhow::{anyhow, Result};
use ast_grep_language::Python;
use r2x_logger as logger;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

// Re-export commonly used types (also used internally)
pub use discovery_types::{
    ArgumentSpec, ConfigField, ConfigSpec, DecoratorRegistration, DiscoveredPlugin, EntryPointInfo,
    IOContract, IOSlot, ImplementationType, InvocationSpec, PluginKind, ResourceSpec, StoreMode,
    StoreSpec, UpgradeSpec,
};
pub use schema_extractor::SchemaExtractor;

type PythonAst = ast_grep_core::AstGrep<ast_grep_core::source::StrDoc<Python>>;

struct CachedFile {
    content: String,
    ast: PythonAst,
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
    /// * `package_version` - Optional package version string
    ///
    /// # Returns
    /// Tuple of (discovered plugins, decorator registrations)
    pub fn discover_plugins(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
        _package_version: Option<&str>,
    ) -> Result<(Vec<DiscoveredPlugin>, Vec<DecoratorRegistration>)> {
        let total_start = Instant::now();
        logger::debug(&format!("AST discovery started for: {}", package_name_full));

        let discovery_root = Self::resolve_discovery_root(package_path, package_name_full);
        logger::debug(&format!("AST discovery root: {:?}", discovery_root));

        // Step 1: Check explicit entry points (entry_points.txt or pyproject.toml)
        let entry_start = Instant::now();
        let pyproject_entries =
            Self::find_pyproject_entry_points(package_path, &discovery_root, package_name_full);
        let entry_point_entries = if !pyproject_entries.is_empty() {
            pyproject_entries
        } else {
            match Self::find_entry_points_txt(package_path, package_name_full, venv_path) {
                Ok(entry_points_path) => {
                    logger::debug(&format!(
                        "Found entry_points.txt at: {:?}",
                        entry_points_path
                    ));

                    let content = std::fs::read_to_string(&entry_points_path)
                        .map_err(|e| anyhow!("Failed to read entry_points.txt: {}", e))?;

                    Self::parse_all_entry_points(&content)
                }
                Err(e) => {
                    logger::debug(&format!(
                        "No entry_points.txt found for '{}': {}",
                        package_name_full, e
                    ));
                    Vec::new()
                }
            }
        };

        logger::debug(&format!(
            "Entry points parsing: {} entries in {:.2}ms",
            entry_point_entries.len(),
            entry_start.elapsed().as_secs_f64() * 1000.0
        ));

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
            return Ok((Vec::new(), Vec::new()));
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
            ) {
                Ok(plugin) => {
                    logger::debug(&format!(
                        "Discovered plugin: {} ({:?})",
                        plugin.name, plugin.kind
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

        // Step 5: Decorator registrations - only extract if needed (lazy)
        // For now, skip full package scanning - decorators can be extracted on-demand
        let decorator_registrations = Vec::new();

        logger::info(&format!(
            "AST discovery total: {} plugins, {} decorators in {:.2}ms for {}",
            plugins.len(),
            decorator_registrations.len(),
            total_start.elapsed().as_secs_f64() * 1000.0,
            package_name_full
        ));

        Ok((plugins, decorator_registrations))
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
            .map(|name| name == normalized_name)
            .unwrap_or(false);

        if path_name_matches || package_path.join("__init__.py").exists() {
            return package_path.to_path_buf();
        }

        package_path.to_path_buf()
    }

    /// Find entry_points.txt for the package
    ///
    /// Optimized to avoid directory scanning by trying direct paths first.
    fn find_entry_points_txt(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
    ) -> Result<std::path::PathBuf> {
        let normalized_name = package_name_full.replace('-', "_");

        // Strategy 1: Look for entry_points.txt directly in package_path (for source packages)
        // This handles editable installs where package_path is the source directory
        let direct_path = package_path.join("entry_points.txt");
        if direct_path.exists() {
            return Ok(direct_path);
        }

        // Strategy 2: Look in parent directory's dist-info (for installed packages)
        // Pattern: ../package_name-*.dist-info/entry_points.txt
        if let Some(parent) = package_path.parent() {
            // Try to find dist-info by scanning just the parent (usually site-packages)
            if let Some(entry_points) = Self::find_dist_info_entry_points(parent, &normalized_name)
            {
                return Ok(entry_points);
            }
        }

        // Strategy 3: Use venv site-packages if provided
        if let Some(venv) = venv_path {
            let venv_path = std::path::PathBuf::from(venv);
            if let Ok(site_packages) = r2x_python::resolve_site_package_path(&venv_path) {
                if let Some(entry_points) =
                    Self::find_dist_info_entry_points(&site_packages, &normalized_name)
                {
                    return Ok(entry_points);
                }
            }
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
        let pyproject_path = match Self::find_pyproject_toml_path(package_path, discovery_root) {
            Some(path) => path,
            None => return Vec::new(),
        };

        let content = match std::fs::read_to_string(&pyproject_path) {
            Ok(content) => content,
            Err(e) => {
                logger::debug(&format!(
                    "Failed to read pyproject.toml at {:?}: {}",
                    pyproject_path, e
                ));
                return Vec::new();
            }
        };

        let entries = Self::parse_pyproject_entry_points(&content);
        if !entries.is_empty() {
            logger::debug(&format!(
                "Parsed {} entry points from pyproject.toml for '{}'",
                entries.len(),
                package_name_full
            ));
        }

        entries
    }

    fn find_pyproject_toml_path(package_path: &Path, discovery_root: &Path) -> Option<PathBuf> {
        let mut seen = std::collections::HashSet::new();
        let candidates = [
            Some(package_path),
            package_path.parent(),
            discovery_root.parent(),
            discovery_root.parent().and_then(|p| p.parent()),
        ];

        for candidate in candidates.into_iter().flatten() {
            let candidate = candidate.to_path_buf();
            if !seen.insert(candidate.clone()) {
                continue;
            }

            let pyproject = candidate.join("pyproject.toml");
            if pyproject.exists() {
                return Some(pyproject);
            }
        }

        None
    }

    fn parse_pyproject_entry_points(content: &str) -> Vec<EntryPointInfo> {
        let mut entries = Vec::new();
        let parsed: toml::Value = match toml::from_str(content) {
            Ok(parsed) => parsed,
            Err(_) => return entries,
        };

        let entry_points = match parsed
            .get("project")
            .and_then(|project| project.get("entry-points"))
            .and_then(|value| value.as_table())
        {
            Some(entry_points) => entry_points,
            None => return entries,
        };

        for (section, values) in entry_points {
            if !Self::is_r2x_section(section) {
                continue;
            }

            let Some(table) = values.as_table() else {
                continue;
            };

            for (name, target) in table {
                let Some(target_str) = target.as_str() else {
                    continue;
                };

                let line = format!("{} = {}", name, target_str);
                if let Some(entry) = Self::parse_entry_point_line(&line, section) {
                    entries.push(entry);
                }
            }
        }

        entries
    }

    /// Find entry_points.txt in a dist-info directory within the given path
    fn find_dist_info_entry_points(
        dir: &Path,
        normalized_name: &str,
    ) -> Option<std::path::PathBuf> {
        let entries = std::fs::read_dir(dir).ok()?;
        let prefix = format!("{}-", normalized_name);

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.starts_with(&prefix) && name_str.ends_with(".dist-info") {
                let entry_points = entry.path().join("entry_points.txt");
                if entry_points.exists() {
                    return Some(entry_points);
                }
            }
        }
        None
    }

    /// Parse all r2x-related entry points from entry_points.txt
    ///
    /// This function scans for all sections that are r2x-related:
    /// - `[r2x_plugin]` - main plugin registration
    /// - `[r2x.*]` - any section starting with "r2x." (e.g., r2x.transforms, r2x.parsers)
    ///
    /// Returns a vector of EntryPointInfo for all discovered entry points.
    fn parse_all_entry_points(content: &str) -> Vec<EntryPointInfo> {
        let mut entries = Vec::new();
        let mut current_section: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Check for section header
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];
                // Check if this section is r2x-related
                if Self::is_r2x_section(section) {
                    current_section = Some(section.to_string());
                } else {
                    current_section = None;
                }
                continue;
            }

            // Parse entry point within a relevant section
            if let Some(ref section) = current_section {
                if let Some(entry) = Self::parse_entry_point_line(line, section) {
                    entries.push(entry);
                }
            }
        }

        logger::debug(&format!(
            "Parsed {} entry points from entry_points.txt",
            entries.len()
        ));
        entries
    }

    /// Check if a section name is r2x-related
    fn is_r2x_section(section: &str) -> bool {
        section == "r2x_plugin" || section.starts_with("r2x.")
    }

    /// Parse a single entry point line in the format: name = module:symbol
    fn parse_entry_point_line(line: &str, section: &str) -> Option<EntryPointInfo> {
        let eq_idx = line.find('=')?;
        let name = line[..eq_idx].trim();
        let value = line[eq_idx + 1..].trim();

        // Handle quoted values (e.g., name = "module:symbol")
        let value = value.trim_matches('"').trim_matches('\'');

        let colon_idx = value.find(':')?;
        let module = value[..colon_idx].trim();
        let symbol = value[colon_idx + 1..].trim();

        if name.is_empty() || module.is_empty() || symbol.is_empty() {
            return None;
        }

        Some(EntryPointInfo {
            name: name.to_string(),
            module: module.to_string(),
            symbol: symbol.to_string(),
            section: section.to_string(),
        })
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
                    "Resolved source file for module '{}': {:?}",
                    module, candidate
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
                    "Resolved source file for module '{}' (as package): {:?}",
                    module, candidate
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
                    name: Self::camel_to_kebab(&class.name),
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
                name: Self::snake_to_kebab(func_name),
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

    /// Convert CamelCase class name to kebab-case plugin name
    ///
    /// Preserves suffixes (unlike the old implementation that stripped them):
    /// - ReEDSParser -> reeds-parser
    /// - MyExporter -> my-exporter
    /// - SimplePlugin -> simple-plugin
    fn camel_to_kebab(class_name: &str) -> String {
        let mut result = String::new();

        for (i, ch) in class_name.chars().enumerate() {
            if ch.is_uppercase() && i > 0 {
                let prev_upper = class_name
                    .chars()
                    .nth(i - 1)
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false);
                let next_upper = class_name
                    .chars()
                    .nth(i + 1)
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false);
                let next_lower = class_name
                    .chars()
                    .nth(i + 1)
                    .map(|c| c.is_lowercase())
                    .unwrap_or(false);

                // Add hyphen when:
                // 1. Starting new word from lowercase, unless entering an acronym
                //    (e.g., "my" -> "P" in "myParser", but NOT "Re" -> "E" in "ReEDS")
                // 2. End of acronym transitioning to new word
                //    (e.g., "S" -> "P" in "ReEDSParser")
                let start_new_word = !prev_upper && !next_upper;
                let end_of_acronym = prev_upper && next_lower;

                if start_new_word || end_of_acronym {
                    result.push('-');
                }
            }
            result.push(ch.to_ascii_lowercase());
        }

        result
    }

    /// Convert snake_case function name to kebab-case plugin name
    fn snake_to_kebab(func_name: &str) -> String {
        func_name.replace('_', "-")
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
                    return package_name.to_string();
                }

                // Check if the first part already matches the package name
                // (happens with editable installs where package_path is source root)
                if module_parts[0] == package_name {
                    return module_parts.join(".");
                }

                return format!("{}.{}", package_name, module_parts.join("."));
            }
        }

        // Fallback: just use package name
        package_name.to_string()
    }

    // =========================================================================
    // DIRECT FILE PARSING - Parse only the files needed for each entry point
    // =========================================================================

    /// Discover a plugin from a direct entry point using targeted file parsing
    ///
    /// This approach only parses the specific file(s) needed for each entry point,
    /// avoiding the overhead of parsing all files in the package upfront.
    fn discover_direct_entry_point(
        package_path: &Path,
        discovery_root: &Path,
        entry: &EntryPointInfo,
        package_name: &str,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        package_cache: &mut Option<PackageAstCache>,
    ) -> Result<DiscoveredPlugin> {
        logger::debug(&format!(
            "Discovering direct entry point: {} = {}:{}",
            entry.name, entry.module, entry.symbol
        ));

        // Resolve the source file
        let source_file = Self::resolve_source_file(package_path, &entry.module, package_name);

        // Determine implementation type based on symbol naming convention
        let implementation = if entry.is_class() {
            ImplementationType::Class
        } else {
            ImplementationType::Function
        };

        // Infer plugin kind from section and symbol
        let kind = entry.infer_kind();

        // Build the fully qualified entry point
        let full_entry = format!("{}.{}", entry.module, entry.symbol);

        // Extract constructor/call arguments and config using direct file parsing
        let (constructor_args, call_args, resources) = if let Some(ref source_path) = source_file {
            // Read file content, using cache to avoid re-reading
            let cached = Self::read_file_cached(file_cache, source_path);

            if let Some(cached) = cached {
                Self::extract_entry_metadata(
                    discovery_root,
                    source_path,
                    cached.as_ref(),
                    entry,
                    &implementation,
                    file_cache,
                    package_cache,
                )
            } else {
                (Vec::new(), Vec::new(), None)
            }
        } else {
            (Vec::new(), Vec::new(), None)
        };

        // Determine the method to call based on plugin kind
        let method = match (&kind, &implementation) {
            (PluginKind::Parser, ImplementationType::Class) => Some("build_system".to_string()),
            (PluginKind::Exporter, ImplementationType::Class) => Some("export".to_string()),
            (PluginKind::Translation, ImplementationType::Class) => Some("run".to_string()),
            _ => None,
        };

        // Build IO contract based on plugin kind
        let io = Self::io_contract_for_kind(&kind);

        let invocation = InvocationSpec {
            implementation,
            method,
            constructor: constructor_args,
            call: call_args,
        };

        Ok(DiscoveredPlugin {
            name: entry.name.clone(),
            kind,
            entry: full_entry,
            invocation,
            io,
            resources,
            upgrade: None,
            description: None,
            tags: vec![entry.section.clone()],
        })
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
    fn extract_entry_metadata(
        discovery_root: &Path,
        source_path: &Path,
        cached: &CachedFile,
        entry: &EntryPointInfo,
        implementation: &ImplementationType,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        package_cache: &mut Option<PackageAstCache>,
    ) -> (Vec<ArgumentSpec>, Vec<ArgumentSpec>, Option<ResourceSpec>) {
        let (constructor_args, call_args) = match implementation {
            ImplementationType::Class => {
                let args = Self::extract_class_init_params(&cached.ast, &entry.symbol);
                (args, Vec::new())
            }
            ImplementationType::Function => {
                let args = Self::extract_function_params(&cached.content, &entry.symbol);
                (Vec::new(), args)
            }
        };

        // Try to extract config class from type hints or class generic
        let config = Self::extract_config_with_fields(
            discovery_root,
            source_path,
            cached,
            entry,
            implementation,
            file_cache,
            package_cache,
        );

        // For functions without a config class, convert call_args to ConfigField format
        let resources = if let Some(c) = config {
            Some(ResourceSpec {
                store: None,
                config: Some(c),
            })
        } else if matches!(implementation, ImplementationType::Function) && !call_args.is_empty() {
            // For functions without config, use function parameters as config fields
            let fields = call_args
                .iter()
                .map(Self::argument_to_config_field)
                .collect();

            Some(ResourceSpec {
                store: None,
                config: Some(ConfigSpec {
                    module: entry.module.to_string(),
                    name: format!("{}Params", entry.symbol),
                    fields,
                }),
            })
        } else {
            None
        };

        (constructor_args, call_args, resources)
    }

    /// Extract config class and fields using direct file parsing
    fn extract_config_with_fields(
        discovery_root: &Path,
        source_path: &Path,
        cached: &CachedFile,
        entry: &EntryPointInfo,
        implementation: &ImplementationType,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
        package_cache: &mut Option<PackageAstCache>,
    ) -> Option<ConfigSpec> {
        // First, find the config class name from the current file
        let config_name = Self::find_config_class_name(
            &cached.ast,
            &cached.content,
            &entry.symbol,
            implementation,
        )?;

        // Try to extract fields from the same file first
        let mut fields = Self::extract_config_fields_from_ast(&cached.ast, &config_name);

        // If no fields found in the same file, search the package using ast-grep
        if fields.is_empty() {
            if package_cache.is_none() {
                *package_cache = Some(PackageAstCache::build(discovery_root));
            }

            if let Some(cache) = package_cache.as_ref() {
                if let Some((_, config_content)) = cache.find_config_class_content(&config_name) {
                    fields = Self::extract_config_fields(config_content, &config_name);
                }
            }
        }

        // If still no fields, try looking in common locations relative to source file
        if fields.is_empty() {
            if let Some(config_content) =
                Self::find_config_in_common_locations(source_path, &config_name, file_cache)
            {
                fields = Self::extract_config_fields(&config_content, &config_name);
            }
        }

        Some(ConfigSpec {
            module: entry.module.to_string(),
            name: config_name,
            fields,
        })
    }

    /// Find config class in common locations (same directory, parent __init__.py, etc.)
    fn find_config_in_common_locations(
        source_path: &Path,
        config_name: &str,
        file_cache: &mut HashMap<PathBuf, Arc<CachedFile>>,
    ) -> Option<String> {
        let parent = source_path.parent()?;
        let class_with_base = format!("class {}(", config_name);
        let class_no_base = format!("class {}:", config_name);

        let candidates = ["__init__.py", "config.py", "configs.py", "types.py"];

        for filename in candidates {
            let candidate = parent.join(filename);
            if let Some(cached) = Self::read_file_cached(file_cache, &candidate) {
                if cached.content.contains(&class_with_base)
                    || cached.content.contains(&class_no_base)
                {
                    return Some(cached.content.clone());
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
        implementation: &ImplementationType,
    ) -> Option<String> {
        if matches!(implementation, ImplementationType::Function) {
            return Self::extract_config_type_from_function_text(content, symbol);
        }
        let root = ast.root();

        // Pattern 1: Class with generic type - class Symbol(Plugin[Config])
        // Look for the class definition first
        let class_pattern = format!("class {}($$$BASES): $$$BODY", symbol);
        let class_matches: Vec<_> = root.find_all(class_pattern.as_str()).collect();

        for class_match in &class_matches {
            // Get the base classes text and look for Plugin[ConfigName]
            let env = class_match.get_env();
            let bases = env.get_multiple_matches("$$$BASES");
            for base in bases {
                let base_text = base.text();
                // Check if this base is Plugin[Something]
                if base_text.contains("Plugin[") {
                    if let Some(start) = base_text.find("Plugin[") {
                        let rest = &base_text[start + 7..];
                        if let Some(end) = rest.find(']') {
                            let config_name = rest[..end].trim();
                            if !config_name.is_empty() {
                                return Some(config_name.to_string());
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Extract config type from function definition in content
    /// Finds "def symbol(..., config: ConfigClass, ...)" and extracts "ConfigClass"
    fn extract_config_type_from_function_text(content: &str, symbol: &str) -> Option<String> {
        // Find "def symbol(" in content
        let search = format!("def {}(", symbol);
        let start = content.find(&search)?;
        let after_start = &content[start + search.len()..];

        // Find closing paren (handle nested parens)
        let paren_end = Self::find_matching_paren(after_start)?;
        let params_text = &after_start[..paren_end];

        // Look for "config:" pattern
        let config_start = params_text.find("config:")?;
        let rest = &params_text[config_start + 7..];

        // Find end of type annotation (comma or end of string)
        let type_end = rest.find(|c| c == ',' || c == ')').unwrap_or(rest.len());
        let config_type = rest[..type_end].trim();

        // Verify it ends with "Config"
        if config_type.ends_with("Config") {
            Some(config_type.to_string())
        } else {
            None
        }
    }

    /// Find the position of matching closing paren, handling nested parens/brackets
    fn find_matching_paren(text: &str) -> Option<usize> {
        let mut depth = 0;
        for (i, ch) in text.char_indices() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' => {
                    if depth == 0 {
                        return Some(i);
                    }
                    depth -= 1;
                }
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        None
    }

    /// Extract __init__ parameters from a class definition
    fn extract_class_init_params(ast: &PythonAst, class_name: &str) -> Vec<ArgumentSpec> {
        let root = ast.root();

        // Find class definition - early return if not found
        let pattern = format!("class {}($$$BASES): $$$BODY", class_name);
        if root.find_all(pattern.as_str()).next().is_none() {
            return Vec::new();
        }

        // Look for __init__ method within the class
        let init_pattern = "def __init__(self, $$$PARAMS): $$$BODY";
        for init_match in root.find_all(init_pattern) {
            if let Some(params) = Self::extract_params_from_match(&init_match) {
                return params;
            }
        }

        Vec::new()
    }

    /// Extract parameters from a function definition using text-based parsing
    fn extract_function_params(content: &str, function_name: &str) -> Vec<ArgumentSpec> {
        // Find "def function_name(" in content
        let search = format!("def {}(", function_name);
        let Some(start) = content.find(&search) else {
            return Vec::new();
        };
        let after_start = &content[start + search.len()..];

        // Find closing paren
        let Some(paren_end) = Self::find_matching_paren(after_start) else {
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

    /// Parse a single function parameter string into ArgumentSpec
    fn parse_function_param(param_str: &str) -> Option<ArgumentSpec> {
        let param_str = param_str.trim();

        // Skip empty, self, *args, **kwargs
        if param_str.is_empty() || param_str == "self" || param_str.starts_with('*') {
            return None;
        }

        let (name, annotation, default) = Self::parse_param_text(param_str);

        if Self::is_skipped_param(&name) {
            return None;
        }

        Some(ArgumentSpec {
            name,
            annotation,
            required: default.is_none(),
            default,
        })
    }

    /// Extract parameters from an AST match
    fn extract_params_from_match(
        func_match: &ast_grep_core::matcher::NodeMatch<'_, ast_grep_core::source::StrDoc<Python>>,
    ) -> Option<Vec<ArgumentSpec>> {
        let env = func_match.get_env();
        let params_nodes = env.get_multiple_matches("$$$PARAMS");

        let params = params_nodes
            .into_iter()
            .filter_map(|param_node| {
                let param_text = param_node.text();

                // Skip self, *args, **kwargs
                if param_text == "self" || param_text.starts_with('*') {
                    return None;
                }

                let (name, annotation, default) = Self::parse_param_text(&param_text);

                if Self::is_skipped_param(&name) {
                    return None;
                }

                Some(ArgumentSpec {
                    name,
                    annotation,
                    required: default.is_none(),
                    default,
                })
            })
            .collect();

        Some(params)
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
            } else {
                return (before_eq.to_string(), None, Some(default));
            }
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
            if name.starts_with('_')
                || !name
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic())
                    .unwrap_or(false)
            {
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

    /// Convert an ArgumentSpec to a ConfigField for function parameter extraction
    fn argument_to_config_field(arg: &ArgumentSpec) -> ConfigField {
        use schema_extractor::parse_union_types_from_annotation;

        let types = arg
            .annotation
            .as_deref()
            .map(parse_union_types_from_annotation)
            .unwrap_or_else(|| vec!["Any".to_string()]);

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

    /// Build IO contract based on plugin kind
    fn io_contract_for_kind(kind: &PluginKind) -> IOContract {
        match kind {
            PluginKind::Parser => IOContract {
                consumes: vec![IOSlot::StoreFolder, IOSlot::ConfigFile],
                produces: vec![IOSlot::System],
            },
            PluginKind::Exporter => IOContract {
                consumes: vec![IOSlot::System, IOSlot::ConfigFile],
                produces: vec![IOSlot::Folder],
            },
            PluginKind::Modifier | PluginKind::Translation => IOContract {
                consumes: vec![IOSlot::System],
                produces: vec![IOSlot::System],
            },
            PluginKind::Upgrader | PluginKind::Utility => IOContract {
                consumes: Vec::new(),
                produces: Vec::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_all_entry_points_r2x_plugin() {
        let content = r#"[r2x_plugin]
reeds = r2x_reeds:ReEDSParser
"#;
        let entries = AstDiscovery::parse_all_entry_points(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "reeds");
        assert_eq!(entries[0].module, "r2x_reeds");
        assert_eq!(entries[0].symbol, "ReEDSParser");
        assert_eq!(entries[0].section, "r2x_plugin");
    }

    #[test]
    fn test_parse_all_entry_points_multiple_sections() {
        let content = r#"[r2x_plugin]
reeds = r2x_reeds:ReEDSParser

[r2x.transforms]
add-pcm-defaults = r2x_reeds.sysmod.pcm_defaults:add_pcm_defaults
add-emission-cap = r2x_reeds.sysmod.emission_cap:add_emission_cap

[console_scripts]
some-cli = some_module:main
"#;
        let entries = AstDiscovery::parse_all_entry_points(content);
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
        let content = r#"[console_scripts]
some-cli = some_module:main

[gui_scripts]
some-gui = some_module:gui_main
"#;
        let entries = AstDiscovery::parse_all_entry_points(content);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_is_r2x_section() {
        assert!(AstDiscovery::is_r2x_section("r2x_plugin"));
        assert!(AstDiscovery::is_r2x_section("r2x.transforms"));
        assert!(AstDiscovery::is_r2x_section("r2x.parsers"));
        assert!(AstDiscovery::is_r2x_section("r2x.exporters"));
        assert!(!AstDiscovery::is_r2x_section("console_scripts"));
        assert!(!AstDiscovery::is_r2x_section("gui_scripts"));
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
    fn test_entry_point_info_infer_kind() {
        let parser_entry = EntryPointInfo {
            name: "reeds".to_string(),
            module: "r2x_reeds".to_string(),
            symbol: "ReEDSParser".to_string(),
            section: "r2x_plugin".to_string(),
        };
        assert_eq!(parser_entry.infer_kind(), PluginKind::Parser);

        let modifier_entry = EntryPointInfo {
            name: "add-pcm-defaults".to_string(),
            module: "r2x_reeds.sysmod".to_string(),
            symbol: "add_pcm_defaults".to_string(),
            section: "r2x.transforms".to_string(),
        };
        assert_eq!(modifier_entry.infer_kind(), PluginKind::Modifier);
    }

    #[test]
    fn test_plugin_extraction() {
        let plugin = DiscoveredPlugin {
            name: "test-parser".to_string(),
            kind: PluginKind::Parser,
            entry: "TestParser".to_string(),
            invocation: InvocationSpec {
                implementation: ImplementationType::Class,
                method: Some("build_system".to_string()),
                constructor: vec![],
                call: vec![],
            },
            io: IOContract {
                consumes: vec![IOSlot::StoreFolder, IOSlot::ConfigFile],
                produces: vec![IOSlot::System],
            },
            resources: None,
            upgrade: None,
            description: None,
            tags: vec![],
        };

        assert_eq!(plugin.name, "test-parser");
        assert_eq!(plugin.kind, PluginKind::Parser);
        assert_eq!(plugin.entry, "TestParser");
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
        let source_single = r#"
def add_pcm_defaults(system: System, config: PCMDefaultsConfig) -> Result[System, str]:
    pass
"#;
        let config_name =
            AstDiscovery::extract_config_type_from_function_text(source_single, "add_pcm_defaults");
        assert_eq!(config_name, Some("PCMDefaultsConfig".to_string()));

        // Test multi-line function with return type
        let source_multi = r#"
def add_pcm_defaults(
    system: System,
    config: PCMDefaultsConfig,
) -> Result[System, str]:
    pass
"#;
        let config_name_multi =
            AstDiscovery::extract_config_type_from_function_text(source_multi, "add_pcm_defaults");
        assert_eq!(config_name_multi, Some("PCMDefaultsConfig".to_string()));

        // Test function without return type
        let source_no_ret = r#"
def add_pcm_defaults(system: System, config: PCMDefaultsConfig):
    pass
"#;
        let config_name_no_ret =
            AstDiscovery::extract_config_type_from_function_text(source_no_ret, "add_pcm_defaults");
        assert_eq!(config_name_no_ret, Some("PCMDefaultsConfig".to_string()));
    }

    #[test]
    fn test_extract_function_params_direct() {
        let source = r#"
def add_pcm_defaults(
    system: System,
    pcm_defaults_fpath: str | None = None,
    pcm_defaults_dict: dict | None = None,
    pcm_defaults_override: bool = False,
) -> System:
    pass
"#;
        let params = AstDiscovery::extract_function_params(source, "add_pcm_defaults");
        // system should be filtered out
        assert!(!params.iter().any(|p| p.name == "system"));
        // Should have the 3 params
        assert_eq!(params.len(), 3);

        let fpath = params
            .iter()
            .find(|p| p.name == "pcm_defaults_fpath")
            .unwrap();
        assert_eq!(fpath.annotation.as_deref(), Some("str | None"));
        assert_eq!(fpath.default.as_deref(), Some("None"));
        assert!(!fpath.required);

        let override_param = params
            .iter()
            .find(|p| p.name == "pcm_defaults_override")
            .unwrap();
        assert_eq!(override_param.annotation.as_deref(), Some("bool"));
        assert_eq!(override_param.default.as_deref(), Some("False"));
    }

    #[test]
    fn test_argument_to_config_field() {
        let arg = discovery_types::ArgumentSpec {
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
        assert_eq!(AstDiscovery::camel_to_kebab("MyParser"), "my-parser");
        assert_eq!(
            AstDiscovery::camel_to_kebab("SimplePlugin"),
            "simple-plugin"
        );
        assert_eq!(AstDiscovery::camel_to_kebab("MyExporter"), "my-exporter");

        // Acronyms (consecutive uppercase)
        assert_eq!(AstDiscovery::camel_to_kebab("ReEDSParser"), "reeds-parser");
        assert_eq!(AstDiscovery::camel_to_kebab("XMLParser"), "xml-parser");
        assert_eq!(AstDiscovery::camel_to_kebab("HTTPClient"), "http-client");

        // Single word
        assert_eq!(AstDiscovery::camel_to_kebab("Parser"), "parser");
        assert_eq!(AstDiscovery::camel_to_kebab("Reeds"), "reeds");

        // All uppercase acronym
        assert_eq!(AstDiscovery::camel_to_kebab("HTTP"), "http");
        assert_eq!(AstDiscovery::camel_to_kebab("API"), "api");
    }

    #[test]
    fn test_snake_to_kebab() {
        assert_eq!(
            AstDiscovery::snake_to_kebab("add_pcm_defaults"),
            "add-pcm-defaults"
        );
        assert_eq!(AstDiscovery::snake_to_kebab("break_gens"), "break-gens");
        assert_eq!(AstDiscovery::snake_to_kebab("simple"), "simple");
        assert_eq!(AstDiscovery::snake_to_kebab("a_b_c"), "a-b-c");
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
        let text_multi = r#"@expose_plugin
def break_gens(
    system: System,
    config: BreakGensConfig,
) -> System:
    pass"#;
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
}
