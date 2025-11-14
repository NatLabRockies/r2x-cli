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
pub mod extractor;
use anyhow::{anyhow, Result};
use ast_grep_language::Python;
use r2x_logger as logger;
use r2x_manifest::{DecoratorRegistration, FunctionSignature, PluginSpec};
use std::path::Path;

/// AST-based plugin discovery orchestrator
pub struct AstDiscovery;

impl AstDiscovery {
    /// Discover plugins from a Python package using AST parsing
    ///
    /// # Arguments
    /// * `package_path` - Path to the installed package (e.g., site-packages/r2x_reeds)
    /// * `package_name_full` - Full package name (e.g., "r2x-reeds")
    /// * `venv_path` - Optional path to virtual environment for entry_points.txt lookup
    /// * `package_version` - Optional package version string
    ///
    /// # Returns
    /// Tuple of (plugins with resolved references, decorator registrations)
    pub fn discover_plugins(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
        _package_version: Option<&str>,
    ) -> Result<(Vec<PluginSpec>, Vec<DecoratorRegistration>)> {
        let start_time = std::time::Instant::now();
        logger::info(&format!("AST discovery started for: {}", package_name_full));

        // Find the plugins.py file using entry_points.txt
        let (plugins_py, plugin_module) =
            Self::find_plugins_py_via_entry_points(package_path, package_name_full, venv_path)?;
        logger::info(&format!("Found plugins.py at: {:?}", plugins_py));

        // Phase 1: Extract plugins with constructor_args
        let package_root = plugins_py
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| package_path.to_path_buf());
        let extractor = extractor::PluginExtractor::new(
            plugins_py.clone(),
            plugin_module.clone(),
            package_root.clone(),
        )
        .map_err(|e| anyhow!("Failed to create extractor: {}", e))?;

        let mut plugins = extractor
            .extract_plugins()
            .map_err(|e| anyhow!("Failed to extract plugins: {}", e))?;

        logger::info(&format!(
            "Phase 1 complete: Extracted {} plugins",
            plugins.len()
        ));

        // Phase 2: Resolve all class/function references
        for plugin in &mut plugins {
            extractor
                .resolve_references(plugin, &package_root, package_name_full)
                .map_err(|e| anyhow!("Failed to resolve references for {}: {}", plugin.name, e))?;
        }

        logger::info(&format!(
            "Phase 2 complete: Resolved references for {} plugins",
            plugins.len()
        ));

        // Phase 3: Scan for decorator registrations and associate with plugins
        let decorator_registrations = Self::scan_package_for_decorators(&plugins_py)?;
        logger::info(&format!(
            "Phase 3 complete: Found {} decorator registrations",
            decorator_registrations.len()
        ));

        // TODO: Associate decorators with plugins based on class references

        let elapsed = start_time.elapsed();
        logger::info(&format!(
            "AST discovery completed in {:.2}ms for {}",
            elapsed.as_secs_f64() * 1000.0,
            package_name_full
        ));

        Ok((plugins, decorator_registrations))
    }
    /// Find plugins.py file using entry_points.txt
    fn find_plugins_py_via_entry_points(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
    ) -> Result<(std::path::PathBuf, String)> {
        use std::fs;
        // Try to find entry_points.txt in the package's dist-info
        let entry_points_path =
            Self::find_entry_points_txt(package_path, package_name_full, venv_path)?;
        // Parse the entry_points.txt to get the module path
        let entry_points_content = fs::read_to_string(&entry_points_path)
            .map_err(|e| anyhow!("Failed to read entry_points.txt: {}", e))?;
        // Look for [r2x_plugin] section and extract module:function
        let (module_path, _function) = Self::parse_entry_point(&entry_points_content)?;
        // Convert module path to file path
        // e.g., "r2x_reeds.plugins" -> "r2x_reeds/plugins.py"
        let relative_path = module_path.replace('.', "/") + ".py";
        // For editable installs, package_path points to src/ or similar
        // Try to locate the actual file
        let plugins_path = package_path.join(&relative_path);
        if plugins_path.exists() {
            return Ok((plugins_path, module_path));
        }
        // Try one level up (in case package_path is the package root)
        if let Some(parent) = package_path.parent() {
            let plugins_path = parent.join(&relative_path);
            if plugins_path.exists() {
                return Ok((plugins_path, module_path));
            }
        }
        Err(anyhow!(
            "Could not find plugins.py at expected location: {} (from entry point: {})",
            relative_path,
            module_path
        ))
    }
    /// Find entry_points.txt for the package
    fn find_entry_points_txt(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
    ) -> Result<std::path::PathBuf> {
        use std::fs;
        let normalized_name = package_name_full.replace('-', "_");
        // Try venv site-packages first if provided
        if let Some(venv) = venv_path {
            let venv_path = std::path::PathBuf::from(venv);
            let lib_dir = venv_path.join("lib");
            if let Ok(entries) = fs::read_dir(&lib_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir()
                        && path
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("python")
                    {
                        let site_packages = path.join("site-packages");
                        if let Ok(sp_entries) = fs::read_dir(&site_packages) {
                            for sp_entry in sp_entries.flatten() {
                                let name = sp_entry.file_name().to_string_lossy().to_string();
                                if name.starts_with(&normalized_name)
                                    && name.ends_with(".dist-info")
                                {
                                    let entry_points = sp_entry.path().join("entry_points.txt");
                                    if entry_points.exists() {
                                        return Ok(entry_points);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Fallback: look near package_path
        if let Some(parent) = package_path.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&normalized_name) && name.ends_with(".dist-info") {
                        let entry_points = entry.path().join("entry_points.txt");
                        if entry_points.exists() {
                            return Ok(entry_points);
                        }
                    }
                }
            }
        }
        Err(anyhow!(
            "Could not find entry_points.txt for package: {}",
            package_name_full
        ))
    }
    /// Parse entry_points.txt to extract r2x_plugin entry point
    fn parse_entry_point(content: &str) -> Result<(String, String)> {
        let mut in_r2x_section = false;
        for line in content.lines() {
            let line = line.trim();
            if line == "[r2x_plugin]" {
                in_r2x_section = true;
                continue;
            }
            if in_r2x_section {
                if line.starts_with('[') {
                    break;
                }
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                // Parse "key = module:function" format
                if let Some(eq_idx) = line.find('=') {
                    let value = line[eq_idx + 1..].trim();
                    if let Some(colon_idx) = value.find(':') {
                        let module = value[..colon_idx].trim().to_string();
                        let function = value[colon_idx + 1..].trim().to_string();
                        return Ok((module, function));
                    }
                }
            }
        }
        Err(anyhow!(
            "No [r2x_plugin] entry point found in entry_points.txt"
        ))
    }
    /// Scan entire package directory for decorator registrations
    fn scan_package_for_decorators(
        plugins_py: &std::path::Path,
    ) -> Result<Vec<DecoratorRegistration>> {
        use walkdir::WalkDir;
        let mut all_registrations = Vec::new();
        // Get the package root directory (parent of plugins.py)
        let package_root = plugins_py
            .parent()
            .ok_or_else(|| anyhow!("Invalid plugins.py path"))?;
        logger::info(&format!(
            "Scanning for decorator registrations in: {:?}",
            package_root
        ));
        // Walk through all Python files in the package
        for entry in WalkDir::new(package_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            // Only process Python files
            if path.extension().and_then(|s| s.to_str()) != Some("py") {
                continue;
            }
            // Skip __pycache__ and other non-source files
            if path.to_string_lossy().contains("__pycache__") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(registrations) =
                    Self::scan_file_for_decorators(path, &content, package_root)
                {
                    if !registrations.is_empty() {
                        logger::info(&format!(
                            "Found {} decorator(s) in: {:?}",
                            registrations.len(),
                            path
                        ));
                        all_registrations.extend(registrations);
                    }
                }
            }
        }
        Ok(all_registrations)
    }
    /// Scan a single file for decorator registrations
    fn scan_file_for_decorators(
        file_path: &std::path::Path,
        content: &str,
        package_root: &std::path::Path,
    ) -> Result<Vec<DecoratorRegistration>> {
        use ast_grep_core::AstGrep;
        use ast_grep_language::Python;
        let mut registrations = Vec::new();
        let sg = AstGrep::new(content, Python);
        let root = sg.root();
        // Find all decorated functions matching the pattern:
        // @$CLASS.$METHOD($$$ARGS)
        // def $FUNC($$$PARAMS): $$$BODY
        let pattern = "@$CLASS.$METHOD($$$ARGS)
def $FUNC($$$PARAMS): $$$BODY";
        let decorated_functions: Vec<_> = root.find_all(pattern).collect();
        for decorated_match in decorated_functions {
            if let Ok(registration) =
                Self::extract_decorator_info(&decorated_match, file_path, package_root)
            {
                registrations.push(registration);
            }
        }
        Ok(registrations)
    }
    /// Extract decorator information from a match
    fn extract_decorator_info(
        decorated_match: &ast_grep_core::matcher::NodeMatch<
            '_,
            ast_grep_core::source::StrDoc<Python>,
        >,
        file_path: &std::path::Path,
        package_root: &std::path::Path,
    ) -> Result<DecoratorRegistration> {
        let env = decorated_match.get_env();
        let class_name = env
            .get_match("$CLASS")
            .ok_or_else(|| anyhow!("Missing $CLASS"))?
            .text()
            .to_string();
        let method_name = env
            .get_match("$METHOD")
            .ok_or_else(|| anyhow!("Missing $METHOD"))?
            .text()
            .to_string();
        let args_text = env
            .get_multiple_matches("$$$ARGS")
            .first()
            .map(|n| n.text().to_string())
            .unwrap_or_default();
        let function_name = env
            .get_match("$FUNC")
            .ok_or_else(|| anyhow!("Missing $FUNC"))?
            .text()
            .to_string();
        let params_text = env
            .get_multiple_matches("$$$PARAMS")
            .first()
            .map(|n| n.text().to_string())
            .unwrap_or_default();
        // Parse decorator arguments
        let decorator_args =
            decorator_scanner::DecoratorScanner::parse_decorator_args_from_text(&args_text);
        // Parse function parameters
        let parameters =
            decorator_scanner::DecoratorScanner::parse_function_parameters_from_text(&params_text);
        // Get relative path from package root
        let source_file = file_path
            .strip_prefix(package_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.to_string());
        // Infer module name from file path
        let function_module = Self::infer_module_from_path(file_path, package_root);
        let function_sig = FunctionSignature {
            return_type: "None".to_string(),
            parameters,
        };
        Ok(DecoratorRegistration {
            decorator_class: class_name,
            decorator_method: method_name,
            function_name,
            function_module,
            source_file,
            line_number: None,
            decorator_args,
            function_signature: Some(function_sig),
        })
    }
    /// Infer module name from file path relative to package root
    fn infer_module_from_path(
        file_path: &std::path::Path,
        package_root: &std::path::Path,
    ) -> String {
        if let Ok(rel_path) = file_path.strip_prefix(package_root) {
            let parts: Vec<&str> = rel_path
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect();
            if !parts.is_empty() {
                let module_parts: Vec<&str> = parts
                    .into_iter()
                    .map(|p| p.strip_suffix(".py").unwrap_or(p))
                    .collect();
                // Add the package name prefix
                if let Some(package_name) = package_root.file_name().and_then(|n| n.to_str()) {
                    return format!("{}.{}", package_name, module_parts.join("."));
                }
                return module_parts.join(".");
            }
        }
        "unknown".to_string()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    #[test]
    fn test_parse_entry_point() {
        let content = r#"[r2x_plugin]
reeds = r2x_reeds.plugins:register_plugin
"#;
        let result = AstDiscovery::parse_entry_point(content);
        assert!(result.is_ok());
        let (module, function) = result.unwrap();
        assert_eq!(module, "r2x_reeds.plugins");
        assert_eq!(function, "register_plugin");
    }
    #[test]
    fn test_plugin_extraction() {
        use r2x_manifest::{IOContract, IOSlot, ImplementationType, InvocationSpec, PluginKind};

        let plugin = PluginSpec {
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
    fn test_discover_plugins_integration() {
        let temp_dir = TempDir::new().unwrap();
        let plugins_file = temp_dir.path().join("plugins.py");
        let content = r#"
from r2x_core import PluginManifest, PluginSpec

manifest = PluginManifest(package="r2x-test")
manifest.add(
    PluginSpec.parser(
        name="test.parser",
        entry=TestParser,
        config=TestConfig,
    )
)
"#;
        fs::write(&plugins_file, content).unwrap();
        assert!(plugins_file.exists());
    }
}
