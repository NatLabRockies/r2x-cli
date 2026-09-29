use r2x_manifest::runtime::{build_runtime_bindings, PluginRole};
use r2x_manifest::types::{Manifest, Package, Plugin};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug)]
pub enum PluginRefError {
    NotFound(String),
    PackageNotPlugin {
        package: String,
        plugins: Vec<String>,
    },
    Ambiguous {
        plugin_ref: String,
        matches: Vec<String>,
    },
}

impl fmt::Display for PluginRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginRefError::NotFound(name) => {
                write!(f, "Plugin '{}' not found in manifest", name)
            }
            PluginRefError::PackageNotPlugin { package, plugins } => {
                if plugins.is_empty() {
                    write!(f, "Package '{}' contains no runnable plugins", package)
                } else {
                    let suggestions = plugins
                        .iter()
                        .map(|plugin| format!("r2x run {plugin} --help"))
                        .collect::<Vec<_>>()
                        .join("\n  ");
                    write!(
                        f,
                        "'{}' is a package, not a plugin. Choose a plugin:\n  {}",
                        package, suggestions
                    )
                }
            }
            PluginRefError::Ambiguous {
                plugin_ref,
                matches,
            } => {
                let suggestions = matches
                    .iter()
                    .map(|candidate| format!("r2x run {candidate}"))
                    .collect::<Vec<_>>()
                    .join("\n  ");
                write!(
                    f,
                    "Plugin reference '{}' is ambiguous. Use a package-qualified name:\n  {}",
                    plugin_ref, suggestions
                )
            }
        }
    }
}

impl std::error::Error for PluginRefError {}

pub(crate) struct ResolvedPlugin<'a> {
    pub(crate) package: &'a Package,
    pub(crate) plugin: &'a Plugin,
}

#[derive(Debug, PartialEq, Eq)]
enum PluginSelector<'a> {
    Name(&'a str),
    Qualified { package: &'a str, plugin: &'a str },
}

pub(crate) fn resolve_plugin_ref<'a>(
    manifest: &'a Manifest,
    plugin_ref: &str,
) -> Result<ResolvedPlugin<'a>, PluginRefError> {
    match parse_plugin_selector(plugin_ref) {
        PluginSelector::Name(plugin_name) => {
            if let Some(resolved) =
                unique_match(plugin_ref, find_plugins_by_name(manifest, plugin_name))?
            {
                return Ok(resolved);
            }

            let packages = find_packages_by_name(manifest, plugin_name);
            if !packages.is_empty() {
                return Err(package_not_plugin_error(manifest, plugin_ref, packages));
            }
            Err(PluginRefError::NotFound(plugin_ref.to_string()))
        }
        PluginSelector::Qualified { package, plugin } => {
            if let Some(resolved) =
                unique_match(plugin_ref, find_plugins_by_name(manifest, plugin_ref))?
            {
                return Ok(resolved);
            }

            let plugin_names = name_variants(plugin);
            let packages = find_packages_by_name(manifest, package);

            let mut matches = Vec::new();
            for package in &packages {
                for candidate in &package.plugins {
                    if plugin_names.contains(candidate.name.as_ref()) {
                        matches.push(ResolvedPlugin {
                            package,
                            plugin: candidate,
                        });
                    }
                }
            }
            if let Some(resolved) = unique_match(plugin_ref, matches)? {
                return Ok(resolved);
            }

            if let Some(role) = alias_role(plugin) {
                let matches = packages
                    .into_iter()
                    .flat_map(|package| {
                        package
                            .plugins
                            .iter()
                            .filter(move |candidate| plugin_role(candidate) == role)
                            .map(move |candidate| ResolvedPlugin {
                                package,
                                plugin: candidate,
                            })
                    })
                    .collect();
                if let Some(resolved) = unique_match(plugin_ref, matches)? {
                    return Ok(resolved);
                }
            }

            Err(PluginRefError::NotFound(plugin_ref.to_string()))
        }
    }
}

fn parse_plugin_selector(plugin_ref: &str) -> PluginSelector<'_> {
    match plugin_ref.split_once('.') {
        Some((package, plugin)) => PluginSelector::Qualified { package, plugin },
        None => PluginSelector::Name(plugin_ref),
    }
}

fn find_plugins_by_name<'a>(manifest: &'a Manifest, plugin_name: &str) -> Vec<ResolvedPlugin<'a>> {
    let names = name_variants(plugin_name);
    let mut matches = Vec::new();
    for package in &manifest.packages {
        for plugin in &package.plugins {
            if names.contains(plugin.name.as_ref()) {
                matches.push(ResolvedPlugin { package, plugin });
            }
        }
    }
    matches
}

fn find_packages_by_name<'a>(manifest: &'a Manifest, package_name: &str) -> Vec<&'a Package> {
    let names = name_variants(package_name);
    manifest
        .packages
        .iter()
        .filter(|package| names.contains(package.name.as_ref()))
        .collect()
}

fn package_not_plugin_error(
    manifest: &Manifest,
    package_ref: &str,
    packages: Vec<&Package>,
) -> PluginRefError {
    let mut plugins: Vec<String> = packages
        .iter()
        .flat_map(|package| {
            package.plugins.iter().map(|plugin| {
                if find_plugins_by_name(manifest, plugin.name.as_ref()).len() == 1 {
                    plugin.name.to_string()
                } else {
                    format!("{}.{}", package.name, plugin.name)
                }
            })
        })
        .collect();
    plugins.sort();
    plugins.dedup();

    PluginRefError::PackageNotPlugin {
        package: package_ref.to_string(),
        plugins,
    }
}

fn unique_match<'a>(
    plugin_ref: &str,
    candidates: Vec<ResolvedPlugin<'a>>,
) -> Result<Option<ResolvedPlugin<'a>>, PluginRefError> {
    match candidates.as_slice() {
        [] => Ok(None),
        [candidate] => Ok(Some(ResolvedPlugin {
            package: candidate.package,
            plugin: candidate.plugin,
        })),
        _ => {
            let mut matches: Vec<String> = candidates
                .iter()
                .map(|candidate| format!("{}.{}", candidate.package.name, candidate.plugin.name))
                .collect();
            matches.sort();
            Err(PluginRefError::Ambiguous {
                plugin_ref: plugin_ref.to_string(),
                matches,
            })
        }
    }
}

fn name_variants(name: &str) -> HashSet<String> {
    [
        name.to_string(),
        name.replace('_', "-"),
        name.replace('-', "_"),
    ]
    .into_iter()
    .collect()
}

fn alias_role(name: &str) -> Option<PluginRole> {
    let normalized = name.replace('-', "_").to_lowercase();
    match normalized.as_str() {
        "parser" => Some(PluginRole::Parser),
        "exporter" => Some(PluginRole::Exporter),
        "upgrader" => Some(PluginRole::Upgrader),
        "modifier" | "transform" | "transformer" => Some(PluginRole::Modifier),
        "translation" | "translator" => Some(PluginRole::Translation),
        "utility" => Some(PluginRole::Utility),
        _ => None,
    }
}

fn plugin_role(plugin: &Plugin) -> PluginRole {
    build_runtime_bindings(plugin).role
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

    fn manifest_with_duplicate_plugin_names() -> Manifest {
        let mut manifest = sample_manifest();
        let mut package = Package {
            name: Arc::from("r2x-plexos"),
            ..Default::default()
        };
        package.plugins.push(Plugin {
            name: Arc::from("reeds-parser"),
            plugin_type: PluginType::Class,
            module: Arc::from("r2x_plexos"),
            class_name: Some(Arc::from("ReEDSParser")),
            ..Default::default()
        });
        manifest.packages.push(package);
        manifest.rebuild_indexes();
        manifest
    }

    #[test]
    fn rejects_unqualified_plugin_names_provided_by_multiple_packages(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let manifest = manifest_with_duplicate_plugin_names();

        let Err(PluginRefError::Ambiguous { matches, .. }) =
            resolve_plugin_ref(&manifest, "reeds-parser")
        else {
            return Err("duplicate plugin names must require package qualification".into());
        };
        assert_eq!(
            matches,
            ["r2x-plexos.reeds-parser", "r2x-reeds.reeds-parser"]
        );
        Ok(())
    }

    #[test]
    fn package_selector_suggests_short_names_unless_a_name_collides(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let manifest = manifest_with_duplicate_plugin_names();

        let Err(PluginRefError::PackageNotPlugin { package, plugins }) =
            resolve_plugin_ref(&manifest, "r2x-reeds")
        else {
            return Err("a package name must not resolve as a plugin".into());
        };
        assert_eq!(package, "r2x-reeds");
        assert_eq!(plugins, ["break-gens", "r2x-reeds.reeds-parser"]);
        Ok(())
    }

    #[test]
    fn package_qualified_name_selects_one_of_the_colliding_plugins() {
        let manifest = manifest_with_duplicate_plugin_names();

        let resolved = resolve_plugin_ref(&manifest, "r2x-plexos.reeds-parser");
        assert!(resolved.is_ok_and(|plugin| plugin.package.name.as_ref() == "r2x-plexos"));
    }

    #[test]
    fn hyphen_and_underscore_name_variants_are_also_collision_checked() {
        let mut manifest = manifest_with_duplicate_plugin_names();
        manifest.packages[1].plugins[0].name = Arc::from("reeds_parser");

        assert!(matches!(
            resolve_plugin_ref(&manifest, "reeds-parser"),
            Err(PluginRefError::Ambiguous { .. })
        ));
    }
}
