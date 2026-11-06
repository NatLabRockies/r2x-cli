use std::ffi::{OsStr, OsString};

pub fn is_requested<'a>(args: impl IntoIterator<Item = &'a OsString>) -> bool {
    let mut command: Option<&OsStr> = None;
    let mut json = false;
    for arg in args {
        if arg == "--" {
            break;
        }
        if arg == "--json" {
            json = true;
        }
        if command.is_none() && !arg.to_string_lossy().starts_with('-') {
            command = Some(arg.as_os_str());
        }
    }
    json && matches!(command, Some(command) if command == "list" || command == "show")
}

pub fn emit_error(code: &str, message: &str, candidates: &[String]) {
    eprintln!(
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "error": { "code": code, "message": message, "candidates": candidates }
        })
    );
}
