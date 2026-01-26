//! Dynamic Python library loading
//!
//! This module handles loading the Python shared library at runtime using
//! dlopen (Unix) or LoadLibrary (Windows). The library is kept loaded for
//! the lifetime of the process to enable PyO3 operations.

use crate::errors::BridgeError;
use r2x_logger as logger;
use std::path::Path;

/// Errors during library loading
#[derive(Debug)]
pub enum LoadError {
    /// Failed to load the library
    LoadFailed(String),
    /// Library path doesn't exist
    NotFound(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::LoadFailed(msg) => write!(f, "Failed to load Python library: {}", msg),
            LoadError::NotFound(path) => write!(f, "Python library not found: {}", path),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<LoadError> for BridgeError {
    fn from(e: LoadError) -> Self {
        BridgeError::Initialization(e.to_string())
    }
}

/// Loaded Python library handle
///
/// This struct holds the loaded library and ensures it stays loaded
/// for the lifetime of the program. The library should not be unloaded
/// while PyO3 is in use.
pub struct PythonLoader {
    /// The loaded library - kept as a field to prevent early unload
    _library: libloading::Library,
}

impl PythonLoader {
    /// Load the Python shared library
    ///
    /// On Unix, this uses RTLD_NOW | RTLD_GLOBAL to ensure all symbols
    /// are resolved immediately and made available globally (required by PyO3).
    ///
    /// On Windows, this uses standard LoadLibrary semantics.
    pub fn load(lib_path: &Path) -> Result<Self, LoadError> {
        if !lib_path.exists() {
            return Err(LoadError::NotFound(lib_path.display().to_string()));
        }

        logger::debug(&format!(
            "Loading Python shared library: {}",
            lib_path.display()
        ));

        #[cfg(unix)]
        {
            Self::load_unix(lib_path)
        }

        #[cfg(windows)]
        {
            Self::load_windows(lib_path)
        }
    }

    /// Unix-specific library loading with RTLD_GLOBAL
    #[cfg(unix)]
    fn load_unix(lib_path: &Path) -> Result<Self, LoadError> {
        use libloading::os::unix::Library;

        // RTLD_NOW: Resolve all symbols immediately
        // RTLD_GLOBAL: Make symbols available to subsequently loaded libraries
        // This is required for Python extension modules to work correctly
        let flags = libc::RTLD_NOW | libc::RTLD_GLOBAL;

        let library = unsafe {
            Library::open(Some(lib_path), flags)
                .map_err(|e| LoadError::LoadFailed(format!("{}: {}", lib_path.display(), e)))?
        };

        logger::debug("Python library loaded successfully with RTLD_GLOBAL");

        Ok(Self {
            _library: library.into(),
        })
    }

    /// Windows-specific library loading
    #[cfg(windows)]
    fn load_windows(lib_path: &Path) -> Result<Self, LoadError> {
        // On Windows, we may need to set the DLL search directory
        // to ensure dependent DLLs can be found
        if let Some(parent) = lib_path.parent() {
            unsafe {
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = parent
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();

                // Add the directory to the DLL search path
                extern "system" {
                    fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
                }
                SetDllDirectoryW(wide.as_ptr());
            }
        }

        let library = unsafe {
            libloading::Library::new(lib_path)
                .map_err(|e| LoadError::LoadFailed(format!("{}: {}", lib_path.display(), e)))?
        };

        logger::debug("Python library loaded successfully");

        Ok(Self { _library: library })
    }

    /// Check if the library was loaded successfully
    #[allow(dead_code)]
    pub fn is_loaded(&self) -> bool {
        // The library is loaded if we have a handle
        true
    }
}

impl std::fmt::Debug for PythonLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PythonLoader")
            .field("loaded", &true)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_error_display() {
        let err = LoadError::LoadFailed("test error".to_string());
        assert!(err.to_string().contains("Failed to load"));

        let err = LoadError::NotFound("/path/to/lib".to_string());
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_load_nonexistent() {
        let result = PythonLoader::load(Path::new("/nonexistent/libpython.so"));
        assert!(result.is_err());
    }
}
