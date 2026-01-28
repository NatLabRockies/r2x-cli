pub(super) const AUTO_PROVIDED_PARAMS: &[&str] = &[
    "store",
    "data_store",
    "stdin",
    "system",
    "path",
    "folder_path",
    "config",
];

pub(super) const JSON_PATH_FIELDS: &[&str] = &["json_path", "path"];

pub(super) const STORE_FIELD_KEYS: &[&str] = &["path", "store", "store_path"];

pub(super) const PATH_FALLBACK_KEYS: &[&str] = &["path", "store_path"];

pub(super) const FOLDER_FIELD_KEYS: &[&str] = &["folder_path", "store_path", "path"];

pub(super) const DEFAULT_OUTPUT_ROOT: &str = "/tmp/r2x-output";
