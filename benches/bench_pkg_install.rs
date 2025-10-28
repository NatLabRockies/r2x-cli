use criterion::{black_box, criterion_group, criterion_main, Criterion};
use r2x::entrypoints;
use std::path::PathBuf;
use std::process::Command;

// Helper to get the venv path (mirrors r2x::python::venv::get_venv_path logic)
fn get_venv_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cache_dir = dirs::cache_dir().ok_or("No cache directory found")?;
    Ok(cache_dir.join("r2x").join("venv"))
}

// Helper to get the Python exe path (mirrors r2x::python::venv::get_venv_python logic)
fn get_venv_python(venv_path: &PathBuf) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let python_exe = if cfg!(windows) {
        venv_path.join("Scripts/python.exe")
    } else {
        venv_path.join("bin/python3")
    };

    if !python_exe.exists() {
        return Err(format!("Python executable not found at: {}", python_exe.display()).into());
    }
    Ok(python_exe)
}

fn bench_plugin_install(c: &mut Criterion) {
    let venv_path = get_venv_path().expect("Failed to get venv path");
    let python_exe = get_venv_python(&venv_path).expect("Failed to get Python exe");

    c.bench_function("plugin_install_pypi", |b| {
        b.iter(|| {
            let output = Command::new("uv")
                .args([
                    "pip",
                    "install",
                    "r2x-reeds",
                    "--python",
                    python_exe.to_str().unwrap(),
                ])
                .output()
                .expect("Install failed");
            black_box(&output); // Prevent compiler optimization
            assert!(output.status.success(), "Install failed: {:?}", output);
        });
    });
}

fn bench_plugin_uninstall(c: &mut Criterion) {
    let venv_path = get_venv_path().expect("Failed to get venv path");
    let python_exe = get_venv_python(&venv_path).expect("Failed to get Python exe");

    c.bench_function("plugin_uninstall_pypi", |b| {
        b.iter(|| {
            // Use sh -c to pipe "y" to uv (avoids interactive prompts)
            let output = Command::new("sh")
                .args([
                    "-c",
                    &format!(
                        "echo y | uv pip uninstall r2x-reeds --python {}",
                        python_exe.to_str().unwrap()
                    ),
                ])
                .output()
                .expect("Uninstall failed");
            black_box(&output); // Prevent compiler optimization
            assert!(output.status.success(), "Uninstall failed: {:?}", output);
        });
    });
}

fn bench_entry_point_discovery(c: &mut Criterion) {
    // Benchmark entry point discovery (assumes plugins are installed)
    c.bench_function("entry_point_discovery", |b| {
        b.iter(|| {
            let result = entrypoints::discover_all_entry_points();
            black_box(&result);
            assert!(result.is_ok());
        });
    });
}

// Configure Criterion with 10 samples
fn configure_criterion() -> Criterion {
    Criterion::default().sample_size(10)
}

criterion_group! {
    name = benches;
    config = configure_criterion();
    targets = bench_plugin_install, bench_plugin_uninstall, bench_entry_point_discovery
}
criterion_main!(benches);
