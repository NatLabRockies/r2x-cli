use super::*;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use tempfile::{NamedTempFile, TempDir};

#[test]
fn test_infer_argument_type_string() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
    };

    assert_eq!(extractor.infer_argument_type(r#""hello""#), "string");
    assert_eq!(extractor.infer_argument_type("'hello'"), "string");
}

#[test]
fn test_infer_argument_type_number() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
    };

    assert_eq!(extractor.infer_argument_type("42"), "number");
    assert_eq!(extractor.infer_argument_type("3.14"), "float");
}

#[test]
fn test_infer_argument_type_enum() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
    };

    assert_eq!(extractor.infer_argument_type("IOType.STDOUT"), "enum_value");
}

#[test]
fn test_infer_argument_type_class() {
    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        content: "def register_plugin(): pass".to_string(),
        import_map: HashMap::new(),
    };

    assert_eq!(
        extractor.infer_argument_type("ReEDSParser"),
        "class_reference"
    );
    assert_eq!(extractor.infer_argument_type("MyClass"), "class_reference");
}

#[test]
fn test_extract_from_real_file_with_dynamic_types() -> Result<()> {
    let content = r#"
from r2x_core.package import Package

def register_plugin() -> Package:
    return Package(
        name="r2x-reeds",
        plugins=[
            ParserPlugin(
                name="reeds-parser",
                obj=ReEDSParser,
                config=ReEDSConfig,
                call_method="build_system",
                io_type=IOType.STDOUT,
            ),
            CustomUpgrader(
                name="custom-upgrader",
                obj=CustomClass,
            ),
        ]
    )
"#;

    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(content.as_bytes())?;
    temp_file.flush()?;

    let extractor = PluginExtractor::new(temp_file.path().to_path_buf())?;
    let plugins = extractor.extract_plugins()?;

    assert!(plugins.len() >= 1);

    Ok(())
}

#[test]
fn test_resolve_references_across_files() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let package_root = temp_dir.path().join("src").join("r2x_reeds");
    fs::create_dir_all(&package_root)?;

    let plugins_py = package_root.join("plugins.py");
    let upgrader_py = package_root.join("upgrader.py");

    let plugins_content = r#"
def register_plugin():
    from .upgrader import ReEDSUpgrader

    return Package(
        name="r2x-reeds",
        plugins=[
            UpgraderPlugin(
                name="reeds-upgrader",
                obj=ReEDSUpgrader,
            ),
        ],
    )
"#;
    fs::write(&plugins_py, plugins_content)?;

    let upgrader_content = r#"
class ReEDSUpgrader:
    def __init__(self, source_path: str, dry_run: bool = False):
        self.source_path = source_path
        self.dry_run = dry_run
"#;
    fs::write(&upgrader_py, upgrader_content)?;

    let extractor = PluginExtractor::new(plugins_py.clone())?;
    assert!(extractor.import_map.contains_key("ReEDSUpgrader"));
    let mut plugins = extractor.extract_plugins()?;
    assert_eq!(plugins.len(), 1);

    extractor.resolve_references(&mut plugins[0], package_root.as_path(), "r2x-reeds")?;

    assert_eq!(plugins[0].resolved_references.len(), 1);
    let resolved = &plugins[0].resolved_references[0];
    assert_eq!(resolved.name, "ReEDSUpgrader");
    assert_eq!(resolved.module, "r2x_reeds.upgrader");
    assert_eq!(resolved.ref_type, "class");
    assert_eq!(resolved.parameters.len(), 2);
    assert_eq!(resolved.parameters[0].name, "source_path");
    assert!(resolved
        .source_file
        .as_ref()
        .map(|path| path.ends_with("upgrader.py"))
        .unwrap_or(false));

    Ok(())
}

#[test]
fn test_resolve_references_nested_module_path() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let package_root = temp_dir.path().join("src").join("pkg_name");
    let nested_dir = package_root.join("upgrader");
    fs::create_dir_all(&nested_dir)?;

    let plugins_py = package_root.join("plugins.py");
    let plugins_content = r#"
def register_plugin():
    from pkg_name.upgrader.data_upgrader import NestedUpgrader

    return Package(
        name="pkg-name",
        plugins=[
            UpgraderPlugin(
                name="nested-upgrader",
                obj=NestedUpgrader,
            ),
        ],
    )
"#;
    fs::write(&plugins_py, plugins_content)?;

    let nested_content = r#"
class NestedUpgrader(PluginUpgrader):
    def __init__(self, level: int):
        self.level = level
"#;
    let nested_file = nested_dir.join("data_upgrader.py");
    fs::write(&nested_file, nested_content)?;

    let extractor = PluginExtractor::new(plugins_py.clone())?;
    let mut plugins = extractor.extract_plugins()?;
    assert_eq!(plugins.len(), 1);

    extractor.resolve_references(&mut plugins[0], package_root.as_path(), "pkg-name")?;

    assert_eq!(plugins[0].resolved_references.len(), 1);
    let resolved = &plugins[0].resolved_references[0];
    assert_eq!(resolved.module, "pkg_name.upgrader.data_upgrader");

    Ok(())
}

#[test]
fn test_extract_class_parameters_from_multiline_init() -> Result<()> {
    let content = r#"
class SampleUpgrader(PluginUpgrader):
    def __init__(
        self,
        path: Path | str,
        steps: list[UpgradeStep] | None = None,
        **kwargs: Any,
    ) -> None:
        pass
"#;

    let extractor = PluginExtractor {
        python_file_path: PathBuf::from("test.py"),
        content: content.to_string(),
        import_map: HashMap::new(),
    };

    let params = extractor.extract_class_parameters_from_content(content, "SampleUpgrader")?;
    assert!(params.iter().any(|p| p.name == "path"));
    Ok(())
}
