use super::*;
use std::collections::HashMap;
use std::fs;
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

    assert_eq!(extractor.infer_argument_type(r#""hello""#), "string");
    assert_eq!(extractor.infer_argument_type("'hello'"), "string");
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

    assert_eq!(extractor.infer_argument_type("42"), "number");
    assert_eq!(extractor.infer_argument_type("3.14"), "float");
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

    assert_eq!(extractor.infer_argument_type("IOType.STDOUT"), "enum_value");
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
        extractor.infer_argument_type("ReEDSParser"),
        "class_reference"
    );
    assert_eq!(extractor.infer_argument_type("MyClass"), "class_reference");
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
