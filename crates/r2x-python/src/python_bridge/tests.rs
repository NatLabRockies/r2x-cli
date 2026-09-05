use super::*;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn bridge_struct_can_be_created() {
    let _bridge = Bridge { _marker: () };
}

#[test]
fn post_import_log_module_cache_skips_previously_enabled_modules() {
    let module_a = "r2x_test_cache_alpha";
    let module_b = "r2x_test_cache_beta";

    let pending = pending_post_import_log_modules(&[module_a, module_b]);
    assert!(pending.contains(&module_a.to_string()));
    assert!(pending.contains(&module_b.to_string()));

    mark_post_import_log_modules_enabled(&[module_a.to_string()]);

    let pending = pending_post_import_log_modules(&[module_a, module_b]);
    assert!(!pending.contains(&module_a.to_string()));
    assert!(pending.contains(&module_b.to_string()));
}

#[test]
fn recognizes_python_executable_names() {
    assert!(is_python_executable_name("python"));
    assert!(is_python_executable_name("python.exe"));
    assert!(is_python_executable_name("python3"));
    assert!(is_python_executable_name("python3.exe"));
    assert!(is_python_executable_name("python3.12"));
    assert!(is_python_executable_name("python3.12.exe"));
    assert!(is_python_executable_name("PYTHON3.13.EXE"));
    assert!(!is_python_executable_name("pythonw.exe"));
    assert!(!is_python_executable_name("python-3.12.exe"));
}

#[test]
fn normalizes_python_home_bin_dir() {
    let home = PathBuf::from("/opt/python/bin");
    assert_eq!(normalize_python_home(&home), PathBuf::from("/opt/python"));
}

#[test]
fn normalizes_python_home_scripts_dir() {
    let home = PathBuf::from("/opt/python/Scripts");
    assert_eq!(normalize_python_home(&home), PathBuf::from("/opt/python"));
}

#[test]
fn normalizes_python_home_executable() {
    let home = PathBuf::from("/opt/python/python3.12");
    assert_eq!(normalize_python_home(&home), PathBuf::from("/opt/python"));
}

#[test]
fn preserves_python_home_prefix() {
    let home = PathBuf::from("/opt/python/cpython-3.12.9-windows-x86_64-none");
    assert_eq!(normalize_python_home(&home), home);
}

#[test]
fn resolves_python_home_prefix_from_pyvenv_cfg() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let venv_path = temp_dir.path().join(".venv");
    if fs::create_dir_all(&venv_path).is_err() {
        return;
    }

    let expected_prefix = temp_dir.path().join("uv-python-prefix");
    let pyvenv_cfg = format!("home = {}\n", expected_prefix.to_string_lossy());
    if fs::write(venv_path.join("pyvenv.cfg"), pyvenv_cfg).is_err() {
        return;
    }

    let result = resolve_python_home(&venv_path);
    assert!(result.is_ok());
    assert!(result.is_ok_and(|path| path == expected_prefix));
}

#[test]
fn resolves_python_home_from_bin_in_pyvenv_cfg() {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let venv_path = temp_dir.path().join(".venv");
    if fs::create_dir_all(&venv_path).is_err() {
        return;
    }

    let expected_prefix = temp_dir.path().join("python-prefix");
    let home_bin = expected_prefix.join("bin");
    let pyvenv_cfg = format!("home = {}\n", home_bin.to_string_lossy());
    if fs::write(venv_path.join("pyvenv.cfg"), pyvenv_cfg).is_err() {
        return;
    }

    let result = resolve_python_home(&venv_path);
    assert!(result.is_ok());
    assert!(result.is_ok_and(|path| path == expected_prefix));
}
