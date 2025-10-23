//! Unit tests for r2x modules

#[test]
fn test_plugin_registry_is_empty() {
    use std::collections::HashMap;
    let registry = r2x::python::plugin::PluginRegistry {
        parsers: HashMap::new(),
        exporters: HashMap::new(),
        modifiers: HashMap::new(),
        filters: HashMap::new(),
    };

    assert!(registry.is_empty());
}

#[test]
fn test_venv_path_helpers() {
    let venv_path = r2x::python::venv::get_venv_path();
    assert!(venv_path.is_ok());

    let path = venv_path.unwrap();
    assert!(path.to_string_lossy().contains("r2x"));
    assert!(path.to_string_lossy().contains("venv"));
}
