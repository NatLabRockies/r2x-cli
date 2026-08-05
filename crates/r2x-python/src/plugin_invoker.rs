//! Plugin invocation and execution

use crate::errors::BridgeError;
use r2x_logger as logger;
use r2x_manifest::runtime::{build_runtime_bindings, PluginRole, RuntimeBindings};
use r2x_manifest::types::Plugin;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// A directory-backed plugin artifact with one JSON entrypoint.
///
/// System artifacts can add sidecar files next to the entrypoint. Consumers
/// must therefore retain and pass the complete bundle directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactBundle {
    root: PathBuf,
    entrypoint: PathBuf,
}

impl ArtifactBundle {
    /// Create a bundle rooted at `root` with a relative JSON entrypoint.
    pub fn new(
        root: impl Into<PathBuf>,
        entrypoint: impl Into<PathBuf>,
    ) -> Result<Self, BridgeError> {
        let entrypoint = entrypoint.into();
        validate_relative_entrypoint(&entrypoint)?;
        Ok(Self {
            root: root.into(),
            entrypoint,
        })
    }

    /// Root directory containing the entrypoint and any sidecars.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Relative path of the JSON entrypoint within [`Self::root`].
    pub fn relative_entrypoint(&self) -> &Path {
        &self.entrypoint
    }

    /// Absolute or relative filesystem path to the JSON entrypoint.
    pub fn entrypoint_path(&self) -> PathBuf {
        self.root.join(&self.entrypoint)
    }
}

fn validate_relative_entrypoint(entrypoint: &Path) -> Result<(), BridgeError> {
    if entrypoint.as_os_str().is_empty() {
        return Err(BridgeError::InvalidArtifact(
            "entrypoint cannot be empty".to_string(),
        ));
    }

    let mut has_file_name = false;
    for component in entrypoint.components() {
        match component {
            Component::Normal(_) => has_file_name = true,
            Component::CurDir => {
                return Err(BridgeError::InvalidArtifact(format!(
                    "entrypoint cannot contain '.' components: {}",
                    entrypoint.display()
                )));
            }
            Component::ParentDir => {
                return Err(BridgeError::InvalidArtifact(format!(
                    "entrypoint cannot traverse its bundle root: {}",
                    entrypoint.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(BridgeError::InvalidArtifact(format!(
                    "entrypoint must be relative to its bundle root: {}",
                    entrypoint.display()
                )));
            }
        }
    }
    if !has_file_name {
        return Err(BridgeError::InvalidArtifact(
            "entrypoint must name a file within its bundle".to_string(),
        ));
    }
    Ok(())
}

/// Type of data produced by an artifact-mode plugin invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactOutputKind {
    /// A `System` persisted through `System.to_json(path)`.
    System,
    /// Generic JSON persisted through Python's JSON backend.
    Json,
    /// No replacement artifact was emitted.
    Empty,
}

/// Result of artifact-mode plugin invocation.
#[derive(Debug)]
pub struct PluginArtifactInvocationResult {
    /// The materialized output type, if any.
    pub output_kind: ArtifactOutputKind,
    /// Optional per-phase timings for diagnostics.
    pub timings: Option<PluginInvocationTimings>,
}

/// Timings for a plugin invocation phase
#[derive(Debug)]
pub struct PluginInvocationTimings {
    pub python_invocation: Duration,
    pub serialization: Duration,
}

/// Direct input supplied to a plugin invocation.
#[derive(Clone, Copy, Debug)]
pub enum PluginInput<'a> {
    /// JSON payload received from standard input.
    Json(&'a str),
    /// JSON entrypoint on disk, loaded relative to its sidecar bundle.
    File(&'a Path),
}

/// Materialized result of running a plugin through the Python bridge.
#[derive(Debug)]
pub enum PluginInvocationOutput {
    /// JSON text that should be emitted or written by the caller.
    Json(String),
    /// A System was written directly to the requested output path.
    Persisted,
    /// The plugin intentionally produced no stream output.
    Empty,
}

/// Result of running a plugin through the Python bridge.
#[derive(Debug)]
pub struct PluginInvocationResult {
    /// Materialized plugin output.
    pub output: PluginInvocationOutput,
    /// Optional per-phase timings for diagnostics.
    pub timings: Option<PluginInvocationTimings>,
}

impl crate::python_bridge::Bridge {
    /// Invoke a plugin through the direct CLI interface.
    ///
    /// Direct invocations treat an unused stream input as an error, redirect
    /// plugin writes away from stdout, and can persist System results directly
    /// to a durable JSON entrypoint.
    pub fn invoke_plugin_direct(
        &self,
        target: &str,
        config_json: &str,
        input: Option<PluginInput<'_>>,
        output_path: Option<&Path>,
        plugin_metadata: Option<&Plugin>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        let runtime_bindings = plugin_metadata.map(build_runtime_bindings);

        if runtime_bindings
            .as_ref()
            .is_some_and(|bindings| bindings.role == PluginRole::Upgrader)
        {
            if input.is_some() {
                return Err(BridgeError::Stream(
                    "upgrader plugins do not accept System input".to_string(),
                ));
            }
            return self.invoke_upgrader_plugin(
                target,
                config_json,
                runtime_bindings.as_ref(),
                plugin_metadata,
                true,
            );
        }

        self.invoke_plugin_regular_direct(
            target,
            config_json,
            input,
            output_path,
            runtime_bindings.as_ref(),
        )
    }

    pub fn invoke_plugin_with_bindings(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        if let Some(bindings) = runtime_bindings {
            if bindings.role == PluginRole::Upgrader {
                logger::debug("Routing to upgrader plugin handler (runtime bindings)");
                return self.invoke_upgrader_plugin(
                    target,
                    config_json,
                    Some(bindings),
                    None,
                    false,
                );
            }
        }

        self.invoke_plugin_regular(target, config_json, stdin_json, runtime_bindings)
    }

    /// Save a System artifact as an infrasys ZIP archive.
    pub fn save_system_artifact_as_zip(
        &self,
        input: &ArtifactBundle,
        output: &Path,
    ) -> Result<(), BridgeError> {
        Self::save_system_artifact_as_zip_native(input, output)
    }

    /// Artifact-mode counterpart of [`Self::invoke_plugin_with_bindings`].
    pub fn invoke_plugin_with_artifact_bindings(
        &self,
        target: &str,
        config_json: &str,
        input: Option<&ArtifactBundle>,
        output: &ArtifactBundle,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginArtifactInvocationResult, BridgeError> {
        if runtime_bindings.is_some_and(|bindings| bindings.role == PluginRole::Upgrader) {
            return Err(BridgeError::UnsupportedArtifactMode(
                "upgrader plugins are not yet supported because registered SYSTEM steps still serialize payloads through Rust".to_string(),
            ));
        }

        validate_output_bundle(input, output)?;

        self.invoke_plugin_regular_with_artifacts(
            target,
            config_json,
            input,
            output,
            runtime_bindings,
        )
    }
}

fn validate_output_bundle(
    input: Option<&ArtifactBundle>,
    output: &ArtifactBundle,
) -> Result<(), BridgeError> {
    if input.is_some_and(|input| input.root() == output.root()) {
        return Err(BridgeError::InvalidArtifact(
            "input and output bundles must use different roots".to_string(),
        ));
    }

    let metadata = match fs::symlink_metadata(output.root()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BridgeError::InvalidArtifact(format!(
            "output bundle root must be a directory: {}",
            output.root().display()
        )));
    }
    if fs::read_dir(output.root())?.next().transpose()?.is_some() {
        return Err(BridgeError::InvalidArtifact(format!(
            "output bundle root must be empty: {}",
            output.root().display()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::plugin_invoker::*;
    use crate::python_bridge::Bridge;
    use r2x_manifest::runtime::{PluginRole, RuntimeBindings};
    use r2x_manifest::types::PluginType;
    use std::error::Error;
    use tempfile::tempdir;

    #[test]
    fn plugin_invocation_result_basics() {
        let result = PluginInvocationResult {
            output: PluginInvocationOutput::Empty,
            timings: None,
        };
        assert!(matches!(result.output, PluginInvocationOutput::Empty));
    }

    #[test]
    fn artifact_bundle_rejects_absolute_and_traversing_entrypoints() {
        let absolute = ArtifactBundle::new("bundle", "/tmp/system.json");
        assert!(absolute.is_err());

        let traversal = ArtifactBundle::new("bundle", "../system.json");
        assert!(traversal.is_err());

        let bundle = ArtifactBundle::new("bundle", "nested/system.json");
        assert!(bundle.is_ok());

        let directory = ArtifactBundle::new("bundle", ".");
        assert!(directory.is_err());

        let current_directory = ArtifactBundle::new("bundle", "./system.json");
        assert!(current_directory.is_err());
    }

    #[test]
    fn artifact_mode_rejects_upgraders_until_their_payload_path_is_native(
    ) -> Result<(), BridgeError> {
        let bridge = Bridge::for_tests();
        let output = ArtifactBundle::new("bundle", "system.json")?;
        let bindings = RuntimeBindings {
            entry_module: "plugin".to_string(),
            entry_name: "Upgrader".to_string(),
            plugin_type: PluginType::Class,
            role: PluginRole::Upgrader,
            call_method: Some("run".to_string()),
            config: None,
            parameters: Vec::new(),
            requires_store: false,
        };

        let error = bridge.invoke_plugin_with_artifact_bindings(
            "plugin:Upgrader",
            "{}",
            None,
            &output,
            Some(&bindings),
        );
        assert!(matches!(
            error,
            Err(BridgeError::UnsupportedArtifactMode(_))
        ));
        Ok(())
    }

    #[test]
    fn artifact_mode_rejects_nonempty_output_bundles() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let output_root = temp.path().join("output");
        std::fs::create_dir_all(&output_root)?;
        std::fs::write(output_root.join("stale.h5"), "stale")?;
        let output = ArtifactBundle::new(&output_root, "system.json")?;

        let result = Bridge::for_tests().invoke_plugin_with_artifact_bindings(
            "missing:plugin",
            "{}",
            None,
            &output,
            None,
        );

        assert!(matches!(result, Err(BridgeError::InvalidArtifact(_))));
        assert!(output_root.join("stale.h5").exists());
        Ok(())
    }
}
