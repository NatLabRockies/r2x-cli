use anyhow::{anyhow, Result};
use ast_grep_core::AstGrep;
use ast_grep_language::Python;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use walkdir::WalkDir;

use r2x_manifest::{DecoratorRegistration, FunctionParameter, FunctionSignature, VarArgType};

/// Scanner for finding @Class.register_* decorators in Python packages using AST parsing
pub struct DecoratorScanner {
    /// Package root directory
    package_root: PathBuf,
}

impl DecoratorScanner {
    /// Create a new decorator scanner for the given package root
    pub fn new(package_root: PathBuf) -> Self {
        debug!("Initializing decorator scanner for: {:?}", package_root);
        DecoratorScanner { package_root }
    }

    /// Scan the entire package for decorated functions
    pub fn scan_for_decorators(&self) -> Result<Vec<DecoratorRegistration>> {
        debug!("Scanning package for decorators: {:?}", self.package_root);

        let mut registrations = Vec::new();
        let mut py_files_scanned = 0;

        // Walk through all Python files
        for entry in WalkDir::new(&self.package_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Only process Python files
            if path.extension().and_then(|s| s.to_str()) != Some("py") {
                continue;
            }

            py_files_scanned += 1;
            debug!("Scanning Python file: {:?}", path);

            // Scan this file for decorators
            match self.scan_file(path) {
                Ok(mut file_registrations) => {
                    registrations.append(&mut file_registrations);
                }
                Err(e) => {
                    debug!("Error scanning file {:?}: {}", path, e);
                }
            }
        }

        info!(
            "Scanned {} Python files, found {} decorator registrations",
            py_files_scanned,
            registrations.len()
        );

        Ok(registrations)
    }

    /// Scan a single Python file for decorators using AST parsing with ast-grep
    fn scan_file(&self, file_path: &Path) -> Result<Vec<DecoratorRegistration>> {
        let content = fs::read_to_string(file_path)?;

        // Use pure ast-grep for decorator discovery
        self.scan_file_with_ast_grep(file_path, &content)
    }

    /// Scan file using ast-grep for pure AST-based decorator discovery
    fn scan_file_with_ast_grep(
        &self,
        file_path: &Path,
        content: &str,
    ) -> Result<Vec<DecoratorRegistration>> {
        let mut registrations = Vec::new();

        // Parse the Python code using ast-grep
        let sg = AstGrep::new(content, Python);
        let root = sg.root();

        // Find all decorated functions matching the pattern:
        // @$CLASS.$METHOD($$$ARGS)
        // def $FUNC($$$PARAMS): $$$BODY
        let pattern = "@$CLASS.$METHOD($$$ARGS)\ndef $FUNC($$$PARAMS): $$$BODY";

        debug!(
            "Searching for decorated functions with pattern: {:?}",
            pattern
        );

        let decorated_functions: Vec<_> = root.find_all(pattern).collect();

        debug!("Found {} decorated functions", decorated_functions.len());

        for decorated_match in decorated_functions {
            match self.extract_from_decorated_match(&decorated_match, file_path) {
                Ok(registration) => registrations.push(registration),
                Err(e) => {
                    debug!("extract_from_decorated_match error: {}", e);
                }
            }
        }

        Ok(registrations)
    }

    /// Extract decorator and function information directly from ast-grep match meta-variables
    fn extract_from_decorated_match<'a>(
        &self,
        decorated_match: &ast_grep_core::matcher::NodeMatch<
            'a,
            ast_grep_core::source::StrDoc<Python>,
        >,
        file_path: &Path,
    ) -> Result<DecoratorRegistration> {
        // Extract meta-variables directly from ast-grep match using the MetaVarEnv
        let env = decorated_match.get_env();
        let decorated_text = decorated_match.text();
        let decorator_line = decorated_text.lines().next().unwrap_or_default().trim();
        let function_line = decorated_text.lines().nth(1).unwrap_or_default().trim();

        let args_text_env = env
            .get_multiple_matches("$$$ARGS")
            .first()
            .map(|n| n.text().to_string())
            .unwrap_or_default();

        let args_text = if args_text_env.is_empty() {
            Self::extract_args_from_decorator_line(decorator_line)
        } else {
            args_text_env
        };

        let (class_name, method_name) = if let (Some(class), Some(method)) =
            (env.get_match("$CLASS"), env.get_match("$METHOD"))
        {
            (class.text().to_string(), method.text().to_string())
        } else if let Some((class, method)) = Self::parse_decorator_target(decorator_line) {
            (class, method)
        } else {
            return Err(anyhow!("Missing decorator class/method meta-variables"));
        };

        let function_name = if let Some(func) = env.get_match("$FUNC") {
            func.text().to_string()
        } else if let Some(name) = Self::extract_function_name_from_line(function_line) {
            name
        } else {
            return Err(anyhow!(
                "Missing $FUNC meta-variable in function definition"
            ));
        };

        let params_text_env = env
            .get_multiple_matches("$$$PARAMS")
            .first()
            .map(|n| n.text().to_string())
            .unwrap_or_default();

        let params_text = if params_text_env.is_empty() {
            Self::extract_params_from_function_line(function_line)
        } else {
            params_text_env
        };

        debug!(
            "Found decorator @{}.{}() from ast-grep meta-variables",
            class_name, method_name
        );
        debug!("Found function: {}", function_name);

        // Parse decorator arguments from extracted text without regex
        let decorator_args = Self::parse_decorator_args_from_text(&args_text);

        // Parse function parameters from extracted text without regex
        let parameters = Self::parse_function_parameters_from_text(&params_text);

        // Get relative path from package root
        let source_file = file_path
            .strip_prefix(&self.package_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.to_string());

        // Infer module name from file path
        let function_module = self.infer_module_from_path(file_path);

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
            line_number: None, // ast-grep doesn't provide line numbers in this context
            decorator_args,
            function_signature: Some(function_sig),
        })
    }

    fn parse_decorator_target(decorator_line: &str) -> Option<(String, String)> {
        if let Some(stripped) = decorator_line.strip_prefix('@') {
            if let Some((class, rest)) = stripped.split_once('.') {
                if let Some((method, _)) = rest.split_once('(') {
                    return Some((class.to_string(), method.to_string()));
                }
            }
        }
        None
    }

    /// Parse decorator arguments like target_version=X, priority=30 without regex
    pub fn parse_decorator_args_from_text(args_str: &str) -> toml::Table {
        let mut table = toml::Table::new();

        // Manual parsing of key=value pairs without regex
        let mut current_arg = String::new();
        let mut depth = 0;

        for ch in args_str.chars() {
            match ch {
                '[' | '{' | '(' => {
                    depth += 1;
                    current_arg.push(ch);
                }
                ']' | '}' | ')' => {
                    depth -= 1;
                    current_arg.push(ch);
                }
                ',' if depth == 0 => {
                    if !current_arg.trim().is_empty() {
                        Self::parse_single_decorator_arg(&current_arg, &mut table);
                    }
                    current_arg.clear();
                }
                _ => current_arg.push(ch),
            }
        }

        // Don't forget the last argument
        if !current_arg.trim().is_empty() {
            Self::parse_single_decorator_arg(&current_arg, &mut table);
        }

        table
    }

    fn extract_args_from_decorator_line(line: &str) -> String {
        if let Some(start) = line.find('(') {
            if let Some(end) = line.rfind(')') {
                return line[start + 1..end].trim().to_string();
            }
        }
        String::new()
    }

    fn extract_params_from_function_line(line: &str) -> String {
        if let Some(start) = line.find('(') {
            if let Some(end) = line.find(')') {
                return line[start + 1..end].trim().to_string();
            }
        }
        String::new()
    }

    fn extract_function_name_from_line(line: &str) -> Option<String> {
        if let Some(stripped) = line.strip_prefix("def ") {
            if let Some((name, _)) = stripped.split_once('(') {
                return Some(name.trim().to_string());
            }
        }
        None
    }

    /// Parse a single decorator argument like key=value
    fn parse_single_decorator_arg(arg: &str, table: &mut toml::Table) {
        let arg = arg.trim();
        if let Some(eq_idx) = arg.find('=') {
            let key = arg[..eq_idx].trim();
            let value_str = arg[eq_idx + 1..].trim();

            // Try to parse value as different types
            let toml_value = if let Ok(num) = value_str.parse::<i64>() {
                toml::Value::Integer(num)
            } else if let Ok(float) = value_str.parse::<f64>() {
                toml::Value::Float(float)
            } else if value_str == "True" || value_str == "False" {
                toml::Value::Boolean(value_str == "True")
            } else {
                // String value - remove quotes
                let clean = value_str
                    .trim_matches(|c| c == '"' || c == '\'' || c == ' ')
                    .to_string();
                toml::Value::String(clean)
            };

            table.insert(key.to_string(), toml_value);
        }
    }

    /// Parse function parameters from signature string without regex
    pub fn parse_function_parameters_from_text(params_str: &str) -> Vec<FunctionParameter> {
        let mut params = Vec::new();

        // Split by comma, but be careful about nested brackets
        let mut current = String::new();
        let mut depth = 0;

        for ch in params_str.chars() {
            match ch {
                '[' | '{' | '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ']' | '}' | ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    if !current.trim().is_empty() {
                        if let Ok(param) = Self::parse_single_parameter_from_text(&current) {
                            params.push(param);
                        }
                    }
                    current.clear();
                }
                _ => current.push(ch),
            }
        }

        // Don't forget the last parameter
        if !current.trim().is_empty() {
            if let Ok(param) = Self::parse_single_parameter_from_text(&current) {
                params.push(param);
            }
        }

        params
    }

    /// Parse a single parameter from a parameter string without regex
    fn parse_single_parameter_from_text(param_str: &str) -> Result<FunctionParameter> {
        let param_str = param_str.trim();

        // Check for *args or **kwargs
        let is_var_arg = if param_str.starts_with("**") {
            Some(VarArgType::Kwargs)
        } else if param_str.starts_with("*") {
            Some(VarArgType::Args)
        } else {
            None
        };

        let param_str = if is_var_arg.is_some() {
            &param_str[2..] // Remove ** or *
        } else {
            param_str
        };

        // Split by colon for type annotation
        let (name, rest) = if let Some(colon_idx) = param_str.find(':') {
            (
                param_str[..colon_idx].trim().to_string(),
                Some(&param_str[colon_idx + 1..]),
            )
        } else {
            (param_str.to_string(), None)
        };

        let (param_type, default) = if let Some(rest) = rest {
            // Has type annotation
            if let Some(eq_idx) = rest.find('=') {
                let typ = rest[..eq_idx].trim().to_string();
                let def = rest[eq_idx + 1..].trim().to_string();
                (typ, Some(def))
            } else {
                (rest.trim().to_string(), None)
            }
        } else {
            // No type annotation, check for default
            if let Some(eq_idx) = param_str.find('=') {
                let typ = "Any".to_string(); // Default type when not specified
                let def = param_str[eq_idx + 1..].trim().to_string();
                (typ, Some(def))
            } else {
                ("Any".to_string(), None)
            }
        };

        Ok(FunctionParameter {
            name,
            param_type,
            default,
            is_keyword_only: false, // Would need additional logic to detect this
            is_var_arg,
        })
    }

    /// Infer module name from file path
    fn infer_module_from_path(&self, file_path: &Path) -> String {
        // Get relative path from package root
        if let Ok(rel_path) = file_path.strip_prefix(&self.package_root) {
            // Convert file path to module path
            // /path/to/package/module/submodule/file.py -> module.submodule.file
            let parts: Vec<&str> = rel_path
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect();

            if !parts.is_empty() {
                let module_parts: Vec<&str> = parts
                    .into_iter()
                    .map(|p| p.strip_suffix(".py").unwrap_or(p))
                    .collect();

                return module_parts.join(".");
            }
        }

        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_decorator_args_from_text() {
        let args_str = "target_version=LATEST_COMMIT, upgrade_type=FILE, priority=30";
        let table = DecoratorScanner::parse_decorator_args_from_text(args_str);

        assert!(table.get("priority").is_some());
        if let Some(toml::Value::Integer(30)) = table.get("priority") {
            // Good
        } else {
            panic!("priority should be 30");
        }
    }

    #[test]
    fn test_parse_function_parameters_from_text() {
        let params_str = "folder: Path, upgrader_context: dict[str, Any] | None = None";
        let params = DecoratorScanner::parse_function_parameters_from_text(params_str);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "folder");
        assert_eq!(params[0].param_type, "Path");
        assert_eq!(params[1].name, "upgrader_context");
    }

    #[test]
    fn test_parse_single_parameter_from_text() {
        let param = DecoratorScanner::parse_single_parameter_from_text("folder: Path").unwrap();
        assert_eq!(param.name, "folder");
        assert_eq!(param.param_type, "Path");
        assert_eq!(param.default, None);

        let param_with_default =
            DecoratorScanner::parse_single_parameter_from_text("timeout: int = 30").unwrap();
        assert_eq!(param_with_default.name, "timeout");
        assert_eq!(param_with_default.param_type, "int");
        assert_eq!(param_with_default.default, Some("30".to_string()));
    }

    #[test]
    fn test_infer_module_from_path() {
        let scanner = DecoratorScanner::new(PathBuf::from("/Users/dev/r2x-reeds"));
        let file_path = PathBuf::from("/Users/dev/r2x-reeds/r2x_reeds/upgrader.py");

        let module = scanner.infer_module_from_path(&file_path);
        assert_eq!(module, "r2x_reeds.upgrader");
    }

    #[test]
    fn test_ast_grep_decorator_discovery() -> Result<()> {
        use std::fs;
        use tempfile::TempDir;

        // Simplified test with just the decorated function (no class context)
        let content = r#"@ReEDSUpgrader.register_step(target_version="LATEST_COMMIT", priority=30)
def move_hmap_file(folder, upgrader_context=None):
    pass
"#;

        // Create a proper temporary directory and .py file
        let temp_dir = TempDir::new()?;
        let py_file_path = temp_dir.path().join("test_decorator.py");
        fs::write(&py_file_path, content)?;

        let scanner = DecoratorScanner::new(temp_dir.path().to_path_buf());

        let mut registrations = scanner.scan_for_decorators()?;

        if registrations.is_empty() {
            let fallback = scanner.scan_file(&py_file_path)?;
            assert!(
                !fallback.is_empty(),
                "Expected to find decorated functions via direct scan"
            );
            registrations = fallback;
        }

        // Check the first one
        if let Some(reg) = registrations.first() {
            assert_eq!(reg.decorator_class, "ReEDSUpgrader");
            assert_eq!(reg.decorator_method, "register_step");
            assert_eq!(reg.function_name, "move_hmap_file");
        }

        Ok(())
    }
}
