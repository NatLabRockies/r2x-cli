//! Package-level AST cache for efficient plugin discovery
//!
//! This module provides a single-pass AST parsing cache that walks the package
//! directory once and parses each Python file once, extracting all necessary
//! information for plugin discovery in a single pass.
//!
//! Performance optimizations:
//! - Single WalkDir traversal (vs 3 separate traversals before)
//! - Single AST parse per file (vs multiple pattern searches)
//! - Kind-based node matching (vs pattern string compilation)

use ast_grep_core::AstGrep;
use ast_grep_language::Python;
use r2x_logger as logger;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::{DirEntry, WalkDir};

/// Extracted class definition from a Python file
#[derive(Debug, Clone)]
pub struct ClassDef {
    /// Class name (e.g., "ReEDSParser")
    pub name: String,
    /// Base class names (e.g., ["Plugin[Config]", "BaseClass"])
    pub bases: Vec<String>,
    /// Generic parameter if Plugin[Config] form (e.g., "Config")
    pub generic_param: Option<String>,
    /// Method names defined in this class
    pub methods: Vec<String>,
    /// The raw class body text for further analysis
    pub body_text: String,
}

/// Extracted function with @expose_plugin decorator
#[derive(Debug, Clone)]
pub struct DecoratedFunc {
    /// Function name
    pub function_name: String,
    /// Raw parameter text for parsing
    pub parameters_text: String,
}

/// Extracted field definition from a class
#[derive(Debug, Clone)]
pub struct FieldDef {
    /// Field name
    pub name: String,
    /// Type annotation text
    pub type_annotation: String,
    /// Full field text including default value
    pub full_text: String,
}

/// Pre-parsed data for a single Python file
#[derive(Debug)]
pub struct ParsedPyFile {
    /// File path
    pub path: PathBuf,
    /// Raw file content
    pub content: String,
    /// Extracted class definitions
    pub classes: Vec<ClassDef>,
    /// Functions decorated with @expose_plugin
    pub decorated_functions: Vec<DecoratedFunc>,
}

/// Cache for a package - walk once, parse once, query many times
pub struct PackageAstCache {
    /// Parsed files indexed by path
    files: HashMap<PathBuf, ParsedPyFile>,
    /// Index: class name -> file path
    class_index: HashMap<String, PathBuf>,
}

impl PackageAstCache {
    fn is_ignored_dir(entry: &DirEntry) -> bool {
        if !entry.file_type().is_dir() {
            return false;
        }

        matches!(
            entry.file_name().to_string_lossy().as_ref(),
            ".git"
                | ".hg"
                | ".svn"
                | ".venv"
                | "venv"
                | "env"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".tox"
                | ".mypy_cache"
                | ".pytest_cache"
                | "__pycache__"
                | "htmlcov"
        )
    }

    /// Build cache by walking package once and parsing each .py file once
    pub fn build(package_root: &Path) -> Self {
        let start = Instant::now();
        let mut files = HashMap::new();
        let mut class_index = HashMap::new();
        let mut file_count = 0;

        for entry in WalkDir::new(package_root)
            .into_iter()
            .filter_entry(|entry| !Self::is_ignored_dir(entry))
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "py"))
        {
            let path = entry.path().to_path_buf();
            if let Ok(content) = std::fs::read_to_string(&path) {
                let parsed = Self::parse_file(&path, content);

                // Build class index
                for class in &parsed.classes {
                    class_index.insert(class.name.clone(), path.clone());
                }

                files.insert(path, parsed);
                file_count += 1;
            }
        }

        logger::debug(&format!(
            "PackageAstCache::build: parsed {} files in {:.2}ms",
            file_count,
            start.elapsed().as_secs_f64() * 1000.0
        ));

        PackageAstCache { files, class_index }
    }

    /// Parse a single file, extracting all needed data in one pass
    fn parse_file(path: &Path, content: String) -> ParsedPyFile {
        let start = Instant::now();

        let sg = AstGrep::new(&content, Python);
        let root = sg.root();

        let classes = Self::extract_all_classes(&root);
        let decorated_functions = Self::extract_all_decorated_functions(&root);

        logger::debug(&format!(
            "parse_file {:?}: {} classes, {} decorators in {:.3}ms",
            path.file_name().unwrap_or_default(),
            classes.len(),
            decorated_functions.len(),
            start.elapsed().as_secs_f64() * 1000.0
        ));

        ParsedPyFile {
            path: path.to_path_buf(),
            content,
            classes,
            decorated_functions,
        }
    }

    /// Extract all class definitions from the AST root using pattern matching
    fn extract_all_classes(
        root: &ast_grep_core::Node<'_, ast_grep_core::source::StrDoc<Python>>,
    ) -> Vec<ClassDef> {
        let mut classes = Vec::new();

        // Pattern 1: Classes with base classes - class Name(BaseClasses): body
        let pattern_with_bases = "class $NAME($$$BASES): $$$BODY";
        for node in root.find_all(pattern_with_bases) {
            let env = node.get_env();

            // Try metavariable first, fallback to text extraction
            let name = env
                .get_match("$NAME")
                .map(|n| n.text().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| Self::extract_class_name_from_text(&node.text()));

            if name.is_empty() {
                continue;
            }

            // Try metavariable first, fallback to text extraction
            let bases_text: String = {
                let from_env: String = env
                    .get_multiple_matches("$$$BASES")
                    .iter()
                    .map(|b| b.text().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                if from_env.is_empty() {
                    Self::extract_bases_from_text(&node.text())
                } else {
                    from_env
                }
            };

            let (bases, generic_param) = Self::parse_bases(&bases_text);
            let body_text = node.text().to_string();
            let methods = Self::extract_method_names(&body_text);

            classes.push(ClassDef {
                name,
                bases,
                generic_param,
                methods,
                body_text,
            });
        }

        // Pattern 2: Classes without base classes - class Name: body
        let pattern_no_bases = "class $NAME: $$$BODY";
        for node in root.find_all(pattern_no_bases) {
            let env = node.get_env();

            // Try metavariable first, fallback to text extraction
            let name = env
                .get_match("$NAME")
                .map(|n| n.text().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| Self::extract_class_name_from_text(&node.text()));

            if name.is_empty() {
                continue;
            }

            // Skip if we already found this class with bases
            if classes.iter().any(|c| c.name == name) {
                continue;
            }

            let body_text = node.text().to_string();
            let methods = Self::extract_method_names(&body_text);

            classes.push(ClassDef {
                name,
                bases: Vec::new(),
                generic_param: None,
                methods,
                body_text,
            });
        }

        classes
    }

    /// Parse base class text into list of bases and extract Plugin[Config] generic param
    fn parse_bases(bases_text: &str) -> (Vec<String>, Option<String>) {
        let mut bases = Vec::new();
        let mut generic_param = None;

        // Split by comma, handling nested brackets
        let mut current = String::new();
        let mut depth = 0;

        for ch in bases_text.chars() {
            match ch {
                '[' | '(' | '{' => {
                    depth += 1;
                    current.push(ch);
                }
                ']' | ')' | '}' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    let base = current.trim().to_string();
                    if !base.is_empty() {
                        // Check for Plugin[Config] pattern
                        if let Some(param) = Self::extract_generic_param(&base, "Plugin") {
                            generic_param = Some(param);
                        }
                        bases.push(base);
                    }
                    current.clear();
                }
                _ => current.push(ch),
            }
        }

        // Don't forget the last base
        let base = current.trim().to_string();
        if !base.is_empty() {
            if let Some(param) = Self::extract_generic_param(&base, "Plugin") {
                generic_param = Some(param);
            }
            bases.push(base);
        }

        (bases, generic_param)
    }

    /// Extract generic parameter from Type[Param] pattern
    fn extract_generic_param(base: &str, type_name: &str) -> Option<String> {
        let prefix = format!("{}[", type_name);
        if base.starts_with(&prefix) && base.ends_with(']') {
            let inner = &base[prefix.len()..base.len() - 1];
            Some(inner.trim().to_string())
        } else {
            None
        }
    }

    /// Extract method names from class body text
    fn extract_method_names(body_text: &str) -> Vec<String> {
        let mut methods = Vec::new();

        // Simple text-based extraction for method names
        // Look for "def method_name(" patterns
        for line in body_text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("def ") {
                if let Some(rest) = trimmed.strip_prefix("def ") {
                    if let Some(paren_idx) = rest.find('(') {
                        let method_name = rest[..paren_idx].trim();
                        methods.push(method_name.to_string());
                    }
                }
            }
        }

        methods
    }

    /// Extract class name from class definition text
    /// Handles both "class Name:" and "class Name(Base):" forms
    fn extract_class_name_from_text(text: &str) -> String {
        // Look for "class " prefix
        if let Some(after_class) = text.strip_prefix("class ") {
            // Find the end of the class name (either '(' or ':')
            let end = after_class
                .find(|c| ['(', ':'].contains(&c))
                .unwrap_or(after_class.len());
            return after_class[..end].trim().to_string();
        }
        String::new()
    }

    /// Extract base classes from class definition text
    /// Handles "class Name(Base1, Base2):" form
    fn extract_bases_from_text(text: &str) -> String {
        // Find the opening paren after "class Name"
        if let Some(open_paren) = text.find('(') {
            // Find the matching closing paren
            if let Some(close_paren) = text.find("):") {
                if close_paren > open_paren {
                    return text[open_paren + 1..close_paren].trim().to_string();
                }
            }
        }
        String::new()
    }

    /// Extract all functions decorated with @expose_plugin using pattern matching
    fn extract_all_decorated_functions(
        root: &ast_grep_core::Node<'_, ast_grep_core::source::StrDoc<Python>>,
    ) -> Vec<DecoratedFunc> {
        let mut results = Vec::new();

        // Try multiple patterns for @expose_plugin decorated functions
        let patterns = [
            "@expose_plugin\ndef $FUNC($$$PARAMS): $$$BODY",
            "@expose_plugin()\ndef $FUNC($$$PARAMS): $$$BODY",
            "@expose_plugin($$$ARGS)\ndef $FUNC($$$PARAMS): $$$BODY",
        ];

        for pattern in patterns {
            for node in root.find_all(pattern) {
                let env = node.get_env();

                // Try to get function name from metavariable, fallback to text extraction
                let function_name = env
                    .get_match("$FUNC")
                    .map(|n| n.text().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| Self::extract_function_name_from_text(&node.text()));

                // Try to get parameters from metavariable, fallback to text extraction
                let parameters_text = {
                    let from_env: String = env
                        .get_multiple_matches("$$$PARAMS")
                        .iter()
                        .map(|p| p.text().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if from_env.is_empty() {
                        Self::extract_params_from_text(&node.text())
                    } else {
                        from_env
                    }
                };

                if !function_name.is_empty() {
                    // Avoid duplicates
                    if !results
                        .iter()
                        .any(|r: &DecoratedFunc| r.function_name == function_name)
                    {
                        results.push(DecoratedFunc {
                            function_name,
                            parameters_text,
                        });
                    }
                }
            }
        }

        results
    }

    /// Extract function name from decorated function text
    /// Handles "@decorator\ndef function_name(params):" form
    fn extract_function_name_from_text(text: &str) -> String {
        // Find "def " after the decorator line(s)
        if let Some(def_pos) = text.find("def ") {
            let after_def = &text[def_pos + 4..];
            // Find the opening paren
            if let Some(paren_pos) = after_def.find('(') {
                return after_def[..paren_pos].trim().to_string();
            }
        }
        String::new()
    }

    /// Extract parameters from function definition text
    /// Handles "def name(param1, param2):" form
    fn extract_params_from_text(text: &str) -> String {
        // Find "def " and then the parentheses
        if let Some(def_pos) = text.find("def ") {
            let after_def = &text[def_pos..];
            if let Some(open_paren) = after_def.find('(') {
                // Find matching close paren (handle nested)
                let params_start = &after_def[open_paren + 1..];
                let mut depth = 1;
                let mut end = 0;
                for (i, ch) in params_start.char_indices() {
                    match ch {
                        '(' | '[' | '{' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = i;
                                break;
                            }
                        }
                        ']' | '}' => depth -= 1,
                        _ => {}
                    }
                }
                if end > 0 {
                    return params_start[..end].trim().to_string();
                }
            }
        }
        String::new()
    }

    // =========================================================================
    // Query Methods - no re-walking or re-parsing
    // =========================================================================

    /// Find a class by name
    pub fn find_class(&self, name: &str) -> Option<&ClassDef> {
        if let Some(path) = self.class_index.get(name) {
            if let Some(file) = self.files.get(path) {
                return file.classes.iter().find(|c| c.name == name);
            }
        }
        None
    }

    /// Find a class and its file path by name
    pub fn find_class_with_path(&self, name: &str) -> Option<(&PathBuf, &ClassDef)> {
        if let Some(path) = self.class_index.get(name) {
            if let Some(file) = self.files.get(path) {
                if let Some(class) = file.classes.iter().find(|c| c.name == name) {
                    return Some((path, class));
                }
            }
        }
        None
    }

    /// Find a Plugin class by symbol name
    /// Returns (file_path, config_class_name) if found
    pub fn find_plugin_class(&self, symbol: &str) -> Option<(&PathBuf, String)> {
        if let Some((path, class)) = self.find_class_with_path(symbol) {
            // Check if this class inherits from Plugin[...]
            if let Some(ref config_name) = class.generic_param {
                return Some((path, config_name.clone()));
            }

            // Also check bases directly for Plugin[Config] pattern
            for base in &class.bases {
                if let Some(param) = Self::extract_generic_param(base, "Plugin") {
                    return Some((path, param));
                }
            }
        }
        None
    }

    /// Find a config class by name and return its file content
    pub fn find_config_class_content(&self, name: &str) -> Option<(&PathBuf, &str)> {
        // First try the class index
        if let Some(path) = self.class_index.get(name) {
            if let Some(file) = self.files.get(path) {
                return Some((path, &file.content));
            }
        }

        // Fallback: search all files for the class definition
        for (path, file) in &self.files {
            let class_def = format!("class {}(", name);
            let class_def_no_base = format!("class {}:", name);
            if file.content.contains(&class_def) || file.content.contains(&class_def_no_base) {
                return Some((path, &file.content));
            }
        }

        None
    }

    /// Get all @expose_plugin decorated functions from all files
    pub fn get_all_decorated_functions(&self) -> Vec<(&PathBuf, &DecoratedFunc)> {
        self.files
            .iter()
            .flat_map(|(path, file)| {
                file.decorated_functions
                    .iter()
                    .map(move |func| (path, func))
            })
            .collect()
    }

    /// Get file content by path
    pub fn get_file_content(&self, path: &Path) -> Option<&str> {
        self.files.get(path).map(|f| f.content.as_str())
    }

    /// Get all parsed files
    pub fn files(&self) -> &HashMap<PathBuf, ParsedPyFile> {
        &self.files
    }

    /// Get the number of parsed files
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_bases() {
        let (bases, generic) = PackageAstCache::parse_bases("Plugin[MyConfig], BaseClass");
        assert_eq!(bases, vec!["Plugin[MyConfig]", "BaseClass"]);
        assert_eq!(generic, Some("MyConfig".to_string()));

        let (bases2, generic2) = PackageAstCache::parse_bases("BaseClass");
        assert_eq!(bases2, vec!["BaseClass"]);
        assert!(generic2.is_none());
    }

    #[test]
    fn test_extract_method_names() {
        let body = r#"
class MyClass:
    def __init__(self):
        pass

    def process(self, data):
        return data

    def on_build(self, system):
        pass
"#;
        let methods = PackageAstCache::extract_method_names(body);
        assert!(methods.contains(&"__init__".to_string()));
        assert!(methods.contains(&"process".to_string()));
        assert!(methods.contains(&"on_build".to_string()));
    }

    #[test]
    fn test_build_cache() -> anyhow::Result<()> {
        let temp_dir = TempDir::new()?;

        // Create a test Python file
        let py_content = r#"
from r2x_core import Plugin, expose_plugin

class MyConfig:
    name: str = "test"

class MyParser(Plugin[MyConfig]):
    def __init__(self):
        pass

    def on_build(self, system):
        return system

@expose_plugin
def my_transform(system):
    return system
"#;

        fs::write(temp_dir.path().join("plugin.py"), py_content)?;

        let cache = PackageAstCache::build(temp_dir.path());

        // Check that files were parsed
        assert_eq!(cache.file_count(), 1);

        // Check class extraction
        let class = cache.find_class("MyParser");
        assert!(class.is_some());
        assert!(
            class.is_some_and(|c| c.generic_param == Some("MyConfig".to_string())
                && c.methods.contains(&"on_build".to_string()))
        );

        // Check Plugin class lookup
        let plugin = cache.find_plugin_class("MyParser");
        assert!(plugin.is_some());
        assert!(plugin.is_some_and(|(_, config_name)| config_name == "MyConfig"));

        // Check decorated function extraction
        let decorated = cache.get_all_decorated_functions();
        assert_eq!(decorated.len(), 1);
        assert_eq!(decorated[0].1.function_name, "my_transform");

        Ok(())
    }

    #[test]
    fn test_find_config_class() -> anyhow::Result<()> {
        let temp_dir = TempDir::new()?;

        let py_content = r#"
class PCMDefaultsConfig:
    path: str = "default.json"
    override: bool = False
"#;

        fs::write(temp_dir.path().join("config.py"), py_content)?;

        let cache = PackageAstCache::build(temp_dir.path());

        let config = cache.find_config_class_content("PCMDefaultsConfig");
        assert!(config.is_some());
        assert!(config.is_some_and(|(_, content)| content.contains("class PCMDefaultsConfig")));

        Ok(())
    }
}
