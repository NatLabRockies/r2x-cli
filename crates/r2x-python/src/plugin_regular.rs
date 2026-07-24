//! Regular plugin invocation (non-upgrader)

use crate::errors::BridgeError;
use crate::plugin_invoker::{
    ArtifactBundle, ArtifactOutputKind, PluginArtifactInvocationResult, PluginInvocationResult,
    PluginInvocationTimings,
};
use crate::python_bridge::Bridge;
use once_cell::sync::Lazy;
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyAny, PyAnyMethods, PyBytes, PyDict, PyDictMethods, PyModule};
use pyo3::{Bound, PyResult};
use r2x_logger as logger;
use r2x_manifest::runtime::{PluginRole, RuntimeBindings};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonBackend {
    OrJson,
    StdJson,
}

pub(crate) struct PythonJson<'py> {
    loads: Bound<'py, PyAny>,
    dumps: Bound<'py, PyAny>,
    std_json_dumps: Option<Bound<'py, PyAny>>,
    backend: JsonBackend,
}

impl<'py> PythonJson<'py> {
    pub(crate) fn import(py: pyo3::Python<'py>) -> Result<Self, BridgeError> {
        // Fast path: prefer orjson for plugin invocation payloads when present.
        if let Ok(orjson) = PyModule::import(py, "orjson") {
            let loads = orjson.getattr("loads");
            let dumps = orjson.getattr("dumps");
            if let (Ok(loads), Ok(dumps)) = (loads, dumps) {
                let std_json_dumps = PyModule::import(py, "json")
                    .map_err(|e| BridgeError::Import("json".to_string(), format!("{}", e)))?
                    .getattr("dumps")?;
                return Ok(Self {
                    loads,
                    dumps,
                    std_json_dumps: Some(std_json_dumps),
                    backend: JsonBackend::OrJson,
                });
            }
        }

        let module = PyModule::import(py, "json")
            .map_err(|e| BridgeError::Import("json".to_string(), format!("{}", e)))?;
        let loads = module.getattr("loads")?;
        let dumps = module.getattr("dumps")?;

        Ok(Self {
            loads,
            dumps,
            std_json_dumps: None,
            backend: JsonBackend::StdJson,
        })
    }

    pub(crate) fn loads(&self, input: &str) -> PyResult<Bound<'py, PyAny>> {
        self.loads.call1((input,))
    }

    pub(crate) fn dumps(&self, value: &Bound<'py, PyAny>) -> PyResult<String> {
        self.dumps_with_kwargs(value, None)
    }

    pub(crate) fn dumps_with_kwargs(
        &self,
        value: &Bound<'py, PyAny>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<String> {
        let rendered = if kwargs.is_some() && self.backend == JsonBackend::OrJson {
            if let Some(std_json_dumps) = self.std_json_dumps.as_ref() {
                std_json_dumps.call((value,), kwargs)?
            } else {
                self.dumps.call((value,), kwargs)?
            }
        } else {
            self.dumps.call((value,), kwargs)?
        };

        if let Ok(text) = rendered.extract::<String>() {
            return Ok(text);
        }

        let bytes = rendered.extract::<Vec<u8>>()?;
        String::from_utf8(bytes).map_err(|error| {
            PyValueError::new_err(format!("Invalid UTF-8 JSON payload: {}", error))
        })
    }

    fn load_path(&self, path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let bytes = path.call_method0("read_bytes")?;
        self.loads.call1((bytes,))
    }

    fn write_path(&self, path: &Bound<'py, PyAny>, value: &Bound<'py, PyAny>) -> PyResult<()> {
        ensure_artifact_parent(path)?;
        let rendered = self.dumps.call1((value,))?;
        if rendered.is_instance_of::<PyBytes>() {
            path.call_method1("write_bytes", (rendered,))?;
        } else {
            path.call_method1("write_text", (rendered, "utf-8"))?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn backend_name(&self) -> &'static str {
        match self.backend {
            JsonBackend::OrJson => "orjson",
            JsonBackend::StdJson => "json",
        }
    }
}

fn python_path<'py>(py: pyo3::Python<'py>, path: &Path) -> Result<Bound<'py, PyAny>, BridgeError> {
    let pathlib = PyModule::import(py, "pathlib")
        .map_err(|error| BridgeError::Import("pathlib".to_string(), error.to_string()))?;
    let path_class = pathlib.getattr("Path")?;
    let rendered = path.to_string_lossy();
    path_class
        .call1((rendered.as_ref(),))
        .map_err(BridgeError::from)
}

fn ensure_artifact_parent(path: &Bound<'_, PyAny>) -> PyResult<()> {
    let parent = path.getattr("parent")?;
    let kwargs = PyDict::new(path.py());
    kwargs.set_item("parents", true)?;
    kwargs.set_item("exist_ok", true)?;
    parent.call_method("mkdir", (), Some(&kwargs))?;
    Ok(())
}

fn load_system_artifact<'py>(
    py: pyo3::Python<'py>,
    input: &ArtifactBundle,
) -> Result<Bound<'py, PyAny>, BridgeError> {
    let path = python_path(py, &input.entrypoint_path())?;
    let system_module = PyModule::import(py, "r2x_core.system")?;
    let system_class = system_module.getattr("System")?;
    let from_json = system_class.getattr("from_json")?;
    from_json.call1((path,)).map_err(|error| {
        BridgeError::Python(format_python_error(
            py,
            error,
            &format!(
                "Failed to load System artifact {}",
                input.entrypoint_path().display()
            ),
        ))
    })
}

fn load_exporter_system_artifact<'py>(
    py: pyo3::Python<'py>,
    input: &ArtifactBundle,
    json: &PythonJson<'py>,
) -> Result<Bound<'py, PyAny>, BridgeError> {
    let entrypoint = python_path(py, &input.entrypoint_path())?;
    let data = json.load_path(&entrypoint).map_err(|error| {
        BridgeError::Python(format_python_error(
            py,
            error,
            &format!(
                "Failed to load exporter artifact {}",
                input.entrypoint_path().display()
            ),
        ))
    })?;
    let root = python_path(py, input.root())?;
    let system_module = PyModule::import(py, "infrasys")?;
    let system_class = system_module.getattr("System")?;
    let from_dict = system_class.getattr("from_dict")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("time_series_read_only", true)?;
    from_dict
        .call((data, root), Some(&kwargs))
        .map_err(|error| {
            BridgeError::Python(format_python_error(
                py,
                error,
                &format!(
                    "Failed to load exporter System artifact {}",
                    input.entrypoint_path().display()
                ),
            ))
        })
}

enum ClassInput<'a> {
    Inline(Option<&'a str>),
    Artifact(Option<&'a ArtifactBundle>),
}

impl ClassInput<'_> {
    fn is_present(&self) -> bool {
        match self {
            Self::Inline(input) => input.is_some(),
            Self::Artifact(input) => input.is_some(),
        }
    }
}

/// Guard that suppresses Python stdout and restores it on drop.
pub(crate) struct StdoutGuard<'py> {
    py: pyo3::Python<'py>,
    original: Option<pyo3::Py<PyAny>>,
}

impl<'py> StdoutGuard<'py> {
    pub(crate) fn new(py: pyo3::Python<'py>, suppress: bool) -> Result<Self, BridgeError> {
        let original = if suppress {
            let sys = PyModule::import(py, "sys")?;
            let io = PyModule::import(py, "io")?;
            let original_stdout = sys.getattr("stdout")?;
            let string_io = io.getattr("StringIO")?.call0()?;
            sys.setattr("stdout", &string_io)?;
            logger::debug("Python stdout suppressed");
            Some(original_stdout.unbind())
        } else {
            None
        };
        Ok(Self { py, original })
    }
}

impl Drop for StdoutGuard<'_> {
    fn drop(&mut self) {
        if let Some(ref stdout) = self.original {
            if let Ok(sys) = PyModule::import(self.py, "sys") {
                let _ = sys.setattr("stdout", stdout.bind(self.py));
                logger::debug("Python stdout restored");
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StdinSignatureSupport {
    accepts_system: bool,
    accepts_stdin: bool,
}

impl StdinSignatureSupport {
    fn accepts_any(&self) -> bool {
        self.accepts_system || self.accepts_stdin
    }
}

static METHOD_STDIN_SIGNATURE_CACHE: Lazy<Mutex<HashMap<String, StdinSignatureSupport>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn should_parse_stdin_for_function_kwargs(
    stdin_json: Option<&str>,
    runtime_bindings: Option<&RuntimeBindings>,
) -> bool {
    let Some(_stdin) = stdin_json else {
        return false;
    };

    match runtime_bindings {
        Some(bindings) => bindings
            .parameters
            .iter()
            .any(|param| param.name.as_ref() == "stdin"),
        // Preserve legacy behavior when runtime metadata is unavailable.
        None => true,
    }
}

fn should_parse_stdin_for_exporter_context(stdin_json: Option<&str>, role: PluginRole) -> bool {
    stdin_json.is_some() && role == PluginRole::Exporter
}

fn ensure_parsed_stdin<'py>(
    stdin_json: Option<&str>,
    json: &PythonJson<'py>,
    parsed_stdin: &mut Option<Bound<'py, PyAny>>,
) -> PyResult<()> {
    if parsed_stdin.is_none() {
        if let Some(stdin) = stdin_json {
            parsed_stdin.replace(json.loads(stdin)?);
        }
    }
    Ok(())
}

impl Bridge {
    pub(crate) fn invoke_plugin_regular(
        &self,
        target: &str,
        config_json: &str,
        stdin_json: Option<&str>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginInvocationResult, BridgeError> {
        pyo3::Python::attach(|py| {
            let _guard = StdoutGuard::new(py, logger::get_no_stdout())?;

            logger::debug_lazy(|| format!("Parsing target: {}", target));
            let parts: Vec<&str> = target.split(':').collect();
            if parts.len() != 2 {
                return Err(BridgeError::InvalidEntryPoint(target.to_string()));
            }
            let module_path = parts[0];
            let callable_path = parts[1];

            logger::debug_lazy(|| format!("Importing module: {}", module_path));
            let module = PyModule::import(py, module_path)
                .map_err(|e| BridgeError::Import(module_path.to_string(), format!("{}", e)))?;

            // Re-enable loguru for this module after import.
            // Python __init__.py files call logger.disable() by convention,
            // which overwrites any enables set before the import.
            let _ = Bridge::enable_loguru_modules_after_import(
                py,
                &[module_path.split('.').next().unwrap_or(module_path)],
            );

            let json = PythonJson::import(py)?;

            logger::debug("Parsing config JSON");
            let config_dict = json
                .loads(config_json)?
                .cast::<pyo3::types::PyDict>()
                .map_err(|e| BridgeError::Python(format!("Config must be a JSON object: {}", e)))?
                .clone();

            logger::debug("Starting plugin invocation");
            let call_start = Instant::now();
            let result_py = if callable_path.contains('.') {
                Self::invoke_class_callable(
                    self,
                    &module,
                    module_path,
                    callable_path,
                    &config_dict,
                    ClassInput::Inline(stdin_json),
                    runtime_bindings,
                    &json,
                )?
            } else {
                logger::debug("Building kwargs for function invocation");
                let stdin_obj =
                    if should_parse_stdin_for_function_kwargs(stdin_json, runtime_bindings) {
                        let Some(stdin) = stdin_json else {
                            return Err(BridgeError::Python(
                                "stdin unexpectedly missing while parsing function kwargs"
                                    .to_string(),
                            ));
                        };
                        logger::debug("Parsing stdin JSON for stdin kwarg injection");
                        Some(json.loads(stdin)?)
                    } else {
                        None
                    };
                let kwargs =
                    Self::build_kwargs(py, &config_dict, stdin_obj.as_ref(), runtime_bindings)?;
                Self::invoke_function_callable(
                    py,
                    &module,
                    module_path,
                    callable_path,
                    stdin_json,
                    stdin_obj.as_ref(),
                    &kwargs,
                    &json,
                )?
            };
            let call_elapsed = call_start.elapsed();
            logger::debug_lazy(|| {
                format!(
                    "Python invocation for '{}' took {}",
                    callable_path,
                    format_duration(call_elapsed)
                )
            });
            logger::debug("Plugin execution completed");

            // For exporters, skip serialization - they write their own output
            // and return PluginContext which we don't need to pass downstream
            let is_exporter = runtime_bindings.is_some_and(|b| b.role == PluginRole::Exporter);

            if is_exporter {
                logger::debug("Exporter plugin completed, skipping result serialization");
                return Ok(PluginInvocationResult {
                    output: "{}".to_string(),
                    timings: Some(PluginInvocationTimings {
                        python_invocation: call_elapsed,
                        serialization: Duration::ZERO,
                    }),
                });
            }

            logger::debug("Serializing result to JSON");

            let result_unwrapped = {
                let type_name: String = result_py
                    .get_type()
                    .getattr("__name__")
                    .and_then(|n| n.extract())
                    .unwrap_or_default();
                if type_name == "Ok" {
                    logger::debug("Unwrapping Ok result type");
                    result_py
                        .getattr("ok_value")
                        .or_else(|_| result_py.getattr("value"))?
                } else if type_name == "Err" {
                    let error_text = format_err_result(py, &result_py);
                    return Err(BridgeError::Python(error_text));
                } else {
                    result_py
                }
            };

            let result_to_serialize =
                if result_unwrapped.hasattr("system")? && result_unwrapped.hasattr("config")? {
                    logger::debug("Result is PluginContext, extracting system for serialization");
                    result_unwrapped.getattr("system")?
                } else {
                    result_unwrapped
                };

            let (json_str, ser_elapsed) = if result_to_serialize.hasattr("to_json")? {
                let ser_start = Instant::now();
                let to_json_result = result_to_serialize.call_method0("to_json")?;
                let json_str = if let Ok(json_bytes) = to_json_result.extract::<Vec<u8>>() {
                    String::from_utf8(json_bytes).map_err(|e| {
                        BridgeError::Python(format!("Invalid UTF-8 in JSON output: {}", e))
                    })?
                } else {
                    json.dumps(&result_to_serialize)?
                };
                let ser_elapsed = ser_start.elapsed();
                logger::debug_lazy(|| {
                    format!(
                        "Serialization for '{}' took {}",
                        callable_path,
                        format_duration(ser_elapsed)
                    )
                });
                (json_str, ser_elapsed)
            } else {
                let ser_start = Instant::now();
                let json_str = json.dumps(&result_to_serialize)?;
                let ser_elapsed = ser_start.elapsed();
                logger::debug_lazy(|| {
                    format!(
                        "Serialization for '{}' took {}",
                        callable_path,
                        format_duration(ser_elapsed)
                    )
                });
                (json_str, ser_elapsed)
            };

            Ok(PluginInvocationResult {
                output: json_str,
                timings: Some(PluginInvocationTimings {
                    python_invocation: call_elapsed,
                    serialization: ser_elapsed,
                }),
            })
        })
    }

    pub(crate) fn save_system_artifact_as_zip_native(
        &self,
        input: &ArtifactBundle,
        output: &Path,
    ) -> Result<(), BridgeError> {
        pyo3::Python::attach(|py| {
            let system_module = PyModule::import(py, "r2x_core.system")?;
            let system_class = system_module.getattr("System")?;
            let entrypoint = python_path(py, &input.entrypoint_path())?;
            let system = system_class
                .getattr("from_json")?
                .call1((entrypoint,))
                .map_err(|error| {
                    BridgeError::Python(format_python_error(
                        py,
                        error,
                        &format!(
                            "Failed to load System artifact {}",
                            input.entrypoint_path().display()
                        ),
                    ))
                })?;

            let archive_base = output.with_extension("");
            let base_path = python_path(py, &archive_base)?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("filename", "system.json")?;
            kwargs.set_item("zip", true)?;
            kwargs.set_item("overwrite", false)?;
            system
                .call_method("save", (base_path,), Some(&kwargs))
                .map_err(|error| {
                    BridgeError::Python(format_python_error(
                        py,
                        error,
                        &format!("Failed to save System ZIP archive {}", output.display()),
                    ))
                })?;

            let archive_metadata = fs::symlink_metadata(output).map_err(|error| {
                BridgeError::InvalidArtifact(format!(
                    "System.save completed without creating ZIP archive {}: {}",
                    output.display(),
                    error
                ))
            })?;
            if archive_metadata.file_type().is_symlink() || !archive_metadata.is_file() {
                return Err(BridgeError::InvalidArtifact(format!(
                    "System.save created a non-file ZIP archive at {}",
                    output.display()
                )));
            }
            match fs::symlink_metadata(&archive_base) {
                Ok(_) => {
                    return Err(BridgeError::InvalidArtifact(format!(
                        "System.save left temporary archive directory behind: {}",
                        archive_base.display()
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }

            Ok(())
        })
    }

    pub(crate) fn invoke_plugin_regular_with_artifacts(
        &self,
        target: &str,
        config_json: &str,
        input: Option<&ArtifactBundle>,
        output: &ArtifactBundle,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<PluginArtifactInvocationResult, BridgeError> {
        pyo3::Python::attach(|py| {
            let _guard = StdoutGuard::new(py, logger::get_no_stdout())?;

            let parts: Vec<&str> = target.split(':').collect();
            if parts.len() != 2 {
                return Err(BridgeError::InvalidEntryPoint(target.to_string()));
            }
            let module_path = parts[0];
            let callable_path = parts[1];

            let module = PyModule::import(py, module_path)
                .map_err(|error| BridgeError::Import(module_path.to_string(), error.to_string()))?;
            let _ = Bridge::enable_loguru_modules_after_import(
                py,
                &[module_path.split('.').next().unwrap_or(module_path)],
            );

            let json = PythonJson::import(py)?;
            let config_dict = json
                .loads(config_json)?
                .cast::<pyo3::types::PyDict>()
                .map_err(|error| {
                    BridgeError::Python(format!("Config must be a JSON object: {}", error))
                })?
                .clone();

            let call_start = Instant::now();
            let result_py = if callable_path.contains('.') {
                Self::invoke_class_callable(
                    self,
                    &module,
                    module_path,
                    callable_path,
                    &config_dict,
                    ClassInput::Artifact(input),
                    runtime_bindings,
                    &json,
                )?
            } else {
                let stdin_obj = if input.is_some()
                    && runtime_bindings.is_some_and(|bindings| {
                        bindings
                            .parameters
                            .iter()
                            .any(|parameter| parameter.name.as_ref() == "stdin")
                    }) {
                    let bundle = input.ok_or_else(|| {
                        BridgeError::Python(
                            "stdin expected but no input artifact was provided".to_string(),
                        )
                    })?;
                    let path = python_path(py, &bundle.entrypoint_path())?;
                    Some(json.load_path(&path).map_err(|error| {
                        BridgeError::Python(format_python_error(
                            py,
                            error,
                            &format!(
                                "Failed to load stdin artifact {}",
                                bundle.entrypoint_path().display()
                            ),
                        ))
                    })?)
                } else {
                    None
                };
                let kwargs =
                    Self::build_kwargs(py, &config_dict, stdin_obj.as_ref(), runtime_bindings)?;
                Self::invoke_function_callable_with_artifact(
                    py,
                    &module,
                    module_path,
                    callable_path,
                    input,
                    &kwargs,
                    &json,
                    runtime_bindings,
                )?
            };
            let call_elapsed = call_start.elapsed();

            let output_path = python_path(py, &output.entrypoint_path())?;
            let is_exporter =
                runtime_bindings.is_some_and(|bindings| bindings.role == PluginRole::Exporter);
            if is_exporter {
                return Ok(PluginArtifactInvocationResult {
                    output_kind: ArtifactOutputKind::Empty,
                    timings: Some(PluginInvocationTimings {
                        python_invocation: call_elapsed,
                        serialization: Duration::ZERO,
                    }),
                });
            }

            let result_unwrapped = {
                let type_name: String = result_py
                    .get_type()
                    .getattr("__name__")
                    .and_then(|name| name.extract())
                    .unwrap_or_default();
                if type_name == "Ok" {
                    result_py
                        .getattr("ok_value")
                        .or_else(|_| result_py.getattr("value"))?
                } else if type_name == "Err" {
                    return Err(BridgeError::Python(format_err_result(py, &result_py)));
                } else {
                    result_py
                }
            };

            let result_to_serialize =
                if result_unwrapped.hasattr("system")? && result_unwrapped.hasattr("config")? {
                    result_unwrapped.getattr("system")?
                } else {
                    result_unwrapped
                };

            let write_start = Instant::now();
            let output_kind = write_artifact_result(&json, &result_to_serialize, &output_path)
                .map_err(|error| {
                    BridgeError::Python(format_python_error(
                        py,
                        error,
                        &format!(
                            "Failed to write artifact {}",
                            output.entrypoint_path().display()
                        ),
                    ))
                })?;
            validate_artifact_output(output, output_kind)?;

            Ok(PluginArtifactInvocationResult {
                output_kind,
                timings: Some(PluginInvocationTimings {
                    python_invocation: call_elapsed,
                    serialization: write_start.elapsed(),
                }),
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_class_callable<'py>(
        _bridge: &Bridge,
        module: &pyo3::Bound<'py, PyModule>,
        module_path: &str,
        callable_path: &str,
        config_dict: &pyo3::Bound<'py, PyDict>,
        input: ClassInput<'_>,
        runtime_bindings: Option<&RuntimeBindings>,
        json: &PythonJson<'py>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let parts: Vec<&str> = callable_path.split('.').collect();
        if parts.len() != 2 {
            return Err(BridgeError::InvalidEntryPoint(callable_path.to_string()));
        }
        let (class_name, method_name) = (parts[0], parts[1]);

        let class = module.getattr(class_name).map_err(|e| {
            BridgeError::Python(format_python_error(
                module.py(),
                e,
                &format!("Failed to get class '{}'", class_name),
            ))
        })?;

        let py = module.py();

        let bindings = runtime_bindings.ok_or_else(|| {
            BridgeError::Python(format!(
                "Runtime bindings required for class-based plugin '{}'",
                class_name
            ))
        })?;

        logger::step(&format!(
            "Instantiating plugin '{}' via from_context (PluginContext interface)",
            class_name
        ));

        let config_params = {
            let params = PyDict::new(py);
            for (key, value) in config_dict.iter() {
                let key_str = key.extract::<String>()?;
                if key_str != "store" && key_str != "data_store" && key_str != "store_path" {
                    params.set_item(key, value)?;
                }
            }
            params
        };

        let config_instance = if bindings.config.is_some() {
            Bridge::instantiate_config_class(py, &config_params, bindings.config.as_ref())?
        } else {
            logger::debug_lazy(|| {
                format!(
                    "Config metadata missing for '{}', discovering from plugin class",
                    class_name
                )
            });
            discover_and_instantiate_config(py, &class, &config_params)?
        };
        logger::step(&format!("Config class instantiated for '{}'", class_name));

        let store_value = config_dict
            .get_item("store")?
            .or_else(|| config_dict.get_item("store_path").ok().flatten())
            .or_else(|| config_dict.get_item("path").ok().flatten());

        let store_instance = if let Some(value) = store_value {
            logger::debug("Creating DataStore from path for PluginContext");
            Some(Bridge::instantiate_data_store(
                py,
                &value,
                Some(&config_instance),
                bindings.config.as_ref(),
            )?)
        } else {
            None
        };

        let mut parsed_stdin_obj: Option<Bound<'py, PyAny>> = None;
        let system_instance = match &input {
            ClassInput::Inline(stdin_json)
                if should_parse_stdin_for_exporter_context(*stdin_json, bindings.role) =>
            {
                logger::step("Deserializing system from stdin for PluginContext");
                ensure_parsed_stdin(*stdin_json, json, &mut parsed_stdin_obj)?;
                let stdin = parsed_stdin_obj
                    .as_ref()
                    .ok_or_else(|| PyValueError::new_err("stdin expected but not provided"))?;

                let system_module = PyModule::import(py, "infrasys")?;
                let system_class = system_module.getattr("System")?;
                let from_dict = system_class.getattr("from_dict")?;

                let tempfile = PyModule::import(py, "tempfile")?;
                let mkdtemp = tempfile.getattr("mkdtemp")?;
                let temp_dir = mkdtemp.call0()?.extract::<String>()?;

                let kwargs_dict = PyDict::new(py);
                kwargs_dict.set_item("time_series_read_only", true)?;
                Some(from_dict.call((stdin, temp_dir), Some(&kwargs_dict))?)
            }
            ClassInput::Artifact(Some(bundle)) if bindings.role == PluginRole::Exporter => {
                logger::step("Deserializing System artifact for exporter PluginContext");
                Some(load_exporter_system_artifact(py, bundle, json)?)
            }
            _ => None,
        };

        let ctx = Bridge::instantiate_plugin_context(
            py,
            &config_instance,
            store_instance.as_ref(),
            system_instance.as_ref(),
        )?;
        logger::step("PluginContext created");

        let from_context = class.getattr("from_context").map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                &format!(
                    "Plugin class '{}' missing from_context classmethod",
                    class_name
                ),
            ))
        })?;

        let instance = from_context.call1((ctx,)).map_err(|err| {
            let raw_msg = err.to_string();
            let mut formatted = format_python_error(
                py,
                err,
                &format!("Failed to instantiate '{}' via from_context", class_name),
            );
            if raw_msg.contains("missing") && raw_msg.contains("required positional argument") {
                formatted.push_str("\n\nHint: This may happen if the plugin metadata cache is stale. Try running:\n  r2x sync");
            }
            BridgeError::Python(formatted)
        })?;
        logger::step(&format!(
            "Plugin '{}' instantiated via from_context",
            class_name
        ));

        let actual_method_name = if instance.hasattr("run")? {
            "run"
        } else {
            method_name
        };
        logger::debug_lazy(|| {
            format!(
                "Using method '{}' for plugin '{}'",
                actual_method_name, class_name
            )
        });
        let method = instance.getattr(actual_method_name).map_err(|e| {
            BridgeError::Python(format_python_error(
                instance.py(),
                e,
                &format!(
                    "Failed to get method '{}.{}'",
                    class_name, actual_method_name
                ),
            ))
        })?;

        let signature_support = if input.is_present() {
            let signature_cache_key = format!("{module_path}:{class_name}.{actual_method_name}");
            match method_stdin_support_cached(&signature_cache_key, &method) {
                Ok(result) => result,
                Err(err) => {
                    logger::debug_lazy(|| {
                        format!(
                            "Failed to inspect method '{}.{}' signature for stdin support: {}",
                            class_name, method_name, err
                        )
                    });
                    StdinSignatureSupport::default()
                }
            }
        } else {
            StdinSignatureSupport::default()
        };

        if signature_support.accepts_stdin {
            let stdin = match &input {
                ClassInput::Inline(stdin_json) => {
                    ensure_parsed_stdin(*stdin_json, json, &mut parsed_stdin_obj)?;
                    parsed_stdin_obj
                        .as_ref()
                        .ok_or_else(|| PyValueError::new_err("stdin expected but not provided"))?
                        .clone()
                }
                ClassInput::Artifact(Some(bundle)) => {
                    let path = python_path(py, &bundle.entrypoint_path())?;
                    json.load_path(&path).map_err(|error| {
                        BridgeError::Python(format_python_error(
                            py,
                            error,
                            &format!(
                                "Failed to load stdin artifact {}",
                                bundle.entrypoint_path().display()
                            ),
                        ))
                    })?
                }
                ClassInput::Artifact(None) => {
                    return Err(BridgeError::Python(
                        "stdin expected but no input artifact was provided".to_string(),
                    ));
                }
            };
            method.call1((stdin,)).map_err(|e| {
                BridgeError::Python(format_python_error(
                    method.py(),
                    e,
                    &format!("Method '{}.{}' failed", class_name, method_name),
                ))
            })
        } else if signature_support.accepts_system {
            logger::step("Method has system - deserializing input to System object");
            let system_obj = match &input {
                ClassInput::Inline(stdin_json) => {
                    let json_bytes =
                        stdin_payload_bytes(*stdin_json, parsed_stdin_obj.as_ref(), json)?;
                    let system_module = PyModule::import(py, "r2x_core.system")?;
                    let system_class = system_module.getattr("System")?;
                    let from_json = system_class.getattr("from_json")?;
                    from_json.call1((json_bytes.as_slice(),))?
                }
                ClassInput::Artifact(Some(bundle)) => load_system_artifact(py, bundle)?,
                ClassInput::Artifact(None) => {
                    return Err(BridgeError::Python(
                        "system expected but no input artifact was provided".to_string(),
                    ));
                }
            };
            method.call1((system_obj,)).map_err(|e| {
                BridgeError::Python(format_python_error(
                    method.py(),
                    e,
                    &format!("Method '{}.{}' failed", class_name, method_name),
                ))
            })
        } else {
            if input.is_present() {
                logger::debug_lazy(|| {
                    format!(
                        "Method '{}.{}' does not declare 'system'/'stdin'; skipping stdin payload",
                        class_name, method_name
                    )
                });
            }
            method.call0().map_err(|e| {
                BridgeError::Python(format_python_error(
                    method.py(),
                    e,
                    &format!("Method '{}.{}' failed", class_name, method_name),
                ))
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_function_callable<'py>(
        py: pyo3::Python<'py>,
        module: &pyo3::Bound<'py, PyModule>,
        module_path: &str,
        callable_path: &str,
        stdin_json: Option<&str>,
        stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
        kwargs: &pyo3::Bound<'py, PyDict>,
        json: &PythonJson<'py>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        logger::debug_lazy(|| format!("Function pattern: {}", callable_path));
        let func = module.getattr(callable_path).map_err(|e| {
            BridgeError::Python(format_python_error(
                module.py(),
                e,
                &format!("Failed to get function '{}'", callable_path),
            ))
        })?;

        logger::step("Function kwargs prepared (before system injection)");
        let signature_support = if stdin_json.is_some() || stdin_obj.is_some() {
            let signature_cache_key = format!("{module_path}:{callable_path}");
            match method_stdin_support_cached(&signature_cache_key, &func) {
                Ok(result) => result,
                Err(err) => {
                    logger::debug_lazy(|| {
                        format!(
                            "Failed to inspect function '{}' signature for stdin support: {}",
                            callable_path, err
                        )
                    });
                    StdinSignatureSupport::default()
                }
            }
        } else {
            StdinSignatureSupport::default()
        };

        if signature_support.accepts_stdin && !kwargs.contains("stdin")? {
            if let Some(stdin) = stdin_obj {
                kwargs.set_item("stdin", stdin)?;
            } else if let Some(stdin) = stdin_json {
                kwargs.set_item("stdin", json.loads(stdin)?)?;
            }
        }

        if signature_support.accepts_system {
            logger::step("Function has stdin - deserializing to System object");
            let json_bytes = stdin_payload_bytes(stdin_json, stdin_obj, json)?;

            let system_module = PyModule::import(py, "r2x_core.system")?;
            let system_class = system_module.getattr("System")?;
            let from_json = system_class.getattr("from_json")?;
            let system_obj = from_json.call1((json_bytes.as_slice(),))?;
            kwargs.set_item("system", system_obj)?;
        } else if stdin_obj.is_some() {
            if signature_support.accepts_stdin {
                logger::debug_lazy(|| {
                    format!(
                        "Function '{}' accepts 'stdin'; skipping System deserialization",
                        callable_path
                    )
                });
            } else {
                logger::debug_lazy(|| {
                    format!(
                        "Function '{}' does not declare 'system'/'stdin'; skipping stdin payload",
                        callable_path
                    )
                });
            }
        } else if stdin_json.is_some() && !signature_support.accepts_any() {
            logger::debug_lazy(|| {
                format!(
                    "Function '{}' does not declare 'system'/'stdin'; skipping stdin payload",
                    callable_path
                )
            });
        }

        logger::debug_lazy(|| {
            let kwarg_keys: Vec<String> = kwargs
                .keys()
                .into_iter()
                .filter_map(|k| k.extract::<String>().ok())
                .collect();
            format!("Final function kwargs keys: {:?}", kwarg_keys)
        });
        func.call((), Some(kwargs)).map_err(|e| {
            BridgeError::Python(format_python_error(
                func.py(),
                e,
                &format!("Function '{}' failed", callable_path),
            ))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_function_callable_with_artifact<'py>(
        py: pyo3::Python<'py>,
        module: &pyo3::Bound<'py, PyModule>,
        module_path: &str,
        callable_path: &str,
        input: Option<&ArtifactBundle>,
        kwargs: &pyo3::Bound<'py, PyDict>,
        json: &PythonJson<'py>,
        runtime_bindings: Option<&RuntimeBindings>,
    ) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
        let func = module.getattr(callable_path).map_err(|error| {
            BridgeError::Python(format_python_error(
                module.py(),
                error,
                &format!("Failed to get function '{}'", callable_path),
            ))
        })?;

        let mut signature_support = if input.is_some() {
            let signature_cache_key = format!("{module_path}:{callable_path}");
            method_stdin_support_cached(&signature_cache_key, &func).unwrap_or_default()
        } else {
            StdinSignatureSupport::default()
        };
        if let Some(bindings) = runtime_bindings {
            for parameter in &bindings.parameters {
                if parameter.name.as_ref() == "stdin" {
                    signature_support.accepts_stdin = true;
                } else if parameter.name.as_ref() == "system" {
                    signature_support.accepts_system = true;
                }
            }
        }

        if signature_support.accepts_stdin && !kwargs.contains("stdin")? {
            let bundle = input.ok_or_else(|| {
                BridgeError::Python("stdin expected but no input artifact was provided".to_string())
            })?;
            let path = python_path(py, &bundle.entrypoint_path())?;
            let stdin = json.load_path(&path).map_err(|error| {
                BridgeError::Python(format_python_error(
                    py,
                    error,
                    &format!(
                        "Failed to load stdin artifact {}",
                        bundle.entrypoint_path().display()
                    ),
                ))
            })?;
            kwargs.set_item("stdin", stdin)?;
        }

        if signature_support.accepts_system {
            let bundle = input.ok_or_else(|| {
                BridgeError::Python(
                    "system expected but no input artifact was provided".to_string(),
                )
            })?;
            kwargs.set_item("system", load_system_artifact(py, bundle)?)?;
        }

        func.call((), Some(kwargs)).map_err(|error| {
            BridgeError::Python(format_python_error(
                func.py(),
                error,
                &format!("Function '{}' failed", callable_path),
            ))
        })
    }
}

fn write_artifact_result<'py>(
    json: &PythonJson<'py>,
    value: &Bound<'py, PyAny>,
    output_path: &Bound<'py, PyAny>,
) -> PyResult<ArtifactOutputKind> {
    if value.is_none() {
        return Ok(ArtifactOutputKind::Empty);
    }

    if value.hasattr("to_json")? {
        ensure_artifact_parent(output_path)?;
        value.call_method1("to_json", (output_path,))?;
        return Ok(ArtifactOutputKind::System);
    }

    if let Ok(text) = value.extract::<String>() {
        let trimmed = text.trim_start();
        if (trimmed.starts_with('{') || trimmed.starts_with('[')) && json.loads(&text).is_ok() {
            ensure_artifact_parent(output_path)?;
            output_path.call_method1("write_text", (text, "utf-8"))?;
            return Ok(ArtifactOutputKind::Json);
        }
    }

    json.write_path(output_path, value)?;
    Ok(ArtifactOutputKind::Json)
}

fn validate_artifact_output(
    output: &ArtifactBundle,
    output_kind: ArtifactOutputKind,
) -> Result<(), BridgeError> {
    if output_kind == ArtifactOutputKind::Empty {
        return Ok(());
    }

    let root_metadata = fs::symlink_metadata(output.root()).map_err(|error| {
        BridgeError::InvalidArtifact(format!(
            "plugin reported {output_kind:?} output without a bundle root at {}: {error}",
            output.root().display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(BridgeError::InvalidArtifact(format!(
            "plugin output bundle root is not a directory: {}",
            output.root().display()
        )));
    }

    let entrypoint = output.entrypoint_path();
    let entrypoint_metadata = fs::symlink_metadata(&entrypoint).map_err(|error| {
        BridgeError::InvalidArtifact(format!(
            "plugin reported {output_kind:?} output without an entrypoint at {}: {error}",
            entrypoint.display()
        ))
    })?;
    if entrypoint_metadata.file_type().is_symlink() || !entrypoint_metadata.is_file() {
        return Err(BridgeError::InvalidArtifact(format!(
            "plugin output entrypoint is not a regular file: {}",
            entrypoint.display()
        )));
    }

    Ok(())
}

fn format_duration(duration: Duration) -> String {
    let total_ms = duration.as_millis();
    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}

/// Format a Python error with full traceback information.
///
/// This function extracts the traceback from a `PyErr` and formats it
/// into a human-readable string with context.
pub(crate) fn format_python_error(py: pyo3::Python<'_>, err: pyo3::PyErr, context: &str) -> String {
    if let Some(traceback_text) = render_traceback(py, &err) {
        format!("{}:\n{}", context, traceback_text)
    } else {
        format!("{}: {}", context, err)
    }
}

/// Render a Python traceback to a string.
pub(crate) fn render_traceback(py: pyo3::Python<'_>, err: &pyo3::PyErr) -> Option<String> {
    let traceback = err.traceback(py)?;
    let traceback_module = PyModule::import(py, "traceback").ok()?;
    let formatter = traceback_module.getattr("format_exception").ok()?;
    let formatted = formatter
        .call1((err.get_type(py), err.value(py), traceback))
        .ok()?;
    let lines: Vec<String> = formatted.extract().ok()?;
    Some(lines.join(""))
}

/// Format a Python exception value with its traceback (if available).
///
/// This is used for exceptions extracted from Result Err variants, where
/// we have a Python object that is an exception but not a PyErr.
pub(crate) fn format_exception_value(py: pyo3::Python<'_>, exc_value: &Bound<'_, PyAny>) -> String {
    // Try to extract traceback using Option chaining
    let traceback_text = (|| -> Option<String> {
        let tb = exc_value.getattr("__traceback__").ok()?;
        if tb.is_none() {
            return None;
        }
        let exc_type = exc_value.getattr("__class__").ok()?;
        let tb_mod = PyModule::import(py, "traceback").ok()?;
        let formatted = tb_mod
            .getattr("format_exception")
            .ok()?
            .call1((exc_type, exc_value, tb))
            .ok()?;
        let lines: Vec<String> = formatted.extract().ok()?;
        Some(lines.join(""))
    })();

    match traceback_text {
        Some(tb) => format!("Plugin returned Err:\n{}", tb),
        None => format!("Plugin returned Err: {}", exc_value),
    }
}

/// Format an Err result object using rust-ok's `format_error()` method.
///
/// Calls `format_error()` on the Err object directly, which handles:
/// - BaseException payloads: renders full traceback with chained causes
/// - String/other payloads: returns str(value)
///
/// Falls back to the legacy `format_exception_value()` path if
/// `format_error()` is unavailable (older rust-ok versions).
pub(crate) fn format_err_result(py: pyo3::Python<'_>, err_result: &Bound<'_, PyAny>) -> String {
    if let Ok(formatted) = err_result.call_method0("format_error") {
        if let Ok(text) = formatted.extract::<String>() {
            if !text.is_empty() {
                return format!("Plugin returned Err:\n{}", text);
            }
        }
    }

    // Fallback for older rust-ok versions without format_error()
    let err_value = err_result
        .getattr("error")
        .or_else(|_| err_result.getattr("err_value"))
        .or_else(|_| err_result.getattr("value"));

    match err_value {
        Ok(val) => format_exception_value(py, &val),
        Err(_) => format!("Plugin returned Err: {}", err_result),
    }
}

fn method_stdin_support(method: &pyo3::Bound<'_, PyAny>) -> PyResult<StdinSignatureSupport> {
    let code = method.getattr("__code__")?;
    let argcount: usize = code.getattr("co_argcount")?.extract()?;
    let kwonly_argcount: usize = code.getattr("co_kwonlyargcount")?.extract()?;
    let varnames: Vec<String> = code.getattr("co_varnames")?.extract()?;
    let usable = (argcount + kwonly_argcount).min(varnames.len());
    let start_index = method
        .getattr("__self__")
        .ok()
        .is_some_and(|bound_self| !bound_self.is_none());
    let start_index = usize::from(start_index);

    if usable <= start_index {
        return Ok(StdinSignatureSupport::default());
    }

    let mut support = StdinSignatureSupport::default();
    for name in &varnames[start_index..usable] {
        if name == "system" {
            support.accepts_system = true;
        } else if name == "stdin" {
            support.accepts_stdin = true;
        }
    }
    Ok(support)
}

fn stdin_payload_bytes<'py>(
    stdin_json: Option<&str>,
    stdin_obj: Option<&pyo3::Bound<'py, PyAny>>,
    json: &PythonJson<'py>,
) -> PyResult<Vec<u8>> {
    if let Some(stdin) = stdin_json {
        return Ok(stdin.as_bytes().to_vec());
    }

    if let Some(stdin) = stdin_obj {
        return json.dumps(stdin).map(|payload| payload.into_bytes());
    }

    Err(PyValueError::new_err(
        "stdin payload requested but no stdin value was provided",
    ))
}

fn method_stdin_support_cached(
    signature_cache_key: &str,
    method: &pyo3::Bound<'_, PyAny>,
) -> PyResult<StdinSignatureSupport> {
    if let Some(cached) = METHOD_STDIN_SIGNATURE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(signature_cache_key)
        .copied()
    {
        return Ok(cached);
    }

    let support = method_stdin_support(method)?;
    METHOD_STDIN_SIGNATURE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(signature_cache_key.to_string(), support);
    Ok(support)
}

#[cfg(test)]
fn method_accepts_stdin_cached(
    signature_cache_key: &str,
    method: &pyo3::Bound<'_, PyAny>,
) -> PyResult<bool> {
    method_stdin_support_cached(signature_cache_key, method).map(|support| support.accepts_any())
}

#[cfg(test)]
fn has_cached_method_signature(signature_cache_key: &str) -> bool {
    METHOD_STDIN_SIGNATURE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(signature_cache_key)
}

#[cfg(test)]
fn remove_cached_method_signature(signature_cache_key: &str) {
    METHOD_STDIN_SIGNATURE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(signature_cache_key);
}

fn discover_and_instantiate_config<'py>(
    py: pyo3::Python<'py>,
    plugin_class: &pyo3::Bound<'py, PyAny>,
    config_params: &pyo3::Bound<'py, PyDict>,
) -> Result<pyo3::Bound<'py, PyAny>, BridgeError> {
    if let Ok(orig_bases) = plugin_class.getattr("__orig_bases__") {
        if let Ok(bases_iter) = orig_bases.try_iter() {
            for base in bases_iter.flatten() {
                if let Ok(args) = base.getattr("__args__") {
                    if let Ok(mut args_list) = args.try_iter() {
                        // Only check the first type argument (the config type)
                        if let Some(Ok(config_type)) = args_list.next() {
                            if config_type.is_callable() {
                                let type_name: String = config_type
                                    .getattr("__name__")
                                    .and_then(|n| n.extract())
                                    .unwrap_or_default();
                                if type_name.contains("Config") || type_name.contains("config") {
                                    logger::debug_lazy(|| {
                                        format!(
                                            "Discovered config class '{}' from __orig_bases__",
                                            type_name
                                        )
                                    });
                                    return config_type
                                        .call((), Some(config_params))
                                        .map_err(|e| {
                                            BridgeError::Python(format_python_error(
                                                py,
                                                e,
                                                &format!(
                                                    "Failed to instantiate discovered config class '{}'",
                                                    type_name
                                                ),
                                            ))
                                        });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Ok(config_class) = plugin_class.getattr("Config") {
        if config_class.is_callable() {
            logger::debug("Discovered nested Config class");
            return config_class.call((), Some(config_params)).map_err(|e| {
                BridgeError::Python(format_python_error(
                    py,
                    e,
                    "Failed to instantiate nested Config class",
                ))
            });
        }
    }

    if let Ok(config_class) = plugin_class.getattr("config_class") {
        if config_class.is_callable() {
            let type_name: String = config_class
                .getattr("__name__")
                .and_then(|n| n.extract())
                .unwrap_or_default();
            logger::debug_lazy(|| {
                format!(
                    "Discovered config class '{}' from config_class attribute",
                    type_name
                )
            });
            return config_class.call((), Some(config_params)).map_err(|e| {
                BridgeError::Python(format_python_error(
                    py,
                    e,
                    "Failed to instantiate config class from config_class attribute",
                ))
            });
        }
    }

    logger::debug("No config class discovered, using PluginConfig from r2x_core");
    let r2x_core = PyModule::import(py, "r2x_core").map_err(|e| {
        BridgeError::Python(format_python_error(py, e, "Failed to import r2x_core"))
    })?;
    let plugin_config_class = r2x_core.getattr("PluginConfig").map_err(|e| {
        BridgeError::Python(format_python_error(
            py,
            e,
            "Failed to get PluginConfig class",
        ))
    })?;
    plugin_config_class
        .call((), Some(config_params))
        .map_err(|e| {
            BridgeError::Python(format_python_error(
                py,
                e,
                "Failed to instantiate PluginConfig",
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::{
        has_cached_method_signature, method_accepts_stdin_cached, method_stdin_support,
        remove_cached_method_signature, should_parse_stdin_for_exporter_context,
        should_parse_stdin_for_function_kwargs, stdin_payload_bytes, PythonJson,
    };
    use crate::plugin_invoker::{ArtifactBundle, ArtifactOutputKind};
    use crate::python_bridge::Bridge;
    use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods, PyModule};
    use r2x_manifest::runtime::RuntimeBindings;
    use r2x_manifest::types::Parameter;
    use std::ffi::CString;
    use tempfile::tempdir;

    fn install_module<'py>(
        py: pyo3::Python<'py>,
        name: &str,
        source: &str,
    ) -> Result<pyo3::Bound<'py, PyModule>, String> {
        let source = CString::new(source).map_err(|error| error.to_string())?;
        let filename = CString::new(format!("{name}.py")).map_err(|error| error.to_string())?;
        let module_name = CString::new(name).map_err(|error| error.to_string())?;
        let module = PyModule::from_code(
            py,
            source.as_c_str(),
            filename.as_c_str(),
            module_name.as_c_str(),
        )
        .map_err(|error| error.to_string())?;
        let sys = PyModule::import(py, "sys").map_err(|error| error.to_string())?;
        let modules_obj = sys.getattr("modules").map_err(|error| error.to_string())?;
        let modules = modules_obj
            .cast::<PyDict>()
            .map_err(|error| error.to_string())?;
        modules
            .set_item(name, &module)
            .map_err(|error| error.to_string())?;
        Ok(module)
    }

    fn install_artifact_test_modules(py: pyo3::Python<'_>) -> Result<(), String> {
        let core = install_module(
            py,
            "r2x_core",
            r"
class PluginContext:
    def __init__(self, config, *, store=None, system=None):
        self.config = config
        self.store = store
        self.system = system
",
        )?;
        let system = install_module(
            py,
            "r2x_core.system",
            r#"
import json
from pathlib import Path

class System:
    def __init__(self, data):
        self.data = data

    @classmethod
    def from_json(cls, path):
        path = Path(path)
        data = json.loads(path.read_text(encoding="utf-8"))
        sidecar = path.parent / data["sidecar"]
        if not sidecar.exists():
            raise RuntimeError(f"missing sidecar: {sidecar}")
        return cls(data)

    def to_json(self, path):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        (path.parent / self.data["sidecar"]).write_text("sidecar", encoding="utf-8")
        path.write_text(json.dumps(self.data), encoding="utf-8")
"#,
        )?;
        core.setattr("system", system)
            .map_err(|error| error.to_string())?;

        install_module(
            py,
            "infrasys",
            r#"
from pathlib import Path

class System:
    @classmethod
    def from_dict(cls, data, parent, *, time_series_read_only):
        if not time_series_read_only:
            raise RuntimeError("exporter must use a read-only System")
        sidecar = Path(parent) / data["sidecar"]
        if not sidecar.exists():
            raise RuntimeError(f"missing exporter sidecar: {sidecar}")
        instance = cls()
        instance.data = data
        return instance
"#,
        )?;

        install_module(
            py,
            "artifact_test_plugins",
            r#"
class Config:
    def __init__(self, **kwargs):
        self.kwargs = kwargs

def echo_stdin(stdin):
    return {"value": stdin["value"]}

def echo_system(system):
    return system

def emit_none():
    return None

def emit_json_text():
    return '{"value":"raw"}'

class BrokenSystem:
    def to_json(self, path):
        return None

def emit_broken_system():
    return BrokenSystem()

class HiddenStdin:
    def __call__(self, **kwargs):
        return {"value": kwargs["stdin"]["value"]}

hidden_stdin = HiddenStdin()

class Modifier:
    @classmethod
    def from_context(cls, context):
        return cls()

    def run(self, system):
        return system

class Exporter:
    @classmethod
    def from_context(cls, context):
        return cls(context)

    def __init__(self, context):
        self.context = context

    def export(self):
        if self.context.system is None:
            raise RuntimeError("missing exporter System")
        return self.context
"#,
        )?;
        Ok(())
    }

    fn system_bundle(root: &std::path::Path, sidecar: &str) -> Result<ArtifactBundle, String> {
        let bundle = ArtifactBundle::new(root, "system.json").map_err(|error| error.to_string())?;
        std::fs::create_dir_all(bundle.root()).map_err(|error| error.to_string())?;
        std::fs::write(
            bundle.entrypoint_path(),
            format!(r#"{{"sidecar":"{sidecar}"}}"#),
        )
        .map_err(|error| error.to_string())?;
        std::fs::write(bundle.root().join(sidecar), "sidecar")
            .map_err(|error| error.to_string())?;
        Ok(bundle)
    }

    fn class_bindings(role: r2x_manifest::runtime::PluginRole) -> RuntimeBindings {
        RuntimeBindings {
            entry_module: "artifact_test_plugins".to_string(),
            entry_name: "Modifier".to_string(),
            plugin_type: r2x_manifest::types::PluginType::Class,
            role,
            call_method: Some("run".to_string()),
            config: Some(r2x_manifest::runtime::RuntimeConfig {
                module: "artifact_test_plugins".to_string(),
                name: "Config".to_string(),
            }),
            parameters: Vec::new(),
            requires_store: false,
        }
    }

    fn function_bindings(
        role: r2x_manifest::runtime::PluginRole,
        parameters: Vec<Parameter>,
    ) -> RuntimeBindings {
        RuntimeBindings {
            entry_module: "artifact_test_plugins".to_string(),
            entry_name: "function".to_string(),
            plugin_type: r2x_manifest::types::PluginType::Function,
            role,
            call_method: None,
            config: None,
            parameters,
            requires_store: false,
        }
    }

    #[test]
    fn python_json_reuses_loaded_callables_for_loads_and_dumps() -> Result<(), String> {
        pyo3::Python::initialize();
        let (python_version, dumped) =
            pyo3::Python::attach(|py| -> Result<(String, String), String> {
                let json = PythonJson::import(py).map_err(|error| error.to_string())?;
                let value = json
                    .loads(r#"{"python": "3.13", "ok": true}"#)
                    .map_err(|error| error.to_string())?;
                let dict = value
                    .cast::<pyo3::types::PyDict>()
                    .map_err(|error| error.to_string())?;
                let python_version = dict
                    .get_item("python")
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "missing python key".to_string())?
                    .extract::<String>()
                    .map_err(|error| error.to_string())?;
                let dumped = json.dumps(&value).map_err(|error| error.to_string())?;

                Ok((python_version, dumped))
            })?;

        assert_eq!(python_version, "3.13");
        assert!(dumped.contains("\"python\""));
        assert!(dumped.contains("\"3.13\""));

        Ok(())
    }

    #[test]
    fn python_json_honors_kwargs_even_when_fast_backend_is_selected() -> Result<(), String> {
        pyo3::Python::initialize();
        let dumped = pyo3::Python::attach(|py| -> Result<String, String> {
            let json = PythonJson::import(py).map_err(|error| error.to_string())?;
            let value = json
                .loads(r#"{"name":"café","ok":true}"#)
                .map_err(|error| error.to_string())?;
            let kwargs = PyDict::new(py);
            kwargs
                .set_item("indent", 2)
                .map_err(|error| error.to_string())?;
            kwargs
                .set_item("ensure_ascii", false)
                .map_err(|error| error.to_string())?;
            json.dumps_with_kwargs(&value, Some(&kwargs))
                .map_err(|error| error.to_string())
        })?;

        assert!(dumped.contains('\n'));
        assert!(dumped.contains("café"));
        Ok(())
    }

    #[test]
    fn python_json_prefers_orjson_and_decodes_bytes_output() -> Result<(), String> {
        pyo3::Python::initialize();
        let (backend, dumped) = pyo3::Python::attach(|py| -> Result<(String, String), String> {
            let fake_orjson_code = CString::new(
                r#"
import json as _json

def loads(value):
    return _json.loads(value)

def dumps(value):
    return _json.dumps(value, separators=(",", ":")).encode("utf-8")
"#,
            )
            .map_err(|error| error.to_string())?;
            let fake_orjson_file =
                CString::new("orjson_test.py").map_err(|error| error.to_string())?;
            let fake_orjson_name = CString::new("orjson").map_err(|error| error.to_string())?;

            let fake_orjson = PyModule::from_code(
                py,
                fake_orjson_code.as_c_str(),
                fake_orjson_file.as_c_str(),
                fake_orjson_name.as_c_str(),
            )
            .map_err(|error| error.to_string())?;

            let sys = PyModule::import(py, "sys").map_err(|error| error.to_string())?;
            let modules_obj = sys.getattr("modules").map_err(|error| error.to_string())?;
            let modules = modules_obj
                .cast::<PyDict>()
                .map_err(|error| error.to_string())?;
            let previous_orjson = modules
                .get_item("orjson")
                .map_err(|error| error.to_string())?;

            modules
                .set_item("orjson", &fake_orjson)
                .map_err(|error| error.to_string())?;

            let test_result = (|| -> Result<(String, String), String> {
                let json = PythonJson::import(py).map_err(|error| error.to_string())?;
                let value = json
                    .loads(r#"{"fast":true}"#)
                    .map_err(|error| error.to_string())?;
                let dumped = json.dumps(&value).map_err(|error| error.to_string())?;
                Ok((json.backend_name().to_string(), dumped))
            })();

            if let Some(previous) = previous_orjson {
                modules
                    .set_item("orjson", previous)
                    .map_err(|error| error.to_string())?;
            } else {
                let _ = modules.del_item("orjson");
            }

            test_result
        })?;

        assert_eq!(backend, "orjson");
        assert_eq!(dumped, r#"{"fast":true}"#);
        Ok(())
    }

    #[test]
    fn method_signature_cache_records_and_reuses_stdin_support() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
class Plugin:
    def run(self, system):
        return system
",
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("plugin_signature_cache_test.py")
                .map_err(|error| error.to_string())?;
            let module_name =
                CString::new("plugin_signature_cache_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let plugin = module
                .getattr("Plugin")
                .and_then(|class| class.call0())
                .map_err(|error| error.to_string())?;
            let method = plugin.getattr("run").map_err(|error| error.to_string())?;

            let cache_key = "plugin-signature-cache:stdin";
            remove_cached_method_signature(cache_key);
            assert!(!has_cached_method_signature(cache_key));

            let first = method_accepts_stdin_cached(cache_key, &method)
                .map_err(|error| error.to_string())?;
            assert!(first);
            assert!(has_cached_method_signature(cache_key));

            let second = method_accepts_stdin_cached(cache_key, &method)
                .map_err(|error| error.to_string())?;
            assert!(second);

            remove_cached_method_signature(cache_key);
            Ok(())
        })
    }

    #[test]
    fn callable_signature_cache_detects_function_stdin_support() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
def run(system):
    return system
",
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("function_signature_cache_test.py")
                .map_err(|error| error.to_string())?;
            let module_name =
                CString::new("function_signature_cache_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let function = module.getattr("run").map_err(|error| error.to_string())?;

            let cache_key = "function-signature-cache:stdin";
            remove_cached_method_signature(cache_key);
            assert!(!has_cached_method_signature(cache_key));

            let first = method_accepts_stdin_cached(cache_key, &function)
                .map_err(|error| error.to_string())?;
            assert!(first);
            assert!(has_cached_method_signature(cache_key));

            let second = method_accepts_stdin_cached(cache_key, &function)
                .map_err(|error| error.to_string())?;
            assert!(second);

            remove_cached_method_signature(cache_key);
            Ok(())
        })
    }

    #[test]
    fn callable_signature_detects_keyword_only_system() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
def run(*, system):
    return system
",
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("keyword_only_signature_test.py")
                .map_err(|error| error.to_string())?;
            let module_name =
                CString::new("keyword_only_signature_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let function = module.getattr("run").map_err(|error| error.to_string())?;

            let accepts_stdin = method_accepts_stdin_cached("keyword-only-system", &function)
                .map_err(|error| error.to_string())?;
            assert!(accepts_stdin);
            remove_cached_method_signature("keyword-only-system");
            Ok(())
        })
    }

    #[test]
    fn callable_signature_rejects_function_without_stdin_parameter() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
def run(config):
    return config
",
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("function_no_stdin_signature_test.py")
                .map_err(|error| error.to_string())?;
            let module_name = CString::new("function_no_stdin_signature_test")
                .map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let function = module.getattr("run").map_err(|error| error.to_string())?;

            let accepts_stdin = method_accepts_stdin_cached("function-no-stdin", &function)
                .map_err(|error| error.to_string())?;
            assert!(!accepts_stdin);
            remove_cached_method_signature("function-no-stdin");
            Ok(())
        })
    }

    #[test]
    fn callable_signature_distinguishes_stdin_and_system_parameters() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
def run_with_stdin(stdin):
    return stdin

def run_with_system(system):
    return system
",
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("function_signature_stdin_system_test.py")
                .map_err(|error| error.to_string())?;
            let module_name = CString::new("function_signature_stdin_system_test")
                .map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let stdin_fn = module
                .getattr("run_with_stdin")
                .map_err(|error| error.to_string())?;
            let system_fn = module
                .getattr("run_with_system")
                .map_err(|error| error.to_string())?;

            let stdin_support =
                method_stdin_support(&stdin_fn).map_err(|error| error.to_string())?;
            let system_support =
                method_stdin_support(&system_fn).map_err(|error| error.to_string())?;

            assert!(stdin_support.accepts_stdin);
            assert!(!stdin_support.accepts_system);
            assert!(system_support.accepts_system);
            assert!(!system_support.accepts_stdin);
            Ok(())
        })
    }

    #[test]
    fn method_signature_distinguishes_stdin_and_system_parameters() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r"
class Plugin:
    def run_with_stdin(self, stdin):
        return stdin

    def run_with_system(self, system):
        return system
",
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("method_signature_stdin_system_test.py")
                .map_err(|error| error.to_string())?;
            let module_name = CString::new("method_signature_stdin_system_test")
                .map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;
            let plugin = module
                .getattr("Plugin")
                .and_then(|class| class.call0())
                .map_err(|error| error.to_string())?;
            let stdin_method = plugin
                .getattr("run_with_stdin")
                .map_err(|error| error.to_string())?;
            let system_method = plugin
                .getattr("run_with_system")
                .map_err(|error| error.to_string())?;

            let stdin_support =
                method_stdin_support(&stdin_method).map_err(|error| error.to_string())?;
            let system_support =
                method_stdin_support(&system_method).map_err(|error| error.to_string())?;

            assert!(stdin_support.accepts_stdin);
            assert!(!stdin_support.accepts_system);
            assert!(system_support.accepts_system);
            assert!(!system_support.accepts_stdin);
            Ok(())
        })
    }

    #[test]
    fn function_call_without_stdin_param_skips_malformed_stdin_json() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r#"
def run():
    return {"ok": True}
"#,
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("malformed_stdin_ignored_test.py")
                .map_err(|error| error.to_string())?;
            let module_name =
                CString::new("malformed_stdin_ignored_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;

            let kwargs = PyDict::new(py);
            let json = PythonJson::import(py).map_err(|error| error.to_string())?;

            let result = Bridge::invoke_function_callable(
                py,
                &module,
                "malformed_stdin_ignored_test",
                "run",
                Some("{not-json"),
                None,
                &kwargs,
                &json,
            )
            .map_err(|error| error.to_string())?;

            let ok = result
                .get_item("ok")
                .map_err(|error| error.to_string())?
                .extract::<bool>()
                .map_err(|error| error.to_string())?;
            assert!(ok);
            Ok(())
        })
    }

    #[test]
    fn function_call_with_stdin_param_skips_system_deserialization() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r#"
def run(stdin):
    return {"value": stdin["ok"]}
"#,
            )
            .map_err(|error| error.to_string())?;
            let file =
                CString::new("stdin_only_function_test.py").map_err(|error| error.to_string())?;
            let module_name =
                CString::new("stdin_only_function_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;

            let kwargs = PyDict::new(py);
            let json = PythonJson::import(py).map_err(|error| error.to_string())?;
            let stdin_obj = json
                .loads(r#"{"ok": true}"#)
                .map_err(|error| error.to_string())?;
            kwargs
                .set_item("stdin", &stdin_obj)
                .map_err(|error| error.to_string())?;

            let result = Bridge::invoke_function_callable(
                py,
                &module,
                "stdin_only_function_test",
                "run",
                Some(r#"{"ok":true}"#),
                Some(&stdin_obj),
                &kwargs,
                &json,
            )
            .map_err(|error| error.to_string())?;

            let value = result
                .get_item("value")
                .map_err(|error| error.to_string())?
                .extract::<bool>()
                .map_err(|error| error.to_string())?;
            assert!(value);
            Ok(())
        })
    }

    #[test]
    fn function_call_injects_stdin_when_runtime_metadata_is_missing() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let code = CString::new(
                r#"
def run(stdin):
    return {"value": stdin["ok"]}
"#,
            )
            .map_err(|error| error.to_string())?;
            let file = CString::new("stdin_injection_fallback_test.py")
                .map_err(|error| error.to_string())?;
            let module_name =
                CString::new("stdin_injection_fallback_test").map_err(|error| error.to_string())?;
            let module =
                PyModule::from_code(py, code.as_c_str(), file.as_c_str(), module_name.as_c_str())
                    .map_err(|error| error.to_string())?;

            let kwargs = PyDict::new(py);
            let json = PythonJson::import(py).map_err(|error| error.to_string())?;

            let result = Bridge::invoke_function_callable(
                py,
                &module,
                "stdin_injection_fallback_test",
                "run",
                Some(r#"{"ok":true}"#),
                None,
                &kwargs,
                &json,
            )
            .map_err(|error| error.to_string())?;

            let value = result
                .get_item("value")
                .map_err(|error| error.to_string())?
                .extract::<bool>()
                .map_err(|error| error.to_string())?;
            assert!(value);
            Ok(())
        })
    }

    #[test]
    fn should_parse_stdin_for_function_kwargs_detects_runtime_param() {
        let with_stdin = RuntimeBindings {
            entry_module: "m".to_string(),
            entry_name: "f".to_string(),
            plugin_type: r2x_manifest::types::PluginType::Function,
            role: r2x_manifest::runtime::PluginRole::Utility,
            call_method: None,
            config: None,
            parameters: vec![Parameter {
                name: "stdin".into(),
                required: false,
                default: None,
                types: Default::default(),
                module: None,
                description: None,
            }],
            requires_store: false,
        };
        assert!(should_parse_stdin_for_function_kwargs(
            Some(r#"{"ok":true}"#),
            Some(&with_stdin)
        ));

        let without_stdin = RuntimeBindings {
            parameters: vec![Parameter {
                name: "config".into(),
                required: false,
                default: None,
                types: Default::default(),
                module: None,
                description: None,
            }],
            ..with_stdin
        };
        assert!(!should_parse_stdin_for_function_kwargs(
            Some(r#"{"ok":true}"#),
            Some(&without_stdin)
        ));
        assert!(should_parse_stdin_for_function_kwargs(
            Some(r#"{"ok":true}"#),
            None
        ));
        assert!(!should_parse_stdin_for_function_kwargs(
            None,
            Some(&without_stdin)
        ));
    }

    #[test]
    fn should_parse_stdin_for_exporter_context_requires_exporter_and_stdin() {
        assert!(should_parse_stdin_for_exporter_context(
            Some(r#"{"ok":true}"#),
            r2x_manifest::runtime::PluginRole::Exporter
        ));
        assert!(!should_parse_stdin_for_exporter_context(
            None,
            r2x_manifest::runtime::PluginRole::Exporter
        ));
        assert!(!should_parse_stdin_for_exporter_context(
            Some(r#"{"ok":true}"#),
            r2x_manifest::runtime::PluginRole::Parser
        ));
    }

    #[test]
    fn stdin_payload_bytes_prefers_raw_stdin_json_when_available() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let json = PythonJson::import(py).map_err(|error| error.to_string())?;
            let module_code = CString::new(
                r"
class NotSerializable:
    pass
obj = NotSerializable()
",
            )
            .map_err(|error| error.to_string())?;
            let module_file =
                CString::new("stdin_payload_prefer_raw.py").map_err(|error| error.to_string())?;
            let module_name =
                CString::new("stdin_payload_prefer_raw").map_err(|error| error.to_string())?;
            let module = PyModule::from_code(
                py,
                module_code.as_c_str(),
                module_file.as_c_str(),
                module_name.as_c_str(),
            )
            .map_err(|error| error.to_string())?;
            let obj = module.getattr("obj").map_err(|error| error.to_string())?;

            let bytes = stdin_payload_bytes(Some(r#"{"raw":true}"#), Some(&obj), &json)
                .map_err(|error| error.to_string())?;

            assert_eq!(bytes, br#"{"raw":true}"#);
            Ok(())
        })
    }

    #[test]
    fn stdin_payload_bytes_falls_back_to_serializing_python_object() -> Result<(), String> {
        pyo3::Python::initialize();
        pyo3::Python::attach(|py| -> Result<(), String> {
            let json = PythonJson::import(py).map_err(|error| error.to_string())?;
            let value = json
                .loads(r#"{"fallback":"ok"}"#)
                .map_err(|error| error.to_string())?;

            let bytes = stdin_payload_bytes(None, Some(&value), &json)
                .map_err(|error| error.to_string())?;
            let rendered = String::from_utf8(bytes).map_err(|error| error.to_string())?;
            assert!(rendered.contains("\"fallback\""));
            assert!(rendered.contains("\"ok\""));
            Ok(())
        })
    }

    #[test]
    fn artifact_mode_keeps_payloads_in_python_and_preserves_sidecars() -> Result<(), String> {
        pyo3::Python::initialize();
        let temp = tempdir().map_err(|error| error.to_string())?;

        pyo3::Python::attach(|py| -> Result<(), String> {
            install_artifact_test_modules(py)?;
            let bridge = Bridge::for_tests();

            let generic_input =
                ArtifactBundle::new(temp.path().join("generic-input"), "input.json")
                    .map_err(|error| error.to_string())?;
            std::fs::create_dir_all(generic_input.root()).map_err(|error| error.to_string())?;
            std::fs::write(generic_input.entrypoint_path(), r#"{"value":"generic"}"#)
                .map_err(|error| error.to_string())?;
            let generic_output =
                ArtifactBundle::new(temp.path().join("generic-output"), "result.json")
                    .map_err(|error| error.to_string())?;

            let generic_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:echo_stdin",
                    "{}",
                    Some(&generic_input),
                    &generic_output,
                    None,
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(generic_result.output_kind, ArtifactOutputKind::Json);
            let generic_json = std::fs::read_to_string(generic_output.entrypoint_path())
                .map_err(|error| error.to_string())?;
            assert!(generic_json.contains("generic"));

            let raw_json_output =
                ArtifactBundle::new(temp.path().join("raw-json-output"), "result.json")
                    .map_err(|error| error.to_string())?;
            let raw_json_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:emit_json_text",
                    "{}",
                    None,
                    &raw_json_output,
                    None,
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(raw_json_result.output_kind, ArtifactOutputKind::Json);
            assert_eq!(
                std::fs::read_to_string(raw_json_output.entrypoint_path())
                    .map_err(|error| error.to_string())?,
                r#"{"value":"raw"}"#
            );

            let hidden_output =
                ArtifactBundle::new(temp.path().join("hidden-output"), "result.json")
                    .map_err(|error| error.to_string())?;
            let stdin_bindings = function_bindings(
                r2x_manifest::runtime::PluginRole::Utility,
                vec![Parameter {
                    name: "stdin".into(),
                    required: true,
                    default: None,
                    types: Default::default(),
                    module: None,
                    description: None,
                }],
            );
            let hidden_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:hidden_stdin",
                    "{}",
                    Some(&generic_input),
                    &hidden_output,
                    Some(&stdin_bindings),
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(hidden_result.output_kind, ArtifactOutputKind::Json);
            assert!(std::fs::read_to_string(hidden_output.entrypoint_path())
                .map_err(|error| error.to_string())?
                .contains("generic"));

            let none_output = ArtifactBundle::new(temp.path().join("none-output"), "result.json")
                .map_err(|error| error.to_string())?;
            let none_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:emit_none",
                    "{}",
                    None,
                    &none_output,
                    None,
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(none_result.output_kind, ArtifactOutputKind::Empty);
            assert!(!none_output.entrypoint_path().exists());

            let broken_output =
                ArtifactBundle::new(temp.path().join("broken-output"), "system.json")
                    .map_err(|error| error.to_string())?;
            let broken_result = bridge.invoke_plugin_with_artifact_bindings(
                "artifact_test_plugins:emit_broken_system",
                "{}",
                None,
                &broken_output,
                None,
            );
            assert!(matches!(
                broken_result,
                Err(crate::errors::BridgeError::InvalidArtifact(_))
            ));

            let system_input = system_bundle(&temp.path().join("system-input"), "series.h5")?;
            let system_output =
                ArtifactBundle::new(temp.path().join("system-output"), "system.json")
                    .map_err(|error| error.to_string())?;
            let system_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:echo_system",
                    "{}",
                    Some(&system_input),
                    &system_output,
                    None,
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(system_result.output_kind, ArtifactOutputKind::System);
            assert!(system_output.entrypoint_path().exists());
            assert!(system_output.root().join("series.h5").exists());

            let class_output = ArtifactBundle::new(temp.path().join("class-output"), "system.json")
                .map_err(|error| error.to_string())?;
            let class_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:Modifier.run",
                    "{}",
                    Some(&system_input),
                    &class_output,
                    Some(&class_bindings(r2x_manifest::runtime::PluginRole::Modifier)),
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(class_result.output_kind, ArtifactOutputKind::System);
            assert!(class_output.root().join("series.h5").exists());

            let exporter_output =
                ArtifactBundle::new(temp.path().join("exporter-output"), "result.json")
                    .map_err(|error| error.to_string())?;
            let mut exporter_bindings = class_bindings(r2x_manifest::runtime::PluginRole::Exporter);
            exporter_bindings.entry_name = "Exporter".to_string();
            exporter_bindings.call_method = Some("export".to_string());
            let exporter_result = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:Exporter.export",
                    "{}",
                    Some(&system_input),
                    &exporter_output,
                    Some(&exporter_bindings),
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(exporter_result.output_kind, ArtifactOutputKind::Empty);
            assert!(!exporter_output.entrypoint_path().exists());

            let inline_exporter = bridge
                .invoke_plugin_with_bindings(
                    "artifact_test_plugins:emit_none",
                    "{}",
                    None,
                    Some(&function_bindings(
                        r2x_manifest::runtime::PluginRole::Exporter,
                        Vec::new(),
                    )),
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(inline_exporter.output, "{}");

            let missing_root = temp.path().join("missing-input");
            std::fs::create_dir_all(&missing_root).map_err(|error| error.to_string())?;
            std::fs::write(
                missing_root.join("system.json"),
                r#"{"sidecar":"missing.h5"}"#,
            )
            .map_err(|error| error.to_string())?;
            let missing_input = ArtifactBundle::new(&missing_root, "system.json")
                .map_err(|error| error.to_string())?;
            let missing_output =
                ArtifactBundle::new(temp.path().join("missing-output"), "system.json")
                    .map_err(|error| error.to_string())?;
            let missing_error = bridge
                .invoke_plugin_with_artifact_bindings(
                    "artifact_test_plugins:echo_system",
                    "{}",
                    Some(&missing_input),
                    &missing_output,
                    None,
                )
                .err()
                .ok_or_else(|| "missing sidecar should fail".to_string())?;
            assert!(missing_error.to_string().contains("missing sidecar"));

            Ok(())
        })
    }
}
