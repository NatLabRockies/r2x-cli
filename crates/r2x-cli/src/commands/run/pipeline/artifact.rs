use crate::artifact_handoff::{publish_handoff, revoke_handoff};
use crate::commands::run::RunError;
use crate::errors::PipelineError;
use r2x_config::Config;
use r2x_logger as logger;
use r2x_python::plugin_invoker::ArtifactBundle;
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) struct PipelineArtifactWorkspace {
    root: PathBuf,
}

impl PipelineArtifactWorkspace {
    pub(super) fn create() -> Result<Self, RunError> {
        let mut config = Config::load().map_err(|error| RunError::Config(error.to_string()))?;
        let cache_root = config
            .ensure_cache_path()
            .map_err(|error| RunError::Config(error.to_string()))?;
        let parent = PathBuf::from(cache_root).join("pipeline-artifacts");
        fs::create_dir_all(&parent).map_err(PipelineError::Io)?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| RunError::Config(format!("System clock error: {error}")))?
            .as_nanos();
        let root = parent.join(format!(
            "run_{timestamp}_{}_{}",
            std::process::id(),
            next_workspace_id()
        ));
        fs::create_dir(&root).map_err(PipelineError::Io)?;
        Ok(Self { root })
    }

    pub(super) fn step_bundle(&self, step_index: usize) -> Result<ArtifactBundle, RunError> {
        ArtifactBundle::new(
            self.root.join(format!("step_{step_index:04}")),
            "system.json",
        )
        .map_err(RunError::Bridge)
    }
}

impl Drop for PipelineArtifactWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn write_bundle_output(
    bundle: &ArtifactBundle,
    output_path: Option<&Path>,
    zip_output: bool,
    suppress_stdout: bool,
) -> Result<(), RunError> {
    if let Some(output_path) = output_path {
        if zip_output {
            save_bundle_as_zip(bundle, output_path)?;
        } else {
            copy_bundle_to_output(bundle, output_path)?;
        }
    } else if !suppress_stdout {
        let mut has_sidecars = false;
        for entry in fs::read_dir(bundle.root()).map_err(PipelineError::Io)? {
            let entry = entry.map_err(PipelineError::Io)?;
            if entry.path() != bundle.entrypoint_path() {
                has_sidecars = true;
                break;
            }
        }
        if has_sidecars {
            let mut config = Config::load().map_err(|error| RunError::Config(error.to_string()))?;
            let cache_root = PathBuf::from(
                config
                    .ensure_cache_path()
                    .map_err(|error| RunError::Config(error.to_string()))?,
            );
            let handoff = publish_handoff(&cache_root, bundle)
                .map_err(|error| RunError::Config(error.to_string()))?;
            let serialized = match serde_json::to_string(&handoff) {
                Ok(serialized) => serialized,
                Err(error) => {
                    let _ = revoke_handoff(&cache_root, &handoff);
                    return Err(RunError::Config(format!(
                        "Failed to serialize artifact handoff: {error}"
                    )));
                }
            };

            let stdout = io::stdout();
            let mut output = stdout.lock();
            if let Err(error) = writeln!(output, "{serialized}") {
                let _ = revoke_handoff(&cache_root, &handoff);
                return Err(RunError::Pipeline(PipelineError::Io(error)));
            }
            return Ok(());
        }
        let mut input = fs::File::open(bundle.entrypoint_path()).map_err(PipelineError::Io)?;
        let stdout = io::stdout();
        let mut output = stdout.lock();
        io::copy(&mut input, &mut output).map_err(PipelineError::Io)?;
    }
    Ok(())
}

fn save_bundle_as_zip(bundle: &ArtifactBundle, output_path: &Path) -> Result<(), RunError> {
    let extension = output_path
        .extension()
        .and_then(|extension| extension.to_str());
    if !extension.is_some_and(|extension| extension == "zip") {
        return Err(RunError::Config(
            "ZIP pipeline output must use a lowercase .zip filename".to_string(),
        ));
    }

    let archive_base = output_path.with_extension("");
    ensure_destination_absent(output_path)?;
    ensure_destination_absent(&archive_base)?;

    let bridge = r2x_python::python_bridge::Bridge::get().map_err(RunError::Bridge)?;
    bridge
        .save_system_artifact_as_zip(bundle, output_path)
        .map_err(RunError::Bridge)
}

fn copy_bundle_to_output(bundle: &ArtifactBundle, output_path: &Path) -> Result<(), RunError> {
    let source_entrypoint = bundle.entrypoint_path();
    if bundle
        .relative_entrypoint()
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return Err(RunError::Config(
            "Pipeline output entrypoint must be at the artifact bundle root".to_string(),
        ));
    }
    let output_parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent).map_err(PipelineError::Io)?;
    let output_name = output_path.file_name().ok_or_else(|| {
        RunError::Config(format!(
            "Pipeline output must name a file: {}",
            output_path.display()
        ))
    })?;

    validate_source_entry(bundle.root())?;

    let entries = fs::read_dir(bundle.root())
        .map_err(PipelineError::Io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(PipelineError::Io)?;
    let mut destinations = HashSet::new();
    let mut found_entrypoint = false;
    let mut plans = Vec::with_capacity(entries.len());

    for entry in entries {
        let source = entry.path();
        let destination = if source == source_entrypoint {
            found_entrypoint = true;
            output_path.to_path_buf()
        } else {
            output_parent.join(entry.file_name())
        };
        if !destinations.insert(destination.clone()) {
            return Err(RunError::Config(format!(
                "Pipeline artifact maps multiple entries to output: {}",
                destination.display()
            )));
        }
        ensure_destination_absent(&destination)?;
        plans.push((source, destination));
    }
    if !found_entrypoint {
        return Err(RunError::Config(format!(
            "Pipeline artifact entrypoint is missing: {}",
            source_entrypoint.display()
        )));
    }

    let staging = create_staging_directory(output_parent)?;
    let staged_copy = (|| -> Result<(), RunError> {
        for (source, destination) in &plans {
            let staged_name = if source == &source_entrypoint {
                output_name
            } else {
                destination.file_name().ok_or_else(|| {
                    RunError::Config(format!(
                        "Pipeline output has no file name: {}",
                        destination.display()
                    ))
                })?
            };
            copy_entry(source, &staging.join(staged_name))?;
        }
        Ok(())
    })();
    if let Err(error) = staged_copy {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    plans.sort_by_key(|(source, _)| source == &source_entrypoint);
    let mut published = Vec::new();
    for (source, destination) in &plans {
        let staged_name = if source == &source_entrypoint {
            output_name
        } else {
            destination.file_name().ok_or_else(|| {
                RunError::Config(format!(
                    "Pipeline output has no file name: {}",
                    destination.display()
                ))
            })?
        };
        if let Err(error) =
            publish_staged_entry(&staging.join(staged_name), destination, &mut published)
        {
            rollback_published_entries(&published);
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    }

    if let Err(error) = fs::remove_dir(&staging) {
        logger::warn(&format!(
            "Failed to remove pipeline output staging directory {}: {}",
            staging.display(),
            error
        ));
    }

    Ok(())
}

fn validate_source_entry(source: &Path) -> Result<(), RunError> {
    let metadata = fs::symlink_metadata(source).map_err(PipelineError::Io)?;
    if metadata.file_type().is_symlink() {
        return Err(RunError::Config(format!(
            "Pipeline artifact contains unsupported symlink: {}",
            source.display()
        )));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(source).map_err(PipelineError::Io)? {
            let entry = entry.map_err(PipelineError::Io)?;
            validate_source_entry(&entry.path())?;
        }
    } else if !metadata.is_file() {
        return Err(RunError::Config(format!(
            "Pipeline artifact contains unsupported filesystem entry: {}",
            source.display()
        )));
    }
    Ok(())
}

fn ensure_destination_absent(destination: &Path) -> Result<(), RunError> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RunError::Pipeline(PipelineError::Io(error))),
        Ok(_) => Err(RunError::Config(format!(
            "Refusing to overwrite pipeline output: {}",
            destination.display()
        ))),
    }
}

fn create_staging_directory(parent: &Path) -> Result<PathBuf, RunError> {
    for _ in 0..16 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| RunError::Config(format!("System clock error: {error}")))?
            .as_nanos();
        let staging = parent.join(format!(
            ".r2x-output-{timestamp}-{}-{}",
            std::process::id(),
            next_workspace_id()
        ));
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(RunError::Pipeline(PipelineError::Io(error))),
        }
    }
    Err(RunError::Config(
        "Unable to allocate a pipeline output staging directory".to_string(),
    ))
}

fn copy_entry(source: &Path, destination: &Path) -> Result<(), RunError> {
    let metadata = fs::symlink_metadata(source).map_err(PipelineError::Io)?;
    if metadata.file_type().is_symlink() {
        return Err(RunError::Config(format!(
            "Pipeline artifact contains unsupported symlink: {}",
            source.display()
        )));
    }

    if metadata.is_dir() {
        ensure_destination_absent(destination)?;
        fs::create_dir(destination).map_err(PipelineError::Io)?;
        for entry in fs::read_dir(source).map_err(PipelineError::Io)? {
            let entry = entry.map_err(PipelineError::Io)?;
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        ensure_destination_absent(destination)?;
        fs::copy(source, destination).map_err(PipelineError::Io)?;
    } else {
        return Err(RunError::Config(format!(
            "Pipeline artifact contains unsupported filesystem entry: {}",
            source.display()
        )));
    }

    Ok(())
}

fn publish_staged_entry(
    source: &Path,
    destination: &Path,
    published: &mut Vec<PathBuf>,
) -> Result<(), RunError> {
    let metadata = fs::symlink_metadata(source).map_err(PipelineError::Io)?;
    if metadata.file_type().is_symlink() {
        return Err(RunError::Config(format!(
            "Pipeline artifact staging contains unsupported symlink: {}",
            source.display()
        )));
    }

    if metadata.is_dir() {
        fs::create_dir(destination).map_err(PipelineError::Io)?;
        published.push(destination.to_path_buf());
        for entry in fs::read_dir(source).map_err(PipelineError::Io)? {
            let entry = entry.map_err(PipelineError::Io)?;
            publish_staged_entry(
                &entry.path(),
                &destination.join(entry.file_name()),
                published,
            )?;
        }
        fs::remove_dir(source).map_err(PipelineError::Io)?;
    } else if metadata.is_file() {
        fs::hard_link(source, destination).map_err(PipelineError::Io)?;
        published.push(destination.to_path_buf());
        fs::remove_file(source).map_err(PipelineError::Io)?;
    } else {
        return Err(RunError::Config(format!(
            "Pipeline artifact staging contains unsupported filesystem entry: {}",
            source.display()
        )));
    }

    Ok(())
}

fn rollback_published_entries(published: &[PathBuf]) {
    for path in published.iter().rev() {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            continue;
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            let _ = fs::remove_dir(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

fn next_workspace_id() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use crate::commands::run::pipeline::artifact::{copy_bundle_to_output, publish_staged_entry};
    use r2x_python::plugin_invoker::ArtifactBundle;
    use std::error::Error;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn bundle_output_copies_entrypoint_and_sidecars_without_rewriting_json(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("system_time_series"))?;
        fs::write(
            source.join("system.json"),
            r#"{"time_series":{"directory":"system_time_series"}}"#,
        )?;
        fs::write(source.join("system_time_series/data.h5"), "sidecar")?;
        let bundle = ArtifactBundle::new(&source, "system.json")?;
        let output = temp.path().join("output/result.json");

        copy_bundle_to_output(&bundle, &output)?;

        assert_eq!(
            fs::read_to_string(&output)?,
            r#"{"time_series":{"directory":"system_time_series"}}"#
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("output/system_time_series/data.h5"))?,
            "sidecar"
        );
        Ok(())
    }

    #[test]
    fn bundle_output_refuses_to_overwrite_existing_files() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join("system.json"), "source")?;
        let bundle = ArtifactBundle::new(&source, "system.json")?;
        let output = temp.path().join("result.json");
        fs::write(&output, "existing")?;

        let result = copy_bundle_to_output(&bundle, &output);

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(output)?, "existing");
        Ok(())
    }

    #[test]
    fn bundle_output_rejects_destination_name_collisions_before_writing(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join("system.json"), "entrypoint")?;
        fs::write(source.join("result.json"), "sidecar")?;
        let bundle = ArtifactBundle::new(&source, "system.json")?;
        let output = temp.path().join("output/result.json");

        let result = copy_bundle_to_output(&bundle, &output);

        assert!(result.is_err());
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn bundle_output_rejects_nested_entrypoints() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("nested"))?;
        fs::write(source.join("nested/system.json"), "entrypoint")?;
        let bundle = ArtifactBundle::new(&source, "nested/system.json")?;
        let output = temp.path().join("result.json");

        let result = copy_bundle_to_output(&bundle, &output);

        assert!(result.is_err());
        assert!(!output.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn bundle_output_rejects_dangling_destination_symlinks() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let temp = tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join("system.json"), "entrypoint")?;
        let bundle = ArtifactBundle::new(&source, "system.json")?;
        let outside = temp.path().join("outside.json");
        let output = temp.path().join("result.json");
        symlink(&outside, &output)?;

        let result = copy_bundle_to_output(&bundle, &output);

        assert!(result.is_err());
        assert!(!outside.exists());
        assert!(fs::symlink_metadata(output).is_ok());
        Ok(())
    }

    #[test]
    fn staged_publication_never_replaces_a_destination() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let source = temp.path().join("staged.json");
        let destination = temp.path().join("result.json");
        fs::write(&source, "staged")?;
        fs::write(&destination, "existing")?;
        let mut published = Vec::new();

        let result = publish_staged_entry(&source, &destination, &mut published);

        assert!(result.is_err());
        assert!(published.is_empty());
        assert_eq!(fs::read_to_string(&destination)?, "existing");
        assert_eq!(fs::read_to_string(&source)?, "staged");
        Ok(())
    }
}
