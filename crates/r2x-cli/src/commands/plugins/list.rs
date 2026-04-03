use crate::commands::plugins::context::PluginContext;
use crate::common::GlobalOpts;
use crate::plugins::error::PluginError;
use crate::plugins::install::get_package_info;
use crate::plugins::package_spec::is_git_url;
use colored::Colorize;
use r2x_manifest::package_discovery::PackageLocator;
use r2x_manifest::types::{Manifest, Package, PackageSource, Plugin};
use std::collections::BTreeMap;

fn source_kind(pkg: &Package, locator: Option<&PackageLocator>) -> PackageSource {
    if pkg.source_kind != PackageSource::Pypi {
        return pkg.source_kind;
    }

    if let Some(uri) = pkg.source_uri.as_deref() {
        if is_git_url(uri) {
            if uri.to_ascii_lowercase().contains("github.com") {
                return PackageSource::Github;
            }
            return PackageSource::Git;
        }

        if pkg.editable_install {
            return PackageSource::Local;
        }
    }

    if let Some(locator) = locator {
        let detected = locator.detect_package_source(pkg.name.as_ref(), pkg.source_uri.as_deref());
        if detected != PackageSource::Pypi {
            return detected;
        }
    }

    if pkg.editable_install {
        return PackageSource::Local;
    }

    PackageSource::Pypi
}

fn format_source(pkg: &Package, locator: Option<&PackageLocator>) -> String {
    source_kind(pkg, locator).label().to_string()
}

fn package_version(pkg: &Package, discovered_version: Option<String>) -> Option<String> {
    discovered_version.or_else(|| {
        let version = pkg.version.as_ref();
        if version.is_empty() || version == "unknown" {
            None
        } else {
            Some(version.to_string())
        }
    })
}

fn format_package_header(pkg: &Package, version: Option<&str>, source_display: &str) -> String {
    let mut header = format!("{}", pkg.name.as_ref().bold().blue());

    if let Some(version) = version {
        header.push_str(&format!("{}", format!(":v{}", version).cyan()));
    }

    header.push_str(&format!(" {}", format!("[{}]", source_display).dimmed()));

    header
}

fn format_github_origin(source_uri: &str) -> Option<String> {
    let prefixes = [
        "git+https://github.com/",
        "https://github.com/",
        "git+http://github.com/",
        "http://github.com/",
        "git+ssh://git@github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
    ];

    for prefix in prefixes {
        if let Some(rest) = source_uri.strip_prefix(prefix) {
            let (repo_path, git_ref) = match rest.rsplit_once('@') {
                Some((path, reference)) if !path.is_empty() && !reference.is_empty() => {
                    (path.to_string(), Some(reference.to_string()))
                }
                _ => (rest.to_string(), None),
            };

            let mut ssh = format!("git@github.com:{}", repo_path);
            if let Some(reference) = git_ref {
                ssh.push('@');
                ssh.push_str(&reference);
            }

            return Some(ssh);
        }
    }

    None
}

fn package_source_display(pkg: &Package, locator: &PackageLocator) -> String {
    let kind = source_kind(pkg, Some(locator));
    if kind == PackageSource::Github {
        let origin_raw = pkg
            .source_uri
            .as_deref()
            .map(ToString::to_string)
            .or_else(|| locator.direct_url_origin(pkg.name.as_ref()));

        if let Some(raw) = origin_raw {
            return format_github_origin(&raw).unwrap_or(raw);
        }
    }

    format_source(pkg, Some(locator))
}

pub fn list_plugins(
    opts: &GlobalOpts,
    plugin_filter: Option<String>,
    module_filter: Option<String>,
    ctx: &PluginContext,
) -> Result<(), PluginError> {
    let manifest = &ctx.manifest;

    let has_plugins = !manifest.is_empty();

    if !has_plugins {
        println!("There are no current plugins installed.\n");
        println!(
            "To install a plugin, run:\n  {} install <package>",
            "r2x".bold().cyan()
        );
        return Ok(());
    }

    // If a plugin filter is provided, show detailed information
    if let Some(ref plugin_name) = plugin_filter {
        return show_plugin_details(
            manifest,
            plugin_name,
            module_filter.as_deref(),
            opts.verbose,
            ctx,
        );
    }

    // Otherwise, show the standard list view
    let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in &manifest.packages {
        if pkg.plugins.is_empty() {
            continue;
        }
        let mut names: Vec<String> = pkg.plugins.iter().map(|p| p.name.to_string()).collect();
        names.sort();
        packages.insert(pkg.name.to_string(), names);
    }

    if has_plugins {
        // Get package version info
        let python_path = &ctx.python_path;
        let uv_path = &ctx.uv_path;

        for (package_name, plugin_names) in &packages {
            // Get package metadata
            let pkg = match manifest
                .packages
                .iter()
                .find(|p| p.name.as_ref() == package_name)
            {
                Some(pkg) => pkg,
                None => continue,
            };

            // Get version info
            let version = package_version(
                pkg,
                get_package_info(uv_path, python_path, package_name)
                    .ok()
                    .and_then(|(v, _)| v),
            );
            let source_display = package_source_display(pkg, &ctx.locator);
            println!(
                "{}",
                format_package_header(pkg, version.as_deref(), &source_display)
            );

            for plugin_name in plugin_names {
                println!("  - {}", plugin_name);
            }
        }
    }

    Ok(())
}

fn show_plugin_details(
    manifest: &Manifest,
    plugin_filter: &str,
    module_filter: Option<&str>,
    verbose_level: u8,
    ctx: &PluginContext,
) -> Result<(), PluginError> {
    // Find the package containing this plugin
    let package = manifest
        .packages
        .iter()
        .find(|pkg| pkg.name.as_ref() == plugin_filter)
        .ok_or_else(|| {
            PluginError::InvalidArgs(format!("Plugin package '{}' not found", plugin_filter))
        })?;

    // Build package header with version and editable info
    let version = package_version(
        package,
        get_package_info(&ctx.uv_path, &ctx.python_path, package.name.as_ref())
            .ok()
            .and_then(|(v, _)| v),
    );
    let source_display = package_source_display(package, &ctx.locator);
    println!(
        "{} {}",
        "Package:".bold().green(),
        format_package_header(package, version.as_deref(), &source_display)
    );
    println!();

    // Filter plugins by module name if provided
    let plugins_to_show: Vec<_> = if let Some(module_name) = module_filter {
        package
            .plugins
            .iter()
            .filter(|p| {
                // Match if the plugin name ends with the module filter
                // e.g., "r2x_reeds.break_gens" matches module "break_gens"
                let name_str = p.name.as_ref();
                let parts: Vec<&str> = name_str.split('.').collect();
                parts.last().is_some_and(|&last| last == module_name)
            })
            .collect()
    } else {
        package.plugins.iter().collect()
    };

    if plugins_to_show.is_empty() {
        return Err(PluginError::InvalidArgs(format!(
            "No plugins found matching the filter criteria in package '{}'",
            plugin_filter
        )));
    }

    for plugin in plugins_to_show {
        if verbose_level > 0 {
            show_plugin_verbose(plugin);
        } else {
            show_plugin_compact(plugin);
        }
        println!();
    }

    Ok(())
}

fn show_plugin_compact(plugin: &Plugin) {
    println!(
        "{} [{:?}]",
        plugin.name.as_ref().bold().cyan(),
        plugin.plugin_type
    );

    // Show module info
    println!("  {}: {}", "Module".dimmed(), plugin.module);

    // Show class or function name
    if let Some(ref class_name) = plugin.class_name {
        println!("  {}: {}", "Class".dimmed(), class_name);
    }
    if let Some(ref function_name) = plugin.function_name {
        println!("  {}: {}", "Function".dimmed(), function_name);
    }

    // Show config if available
    if let Some(ref config_class) = plugin.config_class {
        println!("  {}: {}", "Config".dimmed(), config_class);
    }

    // Show arguments if available
    if !plugin.parameters.is_empty() {
        println!("  {}:", "Arguments".dimmed());
        for param in &plugin.parameters {
            let req_marker = if param.required { "*" } else { " " };
            let default_str = param
                .default
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            println!(
                "    {}{}: {}{}",
                req_marker,
                param.name,
                param.format_types(),
                default_str
            );

            if let Some(ref desc) = param.description {
                println!("      {}", desc.dimmed());
            }
        }
    }
}

fn show_plugin_verbose(plugin: &Plugin) {
    println!("{}", plugin.name.as_ref().bold().cyan());

    println!("  {}: {:?}", "Type".dimmed(), plugin.plugin_type);
    println!("  {}: {}", "Module".dimmed(), plugin.module);

    // Show class or function name
    if let Some(ref class_name) = plugin.class_name {
        println!("  {}: {}", "Class".dimmed(), class_name);
    }
    if let Some(ref function_name) = plugin.function_name {
        println!("  {}: {}", "Function".dimmed(), function_name);
    }

    // Show config info
    if let Some(ref config_class) = plugin.config_class {
        print!("  {}: {}", "Config Class".dimmed(), config_class);
        if let Some(ref config_module) = plugin.config_module {
            print!(" ({})", config_module);
        }
        println!();
    }

    // Show hooks
    if !plugin.hooks.is_empty() {
        println!("  {}:", "Hooks".dimmed());
        for hook in &plugin.hooks {
            println!("    - {}", hook);
        }
    }

    // Show arguments
    if !plugin.parameters.is_empty() {
        println!("  {}:", "Arguments".dimmed());
        for param in &plugin.parameters {
            let req_marker = if param.required { "*" } else { " " };
            let module_str = param
                .module
                .as_ref()
                .map(|m| format!(" ({})", m))
                .unwrap_or_default();
            let default_str = param
                .default
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            println!(
                "    {}{}: {}{}{}",
                req_marker,
                param.name,
                param.format_types(),
                module_str,
                default_str
            );

            if let Some(ref desc) = param.description {
                println!("      {}", desc.dimmed());
            }
        }
    }

    // Show config schema if available
    if !plugin.config_schema.is_empty() {
        println!("  {}:", "Config Schema".dimmed());
        for (field_name, field) in plugin.config_schema.iter() {
            let req_marker = if field.required { "*" } else { "" };
            println!("    {}{}: {:?}", field_name, req_marker, field.field_type);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::plugins::list::{
        format_github_origin, format_package_header, format_source, package_source_display,
        package_version, source_kind,
    };
    use colored::control::set_override;
    use r2x_manifest::package_discovery::PackageLocator;
    use r2x_manifest::types::{Package, PackageSource};
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn package_with_source(source_kind: PackageSource) -> Package {
        Package {
            name: Arc::from("r2x-plexos-to-sienna"),
            version: Arc::from("0.0.0"),
            source_kind,
            ..Default::default()
        }
    }

    #[test]
    fn source_uses_manifest_source_kind() {
        let package = package_with_source(PackageSource::Github);
        assert_eq!(format_source(&package, None), "github");
    }

    #[test]
    fn source_falls_back_to_git_uri_for_legacy_manifest_entries() {
        let mut package = package_with_source(PackageSource::Pypi);
        package.source_uri = Some(Arc::from("git+ssh://git@github.com/NatLabRockies/R2X.git"));
        assert_eq!(format_source(&package, None), "github");
    }

    #[test]
    fn source_falls_back_to_live_dist_info_for_legacy_manifest_entries() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let site_packages = temp_dir.path();
        let dist_info = site_packages.join("r2x_plexos_to_sienna-0.0.0.dist-info");
        if fs::create_dir(&dist_info).is_err() {
            return;
        }
        if fs::write(
            dist_info.join("direct_url.json"),
            r#"{"url":"ssh://git@github.com/NatLabRockies/R2X.git","vcs_info":{"vcs":"git"},"subdirectory":"packages/r2x-plexos-to-sienna"}"#,
        )
        .is_err()
        {
            return;
        }

        let Ok(locator) = PackageLocator::new(site_packages.to_path_buf(), None) else {
            return;
        };
        let package = package_with_source(PackageSource::Pypi);

        assert_eq!(source_kind(&package, Some(&locator)), PackageSource::Github);
    }

    #[test]
    fn standalone_git_direct_url_stays_pypi_without_manifest_source() {
        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let site_packages = temp_dir.path();
        let dist_info = site_packages.join("r2x_sienna-0.1.0.dist-info");
        if fs::create_dir(&dist_info).is_err() {
            return;
        }
        if fs::write(
            dist_info.join("direct_url.json"),
            r#"{"url":"ssh://git@github.com/NREL-Sienna/r2x-sienna","vcs_info":{"vcs":"git"}}"#,
        )
        .is_err()
        {
            return;
        }

        let Ok(locator) = PackageLocator::new(site_packages.to_path_buf(), None) else {
            return;
        };
        let package = Package {
            name: Arc::from("r2x-sienna"),
            version: Arc::from("0.1.0"),
            ..Default::default()
        };

        assert_eq!(source_kind(&package, Some(&locator)), PackageSource::Pypi);
    }

    #[test]
    fn version_falls_back_to_manifest_when_pip_show_is_unavailable() {
        let package = package_with_source(PackageSource::Pypi);
        assert_eq!(package_version(&package, None).as_deref(), Some("0.0.0"));
    }

    #[test]
    fn header_shows_source_prefix_and_version() {
        set_override(false);
        let package = package_with_source(PackageSource::Github);
        assert_eq!(
            format_package_header(&package, Some("0.0.0"), "github"),
            "r2x-plexos-to-sienna:v0.0.0 [github]"
        );
    }

    #[test]
    fn header_omits_version_when_missing() {
        set_override(false);
        let package = package_with_source(PackageSource::Pypi);
        assert_eq!(
            format_package_header(&package, None, "pypi"),
            "r2x-plexos-to-sienna [pypi]"
        );
    }

    #[test]
    fn github_origin_is_rendered_in_ssh_style() {
        assert_eq!(
            format_github_origin("git+https://github.com/NREL/r2x-reeds.git"),
            Some("git@github.com:NREL/r2x-reeds.git".to_string())
        );
    }

    #[test]
    fn github_origin_preserves_git_ref_suffix() {
        assert_eq!(
            format_github_origin("git+https://github.com/NREL/r2x-reeds.git@develop"),
            Some("git@github.com:NREL/r2x-reeds.git@develop".to_string())
        );
    }

    #[test]
    fn github_source_display_uses_source_uri_when_present() {
        let mut package = package_with_source(PackageSource::Github);
        package.source_uri = Some(Arc::from("git+https://github.com/NREL/r2x-reeds.git@main"));

        let Ok(temp_dir) = TempDir::new() else {
            return;
        };
        let Ok(locator) = PackageLocator::new(temp_dir.path().to_path_buf(), None) else {
            return;
        };

        assert_eq!(
            package_source_display(&package, &locator),
            "git@github.com:NREL/r2x-reeds.git@main"
        );
    }
}
