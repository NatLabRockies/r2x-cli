use crate::extractor::PluginExtractor;
use anyhow::Result;
use r2x_manifest::execution_types::{ImplementationType, PluginKind};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_infer_argument_type_string() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        package_root: PathBuf::from("."),
        package_prefix: "test".to_string(),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
        current_module: "test.module".to_string(),
    };

    assert_eq!(PluginExtractor::infer_argument_type(r#""hello""#), "string");
    assert_eq!(PluginExtractor::infer_argument_type("'hello'"), "string");
}

#[test]
fn test_infer_argument_type_number() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        package_root: PathBuf::from("."),
        package_prefix: "test".to_string(),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
        current_module: "test.module".to_string(),
    };

    assert_eq!(PluginExtractor::infer_argument_type("42"), "number");
    assert_eq!(PluginExtractor::infer_argument_type("3.14"), "float");
}

#[test]
fn test_infer_argument_type_enum() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        package_root: PathBuf::from("."),
        package_prefix: "test".to_string(),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
        current_module: "test.module".to_string(),
    };

    assert_eq!(
        PluginExtractor::infer_argument_type("IOType.STDOUT"),
        "enum_value"
    );
}

#[test]
fn test_infer_argument_type_class() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        package_root: PathBuf::from("."),
        package_prefix: "test".to_string(),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
        current_module: "test.module".to_string(),
    };

    assert_eq!(
        PluginExtractor::infer_argument_type("ReEDSParser"),
        "class_reference"
    );
    assert_eq!(
        PluginExtractor::infer_argument_type("MyClass"),
        "class_reference"
    );
}

#[test]
fn test_extract_plugins_from_package_constructor_style() -> Result<()> {
    let content = r#"
from r2x_core import Package, ParserPlugin, UpgraderPlugin, BasePlugin

class ReEDSConfig: ...

class ReEDSParser:
    def __init__(self, config, path: str, data_store=None):
        pass

class ReEDSUpgrader:
    def __init__(self, path: str, steps=None):
        pass

def add_pcm_defaults(system): ...

def register_plugin() -> Package:
    return Package(
        name="r2x-reeds",
        plugins=[
            ParserPlugin(
                name="reeds-parser",
                obj=ReEDSParser,
                call_method="build_system",
                config=ReEDSConfig,
            ),
            UpgraderPlugin(
                name="reeds-upgrader",
                obj=ReEDSUpgrader,
            ),
            BasePlugin(
                name="add-pcm-defaults",
                obj=add_pcm_defaults,
            ),
        ],
    )
"#;

    let temp_dir = TempDir::new()?;
    let pkg_root = temp_dir.path().join("r2x_reeds");
    fs::create_dir_all(&pkg_root)?;
    let plugin_file = pkg_root.join("plugin.py");
    fs::write(&plugin_file, content)?;

    let extractor = PluginExtractor::new(
        plugin_file,
        "r2x_reeds.plugin".to_string(),
        pkg_root.clone(),
    )?;
    let plugins = extractor.extract_plugins()?;

    assert_eq!(plugins.len(), 3);
    assert_eq!(plugins[0].name, "reeds-parser");
    assert_eq!(plugins[0].entry, "r2x_reeds.plugin.ReEDSParser");
    assert_eq!(plugins[0].kind, PluginKind::Parser);
    assert_eq!(
        plugins[0].invocation.method.as_deref(),
        Some("build_system")
    );
    assert!(!plugins[0].invocation.constructor.is_empty());
    assert_eq!(plugins[1].kind, PluginKind::Upgrader);
    assert_eq!(
        plugins[2].invocation.implementation,
        ImplementationType::Function
    );

    Ok(())
}

#[test]
fn test_extract_plugins_from_manifest_add_style() -> Result<()> {
    let content = r#"
from r2x_core import PluginManifest, PluginSpec

class DemoParser:
    def __init__(self, config, json_path: str):
        self.config = config

manifest = PluginManifest(package="demo")

manifest.add(PluginSpec.parser(name="demo.parser", entry=DemoParser))
"#;

    let temp_dir = TempDir::new()?;
    let pkg_root = temp_dir.path().join("demo");
    fs::create_dir_all(&pkg_root)?;
    let plugin_file = pkg_root.join("plugin.py");
    fs::write(&plugin_file, content)?;

    let extractor = PluginExtractor::new(plugin_file, "demo.plugin".to_string(), pkg_root.clone())?;
    let plugins = extractor.extract_plugins()?;

    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name, "demo.parser");
    assert_eq!(plugins[0].entry, "demo.plugin.DemoParser");
    assert_eq!(plugins[0].kind, PluginKind::Parser);
    assert_eq!(
        plugins[0].invocation.implementation,
        ImplementationType::Class
    );
    assert!(!plugins[0].invocation.constructor.is_empty());

    Ok(())
}

#[test]
fn test_extract_parameters_with_inline_comments() -> Result<()> {
    let content = r"
class TestExporter:
    def __init__(
        self,
        data_store=None,
        output_path: str | None = None,
        db=None,  # Allow passing existing DB for testing
        solve_year: int | None = None,  # ReEDS field for filename association
        weather_year: int | None = None,  # ReEDS field for filename association
    ):
        pass
";

    let temp_dir = TempDir::new()?;
    let pkg_root = temp_dir.path().join("test_pkg");
    fs::create_dir_all(&pkg_root)?;
    let test_file = pkg_root.join("test.py");
    fs::write(&test_file, content)?;

    let extractor = PluginExtractor::new(test_file, "test_pkg.test".to_string(), pkg_root.clone())?;

    let params = extractor.extract_class_parameters_from_content(content, "TestExporter")?;

    // Should extract 5 parameters (excluding self)
    assert_eq!(params.len(), 5);

    // Verify parameter names don't contain comments
    assert_eq!(params[0].name, "data_store");
    assert_eq!(params[1].name, "output_path");
    assert_eq!(params[2].name, "db");
    assert_eq!(params[3].name, "solve_year");
    assert_eq!(params[4].name, "weather_year");

    // Verify defaults don't contain comments
    assert_eq!(params[2].default.as_deref(), Some("None"));
    assert_eq!(params[3].default.as_deref(), Some("None"));
    assert_eq!(params[4].default.as_deref(), Some("None"));

    // Verify annotations don't contain comments
    assert_eq!(params[3].annotation.as_deref(), Some("int | None"));
    assert_eq!(params[4].annotation.as_deref(), Some("int | None"));

    Ok(())
}

#[test]
fn test_extract_config_fields_with_inline_comments() -> Result<()> {
    let content = r"
class TestConfig:
    model_name: str
    template: str | None = None  # Template file path
    simulation_config: dict | None = None  # Simulation configuration
";

    let temp_dir = TempDir::new()?;
    let pkg_root = temp_dir.path().join("test_pkg");
    fs::create_dir_all(&pkg_root)?;
    let test_file = pkg_root.join("test.py");
    fs::write(&test_file, content)?;

    let extractor = PluginExtractor::new(test_file, "test_pkg.test".to_string(), pkg_root.clone())?;

    let fields = extractor.extract_config_fields("test_pkg.test", "TestConfig");

    // Should extract 3 fields
    assert_eq!(fields.len(), 3);

    // Verify field names don't contain comments
    assert_eq!(fields[0].name, "model_name");
    assert_eq!(fields[1].name, "template");
    assert_eq!(fields[2].name, "simulation_config");

    // Verify defaults don't contain comments
    assert_eq!(fields[1].default.as_deref(), Some("None"));
    assert_eq!(fields[2].default.as_deref(), Some("None"));

    // Verify types don't contain comments
    assert_eq!(fields[0].types, vec!["str"]);
    assert_eq!(fields[1].types, vec!["str", "None"]);
    assert_eq!(fields[2].types, vec!["dict", "None"]);

    Ok(())
}

#[test]
fn test_extract_multiple_config_fields_separately() -> Result<()> {
    let content = r#"
class PLEXOSConfig:
    model_name: Annotated[str, Field(description="Name of the PLEXOS model.")]
    timeseries_dir: Annotated[
        DirectoryPath | None,
        Field(
            description="Optional subdirectory containing time series files.",
            default=None,
        ),
    ]
    horizon_year: Annotated[int | None, Field(description="Horizon year", default=None)]
    template: Annotated[
        FilePath | None, Field(description="File to the XML to use as template.")
    ] = None
    simulation_config: Annotated[SimulationConfig | None, Field(description="Simulation configuration")] = (
        None
    )
"#;

    let temp_dir = TempDir::new()?;
    let pkg_root = temp_dir.path().join("test_pkg");
    fs::create_dir_all(&pkg_root)?;
    let test_file = pkg_root.join("test.py");
    fs::write(&test_file, content)?;

    let extractor = PluginExtractor::new(test_file, "test_pkg.test".to_string(), pkg_root.clone())?;

    let fields = extractor.extract_config_fields("test_pkg.test", "PLEXOSConfig");

    // Should extract 5 separate fields, not concatenate them
    assert_eq!(
        fields.len(),
        5,
        "Expected 5 separate fields but got {}",
        fields.len()
    );

    // Verify each field is extracted separately
    assert_eq!(fields[0].name, "model_name");
    assert_eq!(fields[1].name, "timeseries_dir");
    assert_eq!(fields[2].name, "horizon_year");
    assert_eq!(fields[3].name, "template");
    assert_eq!(fields[4].name, "simulation_config");

    // Verify model_name doesn't contain timeseries_dir content
    let model_name_types_str = fields[0].types.join(" | ");
    assert!(
        !model_name_types_str.contains("timeseries_dir"),
        "model_name types should not contain timeseries_dir: {}",
        model_name_types_str
    );

    // Verify timeseries_dir doesn't contain horizon_year content
    let timeseries_types_str = fields[1].types.join(" | ");
    assert!(
        !timeseries_types_str.contains("horizon_year"),
        "timeseries_dir types should not contain horizon_year: {}",
        timeseries_types_str
    );

    Ok(())
}
