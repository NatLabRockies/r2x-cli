use r2x_manifest::execution_types::PluginKind;
use r2x_manifest::runtime::build_runtime_bindings_from_plugin;
use r2x_manifest::types::{Manifest, Package, Plugin};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug)]
pub enum PluginRefError {
    NotFound(String),
    Ambiguous {
        plugin_ref: String,
        package: String,
        matches: Vec<String>,
    },
}

impl fmt::Display for PluginRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginRefError::NotFound(name) => {
                write!(f, "Plugin '{}' not found in manifest", name)
            }
            PluginRefError::Ambiguous {
                plugin_ref,
                package,
                matches,
            } => {
                write!(
                    f,
                    "Plugin reference '{}' is ambiguous in package '{}': {}",
                    plugin_ref,
                    package,
                    matches.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for PluginRefError {}

pub struct ResolvedPlugin<'a> {
    pub package: &'a Package,
    pub plugin: &'a Plugin,
}

pub fn resolve_plugin_ref<'a>(
    manifest: &'a Manifest,
    plugin_ref: &str,
) -> Result<ResolvedPlugin<'a>, PluginRefError> {
    if let Some(resolved) = find_plugin_by_name(manifest, plugin_ref) {
        return Ok(resolved);
    }

    if let Some((package_part, plugin_part)) = plugin_ref.split_once('.') {
        for package_name in name_variants(package_part) {
            let Some(package) = manifest.get_package(&package_name) else {
                continue;
            };

            if let Some(plugin) = find_plugin_in_package(package, plugin_part) {
                return Ok(ResolvedPlugin { package, plugin });
            }

            if let Some(kind) = alias_kind(plugin_part) {
                let matches: Vec<&Plugin> = package
                    .plugins
                    .iter()
                    .filter(|plugin| plugin_kind(plugin) == kind)
                    .collect();

                match matches.len() {
                    0 => {}
                    1 => {
                        return Ok(ResolvedPlugin {
                            package,
                            plugin: matches[0],
                        });
                    }
                    _ => {
                        let names = matches
                            .iter()
                            .map(|plugin| plugin.name.to_string())
                            .collect();
                        return Err(PluginRefError::Ambiguous {
                            plugin_ref: plugin_ref.to_string(),
                            package: package.name.to_string(),
                            matches: names,
                        });
                    }
                }
            }
        }
    }

    Err(PluginRefError::NotFound(plugin_ref.to_string()))
}

fn find_plugin_by_name<'a>(
    manifest: &'a Manifest,
    plugin_name: &str,
) -> Option<ResolvedPlugin<'a>> {
    for candidate in name_variants(plugin_name) {
        if let Some((package, plugin)) = manifest.packages.iter().find_map(|package| {
            package
                .plugins
                .iter()
                .find(|plugin| plugin.name.as_ref() == candidate)
                .map(|plugin| (package, plugin))
        }) {
            return Some(ResolvedPlugin { package, plugin });
        }
    }
    None
}

fn find_plugin_in_package<'a>(package: &'a Package, plugin_name: &str) -> Option<&'a Plugin> {
    for candidate in name_variants(plugin_name) {
        if let Some(plugin) = package
            .plugins
            .iter()
            .find(|plugin| plugin.name.as_ref() == candidate)
        {
            return Some(plugin);
        }
    }
    None
}

fn name_variants(name: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut variants = Vec::new();
    for candidate in [
        name.to_string(),
        name.replace('_', "-"),
        name.replace('-', "_"),
    ] {
        if seen.insert(candidate.clone()) {
            variants.push(candidate);
        }
    }
    variants
}

fn alias_kind(name: &str) -> Option<PluginKind> {
    let normalized = name.replace('-', "_").to_lowercase();
    match normalized.as_str() {
        "parser" => Some(PluginKind::Parser),
        "exporter" => Some(PluginKind::Exporter),
        "upgrader" => Some(PluginKind::Upgrader),
        "modifier" | "transform" | "transformer" => Some(PluginKind::Modifier),
        "translation" | "translator" => Some(PluginKind::Translation),
        "utility" => Some(PluginKind::Utility),
        _ => None,
    }
}

fn plugin_kind(plugin: &Plugin) -> PluginKind {
    build_runtime_bindings_from_plugin(plugin).plugin_kind
}

#[cfg(test)]
mod tests {
    use crate::manifest_lookup::*;
    use r2x_manifest::types::PluginType;
    use std::sync::Arc;

    fn sample_manifest() -> Manifest {
        let mut manifest = Manifest::default();
        let mut package = Package {
            name: Arc::from("r2x-reeds"),
            ..Default::default()
        };

        package.plugins.push(Plugin {
            name: Arc::from("reeds-parser"),
            plugin_type: PluginType::Class,
            module: Arc::from("r2x_reeds"),
            class_name: Some(Arc::from("ReEDSParser")),
            ..Default::default()
        });

        package.plugins.push(Plugin {
            name: Arc::from("break-gens"),
            plugin_type: PluginType::Function,
            module: Arc::from("r2x_reeds.sysmod.break_gens"),
            function_name: Some(Arc::from("break_generators")),
            ..Default::default()
        });

        manifest.packages.push(package);
        manifest.rebuild_indexes();
        manifest
    }

    #[test]
    fn resolves_plugin_by_exact_name() {
        let manifest = sample_manifest();
        let resolved = resolve_plugin_ref(&manifest, "reeds-parser");
        assert!(resolved.is_ok_and(|r| r.plugin.name.as_ref() == "reeds-parser"));
    }

    #[test]
    fn resolves_plugin_by_package_prefix() {
        let manifest = sample_manifest();
        let resolved = resolve_plugin_ref(&manifest, "r2x-reeds.reeds-parser");
        assert!(resolved.is_ok_and(|r| r.plugin.name.as_ref() == "reeds-parser"));
    }

    #[test]
    fn resolves_plugin_with_underscore_variants() {
        let manifest = sample_manifest();
        let resolved = resolve_plugin_ref(&manifest, "r2x_reeds.break_gens");
        assert!(resolved.is_ok_and(|r| r.plugin.name.as_ref() == "break-gens"));
    }

    #[test]
    fn resolves_plugin_kind_alias() {
        let manifest = sample_manifest();
        let resolved = resolve_plugin_ref(&manifest, "r2x-reeds.parser");
        assert!(resolved.is_ok_and(|r| r.plugin.name.as_ref() == "reeds-parser"));
    }
}
