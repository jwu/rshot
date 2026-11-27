use std::{
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{self, PathBuf},
};

use rshot_core::{
    annotation::{Spec, render_owned},
    png,
};
use serde_json::{Map, json};

use super::grid::open_image;
use crate::{
    error::{CliError, ExitStatus},
    output::{CommandOutput, ImageArtifact},
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    /// Annotation spec file, - for stdin, or an inline JSON object beginning with {.
    #[arg(long, value_name = "PATH|-|INLINE_JSON")]
    spec: OsString,
    /// Output PNG. Defaults to <input stem>.annotated.png beside the input.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let input = open_image(&args.input)?;
    let spec = read_spec(args.spec)?;
    let (width, height) = input.dimensions();
    let rendered = render_owned(input, &spec).map_err(|error| {
        CliError::new(
            ExitStatus::Render,
            "annotation_render_failed",
            error.to_string(),
        )
    })?;

    let out = args.out.unwrap_or_else(|| {
        let mut name = args.input.file_stem().unwrap_or_default().to_os_string();
        name.push(".annotated.png");
        args.input.with_file_name(name)
    });
    let artifact = ImageArtifact::new(out, rendered.image.width(), rendered.image.height())?;
    let source = path::absolute(&args.input)?;
    let source = source.to_str().ok_or_else(|| {
        CliError::new(
            ExitStatus::Usage,
            "invalid_input_path",
            "The input path must be valid UTF-8.",
        )
    })?;
    let fields = Map::from_iter([
        ("image".into(), json!(artifact)),
        (
            "source".into(),
            json!({"path": source, "width": width, "height": height}),
        ),
        ("legend".into(), json!(rendered.legend)),
        ("marks".into(), json!(rendered.marks)),
    ]);
    png::save(&rendered.image, artifact.path())?;
    let mut output = CommandOutput::new(fields, "").with_artifact(&artifact);
    for warning in rendered.warnings {
        output = output.with_warning(format!(
            "marks[{}].{}: {}",
            warning.index, warning.field, warning.message
        ));
    }
    Ok(output)
}

fn read_spec(argument: OsString) -> Result<Spec, CliError> {
    let bytes = if argument == "-" {
        let mut contents = Vec::new();
        io::stdin().lock().read_to_end(&mut contents)?;
        contents
    } else if argument.as_encoded_bytes().starts_with(b"{") {
        argument.into_encoded_bytes()
    } else {
        let path = PathBuf::from(argument);
        fs::read(&path).map_err(|error| {
            CliError::new(
                ExitStatus::Io,
                "io_error",
                format!("Could not read spec {}: {error}", path.display()),
            )
        })?
    };
    let invalid_spec = |message| {
        CliError::new(ExitStatus::Usage, "invalid_spec", message).with_hint(
            "Use a UTF-8 JSON object with schema_version 1. Check the reported field against docs/annotation-spec.md.",
        )
    };
    let json = std::str::from_utf8(&bytes).map_err(|error| invalid_spec(error.to_string()))?;
    Spec::from_json(json).map_err(|error| invalid_spec(error.to_string()))
}
