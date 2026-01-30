use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during Python bridge operations
#[derive(Error, Debug)]
pub enum BridgeError {
    #[error("Python error: {0}")]
    Python(String),

    #[error("Failed to import module '{0}': {1}")]
    Import(String, String),

    #[error("Python venv not found or invalid at: {0}")]
    VenvNotFound(PathBuf),

    #[error("r2x-core is not installed in the Python environment")]
    R2XCoreNotInstalled,

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),

    #[error("Failed to initialize Python interpreter: {0}")]
    Initialization(String),

    #[error("Python library not found: {0}")]
    PythonLibraryNotFound(String),

    #[error("Plugin '{0}' not found")]
    PluginNotFound(String),

    #[error("Invalid entry point format: {0}")]
    InvalidEntryPoint(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

/// Generic conversion from PyErr to BridgeError.
///
/// NOTE: This conversion loses the Python traceback information!
/// For user-facing errors where tracebacks are important (plugin failures,
/// config instantiation, etc.), use `format_python_error()` or
/// `format_exception_value()` from plugin_regular.rs instead.
impl From<pyo3::PyErr> for BridgeError {
    fn from(err: pyo3::PyErr) -> Self {
        BridgeError::Python(format!("{}", err))
    }
}
