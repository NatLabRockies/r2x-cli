use anyhow::{anyhow, Result};
use ast_grep_core::AstGrep;
use ast_grep_language::Python;
use r2x_manifest::{ConstructorArg, DiscoveryPlugin};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info};

mod args;
mod parameters;
mod resolver;

#[cfg(test)]
mod tests;

/// Extract plugin definitions from Python source code using pure AST parsing
pub struct PluginExtractor {
    pub(crate) python_file_path: PathBuf,
    pub(crate) content: String,
    pub(crate) import_map: HashMap<String, String>,
}

impl PluginExtractor {
    /// Create a new extractor for a Python file
    pub fn new(python_file_path: PathBuf) -> Result<Self> {
        debug!("Initializing plugin extractor for: {:?}", python_file_path);

        let content = fs::read_to_string(&python_file_path)?;
        if !content.contains("def register_plugin") {
            return Err(anyhow!(
                "No register_plugin function found in: {:?}",
                python_file_path
            ));
        }

        let import_map = PluginExtractor::build_import_map_static(&content);

        Ok(PluginExtractor {
            python_file_path,
            content,
            import_map,
        })
    }

    /// Extract all plugins from the register_plugin() function using pure AST parsing
    pub fn extract_plugins(&self) -> Result<Vec<DiscoveryPlugin>> {
        debug!(
            "Extracting plugins via pure AST parsing from: {:?}",
            self.python_file_path
        );

        let sg = AstGrep::new(&self.content, Python);
        let root = sg.root();
        let package_calls: Vec<_> = root.find_all("Package($$$_)").collect();

        if package_calls.is_empty() {
            return Err(anyhow!("No Package() call found"));
        }

        debug!("Found {} Package() calls", package_calls.len());
        let mut plugins = Vec::new();

        for package_match in package_calls {
            let package_text = package_match.text();
            if !package_text.contains("plugins") {
                debug!("Package() call doesn't have plugins parameter, skipping");
                continue;
            }

            if let Some(plugins_start) = package_text.find("plugins") {
                let after_plugins = &package_text[plugins_start..];
                if let Some(bracket_pos) = after_plugins.find(|c| c == '[' || c == '(') {
                    let bracket_char = if after_plugins.chars().nth(bracket_pos) == Some('[') {
                        ('[', ']')
                    } else {
                        ('(', ')')
                    };

                    let mut depth = 0;
                    let mut plugins_content_end = bracket_pos;
                    for (i, ch) in after_plugins.chars().enumerate().skip(bracket_pos) {
                        if ch == bracket_char.0 {
                            depth += 1;
                        } else if ch == bracket_char.1 {
                            depth -= 1;
                            if depth == 0 {
                                plugins_content_end = i;
                                break;
                            }
                        }
                    }

                    let plugins_list = &after_plugins[bracket_pos + 1..plugins_content_end];
                    let sg_list = AstGrep::new(plugins_list, Python);
                    let list_root = sg_list.root();
                    let all_calls: Vec<_> = list_root.find_all("$FUNC($$$ARGS)").collect();

                    for call_match in all_calls {
                        let call_text = call_match.text().to_string();
                        if let Ok(plugin) = self.extract_plugin_from_call_match(&call_text) {
                            debug!("Extracted plugin: {}", plugin.name);
                            plugins.push(plugin);
                        }
                    }
                }
            }
        }

        info!("Extracted {} plugins from register_plugin()", plugins.len());
        Ok(plugins)
    }

    fn extract_plugin_from_call_match(&self, call_text: &str) -> Result<DiscoveryPlugin> {
        debug!(
            "Parsing plugin instantiation from call match: {:?}",
            call_text.lines().next()
        );

        let plugin_type = call_text
            .split('(')
            .next()
            .ok_or_else(|| anyhow!("Cannot extract function name from call"))?
            .trim()
            .to_string();

        debug!("Detected plugin type: {}", plugin_type);

        let constructor_args = self.extract_keyword_arguments_from_text(call_text)?;
        let plugin_name = self.find_kwarg_value(&constructor_args, "name")?;

        Ok(DiscoveryPlugin {
            name: plugin_name,
            plugin_type,
            constructor_args,
            resolved_references: Vec::new(),
            decorators: Vec::new(),
        })
    }

    fn build_import_map_static(content: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let register_plugin_start = if let Some(pos) = content.find("def register_plugin") {
            pos
        } else {
            0
        };

        let content_to_scan = &content[register_plugin_start..];

        for line in content_to_scan.lines() {
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
}
