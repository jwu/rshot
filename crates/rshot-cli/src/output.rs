use std::{
    io::{self, Write},
    path::{self, Path},
};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::error::{CliError, ExitStatus};

const SCHEMA_VERSION: u8 = 1;

#[derive(Serialize)]
pub struct ImageArtifact {
    path: String,
    width: u32,
    height: u32,
}

#[allow(dead_code)]
impl ImageArtifact {
    pub fn new(path: impl AsRef<Path>, width: u32, height: u32) -> Result<Self, CliError> {
        let path = path::absolute(path)?;
        let path = path
            .to_str()
            .filter(|path| !path.contains(['\r', '\n']))
            .ok_or_else(|| {
                CliError::new(
                    ExitStatus::Usage,
                    "invalid_output_path",
                    "The output path must be valid UTF-8 and fit on one line.",
                )
            })?;
        Ok(Self {
            path: path.to_owned(),
            width,
            height,
        })
    }

    pub fn path(&self) -> &Path {
        Path::new(&self.path)
    }
}

#[derive(Debug)]
pub struct CommandOutput {
    fields: Map<String, Value>,
    text: String,
    paths: Vec<String>,
    warnings: Vec<String>,
}

impl CommandOutput {
    pub fn new(fields: Map<String, Value>, text: impl Into<String>) -> Self {
        Self {
            fields,
            text: text.into(),
            paths: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn with_artifact(mut self, artifact: &ImageArtifact) -> Self {
        self.paths.push(artifact.path.clone());
        self
    }

    #[allow(dead_code)]
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }
}

pub fn emit(
    command: &str,
    json: bool,
    result: Result<CommandOutput, CliError>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<u8> {
    match result {
        Ok(output) => {
            for warning in &output.warnings {
                writeln!(stderr, "Warning: {warning}")?;
            }
            if json {
                let mut fields = output.fields;
                fields.remove("error");
                fields.insert("schema_version".into(), json!(SCHEMA_VERSION));
                fields.insert("ok".into(), json!(true));
                fields.insert("command".into(), json!(command));
                fields.insert("warnings".into(), json!(output.warnings));
                writeln!(stdout, "{}", Value::Object(fields))?;
            } else if output.paths.is_empty() {
                writeln!(stdout, "{}", output.text)?;
            } else {
                for path in output.paths {
                    writeln!(stdout, "{path}")?;
                }
            }
            Ok(0)
        }
        Err(error) => {
            if json {
                let response = json!({
                    "schema_version": SCHEMA_VERSION,
                    "ok": false,
                    "command": command,
                    "error": error,
                });
                writeln!(stdout, "{response}")?;
            } else {
                writeln!(stderr, "{}", error.message)?;
                if let Some(hint) = &error.hint {
                    writeln!(stderr, "Hint: {hint}")?;
                }
            }
            Ok(error.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_success_includes_command_fields_and_warnings() {
        let image = ImageArtifact::new("output.png", 320, 240).unwrap();
        assert!(image.path().is_absolute());
        let fields = Map::from_iter([("image".into(), json!(image)), ("ok".into(), json!(false))]);
        let result = CommandOutput::new(fields, "Saved image.")
            .with_artifact(&image)
            .with_warning("Test warning.");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        assert_eq!(
            emit("capture", true, Ok(result), &mut stdout, &mut stderr).unwrap(),
            0
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap(),
            json!({
                "schema_version": 1,
                "ok": true,
                "command": "capture",
                "image": { "path": path::absolute("output.png").unwrap(), "width": 320, "height": 240 },
                "warnings": ["Test warning."],
            })
        );
        assert_eq!(stderr, b"Warning: Test warning.\n");
    }

    #[test]
    fn text_success_lists_absolute_paths_or_prints_a_summary() {
        let image = ImageArtifact::new("output.png", 320, 240).unwrap();
        let other = ImageArtifact::new("second.png", 200, 200).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = CommandOutput::new(Map::new(), "Saved images.")
            .with_artifact(&image)
            .with_artifact(&other);
        assert_eq!(
            emit("capture", false, Ok(result), &mut stdout, &mut stderr).unwrap(),
            0
        );
        assert_eq!(
            stdout,
            format!("{}\n{}\n", image.path().display(), other.path().display()).into_bytes()
        );

        stdout.clear();
        let result = CommandOutput::new(Map::new(), "No windows found.");
        assert_eq!(
            emit("windows", false, Ok(result), &mut stdout, &mut stderr).unwrap(),
            0
        );
        assert_eq!(stdout, b"No windows found.\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn failure_includes_a_hint_and_uses_the_error_exit_code() {
        for json in [true, false] {
            let error = CliError::new(
                ExitStatus::AmbiguousTarget,
                "ambiguous_target",
                "Multiple windows matched.",
            )
            .with_hint("Specify a window ID.");
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                emit("capture", json, Err(error), &mut stdout, &mut stderr).unwrap(),
                5
            );
            if json {
                assert_eq!(
                    serde_json::from_slice::<Value>(&stdout).unwrap(),
                    json!({
                        "schema_version": 1,
                        "ok": false,
                        "command": "capture",
                        "error": {
                            "kind": "ambiguous_target",
                            "message": "Multiple windows matched.",
                            "hint": "Specify a window ID.",
                        },
                    })
                );
                assert!(stderr.is_empty());
            } else {
                assert!(stdout.is_empty());
                assert_eq!(
                    stderr,
                    b"Multiple windows matched.\nHint: Specify a window ID.\n"
                );
            }
        }
    }
}
