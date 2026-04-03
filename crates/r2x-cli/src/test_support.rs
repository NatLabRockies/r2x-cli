/// Execute a test with an isolated temporary config file by setting `R2X_CONFIG`.
/// This helper serializes all env-var mutation across the crate.
pub(crate) fn with_temp_config(f: impl FnOnce()) {
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    let config_path = dir.path().join("config.toml");
    let previous = std::env::var_os("R2X_CONFIG");

    std::env::set_var("R2X_CONFIG", &config_path);
    f();

    match previous {
        Some(value) => std::env::set_var("R2X_CONFIG", value),
        None => std::env::remove_var("R2X_CONFIG"),
    }
}
