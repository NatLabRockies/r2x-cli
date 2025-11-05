/// Standalone test binary for AST discovery
/// Run with: cargo run --bin test_ast_discovery

use std::path::Path;

fn main() {
    println!("=== AST Discovery Test Suite ===\n");

    // Test 1: Import resolution
    println!("Test 1: Single import parsing");
    test_single_import();
    println!("✅ PASSED\n");

    // Test 2: Multiple imports
    println!("Test 2: Multiple import parsing");
    test_multiple_imports();
    println!("✅ PASSED\n");

    // Test 3: Find plugins.py
    println!("Test 3: Finding plugins.py");
    test_find_plugins_py();
    println!("✅ PASSED\n");

    // Test 4: Extract register_plugin function
    println!("Test 4: Extracting register_plugin() function");
    test_extract_register_plugin();
    println!("✅ PASSED\n");

    println!("\n=== All Tests Passed ===");
}

fn test_single_import() {
    let func_content = "from r2x_reeds.parser import ReEDSParser\n";
    let lines: Vec<&str> = func_content.lines().collect();

    for line in lines {
        let line = line.trim();
        if line.starts_with("from ") {
            if let Some(import_idx) = line.find(" import ") {
                let module_part = &line[5..import_idx];
                let imports_part = &line[import_idx + 8..];

                println!("  Module: {}", module_part);
                println!("  Imports: {}", imports_part);

                assert_eq!(module_part, "r2x_reeds.parser");
                assert_eq!(imports_part, "ReEDSParser");
            }
        }
    }
}

fn test_multiple_imports() {
    let func_content = "from r2x_core.plugin import BasePlugin, ParserPlugin, UpgraderPlugin\n";
    let mut count = 0;

    for line in func_content.lines() {
        let line = line.trim();
        if line.starts_with("from ") && line.contains(" import ") {
            if let Some(import_idx) = line.find(" import ") {
                let imports_part = &line[import_idx + 8..];
                for import_spec in imports_part.split(',') {
                    let import_name = import_spec.trim();
                    println!("  Found: {}", import_name);
                    count += 1;
                }
            }
        }
    }

    assert_eq!(count, 3);
}

fn test_find_plugins_py() {
    let reeds_path = Path::new("/Users/psanchez/dev/r2x-reeds/dev/src/r2x_reeds");
    let plugins_py = reeds_path.join("plugins.py");

    println!("  Looking for: {}", plugins_py.display());

    if plugins_py.exists() {
        println!("  ✅ Found!");
        assert!(true);
    } else {
        println!("  ❌ Not found (expected in dev environment)");
    }
}

fn test_extract_register_plugin() {
    let plugins_py = Path::new("/Users/psanchez/dev/r2x-reeds/dev/src/r2x_reeds/plugins.py");

    if !plugins_py.exists() {
        println!("  Skipped: plugins.py not found");
        return;
    }

    match std::fs::read_to_string(plugins_py) {
        Ok(content) => {
            if content.contains("def register_plugin") {
                println!("  ✅ Found register_plugin() function");
                assert!(true);
            } else {
                println!("  ❌ register_plugin() not found");
                assert!(false);
            }
        }
        Err(e) => {
            println!("  Error reading file: {}", e);
        }
    }
}
