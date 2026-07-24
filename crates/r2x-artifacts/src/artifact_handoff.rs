use r2x_python::plugin_invoker::ArtifactBundle;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const HANDOFF_VERSION: u8 = 1;
const PIPELINE_ARTIFACTS_DIR: &str = "pipeline-artifacts";
const HANDOFFS_DIR: &str = "handoffs";

#[derive(Debug, Error)]
pub enum ArtifactHandoffError {
    #[error("invalid artifact handoff: {0}")]
    Invalid(String),

    #[error("artifact handoff I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("failed to serialize artifact handoff: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// A local, cache-backed handoff for a sidecar-containing pipeline artifact.
///
/// The envelope intentionally carries an identifier and relative entrypoint,
/// never a caller-controlled filesystem path.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactHandoffEnvelope {
    #[serde(rename = "r2x_artifact_handoff")]
    version: u8,
    id: String,
    entrypoint: String,
}

impl ArtifactHandoffEnvelope {
    fn new(id: String, entrypoint: String) -> Self {
        Self {
            version: HANDOFF_VERSION,
            id,
            entrypoint,
        }
    }
}

/// A claimed artifact handoff. Its bundle is deleted when the reader exits.
#[derive(Debug)]
pub struct ClaimedArtifactHandoff {
    bundle: ArtifactBundle,
}

impl ClaimedArtifactHandoff {
    pub fn entrypoint_path(&self) -> PathBuf {
        self.bundle.entrypoint_path()
    }
}

impl Drop for ClaimedArtifactHandoff {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.bundle.root());
    }
}

pub fn parse_handoff_envelope(
    input: &str,
) -> Result<Option<ArtifactHandoffEnvelope>, ArtifactHandoffError> {
    let value: serde_json::Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if !object.contains_key("r2x_artifact_handoff") {
        return Ok(None);
    }

    let envelope = serde_json::from_value(value)?;
    validate_envelope(&envelope)?;
    Ok(Some(envelope))
}

pub fn publish_handoff(
    cache_root: &Path,
    bundle: &ArtifactBundle,
) -> Result<ArtifactHandoffEnvelope, ArtifactHandoffError> {
    validate_bundle(bundle)?;

    let pipeline_root = pipeline_artifacts_root(cache_root)?;
    let source_root = canonical_bundle_root(bundle.root())?;
    if !source_root.starts_with(&pipeline_root) {
        return Err(ArtifactHandoffError::Invalid(format!(
            "pipeline artifact is outside the cache workspace: {}",
            source_root.display()
        )));
    }

    let handoff_root = handoffs_root(cache_root)?;
    let entrypoint = bundle
        .relative_entrypoint()
        .to_str()
        .ok_or_else(|| ArtifactHandoffError::Invalid("entrypoint is not valid UTF-8".to_string()))?
        .to_string();

    for _ in 0..16 {
        let id = next_handoff_id()?;
        let destination = handoff_root.join(&id);
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => continue,
        }

        fs::rename(bundle.root(), &destination)?;
        return Ok(ArtifactHandoffEnvelope::new(id, entrypoint));
    }

    Err(ArtifactHandoffError::Invalid(
        "unable to allocate an artifact handoff identifier".to_string(),
    ))
}

pub fn revoke_handoff(
    cache_root: &Path,
    envelope: &ArtifactHandoffEnvelope,
) -> Result<(), ArtifactHandoffError> {
    validate_envelope(envelope)?;
    let path = handoffs_root(cache_root)?.join(&envelope.id);
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ArtifactHandoffError::Invalid(format!(
            "handoff root is not a directory: {}",
            path.display()
        )));
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

pub fn claim_handoff(
    cache_root: &Path,
    envelope: &ArtifactHandoffEnvelope,
) -> Result<ClaimedArtifactHandoff, ArtifactHandoffError> {
    validate_envelope(envelope)?;

    let handoff_root = handoffs_root(cache_root)?;
    let source = handoff_root.join(&envelope.id);
    let entrypoint = PathBuf::from(&envelope.entrypoint);
    let bundle = ArtifactBundle::new(&source, entrypoint.clone())
        .map_err(|error| ArtifactHandoffError::Invalid(error.to_string()))?;
    validate_bundle(&bundle)?;

    let claimed_root = handoff_root.join(format!(".claimed-{}", envelope.id));
    match fs::symlink_metadata(&claimed_root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => {
            return Err(ArtifactHandoffError::Invalid(format!(
                "artifact handoff is already claimed: {}",
                envelope.id
            )));
        }
    }
    fs::rename(&source, &claimed_root)?;

    let claimed = ClaimedArtifactHandoff {
        bundle: ArtifactBundle::new(claimed_root, entrypoint)
            .map_err(|error| ArtifactHandoffError::Invalid(error.to_string()))?,
    };
    validate_bundle(&claimed.bundle)?;
    Ok(claimed)
}

fn validate_envelope(envelope: &ArtifactHandoffEnvelope) -> Result<(), ArtifactHandoffError> {
    if envelope.version != HANDOFF_VERSION {
        return Err(ArtifactHandoffError::Invalid(format!(
            "unsupported handoff version: {}",
            envelope.version
        )));
    }
    if envelope.id.is_empty()
        || envelope.id.len() > 128
        || !envelope
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ArtifactHandoffError::Invalid(
            "handoff identifier must contain only ASCII letters, digits, '_' or '-'".to_string(),
        ));
    }
    if envelope.entrypoint != "system.json" {
        return Err(ArtifactHandoffError::Invalid(format!(
            "unsupported handoff entrypoint: {}",
            envelope.entrypoint
        )));
    }
    Ok(())
}

fn validate_bundle(bundle: &ArtifactBundle) -> Result<(), ArtifactHandoffError> {
    let root = canonical_bundle_root(bundle.root())?;
    validate_tree(&root)?;

    let entrypoint = bundle.entrypoint_path();
    let metadata = fs::symlink_metadata(&entrypoint)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ArtifactHandoffError::Invalid(format!(
            "artifact entrypoint is not a regular file: {}",
            entrypoint.display()
        )));
    }
    Ok(())
}

fn validate_tree(path: &Path) -> Result<(), ArtifactHandoffError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(ArtifactHandoffError::Invalid(format!(
            "artifact bundle contains a symlink: {}",
            path.display()
        )));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_tree(&entry?.path())?;
        }
    } else if !metadata.is_file() {
        return Err(ArtifactHandoffError::Invalid(format!(
            "artifact bundle contains an unsupported filesystem entry: {}",
            path.display()
        )));
    }
    Ok(())
}

fn canonical_bundle_root(root: &Path) -> Result<PathBuf, ArtifactHandoffError> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ArtifactHandoffError::Invalid(format!(
            "artifact bundle root is not a directory: {}",
            root.display()
        )));
    }
    Ok(fs::canonicalize(root)?)
}

fn pipeline_artifacts_root(cache_root: &Path) -> Result<PathBuf, ArtifactHandoffError> {
    let root = cache_root.join(PIPELINE_ARTIFACTS_DIR);
    fs::create_dir_all(&root)?;
    Ok(fs::canonicalize(root)?)
}

fn handoffs_root(cache_root: &Path) -> Result<PathBuf, ArtifactHandoffError> {
    let pipeline_root = pipeline_artifacts_root(cache_root)?;
    let root = pipeline_root.join(HANDOFFS_DIR);
    match fs::symlink_metadata(&root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ArtifactHandoffError::Invalid(format!(
                    "handoff root is not a directory: {}",
                    root.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        },
        Err(error) => return Err(error.into()),
    }

    let canonical_root = fs::canonicalize(&root)?;
    if !canonical_root.starts_with(&pipeline_root) {
        return Err(ArtifactHandoffError::Invalid(format!(
            "handoff root escapes the pipeline artifact cache: {}",
            canonical_root.display()
        )));
    }
    Ok(canonical_root)
}

fn next_handoff_id() -> Result<String, ArtifactHandoffError> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ArtifactHandoffError::Invalid(format!("system clock error: {error}")))?
        .as_nanos();
    Ok(format!(
        "handoff_{timestamp}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use crate::artifact_handoff::{claim_handoff, parse_handoff_envelope, publish_handoff};
    use r2x_python::plugin_invoker::ArtifactBundle;
    use std::error::Error;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn handoff_moves_and_cleans_sidecar_bundle() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let cache_root = temp.path().join("cache");
        let source_root = cache_root
            .join("pipeline-artifacts")
            .join("run")
            .join("step_0001");
        fs::create_dir_all(source_root.join("system_time_series"))?;
        fs::write(source_root.join("system.json"), "{}")?;
        fs::write(
            source_root.join("system_time_series/time_series_metadata.db"),
            "sidecar",
        )?;
        let bundle = ArtifactBundle::new(&source_root, "system.json")?;

        let envelope = publish_handoff(&cache_root, &bundle)?;
        assert!(!source_root.exists());
        let serialized = serde_json::to_string(&envelope)?;
        let parsed = parse_handoff_envelope(&serialized)?.ok_or("handoff not detected")?;

        let claimed = claim_handoff(&cache_root, &parsed)?;
        let entrypoint = claimed.entrypoint_path();
        assert!(entrypoint.is_file());
        assert!(entrypoint.parent().is_some_and(|parent| parent
            .join("system_time_series/time_series_metadata.db")
            .is_file()));
        let claimed_root = entrypoint
            .parent()
            .ok_or("entrypoint has no parent")?
            .to_path_buf();
        drop(claimed);
        assert!(!claimed_root.exists());
        Ok(())
    }

    #[test]
    fn plain_system_json_is_not_a_handoff() -> Result<(), Box<dyn Error>> {
        assert!(parse_handoff_envelope(r#"{"time_series":{"directory":"series"}}"#)?.is_none());
        Ok(())
    }

    #[test]
    fn handoff_rejects_traversing_identifier() {
        let input = r#"{"r2x_artifact_handoff":1,"id":"../outside","entrypoint":"system.json"}"#;
        assert!(parse_handoff_envelope(input).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn handoff_rejects_symlinked_handoff_directory() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let temp = tempdir()?;
        let cache_root = temp.path().join("cache");
        let source_root = cache_root
            .join("pipeline-artifacts")
            .join("run")
            .join("step_0001");
        fs::create_dir_all(&source_root)?;
        fs::write(source_root.join("system.json"), "{}")?;
        let bundle = ArtifactBundle::new(&source_root, "system.json")?;
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside)?;
        symlink(
            &outside,
            cache_root.join("pipeline-artifacts").join("handoffs"),
        )?;

        assert!(publish_handoff(&cache_root, &bundle).is_err());
        assert!(source_root.exists());
        Ok(())
    }
}
