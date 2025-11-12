use super::*;
use r2x_manifest::types::{ParameterEntry, ResolvedReference};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone, Copy)]
enum ReferenceKind {
    Class,
    Function,
}

impl ReferenceKind {
    fn as_str(&self) -> &'static str {
        match self {
            ReferenceKind::Class => "class",
            ReferenceKind::Function => "function",
        }
    }
}

struct DefinitionMatch {
    kind: ReferenceKind,
    module_path: String,
    file_path: PathBuf,
    parameters: Vec<ParameterEntry>,
    return_annotation: Option<String>,
}

impl PluginExtractor {
    pub fn resolve_references(
        &self,
        plugin: &mut DiscoveryPlugin,
        package_root: &Path,
        package_name: &str,
    ) -> Result<()> {
        let mut resolved_refs = Vec::new();

        for arg in &plugin.constructor_args {
            let prefer_class = arg.arg_type == "class_reference"
                || (arg.arg_type == "identifier"
                    && arg
                        .value
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false));

            if arg.arg_type == "class_reference" || arg.arg_type == "identifier" {
                match self.resolve_single_reference(
                    &arg.value,
                    package_root,
                    package_name,
                    prefer_class,
                ) {
                    Ok(resolved) => {
                        resolved_refs.push(resolved);
                    }
                    Err(e) => {
                        debug!(
                            "Failed to resolve reference {} for plugin {}: {}",
                            arg.value, plugin.name, e
                        );
                    }
                }
            }
        }

        plugin.resolved_references = resolved_refs;
        Ok(())
    }

    fn resolve_single_reference(
        &self,
        reference: &str,
        package_root: &Path,
        package_name: &str,
        prefer_class: bool,
    ) -> Result<ResolvedReference> {
        let definition =
            self.locate_definition(reference, package_root, package_name, prefer_class)?;

        let source_file = self
            .relative_source_path(&definition.file_path, package_root)
            .or_else(|| Some(definition.file_path.to_string_lossy().to_string()));

        Ok(ResolvedReference {
            key: reference.to_string(),
            ref_type: definition.kind.as_str().to_string(),
            module: definition.module_path,
            name: reference.to_string(),
            source_file,
            parameters: definition.parameters,
            return_annotation: definition.return_annotation,
        })
    }

    fn locate_definition(
        &self,
        reference: &str,
        package_root: &Path,
        package_name: &str,
        prefer_class: bool,
    ) -> Result<DefinitionMatch> {
        if let Some(def) = self.try_locate_in_file(
            reference,
            prefer_class,
            &self.python_file_path,
            &self.content,
            package_root,
            package_name,
        ) {
            return Ok(def);
        }

        if let Some(module_hint) = self.import_map.get(reference) {
            if let Some(def) = self.locate_via_import(
                reference,
                module_hint,
                prefer_class,
                package_root,
                package_name,
            ) {
                return Ok(def);
            }
        }

        self.scan_package_for_definition(reference, package_root, package_name, prefer_class)
    }

    fn try_locate_in_file(
        &self,
        reference: &str,
        prefer_class: bool,
        file_path: &Path,
        content: &str,
        package_root: &Path,
        package_name: &str,
    ) -> Option<DefinitionMatch> {
        let module_path = self.build_module_path(file_path, package_root, package_name);
        self.try_parse_definition(reference, prefer_class, content, file_path, module_path)
    }

    fn try_parse_definition(
        &self,
        reference: &str,
        prefer_class: bool,
        content: &str,
        file_path: &Path,
        module_path: String,
    ) -> Option<DefinitionMatch> {
        let mut attempts = if prefer_class {
            vec![ReferenceKind::Class, ReferenceKind::Function]
        } else {
            vec![ReferenceKind::Function, ReferenceKind::Class]
        };

        for kind in attempts.drain(..) {
            match kind {
                ReferenceKind::Class => {
                    if let Ok(parameters) =
                        self.extract_class_parameters_from_content(content, reference)
                    {
                        return Some(DefinitionMatch {
                            kind,
                            module_path: module_path.clone(),
                            file_path: file_path.to_path_buf(),
                            parameters,
                            return_annotation: None,
                        });
                    }
                }
                ReferenceKind::Function => {
                    if let Ok(parameters) =
                        self.extract_function_parameters_from_content(content, reference)
                    {
                        let return_annotation =
                            self.extract_function_return_type_from_content(content, reference);
                        return Some(DefinitionMatch {
                            kind,
                            module_path: module_path.clone(),
                            file_path: file_path.to_path_buf(),
                            parameters,
                            return_annotation,
                        });
                    }
                }
            }
        }

        // As a final fallback, try the opposite order if nothing matched.
        None
    }

    fn locate_via_import(
        &self,
        reference: &str,
        module_hint: &str,
        prefer_class: bool,
        package_root: &Path,
        package_name: &str,
    ) -> Option<DefinitionMatch> {
        let resolved_module = self.resolve_module_hint(module_hint, package_root, package_name);

        for candidate in self
            .module_to_candidate_paths(&resolved_module, package_root)
            .into_iter()
        {
            if let Ok(content) = fs::read_to_string(&candidate) {
                if let Some(def) = self.try_parse_definition(
                    reference,
                    prefer_class,
                    &content,
                    &candidate,
                    resolved_module.clone(),
                ) {
                    return Some(def);
                }
            }
        }

        None
    }

    fn scan_package_for_definition(
        &self,
        reference: &str,
        package_root: &Path,
        package_name: &str,
        prefer_class: bool,
    ) -> Result<DefinitionMatch> {
        for entry in WalkDir::new(package_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("py") {
                continue;
            }
            if path == self.python_file_path {
                continue;
            }

            if let Ok(content) = fs::read_to_string(path) {
                if let Some(def) = self.try_locate_in_file(
                    reference,
                    prefer_class,
                    path,
                    &content,
                    package_root,
                    package_name,
                ) {
                    return Ok(def);
                }
            }
        }

        Err(anyhow!("Unable to resolve reference '{}'", reference))
    }

    fn build_module_path(
        &self,
        file_path: &Path,
        package_root: &Path,
        package_name: &str,
    ) -> String {
        let package_prefix = package_name.replace('-', "_");
        if let Ok(relative) = file_path.strip_prefix(package_root) {
            let mut module_path = relative.to_string_lossy().to_string();
            module_path = module_path
                .trim_end_matches(".py")
                .replace('\\', ".")
                .replace('/', ".");

            if module_path.ends_with(".__init__") {
                module_path = module_path
                    .trim_end_matches(".__init__")
                    .trim_end_matches('.')
                    .to_string();
            }

            module_path = module_path.trim_matches('.').to_string();

            if module_path.is_empty() {
                return package_prefix;
            }

            if module_path.starts_with(&package_prefix) {
                return module_path;
            }

            if let Some(root_name) = package_root.file_name().and_then(|n| n.to_str()) {
                if module_path.starts_with(root_name) {
                    return module_path;
                }
            }

            format!("{}.{}", package_prefix, module_path)
        } else {
            package_prefix
        }
    }

    fn resolve_module_hint(
        &self,
        module_hint: &str,
        package_root: &Path,
        package_name: &str,
    ) -> String {
        if module_hint.starts_with('.') {
            let current_module =
                self.build_module_path(&self.python_file_path, package_root, package_name);
            self.apply_relative_module(&current_module, module_hint)
        } else {
            module_hint.to_string()
        }
    }

    fn apply_relative_module(&self, current_module: &str, relative: &str) -> String {
        let mut dot_count = 0;
        for ch in relative.chars() {
            if ch == '.' {
                dot_count += 1;
            } else {
                break;
            }
        }

        let mut base_parts: Vec<&str> = current_module.split('.').collect();
        for _ in 0..dot_count {
            base_parts.pop();
        }

        let remainder = relative.trim_start_matches('.');
        if !remainder.is_empty() {
            base_parts.extend(remainder.split('.').filter(|s| !s.is_empty()));
        }

        base_parts.join(".")
    }

    fn module_to_candidate_paths(&self, module_path: &str, package_root: &Path) -> Vec<PathBuf> {
        let normalized = module_path.replace('.', "/");
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        let base_candidates = if normalized.is_empty() {
            vec![package_root.join("__init__.py")]
        } else {
            vec![
                package_root.join(format!("{}.py", normalized)),
                package_root.join(&normalized).join("__init__.py"),
            ]
        };

        for path in base_candidates {
            if seen.insert(path.clone()) {
                candidates.push(path);
            }
        }

        if let Some(root_name) = package_root.file_name().and_then(|n| n.to_str()) {
            if normalized.starts_with(root_name) {
                let trimmed = normalized[root_name.len()..].trim_start_matches('/');
                let extra = if trimmed.is_empty() {
                    vec![package_root.join("__init__.py")]
                } else {
                    vec![
                        package_root.join(format!("{}.py", trimmed)),
                        package_root.join(trimmed).join("__init__.py"),
                    ]
                };

                for path in extra {
                    if seen.insert(path.clone()) {
                        candidates.push(path);
                    }
                }
            }
        }

        candidates
    }

    fn relative_source_path(&self, file_path: &Path, package_root: &Path) -> Option<String> {
        file_path
            .strip_prefix(package_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.to_string())
    }
}
