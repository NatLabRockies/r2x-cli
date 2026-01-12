use anyhow::{anyhow, Result};
use ast_grep_core::AstGrep;
use ast_grep_language::Python;
use r2x_manifest::{
    ArgumentSpec, ConfigField, ConfigSpec, IOContract, IOSlot, ImplementationType, InvocationSpec,
    PluginKind, PluginSpec, ResourceSpec, StoreMode, StoreSpec,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info};

mod args;
#[allow(dead_code)]
mod parameters;

#[cfg(test)]
mod tests;

pub struct PluginExtractor {
    pub(crate) python_file_path: PathBuf,
    pub(crate) package_root: PathBuf,
    pub(crate) package_prefix: String,
    pub(crate) content: String,
    pub(crate) import_map: HashMap<String, String>,
    pub(crate) current_module: String,
}

impl PluginExtractor {
    pub fn new(
        python_file_path: PathBuf,
        module_path: String,
        package_root: PathBuf,
    ) -> Result<Self> {
        debug!("Initializing plugin extractor for: {:?}", python_file_path);

        let content = fs::read_to_string(&python_file_path)?;
        let package_prefix = module_path.split('.').next().unwrap_or("").to_string();
        let import_map = Self::build_import_map_static(&content);

        Ok(PluginExtractor {
            python_file_path,
            package_root,
            package_prefix,
            content,
            import_map,
            current_module: module_path,
        })
    }

    pub fn extract_plugins(&self) -> Result<Vec<PluginSpec>> {
        debug!(
            "Extracting plugins via AST parsing from: {:?}",
            self.python_file_path
        );

        let sg = AstGrep::new(&self.content, Python);
        let root = sg.root();

        let manifest_add_calls: Vec<_> = root.find_all("manifest.add($$$_)").collect();

        if !manifest_add_calls.is_empty() {
            debug!("Found {} manifest.add() calls", manifest_add_calls.len());
            let mut plugins = Vec::new();

            for add_match in manifest_add_calls {
                let add_text = add_match.text();
                match self.extract_plugin_from_add_call(add_text.as_ref()) {
                    Ok(plugin) => {
                        debug!("Extracted plugin: {}", plugin.name);
                        plugins.push(plugin);
                    }
                    Err(err) => {
                        debug!(
                            "Failed to parse manifest.add() call '{}': {}",
                            add_text.lines().next().unwrap_or(""),
                            err
                        );
                    }
                }
            }

            info!(
                "Extracted {} plugins from manifest.add() helpers",
                plugins.len()
            );
            return Ok(plugins);
        }

        let constructor_plugins = self.extract_plugins_from_constructor_calls()?;
        if !constructor_plugins.is_empty() {
            info!(
                "Extracted {} plugins from Package-based constructors",
                constructor_plugins.len()
            );
            return Ok(constructor_plugins);
        }

        Err(anyhow!(
            "No manifest.add() helpers or plugin constructors found"
        ))
    }

    fn extract_plugin_from_add_call(&self, add_text: &str) -> Result<PluginSpec> {
        debug!(
            "Parsing PluginSpec from manifest.add(): {}",
            add_text.lines().next().unwrap_or("")
        );

        let sg = AstGrep::new(add_text, Python);
        let root = sg.root();

        let plugin_spec_calls: Vec<_> = root.find_all("PluginSpec.$METHOD($$$ARGS)").collect();

        if plugin_spec_calls.is_empty() {
            return Err(anyhow!("No PluginSpec helper call found in manifest.add()"));
        }

        let spec_match = &plugin_spec_calls[0];
        let method = spec_match
            .get_node()
            .field("function")
            .and_then(|func| func.field("attribute"))
            .map(|attr| attr.text().to_string())
            .ok_or_else(|| anyhow!("Missing helper method"))?;

        let kind = match method.as_str() {
            "parser" => PluginKind::Parser,
            "exporter" => PluginKind::Exporter,
            "function" => PluginKind::Modifier,
            "upgrader" => PluginKind::Upgrader,
            "utility" => PluginKind::Utility,
            "translation" => PluginKind::Translation,
            _ => return Err(anyhow!("Unknown PluginSpec helper method: {}", method)),
        };

        debug!("Detected plugin kind: {:?}", kind);

        let call_text = spec_match.text();
        let kwargs = self.extract_keyword_arguments_from_text(call_text.as_ref())?;

        let name = self.find_kwarg_value(&kwargs, "name")?;
        let entry_value = self.find_kwarg_value(&kwargs, "entry")?;
        let entry = self.qualify_symbol(&entry_value);

        // Infer implementation type from the entry point
        let implementation = Self::infer_invocation_type(&entry);

        let (constructor_args, call_args) = match implementation {
            ImplementationType::Class => {
                let constructor = self.resolve_entry_parameters(&entry, &ImplementationType::Class);
                (constructor, Vec::new())
            }
            ImplementationType::Function => {
                let call = self.resolve_entry_parameters(&entry, &ImplementationType::Function);
                (Vec::new(), call)
            }
        };

        let description = self.find_optional_kwarg_by_role(&kwargs, args::KwArgRole::Description);

        let method_param = self.find_optional_kwarg_by_role(&kwargs, args::KwArgRole::Method);
        let resolved_method = method_param.or_else(|| Self::default_method_for_kind(&kind));

        let invocation = InvocationSpec {
            implementation,
            method: resolved_method,
            constructor: constructor_args,
            call: call_args,
        };

        let io = self.infer_io_contract(&kind);

        let resources = self.extract_resources(&kwargs);

        Ok(PluginSpec {
            name,
            kind,
            entry,
            invocation,
            io,
            resources,
            upgrade: None,
            description,
            tags: Vec::new(),
        })
    }

    fn extract_plugins_from_constructor_calls(&self) -> Result<Vec<PluginSpec>> {
        let sg = AstGrep::new(&self.content, Python);
        let root = sg.root();
        let mut plugins = Vec::new();

        for plugin_match in root.find_all("$PLUGIN($$$ARGS)") {
            let env = plugin_match.get_env();
            let Some(callee) = env.get_match("PLUGIN") else {
                continue;
            };
            let callee_text = callee.text();
            if !Self::looks_like_plugin_constructor(callee_text.as_ref()) {
                continue;
            }

            let constructor_name = callee_text.to_string();
            let call_text = plugin_match.text();
            match self.build_plugin_from_constructor(&constructor_name, call_text.as_ref()) {
                Ok(plugin) => {
                    debug!(
                        "Extracted plugin '{}' via constructor {}",
                        plugin.name, constructor_name
                    );
                    plugins.push(plugin);
                }
                Err(err) => {
                    debug!(
                        "Failed to parse constructor '{}' at '{}': {}",
                        constructor_name,
                        call_text.lines().next().unwrap_or_default(),
                        err
                    );
                }
            }
        }

        Ok(plugins)
    }

    fn build_plugin_from_constructor(
        &self,
        constructor: &str,
        call_text: &str,
    ) -> Result<PluginSpec> {
        let kwargs = self.extract_keyword_arguments_from_text(call_text)?;
        let name = self.find_kwarg_by_role(&kwargs, args::KwArgRole::Name)?;
        let entry = self.find_entry_reference(&kwargs)?;
        let description = self.find_optional_kwarg_by_role(&kwargs, args::KwArgRole::Description);
        let method_param = self.find_optional_kwarg_by_role(&kwargs, args::KwArgRole::Method);

        // Infer implementation type from the entry point
        let implementation = Self::infer_invocation_type(&entry);

        let (constructor_args, call_args) = match implementation {
            ImplementationType::Class => {
                let constructor = self.resolve_entry_parameters(&entry, &ImplementationType::Class);
                (constructor, Vec::new())
            }
            ImplementationType::Function => {
                let call = self.resolve_entry_parameters(&entry, &ImplementationType::Function);
                (Vec::new(), call)
            }
        };

        let kind = self.infer_kind_from_constructor(constructor);
        let resolved_method = method_param.or_else(|| Self::default_method_for_kind(&kind));

        let invocation = InvocationSpec {
            implementation,
            method: resolved_method,
            constructor: constructor_args,
            call: call_args,
        };

        let io = self.infer_io_contract(&kind);
        let resources = self.extract_resources(&kwargs);

        Ok(PluginSpec {
            name,
            kind,
            entry,
            invocation,
            io,
            resources,
            upgrade: None,
            description,
            tags: Vec::new(),
        })
    }

    fn infer_invocation_type(entry: &str) -> ImplementationType {
        let ident = entry.rsplit('.').next().unwrap_or(entry);
        if ident
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
        {
            ImplementationType::Class
        } else {
            ImplementationType::Function
        }
    }

    fn find_entry_reference(&self, kwargs: &[args::KwArg]) -> Result<String> {
        let symbol = self.find_kwarg_by_role(kwargs, args::KwArgRole::EntryReference)?;
        Ok(self.qualify_symbol(&symbol))
    }

    fn find_kwarg_by_role(&self, kwargs: &[args::KwArg], role: args::KwArgRole) -> Result<String> {
        kwargs
            .iter()
            .find(|kw| kw.role == role)
            .map(|kw| kw.value.clone())
            .ok_or_else(|| anyhow!("Argument with role {:?} not found", role))
    }

    fn find_optional_kwarg_by_role(
        &self,
        kwargs: &[args::KwArg],
        role: args::KwArgRole,
    ) -> Option<String> {
        kwargs
            .iter()
            .find(|kw| kw.role == role)
            .map(|kw| kw.value.clone())
    }

    fn resolve_entry_parameters(
        &self,
        entry: &str,
        implementation: &ImplementationType,
    ) -> Vec<ArgumentSpec> {
        let (module_path, symbol) = match Self::split_entry(entry) {
            Some(parts) => parts,
            None => {
                debug!("Failed to split entry point: {}", entry);
                return Vec::new();
            }
        };

        // Try to resolve parameters, following imports if necessary
        let entries = self.resolve_parameters_with_import_follow(
            &module_path,
            &symbol,
            implementation,
            0, // depth counter to prevent infinite loops
        );

        entries
            .into_iter()
            .map(|param| ArgumentSpec {
                name: param.name,
                annotation: param.annotation,
                default: param.default,
                required: param.is_required,
            })
            .collect()
    }

    /// Resolve parameters for a symbol, following imports if the symbol is re-exported
    fn resolve_parameters_with_import_follow(
        &self,
        module_path: &str,
        symbol: &str,
        implementation: &ImplementationType,
        depth: usize,
    ) -> Vec<parameters::ParameterEntry> {
        const MAX_IMPORT_DEPTH: usize = 5;

        if depth > MAX_IMPORT_DEPTH {
            debug!(
                "Max import depth reached while resolving '{}' in '{}'",
                symbol, module_path
            );
            return Vec::new();
        }

        let source = match self.load_module_source(module_path) {
            Some(src) => src,
            None => {
                debug!(
                    "Unable to load module '{}' while resolving parameters for '{}'",
                    module_path, symbol
                );
                return Vec::new();
            }
        };

        // First, try to extract parameters directly from this module
        let result = match implementation {
            ImplementationType::Class => {
                self.extract_class_parameters_from_content(&source, symbol)
            }
            ImplementationType::Function => {
                self.extract_function_parameters_from_content(&source, symbol)
            }
        };

        match result {
            Ok(entries) => entries,
            Err(_) => {
                // Symbol not found directly - check if it's imported from another module
                debug!(
                    "Symbol '{}' not found in '{}', checking for imports",
                    symbol, module_path
                );

                if let Some(import_source) =
                    self.find_import_source_module(&source, symbol, module_path)
                {
                    debug!(
                        "Found import: '{}' is imported from '{}'",
                        symbol, import_source
                    );
                    // Recursively resolve from the source module
                    self.resolve_parameters_with_import_follow(
                        &import_source,
                        symbol,
                        implementation,
                        depth + 1,
                    )
                } else {
                    debug!(
                        "No import found for '{}' in '{}', returning empty",
                        symbol, module_path
                    );
                    Vec::new()
                }
            }
        }
    }

    /// Find the source module for an imported symbol
    /// Handles patterns like:
    /// - `from .parser import ReEDSParser`
    /// - `from .submodule.parser import ReEDSParser`
    /// - `from r2x_reeds.parser import ReEDSParser`
    fn find_import_source_module(
        &self,
        content: &str,
        symbol: &str,
        current_module: &str,
    ) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();

            // Match: from X import Y or from X import Y, Z, ...
            if let Some(rest) = trimmed.strip_prefix("from ") {
                if let Some(import_idx) = rest.find(" import ") {
                    let from_part = rest[..import_idx].trim();
                    let import_part = rest[import_idx + 8..].trim();

                    // Check if our symbol is in the import list
                    let imports: Vec<&str> = import_part
                        .split(',')
                        .map(|s| {
                            // Handle "X as Y" - we want X
                            s.trim().split(" as ").next().unwrap_or(s.trim()).trim()
                        })
                        .collect();

                    if imports.contains(&symbol) {
                        // Resolve the module path
                        return Some(self.resolve_relative_import(from_part, current_module));
                    }
                }
            }
        }
        None
    }

    /// Resolve a relative import path to an absolute module path
    /// - `.parser` in module `r2x_reeds` -> `r2x_reeds.parser`
    /// - `..utils` in module `r2x_reeds.sub` -> `r2x_reeds.utils`
    /// - `r2x_reeds.parser` -> `r2x_reeds.parser` (already absolute)
    fn resolve_relative_import(&self, from_part: &str, current_module: &str) -> String {
        if !from_part.starts_with('.') {
            // Absolute import
            return from_part.to_string();
        }

        // Count leading dots for relative import level
        let dot_count = from_part.chars().take_while(|&c| c == '.').count();
        let relative_path = &from_part[dot_count..];

        // Split current module into parts
        let mut parts: Vec<&str> = current_module.split('.').collect();

        // Go up `dot_count - 1` levels (one dot means same package)
        for _ in 0..(dot_count.saturating_sub(1)) {
            parts.pop();
        }

        // If we have a relative path, append it
        if !relative_path.is_empty() {
            // Replace the last part or append
            if dot_count == 1 {
                // Single dot - same package, replace __init__ reference
                // e.g., `.parser` in `r2x_reeds` -> `r2x_reeds.parser`
                parts.push(relative_path);
            } else {
                parts.push(relative_path);
            }
        }

        parts.join(".")
    }

    fn split_entry(entry: &str) -> Option<(String, String)> {
        if let Some(idx) = entry.rfind('.') {
            let module = entry[..idx].to_string();
            let symbol = entry[idx + 1..].to_string();
            Some((module, symbol))
        } else {
            None
        }
    }

    fn infer_kind_from_constructor(&self, constructor: &str) -> PluginKind {
        let lowered = constructor
            .rsplit('.')
            .next()
            .unwrap_or(constructor)
            .to_lowercase();
        if lowered.contains("parser") {
            PluginKind::Parser
        } else if lowered.contains("export") {
            PluginKind::Exporter
        } else if lowered.contains("upgrade") {
            PluginKind::Upgrader
        } else if lowered.contains("modif") {
            PluginKind::Modifier
        } else {
            PluginKind::Utility
        }
    }

    fn default_method_for_kind(kind: &PluginKind) -> Option<String> {
        match kind {
            PluginKind::Parser => Some("build_system".to_string()),
            PluginKind::Exporter => Some("export".to_string()),
            PluginKind::Translation => Some("run".to_string()),
            _ => None,
        }
    }

    fn looks_like_plugin_constructor(callee: &str) -> bool {
        callee
            .rsplit('.')
            .next()
            .map(|segment| segment.trim().ends_with("Plugin"))
            .unwrap_or(false)
    }

    fn infer_io_contract(&self, kind: &PluginKind) -> IOContract {
        match kind {
            PluginKind::Parser => IOContract {
                consumes: vec![IOSlot::StoreFolder, IOSlot::ConfigFile],
                produces: vec![IOSlot::System],
            },
            PluginKind::Exporter => IOContract {
                consumes: vec![IOSlot::System, IOSlot::ConfigFile],
                produces: vec![IOSlot::Folder],
            },
            PluginKind::Modifier => IOContract {
                consumes: vec![IOSlot::System],
                produces: vec![IOSlot::System],
            },
            _ => IOContract {
                consumes: Vec::new(),
                produces: Vec::new(),
            },
        }
    }

    fn extract_resources(&self, kwargs: &[args::KwArg]) -> Option<ResourceSpec> {
        let config = kwargs
            .iter()
            .find(|arg| arg.role == args::KwArgRole::Config)
            .map(|arg| {
                let config_class = arg.value.trim().to_string();
                let module = self
                    .import_map
                    .get(&config_class)
                    .map(|m| self.normalize_module_path(m))
                    .unwrap_or_else(|| self.current_module.clone());
                let fields = self.extract_config_fields(&module, &config_class);

                ConfigSpec {
                    module,
                    name: config_class,
                    fields,
                }
            });

        let store = kwargs
            .iter()
            .find(|arg| arg.role == args::KwArgRole::Store)
            .map(|arg| {
                let value = arg.value.trim();
                if value == "True" || value == "true" {
                    StoreSpec {
                        mode: StoreMode::Folder,
                        path: None,
                    }
                } else {
                    StoreSpec {
                        mode: StoreMode::Folder,
                        path: Some(value.trim_matches('"').to_string()),
                    }
                }
            });

        if config.is_some() || store.is_some() {
            Some(ResourceSpec { store, config })
        } else {
            None
        }
    }

    fn build_import_map_static(content: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }

            if line.starts_with("from ") && line.contains(" import ") {
                if let Some(import_idx) = line.find(" import ") {
                    let module = line[5..import_idx].trim();
                    let imports_part = line[import_idx + 8..].trim();

                    for import_item in imports_part.split(',') {
                        let import_item = import_item.trim();
                        if import_item.ends_with('\\') || import_item.is_empty() {
                            continue;
                        }

                        let class_name = if let Some(as_idx) = import_item.find(" as ") {
                            import_item[as_idx + 4..].trim()
                        } else {
                            import_item
                        };

                        let class_name = class_name
                            .trim_matches(|c| c == '(' || c == ')' || c == ',')
                            .trim();

                        if !class_name.is_empty() && !class_name.starts_with('#') {
                            map.insert(class_name.to_string(), module.to_string());
                            debug!("Mapped class {} to module {}", class_name, module);
                        }
                    }
                }
            }
        }

        debug!("Built import map with {} entries", map.len());
        map
    }

    pub fn resolve_references(
        &self,
        _plugin: &mut PluginSpec,
        _package_root: &std::path::Path,
        _package_name: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn qualify_symbol(&self, symbol: &str) -> String {
        if symbol.contains('.') || self.current_module.is_empty() {
            return symbol.to_string();
        }

        let module = self
            .import_map
            .get(symbol)
            .map(|m| self.normalize_module_path(m))
            .unwrap_or_else(|| self.current_module.clone());

        if module.is_empty() {
            symbol.to_string()
        } else {
            format!("{}.{}", module, symbol)
        }
    }

    fn normalize_module_path(&self, module: &str) -> String {
        if module.starts_with('.') {
            return self.resolve_relative_module(module);
        }
        module.to_string()
    }

    fn resolve_relative_module(&self, module: &str) -> String {
        let mut base_parts: Vec<&str> = if self.current_module.is_empty() {
            Vec::new()
        } else {
            self.current_module.split('.').collect()
        };

        let bytes = module.as_bytes();
        let mut idx = 0usize;
        while idx < bytes.len() && bytes[idx] == b'.' {
            if !base_parts.is_empty() {
                base_parts.pop();
            }
            idx += 1;
        }

        let remainder = module[idx..].trim_matches('.');
        if !remainder.is_empty() {
            for part in remainder.split('.') {
                if !part.is_empty() {
                    base_parts.push(part);
                }
            }
        }

        base_parts.join(".")
    }

    fn resolve_module_file(&self, module: &str) -> Option<PathBuf> {
        let module = if module.is_empty() {
            self.current_module.clone()
        } else {
            module.to_string()
        };

        let mut parts: Vec<&str> = module.split('.').collect();
        if !self.package_prefix.is_empty() && parts.first() == Some(&self.package_prefix.as_str()) {
            parts.remove(0);
        }

        let mut path = self.package_root.clone();
        if parts.is_empty() {
            path.push("__init__.py");
            return Some(path);
        }

        for part in &parts {
            path.push(part);
        }
        path.set_extension("py");
        Some(path)
    }

    fn load_module_source(&self, module: &str) -> Option<String> {
        let path = self.resolve_module_file(module)?;
        fs::read_to_string(path).ok()
    }

    fn extract_config_fields(&self, module: &str, class_name: &str) -> Vec<ConfigField> {
        let source = match self.load_module_source(module) {
            Some(src) => src,
            None => return Vec::new(),
        };

        let mut fields = Vec::new();
        let mut in_class = false;
        let mut class_indent = 0usize;
        let mut capturing = false;
        let mut buffer = String::new();
        let mut bracket_depth = 0i32;
        let mut seen_equal = false;
        let mut docstring: Option<String> = None;

        for line in source.lines() {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();

            if !in_class {
                if let Some(rest) = trimmed.strip_prefix("class ") {
                    if rest.starts_with(class_name) {
                        let suffix = &rest[class_name.len()..];
                        if suffix.starts_with('(') || suffix.starts_with(':') {
                            in_class = true;
                            class_indent = indent;
                        }
                    }
                }
                continue;
            }

            if let Some(delim) = docstring.clone() {
                if trimmed.contains(&delim) {
                    docstring = None;
                }
                continue;
            }
            if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
                let delim = if trimmed.starts_with("\"\"\"") {
                    "\"\"\"".to_string()
                } else {
                    "'''".to_string()
                };
                if !trimmed.ends_with(&delim) || trimmed.len() == delim.len() {
                    docstring = Some(delim);
                }
                continue;
            }

            if indent <= class_indent && trimmed.starts_with("class ") && !capturing {
                break;
            }

            if trimmed.starts_with("def ") || trimmed.starts_with("@") {
                capturing = false;
                continue;
            }

            if !capturing {
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if !(trimmed
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_')
                    .unwrap_or(false))
                {
                    continue;
                }
                if trimmed.contains(':') || trimmed.contains('=') {
                    buffer.clear();
                    buffer.push_str(trimmed);
                    capturing = true;
                    bracket_depth = Self::bracket_delta(trimmed);
                    seen_equal = Self::line_contains_equals(trimmed);
                    if seen_equal && bracket_depth <= 0 {
                        if let Some(field) = Self::parse_config_field_definition(&buffer) {
                            fields.push(field);
                        }
                        capturing = false;
                    }
                    continue;
                }
            } else {
                buffer.push(' ');
                buffer.push_str(trimmed);
                bracket_depth += Self::bracket_delta(trimmed);
                if !seen_equal && Self::line_contains_equals(trimmed) {
                    seen_equal = true;
                }
                if seen_equal && bracket_depth <= 0 {
                    if let Some(field) = Self::parse_config_field_definition(&buffer) {
                        fields.push(field);
                    }
                    capturing = false;
                }
            }
        }

        fields
    }

    fn line_contains_equals(line: &str) -> bool {
        let mut depth = 0i32;
        let mut chars = line.chars().peekable();
        let mut in_str: Option<char> = None;
        while let Some(ch) = chars.next() {
            if let Some(quote) = in_str {
                if ch == '\\' {
                    chars.next();
                    continue;
                }
                if ch == quote {
                    in_str = None;
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                in_str = Some(ch);
                continue;
            }
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '=' if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    fn bracket_delta(line: &str) -> i32 {
        let mut depth = 0;
        let mut chars = line.chars().peekable();
        let mut in_str: Option<char> = None;
        while let Some(ch) = chars.next() {
            if let Some(quote) = in_str {
                if ch == '\\' {
                    chars.next();
                    continue;
                }
                if ch == quote {
                    in_str = None;
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                in_str = Some(ch);
                continue;
            }
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        depth
    }

    fn parse_config_field_definition(definition: &str) -> Option<ConfigField> {
        let (name_part, remainder) = definition.split_once(':')?;
        let name = name_part.trim();
        if name.is_empty() || name.starts_with("class") || name.starts_with("def") {
            return None;
        }

        let rest = remainder.trim();
        let (annotation, default) = Self::split_annotation_and_default(rest);
        let annotation = annotation.filter(|s| !s.is_empty());
        let default = default.and_then(|value| {
            let cleaned = value.split('#').next().unwrap().trim();
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned.trim().to_string())
            }
        });
        let required = default.is_none();

        Some(ConfigField {
            name: name.to_string(),
            annotation,
            default,
            required,
        })
    }

    fn split_annotation_and_default(text: &str) -> (Option<String>, Option<String>) {
        let mut depth = 0i32;
        let mut in_str: Option<char> = None;
        for (idx, ch) in text.char_indices() {
            if let Some(quote) = in_str {
                if ch == '\\' {
                    continue;
                }
                if ch == quote {
                    in_str = None;
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                in_str = Some(ch);
                continue;
            }
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '=' if depth <= 0 => {
                    let annotation = text[..idx].trim();
                    let default = text[idx + 1..].trim();
                    return (
                        (!annotation.is_empty()).then(|| annotation.to_string()),
                        Some(default.to_string()),
                    );
                }
                _ => {}
            }
        }
        ((!text.is_empty()).then(|| text.trim().to_string()), None)
    }
}
