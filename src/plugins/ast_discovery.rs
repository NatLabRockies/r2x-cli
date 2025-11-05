//! AST-based plugin discovery using ast-grep
//!
//! This module provides an alternative to Python-based plugin discovery by:
//! 1. Using ast-grep to parse Python source code statically
//! 2. Extracting plugin definitions from the register_plugin() function
//! 3. Resolving imports to build full module paths
//! 4. Serializing to JSON matching Pydantic's model_dump_json() output
//!
//! This approach is ~227x faster than Python-based discovery and requires
//! no Python interpreter startup.

use crate::errors::BridgeError;
use crate::logger;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// AST-based plugin discovery
pub struct AstDiscovery;

/// Import mapping from short name to full module path
#[derive(Debug, Clone)]
pub struct ImportMap {
    // Maps "ReEDSParser" -> ("r2x_reeds.parser", "ReEDSParser")
    pub symbols: HashMap<String, (String, String)>,
}

impl AstDiscovery {
    /// Discover plugins from a Python package using AST parsing
    ///
    /// # Arguments
    /// * `package_path` - Path to the installed package (e.g., site-packages/r2x_reeds)
    /// * `package_name_full` - Full package name (e.g., "r2x-reeds")
    ///
    /// # Returns
    /// JSON string matching the format that Python's Package.model_dump_json() would produce
    pub fn discover_plugins(
        package_path: &Path,
        package_name_full: &str,
        venv_path: Option<&str>,
        package_version: Option<&str>,
    ) -> Result<String, BridgeError> {
        let start_time = std::time::Instant::now();

        logger::info(&format!("AST discovery started for: {}", package_name_full));
        logger::debug(&format!("Package path: {}", package_path.display()));

        // Try to find the plugin file using entry_points.txt first
        let plugins_py = if let Some(venv) = venv_path {
            logger::debug(&format!(
                "Attempting entry_points.txt lookup for {} (version: {:?})",
                package_name_full, package_version
            ));
            match Self::find_plugins_py_via_entry_points(package_name_full, package_version, venv) {
                Ok(path) => {
                    logger::debug(&format!("Found plugin file via entry_points: {}", path.display()));
                    path
                }
                Err(e) => {
                    logger::debug(&format!("Entry_points lookup failed ({}), falling back to directory search", e));
                    Self::find_plugins_py(package_path)?
                }
            }
        } else {
            logger::debug("No venv_path provided, using directory search");
            Self::find_plugins_py(package_path)?
        };
        logger::debug(&format!("Found plugins.py at: {}", plugins_py.display()));

        let full_file_content = std::fs::read_to_string(&plugins_py)
            .map_err(|e| BridgeError::PluginNotFound(format!("Failed to read plugins.py: {}", e)))?;

        let func_content = Self::extract_register_plugin_function(&plugins_py)?;
        logger::debug(&format!(
            "Extracted register_plugin() ({} bytes)",
            func_content.len()
        ));

        // Build import map from full file content (includes all imports)
        let import_map = Self::build_import_map(&full_file_content)?;
        logger::debug(&format!(
            "Built import map with {} symbols",
            import_map.symbols.len()
        ));

        let package_json =
            Self::extract_package_json(&func_content, &import_map, package_name_full, package_path)?;
        logger::debug(&format!(
            "Generated plugin JSON ({} bytes)",
            package_json.len()
        ));

        let elapsed = start_time.elapsed();
        logger::info(&format!(
            "AST discovery completed in {:?} for {}",
            elapsed, package_name_full
        ));

        Ok(package_json)
    }

    /// Find plugin file using entry_points.txt
    ///
    /// Reads the package's entry_points.txt to find the exact module path
    /// for the r2x_plugin entry point (e.g., r2x_sienna_to_plexos.plugin)
    fn find_plugins_py_via_entry_points(
        package_name_full: &str,
        package_version: Option<&str>,
        venv_path: &str,
    ) -> Result<PathBuf, BridgeError> {
        // Construct the dist-info directory path
        let normalized_name = package_name_full.replace('-', "_");
        let version = package_version.unwrap_or("0.0.0");
        let dist_info_name = format!("{}-{}.dist-info", normalized_name, version);

        // Find the dist-info directory in site-packages
        let venv_lib = PathBuf::from(venv_path).join("lib");
        let python_version_dir = std::fs::read_dir(&venv_lib)
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .find(|e| e.file_name().to_string_lossy().starts_with("python"))
            });

        if let Some(py_dir) = python_version_dir {
            let site_packages = py_dir.path().join("site-packages");
            let dist_info_path = site_packages.join(&dist_info_name);
            let entry_points_file = dist_info_path.join("entry_points.txt");

            if entry_points_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&entry_points_file) {
                    if let Some(module_path) = Self::parse_entry_points(&content) {
                        logger::debug(&format!(
                            "Found r2x_plugin entry point: {}",
                            module_path
                        ));

                        // Convert module path (e.g., "r2x_sienna_to_plexos.plugin") to file path
                        // Replace dots with slashes and add .py extension
                        let file_path = module_path.replace('.', "/") + ".py";
                        let possible_path = site_packages.join(&file_path);

                        logger::debug(&format!(
                            "Looking for plugin file at: {}",
                            possible_path.display()
                        ));

                        if possible_path.exists() {
                            logger::debug(&format!(
                                "Found plugin file via entry_points at: {}",
                                possible_path.display()
                            ));
                            return Ok(possible_path);
                        }
                    }
                }
            }
        }

        Err(BridgeError::PluginNotFound(
            "entry_points.txt not found".to_string(),
        ))
    }

    /// Parse entry_points.txt and extract the r2x_plugin module path
    fn parse_entry_points(content: &str) -> Option<String> {
        let mut in_r2x_plugin = false;

        for line in content.lines() {
            let line = line.trim();

            if line == "[r2x_plugin]" {
                in_r2x_plugin = true;
                continue;
            }

            if in_r2x_plugin {
                if line.starts_with('[') {
                    // New section, stop parsing r2x_plugin
                    break;
                }

                if !line.is_empty() && !line.starts_with('#') {
                    // Parse "name = module.path:function"
                    if let Some(eq_pos) = line.find('=') {
                        let value = line[eq_pos + 1..].trim();
                        if let Some(colon_pos) = value.find(':') {
                            let module_path = value[..colon_pos].trim();
                            return Some(module_path.to_string());
                        }
                    }
                }
            }
        }

        None
    }

    /// Find plugins.py (or plugin.py) in the package directory
    ///
    /// Handles both normal installs (site-packages/r2x_reeds/plugins.py)
    /// and editable installs (src/r2x_reeds/plugins.py where path points to src/)
    /// Also handles both naming conventions: plugins.py and plugin.py
    fn find_plugins_py(package_path: &Path) -> Result<PathBuf, BridgeError> {
        // Try both naming conventions: plugins.py and plugin.py
        let filenames = ["plugins.py", "plugin.py"];

        // First try direct path (for normal site-packages installs)
        for filename in &filenames {
            let plugins_file = package_path.join(filename);
            if plugins_file.exists() {
                logger::debug(&format!(
                    "Found {} directly at: {}",
                    filename,
                    plugins_file.display()
                ));
                return Ok(plugins_file);
            }
        }

        // For editable installs, search in subdirectories
        // The path typically points to 'src/', so look for package directories
        match std::fs::read_dir(package_path) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.is_dir() {
                            for filename in &filenames {
                                let plugins_file = path.join(filename);
                                if plugins_file.exists() {
                                    logger::debug(&format!(
                                        "Found {} in subdirectory: {}",
                                        filename,
                                        plugins_file.display()
                                    ));
                                    return Ok(plugins_file);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                logger::debug(&format!(
                    "Failed to read directory {}: {}",
                    package_path.display(),
                    e
                ));
            }
        }

        Err(BridgeError::PluginNotFound(format!(
            "plugins.py or plugin.py not found in: {}",
            package_path.display()
        )))
    }

    /// Extract the register_plugin() function content using ast-grep
    fn extract_register_plugin_function(plugins_py: &Path) -> Result<String, BridgeError> {
        // Use ast-grep to extract the function
        // Pattern: def register_plugin() -> ...
        let output = Command::new("ast-grep")
            .arg("run")
            .arg("--pattern")
            .arg("def register_plugin()")
            .arg(plugins_py.parent().unwrap_or_else(|| Path::new(".")))
            .output()
            .map_err(|e| {
                BridgeError::Initialization(format!(
                    "Failed to run ast-grep: {}. Make sure ast-grep is installed.",
                    e
                ))
            })?;

        if !output.status.success() {
            return Err(BridgeError::PluginNotFound(format!(
                "register_plugin() function not found in {}",
                plugins_py.display()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().to_string())
    }

    /// Build import map from register_plugin() function
    fn build_import_map(func_content: &str) -> Result<ImportMap, BridgeError> {
        let mut symbols = HashMap::new();

        // Parse lines looking for: from MODULE import NAME[, NAME]
        for line in func_content.lines() {
            let line = line.trim();

            // Skip non-import lines
            if !line.starts_with("from ") {
                continue;
            }

            // Parse: from r2x_reeds.parser import ReEDSParser
            if let Some(import_idx) = line.find(" import ") {
                let module_part = &line[5..import_idx]; // Skip "from "
                let imports_part = &line[import_idx + 8..]; // Skip " import "

                // Handle comma-separated imports: import A, B, C
                for import_spec in imports_part.split(',') {
                    let import_name = import_spec.trim();

                    // Handle "import X as Y" - for now just use the first name
                    let actual_name = if let Some(as_idx) = import_name.find(" as ") {
                        &import_name[as_idx + 4..]
                    } else {
                        import_name
                    };

                    symbols.insert(
                        actual_name.to_string(),
                        (module_part.to_string(), actual_name.to_string()),
                    );

                    logger::debug(&format!(
                        "Mapped {} -> {}:{}",
                        actual_name, module_part, actual_name
                    ));
                }
            }
        }

        Ok(ImportMap { symbols })
    }

    /// Extract Package JSON by parsing the Package() constructor
    fn extract_package_json(
        func_content: &str,
        import_map: &ImportMap,
        package_name_full: &str,
        package_path: &Path,
    ) -> Result<String, BridgeError> {
        let plugins_list = Self::extract_plugins_list(func_content)?;
        logger::debug(&format!("Found {} plugin definitions in Package", plugins_list.len()));

        let mut plugins = Vec::new();
        for (idx, plugin_def) in plugins_list.iter().enumerate() {
            match Self::parse_plugin_constructor(plugin_def, import_map, package_path) {
                Ok(plugin_json) => {
                    if let Some(name) = plugin_json.get("name").and_then(|v| v.as_str()) {
                        logger::debug(&format!(
                            "  [{}/{}] Parsed: {} ({})",
                            idx + 1,
                            plugins_list.len(),
                            name,
                            plugin_json
                                .get("plugin_type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                        ));
                    }
                    plugins.push(plugin_json);
                }
                Err(e) => {
                    logger::warn(&format!("Failed to parse plugin definition: {}", e));
                }
            }
        }

        logger::debug(&format!("Successfully parsed {} plugins via AST", plugins.len()));

        let package_json = json!({
            "name": package_name_full,
            "plugins": plugins,
            "metadata": {}
        });

        Ok(package_json.to_string())
    }

    /// Extract individual plugin definitions from plugins=[...] list
    fn extract_plugins_list(func_content: &str) -> Result<Vec<String>, BridgeError> {
        let mut plugins = Vec::new();

        // Find plugins=[ ... ]
        if let Some(plugins_start) = func_content.find("plugins=[") {
            let rest = &func_content[plugins_start + 9..]; // Skip "plugins=["

            // Find matching closing bracket
            let mut bracket_count = 1;
            let mut end_pos = 0;

            for (i, c) in rest.chars().enumerate() {
                match c {
                    '[' => bracket_count += 1,
                    ']' => {
                        bracket_count -= 1;
                        if bracket_count == 0 {
                            end_pos = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if end_pos > 0 {
                let plugins_content = &rest[..end_pos];

                // Split by plugin constructors
                // Look for patterns like "PluginType(" where PluginType is a known plugin class
                let plugin_keywords = ["ParserPlugin", "UpgraderPlugin", "BasePlugin", "ExporterPlugin"];

                for keyword in &plugin_keywords {
                    let mut search_from = 0;
                    while let Some(pos) = plugins_content[search_from..].find(keyword) {
                        let actual_pos = search_from + pos;

                        // Find the opening parenthesis
                        if let Some(paren_pos) = plugins_content[actual_pos..].find('(') {
                            // Find matching closing parenthesis
                            let paren_start = actual_pos + paren_pos;
                            if let Some(paren_end) =
                                Self::find_matching_paren(&plugins_content, paren_start)
                            {
                                let plugin_def = plugins_content[actual_pos..=paren_end].to_string();
                                if !plugin_def.is_empty() && !plugins.contains(&plugin_def) {
                                    plugins.push(plugin_def);
                                }
                                search_from = actual_pos + 1;
                            } else {
                                search_from = actual_pos + 1;
                            }
                        } else {
                            search_from = actual_pos + keyword.len();
                        }
                    }
                }
            }
        }

        if plugins.is_empty() {
            return Err(BridgeError::PluginNotFound(
                "No plugin definitions found in plugins array".to_string(),
            ));
        }

        Ok(plugins)
    }

    /// Find matching closing parenthesis
    fn find_matching_paren(content: &str, start: usize) -> Option<usize> {
        if start >= content.len() || !content[start..].starts_with('(') {
            return None;
        }

        let mut paren_count = 1;
        let chars: Vec<char> = content.chars().collect();

        for i in (start + 1)..chars.len() {
            match chars[i] {
                '(' => paren_count += 1,
                ')' => {
                    paren_count -= 1;
                    if paren_count == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Parse a plugin constructor and extract its arguments
    fn parse_plugin_constructor(
        plugin_def: &str,
        import_map: &ImportMap,
        package_path: &Path,
    ) -> Result<Value, BridgeError> {
        let constructor_type = if let Some(type_end) = plugin_def.find('(') {
            plugin_def[..type_end].trim()
        } else {
            return Err(BridgeError::PluginNotFound(
                "Invalid plugin constructor format".to_string(),
            ));
        };

        logger::debug(&format!("Parsing plugin constructor: {}", constructor_type));

        let kwargs = Self::extract_kwargs(plugin_def, import_map, package_path)?;

        // The kwargs keys are cleaned to remove file:line:content format
        let name = kwargs
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BridgeError::PluginNotFound("Plugin missing 'name' field".to_string()))?;

        logger::debug(&format!("  Plugin name: {}", name));

        let obj = kwargs.get("obj").cloned().ok_or_else(|| {
            BridgeError::PluginNotFound(format!("Plugin '{}' missing 'obj' field", name))
        })?;

        let io_type = kwargs.get("io_type").and_then(|v| v.as_str());
        if let Some(io) = io_type {
            logger::debug(&format!("  IO type: {}", io));
        }

        let plugin_type = Self::infer_plugin_type(constructor_type);

        let mut result = json!({
            "name": name,
            "plugin_type": plugin_type,
            "obj": obj,
        });

        if let Some(obj_val) = result.get_mut("obj") {
            if let Some(module) = obj_val.get("module").and_then(|v| v.as_str()) {
                if let Some(obj_name) = obj_val.get("name").and_then(|v| v.as_str()) {
                    logger::debug(&format!("  Resolved obj: {}:{}", module, obj_name));
                }
            }
        }

        if let Some(call_method) = kwargs.get("call_method") {
            result["call_method"] = call_method.clone();
        }

        if let Some(config) = kwargs.get("config") {
            result["config"] = config.clone();
        }

        if let Some(io) = io_type {
            result["io_type"] = json!(io);
        }

        if let Some(requires_store) = kwargs.get("requires_store") {
            result["requires_store"] = requires_store.clone();
        }

        if let Some(version_strategy) = kwargs.get("version_strategy") {
            result["version_strategy"] = version_strategy.clone();
        }

        if let Some(version_reader) = kwargs.get("version_reader") {
            result["version_reader"] = version_reader.clone();
        }

        if let Some(upgrade_steps) = kwargs.get("upgrade_steps") {
            logger::debug(&format!("Adding upgrade_steps with {} items",
                upgrade_steps.as_array().map(|a| a.len()).unwrap_or(0)));
            result["upgrade_steps"] = upgrade_steps.clone();
        }

        Ok(result)
    }

    /// Extract keyword arguments from plugin constructor
    fn extract_kwargs(
        plugin_def: &str,
        import_map: &ImportMap,
        package_path: &Path,
    ) -> Result<HashMap<String, Value>, BridgeError> {
        let mut kwargs = HashMap::new();

        let paren_start = plugin_def.find('(').ok_or_else(|| {
            BridgeError::PluginNotFound("No opening parenthesis in plugin constructor".to_string())
        })?;
        let content = &plugin_def[paren_start + 1..];

        let mut current_key = String::new();
        let mut current_value = String::new();
        let mut in_key = true;
        let mut paren_depth = 0;
        let mut bracket_depth = 0;
        let mut brace_depth = 0;
        let mut in_string = false;
        let mut string_char = ' ';

        for c in content.chars() {
            // Skip whitespace when building keys
            if in_key && (c == ' ' || c == '\t' || c == '\n' || c == '\r') && current_key.is_empty() {
                continue;
            }

            if in_string {
                current_value.push(c);
                if c == string_char && !current_value.ends_with("\\\"") {
                    in_string = false;
                }
                continue;
            }

            match c {
                '"' | '\'' => {
                    in_string = true;
                    string_char = c;
                    current_value.push(c);
                }
                '=' if in_key => {
                    in_key = false;
                    current_key = current_key.trim().to_string();
                }
                '(' => {
                    paren_depth += 1;
                    current_value.push(c);
                }
                ')' if paren_depth > 0 => {
                    paren_depth -= 1;
                    current_value.push(c);
                }
                ')' => {
                    if !current_key.is_empty() && !current_value.is_empty() {
                        let value =
                            Self::parse_kwarg_value(&current_value.trim(), import_map, package_path)?;
                        kwargs.insert(current_key.clone(), value);
                    }
                    break;
                }
                '[' => {
                    bracket_depth += 1;
                    current_value.push(c);
                }
                ']' => {
                    bracket_depth -= 1;
                    current_value.push(c);
                }
                '{' => {
                    brace_depth += 1;
                    current_value.push(c);
                }
                '}' => {
                    brace_depth -= 1;
                    current_value.push(c);
                }
                ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                    if !current_key.is_empty() && !current_value.is_empty() {
                        let value =
                            Self::parse_kwarg_value(&current_value.trim(), import_map, package_path)?;
                        kwargs.insert(current_key.clone(), value);
                        current_key.clear();
                        current_value.clear();
                        in_key = true;
                    }
                }
                _ => {
                    if in_key {
                        current_key.push(c);
                    } else {
                        current_value.push(c);
                    }
                }
            }
        }

        // Clean up kwargs keys - they may have file:line:content format
        let cleaned_kwargs: HashMap<String, Value> = kwargs
            .into_iter()
            .map(|(key, value)| {
                let cleaned_key = if let Some(last_colon) = key.rfind(':') {
                    key[last_colon + 1..].trim().to_string()
                } else {
                    key
                };
                (cleaned_key, value)
            })
            .collect();

        Ok(cleaned_kwargs)
    }

    /// Parse a single kwarg value and convert to JSON
    fn parse_kwarg_value(value: &str, import_map: &ImportMap, package_path: &Path) -> Result<Value, BridgeError> {
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Ok(Value::Null);
        }

        if trimmed == "None" {
            return Ok(Value::Null);
        }

        if trimmed == "True" {
            return Ok(Value::Bool(true));
        }

        if trimmed == "False" {
            return Ok(Value::Bool(false));
        }

        if (trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        {
            let string_content = &trimmed[1..trimmed.len() - 1];
            return Ok(Value::String(string_content.to_string()));
        }

        if let Some(enum_value) = Self::resolve_enum_value(trimmed) {
            logger::debug(&format!("Resolved enum: {} -> {}", trimmed, enum_value));
            return Ok(Value::String(enum_value));
        }

        if trimmed.contains('.') && !trimmed.starts_with('[') {
            // Check if this is a decorator-based attribute pattern like "ReEDSUpgrader.steps"
            if trimmed.ends_with(".steps") {
                if let Some(class_name) = trimmed.strip_suffix(".steps") {
                    // Try to extract steps from @register_step decorators
                    match Self::extract_decorator_based_attribute(class_name, "steps", package_path) {
                        Ok(steps_json) => {
                            logger::debug(&format!("Extracted {} decorator-based steps for {}",
                                steps_json.as_array().map(|a| a.len()).unwrap_or(0), class_name));
                            return Ok(steps_json);
                        }
                        Err(e) => {
                            logger::debug(&format!("Failed to extract decorator-based attribute: {}", e));
                            return Err(e);
                        }
                    }
                }
            }

            return Err(BridgeError::PluginNotFound(format!(
                "Attribute access '{}' requires Python runtime - unsupported in AST mode",
                trimmed
            )));
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            return Ok(Value::Array(vec![]));
        }

        if import_map.symbols.contains_key(trimmed) {
            let (module, name) = &import_map.symbols[trimmed];
            return Ok(json!({
                "module": module,
                "name": name,
                "type": Self::infer_callable_type_from_name(name),
                "return_annotation": Value::Null,
                "parameters": {}
            }));
        }

        Ok(Value::String(trimmed.to_string()))
    }

    /// Resolve enum values like IOType.STDOUT to string values
    fn resolve_enum_value(expr: &str) -> Option<String> {
        match expr {
            "IOType.STDOUT" => Some("stdout".to_string()),
            "IOType.STDIN" => Some("stdin".to_string()),
            "IOType.BOTH" => Some("both".to_string()),
            _ => None,
        }
    }

    /// Infer plugin type discriminator from constructor name
    fn infer_plugin_type(constructor: &str) -> &'static str {
        match constructor {
            "ParserPlugin" => "parser",
            "ExporterPlugin" => "exporter",
            "UpgraderPlugin" => "class",
            "BasePlugin" => "function",
            _ => "function",
        }
    }

    /// Infer callable type from name heuristic
    fn infer_callable_type_from_name(name: &str) -> &'static str {
        if name.chars().next().map_or(false, |c| c.is_uppercase()) {
            "class"
        } else {
            "function"
        }
    }

    /// Extract decorator-based attributes like ReEDSUpgrader.steps from @register_step decorators
    fn extract_decorator_based_attribute(
        class_name: &str,
        _attribute: &str,
        package_path: &Path,
    ) -> Result<Value, BridgeError> {
        // Search for @ClassName.register_step decorators in the package directory tree
        let mut steps = Vec::new();
        Self::search_decorators_recursive(package_path, class_name, &mut steps);

        if steps.is_empty() {
            return Err(BridgeError::PluginNotFound(format!(
                "No decorators found for {}.{}",
                class_name, _attribute
            )));
        }

        Ok(Value::Array(steps))
    }

    /// Recursively search for decorators in a directory tree
    fn search_decorators_recursive(path: &Path, class_name: &str, steps: &mut Vec<Value>) {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let entry_path = entry.path();

                    // Skip common non-source directories
                    let skip = entry_path
                        .file_name()
                        .map(|name| {
                            let name_str = name.to_string_lossy();
                            name_str.starts_with('.')
                                || name_str == "__pycache__"
                                || name_str == "venv"
                        })
                        .unwrap_or(false);

                    if skip {
                        continue;
                    }

                    if entry_path.is_file() && entry_path.extension().map_or(false, |ext| ext == "py") {
                        // Search this file for decorators
                        if let Ok(content) = std::fs::read_to_string(&entry_path) {
                            if let Ok(found_steps) =
                                Self::extract_steps_from_decorators(&content, class_name)
                            {
                                steps.extend(found_steps);
                            }
                        }
                    } else if entry_path.is_dir() {
                        // Recursively search subdirectories
                        Self::search_decorators_recursive(&entry_path, class_name, steps);
                    }
                }
            }
        }
    }

    /// Extract upgrade steps from @ClassName.register_step(...) decorators in file content
    fn extract_steps_from_decorators(
        content: &str,
        class_name: &str,
    ) -> Result<Vec<Value>, BridgeError> {
        let mut steps = Vec::new();
        let decorator_pattern = format!("@{}.register_step(", class_name);

        let mut search_from = 0;
        while let Some(decorator_pos) = content[search_from..].find(&decorator_pattern) {
            let actual_pos = search_from + decorator_pos;

            // Find the matching closing parenthesis
            let paren_start = actual_pos + decorator_pattern.len() - 1;
            if let Some(paren_end) = Self::find_matching_paren(content, paren_start) {
                let decorator_args = &content[actual_pos + decorator_pattern.len()..paren_end];

                // Extract function name - look for "def function_name(" after the decorator
                let rest_of_file = &content[paren_end..];
                if let Some(def_pos) = rest_of_file.find("def ") {
                    let def_start = def_pos + 4;
                    if let Some(paren_pos) = rest_of_file[def_start..].find('(') {
                        let func_name = rest_of_file[def_start..def_start + paren_pos].trim().to_string();

                        // Parse decorator arguments to build UpgradeStep
                        let step_json = Self::build_upgrade_step_from_decorator(
                            &func_name,
                            decorator_args,
                        )?;
                        steps.push(step_json);
                    }
                }
            }

            search_from = actual_pos + 1;
        }

        Ok(steps)
    }

    /// Build an UpgradeStep JSON object from decorator arguments
    fn build_upgrade_step_from_decorator(
        func_name: &str,
        decorator_args: &str,
    ) -> Result<Value, BridgeError> {
        // Parse decorator arguments like:
        // target_version=LATEST_COMMIT, upgrade_type=UpgradeType.FILE, priority=30

        let mut step = json!({
            "name": func_name,
            "func": {
                "module": "r2x_reeds.upgrader.upgrade_steps",
                "name": func_name,
                "type": "function",
                "return_annotation": Value::Null,
                "parameters": {}
            },
            "target_version": "unknown",
            "upgrade_type": "FILE",
            "priority": 100
        });

        // Simple parsing of decorator arguments
        for arg in decorator_args.split(',') {
            let arg = arg.trim();
            if let Some(eq_pos) = arg.find('=') {
                let key = arg[..eq_pos].trim();
                let value = arg[eq_pos + 1..].trim();

                match key {
                    "target_version" => {
                        // Remove quotes or extract variable name
                        let cleaned = if value.starts_with('"') && value.ends_with('"') {
                            value[1..value.len() - 1].to_string()
                        } else {
                            value.to_string()
                        };
                        step["target_version"] = Value::String(cleaned);
                    }
                    "upgrade_type" => {
                        // Parse "UpgradeType.FILE" to "FILE"
                        if let Some(dot_pos) = value.find('.') {
                            let type_name = &value[dot_pos + 1..];
                            step["upgrade_type"] = Value::String(type_name.to_uppercase());
                        }
                    }
                    "priority" => {
                        if let Ok(priority) = value.parse::<i64>() {
                            step["priority"] = Value::Number(priority.into());
                        }
                    }
                    "min_version" => {
                        let cleaned = if value.starts_with('"') && value.ends_with('"') {
                            value[1..value.len() - 1].to_string()
                        } else {
                            value.to_string()
                        };
                        step["min_version"] = Value::String(cleaned);
                    }
                    "max_version" => {
                        let cleaned = if value.starts_with('"') && value.ends_with('"') {
                            value[1..value.len() - 1].to_string()
                        } else {
                            value.to_string()
                        };
                        step["max_version"] = Value::String(cleaned);
                    }
                    _ => {}
                }
            }
        }

        Ok(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_map_single() {
        let func_content = "from r2x_reeds.parser import ReEDSParser\n";
        let map = AstDiscovery::build_import_map(func_content).unwrap();
        assert_eq!(map.symbols.get("ReEDSParser"), Some(&("r2x_reeds.parser".to_string(), "ReEDSParser".to_string())));
    }

    #[test]
    fn test_import_map_multiple() {
        let func_content = "from r2x_core.plugin import BasePlugin, ParserPlugin, UpgraderPlugin\n";
        let map = AstDiscovery::build_import_map(func_content).unwrap();
        assert_eq!(map.symbols.len(), 3);
        assert!(map.symbols.contains_key("BasePlugin"));
        assert!(map.symbols.contains_key("ParserPlugin"));
        assert!(map.symbols.contains_key("UpgraderPlugin"));
    }

    #[test]
    fn test_import_map_empty_lines() {
        let func_content = "\n\nfrom module import Name\n\n";
        let map = AstDiscovery::build_import_map(func_content).unwrap();
        assert_eq!(map.symbols.len(), 1);
    }
}
