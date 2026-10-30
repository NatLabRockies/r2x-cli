use clap::Parser;
use r2x_manifest::errors::ManifestError;
use r2x_manifest::types::{Manifest, Package};
use r2x_python::errors::BridgeError;
use r2x_python::python_bridge::Bridge;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

mod strict_json;

#[derive(Parser, Debug)]
pub struct ShowCommand {
    /// Installed package whose mappings should be shown
    package: String,
    /// Name of a declared file, table, or translation rule
    item: Option<String>,
    /// Show mappings from this configuration directory instead of the package default
    #[arg(long, value_name = "DIR")]
    config_dir: Option<PathBuf>,
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Error)]
pub enum ShowError {
    #[error("Package '{0}' not found. Use `r2x list` to see installed packages.")]
    PackageNotFound(String),
    #[error("Package '{0}' has no discovered configuration class. Use `r2x sync` to refresh plugin metadata.")]
    MissingConfig(String),
    #[error("Package '{package}' has multiple configuration directories: {paths:?}. Select one with --config-dir.")]
    MultipleConfigs {
        package: String,
        paths: Vec<PathBuf>,
    },
    #[error("Cannot access configuration directory {path}: {source}")]
    ConfigDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Configuration path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error(
        "Mapping '{item}' not found in '{package}'. Use `r2x show {package}` with the same configuration options to list mappings."
    )]
    ItemNotFound { package: String, item: String },
    #[error("Mapping '{item}' is ambiguous. Unique candidates: {choices:?}. Some mappings may lack a unique selector.")]
    AmbiguousItem { item: String, choices: Vec<String> },
    #[error("Failed to read mapping {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Invalid JSON in mapping {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("Cannot show mapping {path}: {reason}")]
    InvalidMapping { path: PathBuf, reason: String },
    #[error("Unsupported mapping {path}: {reason}")]
    UnsupportedMapping { path: PathBuf, reason: String },
    #[error("Failed to serialize JSON output: {0}")]
    Output(#[source] serde_json::Error),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
}

impl ShowError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PackageNotFound(_) => "PACKAGE_NOT_FOUND",
            Self::MissingConfig(_) | Self::ConfigDirectory { .. } | Self::NotDirectory(_) => {
                "CONFIGURATION_UNAVAILABLE"
            }
            Self::MultipleConfigs { .. } => "AMBIGUOUS_CONFIGURATION",
            Self::ItemNotFound { .. } => "MAPPING_NOT_FOUND",
            Self::AmbiguousItem { .. } => "AMBIGUOUS_MAPPING",
            Self::Read { .. } => "IO_ERROR",
            Self::Json { .. } => "INVALID_JSON",
            Self::InvalidMapping { .. } => "INVALID_MAPPING",
            Self::UnsupportedMapping { .. } => "UNSUPPORTED_MAPPING",
            Self::Output(_) => "OUTPUT_ERROR",
            Self::Manifest(_) => "MANIFEST_ERROR",
            Self::Bridge(_) => "PYTHON_ERROR",
        }
    }

    pub fn candidates(&self) -> Vec<String> {
        match self {
            Self::MultipleConfigs { paths, .. } => paths
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect(),
            Self::AmbiguousItem { choices, .. } => choices.clone(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum MappingKind {
    File,
    Table,
    Rule,
}

impl MappingKind {
    fn filename(self) -> &'static str {
        match self {
            Self::File => "file_mapping.json",
            Self::Table => "table_mapping.json",
            Self::Rule => "translation_rules.json",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::File => "Files",
            Self::Table => "Tables",
            Self::Rule => "Translation rules",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Table => "Table",
            Self::Rule => "Translation rule",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Table => "table",
            Self::Rule => "rule",
        }
    }
}

struct MappingRecord {
    name: String,
    selector: Option<String>,
    value: Value,
}

struct MappingAsset {
    kind: MappingKind,
    records: Option<Vec<MappingRecord>>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ConfigOrigin {
    PackageDefault,
    Override,
}

impl ConfigOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::PackageDefault => "package default",
            Self::Override => "override",
        }
    }
}

#[derive(Serialize)]
struct ConfigurationJson<'a> {
    origin: ConfigOrigin,
    path: &'a Path,
}

#[derive(Serialize)]
struct MappingSummaryJson<'a> {
    name: &'a str,
    selector: Option<&'a str>,
}

#[derive(Serialize)]
struct AssetJson<'a> {
    status: &'static str,
    source: &'static str,
    items: Vec<MappingSummaryJson<'a>>,
}

#[derive(Serialize)]
struct MappingsJson<'a> {
    files: AssetJson<'a>,
    tables: AssetJson<'a>,
    rules: AssetJson<'a>,
}

#[derive(Serialize)]
struct InventoryJson<'a> {
    schema_version: u8,
    package: &'a str,
    configuration: ConfigurationJson<'a>,
    mappings: MappingsJson<'a>,
}

#[derive(Serialize)]
struct SelectedItemJson<'a> {
    kind: MappingKind,
    name: &'a str,
    selector: Option<&'a str>,
    source: &'static str,
    declaration: &'a Value,
}

#[derive(Serialize)]
struct ItemJson<'a> {
    schema_version: u8,
    package: &'a str,
    configuration: ConfigurationJson<'a>,
    item: SelectedItemJson<'a>,
}

pub fn handle_show(cmd: ShowCommand) -> Result<(), ShowError> {
    let manifest = Manifest::load()?;
    let package = manifest
        .get_package(&cmd.package)
        .ok_or_else(|| ShowError::PackageNotFound(cmd.package.clone()))?;
    let (config_path, config_source) = match cmd.config_dir {
        Some(path) => (path, ConfigOrigin::Override),
        None => (
            resolve_config_path(package, cmd.json)?,
            ConfigOrigin::PackageDefault,
        ),
    };
    let metadata =
        std::fs::metadata(&config_path).map_err(|source| ShowError::ConfigDirectory {
            path: config_path.clone(),
            source,
        })?;
    if !metadata.is_dir() {
        return Err(ShowError::NotDirectory(config_path));
    }
    let configuration = ConfigurationJson {
        origin: config_source,
        path: &config_path,
    };
    if let Some(item) = cmd.item {
        let assets = match qualified_selector(&item) {
            Some((kind, _)) => vec![load_mapping(&config_path, kind)?],
            None => Vec::from(load_all_mappings(&config_path)?),
        };
        let (kind, record) = find_item(&assets, &package.name, &item)?;
        if cmd.json {
            let output = ItemJson {
                schema_version: 1,
                package: &package.name,
                configuration,
                item: SelectedItemJson {
                    kind,
                    name: &record.name,
                    selector: record.selector.as_deref(),
                    source: kind.filename(),
                    declaration: &record.value,
                },
            };
            println!(
                "{}",
                serde_json::to_string(&output).map_err(ShowError::Output)?
            );
        } else {
            print_record(kind, record);
        }
        return Ok(());
    }

    let assets = load_all_mappings(&config_path)?;
    if cmd.json {
        let [files, tables, rules] = &assets;
        let output = InventoryJson {
            schema_version: 1,
            package: &package.name,
            configuration,
            mappings: MappingsJson {
                files: asset_json(files),
                tables: asset_json(tables),
                rules: asset_json(rules),
            },
        };
        println!(
            "{}",
            serde_json::to_string(&output).map_err(ShowError::Output)?
        );
        return Ok(());
    }

    println!("{}", package.name);
    println!(
        "  Configuration: {} ({})",
        config_path.display(),
        config_source.label()
    );
    for asset in assets {
        println!("\n{}:", asset.kind.title());
        match asset.records {
            None => println!("  (not provided)"),
            Some(records) if records.is_empty() => println!("  (empty)"),
            Some(records) => {
                for record in records {
                    let selector = record.selector.as_deref().unwrap_or("no unique selector");
                    println!("  {}  [{selector}]", record.name);
                }
            }
        }
    }
    let override_hint = if matches!(config_source, ConfigOrigin::Override) {
        " --config-dir <directory>"
    } else {
        ""
    };
    println!(
        "\nShow an item: r2x show {} <selector>{override_hint}",
        package.name
    );
    Ok(())
}

fn resolve_config_path(package: &Package, machine_query: bool) -> Result<PathBuf, ShowError> {
    let configs: BTreeSet<(&str, &str)> = package
        .plugins
        .iter()
        .filter_map(|plugin| {
            Some((
                plugin.config_module.as_deref()?,
                plugin.config_class.as_deref()?,
            ))
        })
        .collect();
    if configs.is_empty() {
        return Err(ShowError::MissingConfig(package.name.to_string()));
    }

    let bridge = if machine_query {
        Bridge::get_existing()?
    } else {
        Bridge::get()?
    };
    let mut paths = BTreeSet::new();
    for (module, class) in configs {
        paths.insert(bridge.package_config_path(module, class)?);
    }
    if paths.len() != 1 {
        return Err(ShowError::MultipleConfigs {
            package: package.name.to_string(),
            paths: paths.into_iter().collect(),
        });
    }
    paths
        .into_iter()
        .next()
        .ok_or_else(|| ShowError::MissingConfig(package.name.to_string()))
}

fn asset_json(asset: &MappingAsset) -> AssetJson<'_> {
    let status = match &asset.records {
        None => "not_provided",
        Some(records) if records.is_empty() => "empty",
        Some(_) => "present",
    };
    let items = asset
        .records
        .iter()
        .flatten()
        .map(|record| MappingSummaryJson {
            name: &record.name,
            selector: record.selector.as_deref(),
        })
        .collect();
    AssetJson {
        status,
        source: asset.kind.filename(),
        items,
    }
}

fn qualified_selector(item: &str) -> Option<(MappingKind, &str)> {
    item.split_once(':').and_then(|(prefix, name)| {
        [MappingKind::File, MappingKind::Table, MappingKind::Rule]
            .into_iter()
            .find(|kind| kind.prefix() == prefix)
            .map(|kind| (kind, name))
    })
}

fn find_item<'a>(
    assets: &'a [MappingAsset],
    package: &str,
    item: &str,
) -> Result<(MappingKind, &'a MappingRecord), ShowError> {
    let qualifier = qualified_selector(item);
    let selected = qualifier.map_or(item, |(_, name)| name);
    let mut matches: Vec<_> = assets
        .iter()
        .filter(|asset| qualifier.map_or(true, |(kind, _)| asset.kind == kind))
        .flat_map(|asset| {
            asset
                .records
                .iter()
                .flatten()
                .filter(move |record| {
                    record.name == selected
                        || (asset.kind == MappingKind::Rule
                            && record.value.get("source_type").and_then(Value::as_str)
                                == Some(selected))
                })
                .map(move |record| (asset.kind, record))
        })
        .collect();
    if matches.iter().any(|(_, record)| record.name == selected) {
        matches.retain(|(_, record)| record.name == selected);
    }
    match matches.as_slice() {
        [] => Err(ShowError::ItemNotFound {
            package: package.to_string(),
            item: item.to_string(),
        }),
        [(kind, record)] => Ok((*kind, *record)),
        _ => Err(ShowError::AmbiguousItem {
            item: item.to_string(),
            choices: matches
                .iter()
                .filter_map(|(_, record)| record.selector.clone())
                .collect(),
        }),
    }
}

fn print_record(kind: MappingKind, record: &MappingRecord) {
    println!("{}: {}", kind.label(), record.name);
    let Value::Object(fields) = &record.value else {
        println!("  {}", display_value(&record.value));
        return;
    };
    if let Some(source) = fields.get("source") {
        println!("  source: {}", display_value(source));
    }
    if let Some(Value::Array(operations)) = fields.get("operations") {
        println!("  operations:");
        for (index, operation) in operations.iter().enumerate() {
            if let Some((name, details)) = operation.as_object().and_then(|fields| {
                fields
                    .get("operation")
                    .and_then(Value::as_str)
                    .map(|name| (name, fields))
            }) {
                println!("    {}. {}", index + 1, name);
                for (key, value) in details {
                    if key != "operation" {
                        println!("       {}: {}", key, display_value(value));
                    }
                }
            } else {
                println!("    {}. {}", index + 1, display_value(operation));
            }
        }
    }
    for (key, value) in fields {
        if key == "name" || key == "source" || key == "operations" {
            continue;
        }
        if let Value::Object(entries) = value {
            if !entries.is_empty() {
                println!("  {key}:");
                for (name, value) in entries {
                    println!("    {}: {}", name, display_value(value));
                }
                continue;
            }
        }
        println!("  {}: {}", key, display_value(value));
    }
    if let Some(operations) = fields.get("operations").filter(|value| !value.is_array()) {
        println!("  operations: {}", display_value(operations));
    }
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn load_all_mappings(directory: &Path) -> Result<[MappingAsset; 3], ShowError> {
    Ok([
        load_mapping(directory, MappingKind::File)?,
        load_mapping(directory, MappingKind::Table)?,
        load_mapping(directory, MappingKind::Rule)?,
    ])
}

fn load_mapping(directory: &Path, kind: MappingKind) -> Result<MappingAsset, ShowError> {
    let path = directory.join(kind.filename());
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(MappingAsset {
                kind,
                records: None,
            });
        }
        Err(source) => return Err(ShowError::Read { path, source }),
    };
    let value: Value = strict_json::parse(&contents).map_err(|source| ShowError::Json {
        path: path.clone(),
        source,
    })?;
    let mut records = match value {
        Value::Array(records) => records
            .into_iter()
            .map(|value| {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        if kind != MappingKind::Rule {
                            return None;
                        }
                        let source = value.get("source_type")?;
                        let target = value.get("target_type")?;
                        let mut name =
                            format!("{} -> {}", display_value(source), display_value(target));
                        if let Some(version) = value.get("version") {
                            name.push_str(&format!(" (v{})", display_value(version)));
                        }
                        Some(name)
                    })
                    .ok_or_else(|| ShowError::InvalidMapping {
                        path: path.clone(),
                        reason: "each record must have a name or translation source and target"
                            .to_string(),
                    })?;
                Ok(MappingRecord {
                    name,
                    selector: None,
                    value,
                })
            })
            .collect::<Result<Vec<_>, ShowError>>()?,
        Value::Object(records) if kind == MappingKind::Rule && records.is_empty() => Vec::new(),
        Value::Object(_) if kind == MappingKind::Rule => {
            return Err(ShowError::UnsupportedMapping {
                path,
                reason: "unsupported translation rules object; expected an array of records"
                    .to_string(),
            });
        }
        _ => {
            return Err(ShowError::InvalidMapping {
                path,
                reason: "expected an array of named records".to_string(),
            });
        }
    };
    if kind != MappingKind::Rule {
        let mut names = BTreeSet::new();
        for record in &records {
            if !names.insert(record.name.as_str()) {
                return Err(ShowError::InvalidMapping {
                    path,
                    reason: format!("duplicate name '{}'", record.name),
                });
            }
        }
    }
    assign_selectors(kind, &mut records);
    Ok(MappingAsset {
        kind,
        records: Some(records),
    })
}

fn assign_selectors(kind: MappingKind, records: &mut [MappingRecord]) {
    let mut names = BTreeMap::<String, usize>::new();
    let mut sources = BTreeMap::<String, usize>::new();
    for record in records.iter() {
        *names.entry(record.name.clone()).or_default() += 1;
        if kind == MappingKind::Rule {
            if let Some(source) = record.value.get("source_type").and_then(Value::as_str) {
                *sources.entry(source.to_string()).or_default() += 1;
            }
        }
    }
    for record in records {
        let name = if names.get(&record.name) == Some(&1) {
            Some(record.name.as_str())
        } else if kind == MappingKind::Rule {
            record
                .value
                .get("source_type")
                .and_then(Value::as_str)
                .filter(|source| !names.contains_key(*source) && sources.get(*source) == Some(&1))
        } else {
            None
        };
        record.selector = name.map(|name| format!("{}:{name}", kind.prefix()));
    }
}
