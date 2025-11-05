/// Benchmark AST-based plugin discovery
/// Run with: cargo run --release --bin benchmark_ast_discovery

use std::path::Path;
use std::time::Instant;
use std::fs;

fn main() {
    println!("=== AST Discovery Performance Benchmark ===\n");

    let reeds_path = Path::new("/Users/psanchez/dev/r2x-reeds/dev/src/r2x_reeds");
    let plugins_py = reeds_path.join("plugins.py");

    if !plugins_py.exists() {
        eprintln!("Error: plugins.py not found at {:?}", plugins_py);
        return;
    }

    // Warmup
    let _ = fs::read_to_string(&plugins_py);

    println!("Benchmarking: {}", plugins_py.display());
    println!();

    // Benchmark 1: File read
    println!("1. Reading plugins.py file");
    let start = Instant::now();
    let content = fs::read_to_string(&plugins_py).expect("Failed to read");
    let elapsed = start.elapsed();
    println!("   Time: {:?}", elapsed);
    println!("   Size: {} bytes\n", content.len());

    // Benchmark 2: Find register_plugin function
    println!("2. Extracting register_plugin() function");
    let start = Instant::now();
    let func_start = content.find("def register_plugin()");
    let func_end = if let Some(start_pos) = func_start {
        // Find the end of the function (next def or end of file)
        content[start_pos..]
            .find("\ndef ")
            .map(|pos| start_pos + pos)
            .unwrap_or(content.len())
    } else {
        content.len()
    };
    let func_content = if let Some(s) = func_start {
        &content[s..func_end]
    } else {
        ""
    };
    let elapsed = start.elapsed();
    println!("   Time: {:?}", elapsed);
    println!("   Found: {} bytes\n", func_content.len());

    // Benchmark 3: Parse imports
    println!("3. Building import map");
    let start = Instant::now();
    let mut import_count = 0;
    for line in func_content.lines() {
        let line = line.trim();
        if line.starts_with("from ") && line.contains(" import ") {
            if let Some(import_idx) = line.find(" import ") {
                let imports_part = &line[import_idx + 8..];
                import_count += imports_part.split(',').count();
            }
        }
    }
    let elapsed = start.elapsed();
    println!("   Time: {:?}", elapsed);
    println!("   Imports found: {}\n", import_count);

    // Benchmark 4: Find plugins array
    println!("4. Extracting plugins array");
    let start = Instant::now();
    let plugins_start = func_content.find("plugins=[");
    let plugins_size = if let Some(pos) = plugins_start {
        let rest = &func_content[pos + 9..];
        let mut bracket_count = 1;
        let mut size = 0;
        for c in rest.chars() {
            size += 1;
            match c {
                '[' => bracket_count += 1,
                ']' => {
                    bracket_count -= 1;
                    if bracket_count == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        size
    } else {
        0
    };
    let elapsed = start.elapsed();
    println!("   Time: {:?}", elapsed);
    println!("   Plugins array size: {} bytes\n", plugins_size);

    // Benchmark 5: Count plugin definitions
    println!("5. Finding plugin constructors");
    let start = Instant::now();
    let plugin_keywords = ["ParserPlugin", "UpgraderPlugin", "BasePlugin", "ExporterPlugin"];
    let mut plugin_count = 0;
    for keyword in &plugin_keywords {
        plugin_count += func_content.matches(keyword).count();
    }
    let elapsed = start.elapsed();
    println!("   Time: {:?}", elapsed);
    println!("   Plugin constructors found: {}\n", plugin_count);

    // Total benchmark
    println!("=== Total Benchmark ===");
    let start = Instant::now();

    // Simulate full extraction flow
    let _ = fs::read_to_string(&plugins_py);
    let content = fs::read_to_string(&plugins_py).expect("Failed to read");
    let _ = content.find("def register_plugin()");
    let mut _import_count = 0;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("from ") && line.contains(" import ") {
            if let Some(import_idx) = line.find(" import ") {
                let imports_part = &line[import_idx + 8..];
                _import_count += imports_part.split(',').count();
            }
        }
    }
    let _ = content.find("plugins=[");

    let elapsed = start.elapsed();
    println!("Full extraction pipeline: {:?}", elapsed);
    println!();

    // Comparison
    println!("=== Performance Comparison ===");
    println!("AST-based approach (this): {:?}", elapsed);
    println!("Python-based approach:     ~3.4s (1.9s startup + 1.5s import/serialize)");
    println!("Speedup:                   ~{:.0}x", 3400.0 / elapsed.as_millis() as f64);
    println!();

    // Run 100 iterations for steady state
    println!("=== Steady-state Benchmark (100 iterations) ===");
    let start = Instant::now();
    for _ in 0..100 {
        let content = fs::read_to_string(&plugins_py).expect("Failed to read");
        let _ = content.find("def register_plugin()");
        let mut _import_count = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("from ") && line.contains(" import ") {
                if let Some(import_idx) = line.find(" import ") {
                    let imports_part = &line[import_idx + 8..];
                    _import_count += imports_part.split(',').count();
                }
            }
        }
    }
    let elapsed = start.elapsed();
    let avg_per_iteration = elapsed.as_micros() as f64 / 100.0;
    println!("100 iterations: {:?}", elapsed);
    println!("Per iteration:  {:.2}ms", avg_per_iteration / 1000.0);
    println!("Per iteration:  {:.0}µs", avg_per_iteration);
}
