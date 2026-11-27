use std::path::{self, Path, PathBuf};

use image::{ImageReader, RgbaImage};
use rshot_core::{
    locate::{GridOptions, LocateWarning, Rendered, grid_owned},
    png,
};
use serde_json::{Map, Value, json};

use crate::{
    error::{CliError, ExitStatus},
    output::{CommandOutput, ImageArtifact},
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    /// Output file. Defaults to <input stem>.grid.png beside the input.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
    /// Grid spacing in source pixels, with a minimum of 10.
    #[arg(long, value_name = "N", default_value_t = GridOptions::default().step)]
    step: u32,
    /// Label every Nth grid line. Use 0 to hide labels.
    #[arg(long, value_name = "N", default_value_t = GridOptions::default().label_every)]
    label_every: u32,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let input = open_image(&args.input)?;
    let dimensions = input.dimensions();
    let rendered = grid_owned(
        input,
        GridOptions {
            step: args.step,
            label_every: args.label_every,
            ..GridOptions::default()
        },
    );
    let metadata = rendered.metadata;
    let fields = Map::from_iter([
        ("step".into(), json!(metadata.step)),
        ("columns".into(), json!(metadata.columns)),
        ("rows".into(), json!(metadata.rows)),
    ]);
    save_rendered(&args.input, dimensions, args.out, "grid", rendered, fields)
}

pub(super) fn open_image(path: &Path) -> Result<RgbaImage, CliError> {
    Ok(ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?
        .into_rgba8())
}

pub(super) fn save_rendered<M>(
    input: &Path,
    (source_width, source_height): (u32, u32),
    out: Option<PathBuf>,
    command: &str,
    rendered: Rendered<M>,
    mut fields: Map<String, Value>,
) -> Result<CommandOutput, CliError> {
    let out = out.unwrap_or_else(|| {
        let mut name = input.file_stem().unwrap_or_default().to_os_string();
        name.push(format!(".{command}.png"));
        input.with_file_name(name)
    });
    let artifact = ImageArtifact::new(out, rendered.image.width(), rendered.image.height())?;
    let source_path = path::absolute(input)?;
    let source_path = source_path.to_str().ok_or_else(|| {
        CliError::new(
            ExitStatus::Usage,
            "invalid_input_path",
            "The input path must be valid UTF-8.",
        )
    })?;
    fields.insert("image".into(), json!(artifact));
    fields.insert(
        "source".into(),
        json!({
            "path": source_path,
            "width": source_width,
            "height": source_height,
        }),
    );
    let mut output = CommandOutput::new(fields, "").with_artifact(&artifact);
    for warning in rendered.warnings {
        match warning {
            LocateWarning::RegionClipped { .. } => {
                output = output.with_warning("The zoom region was clipped to the source image.");
            }
            LocateWarning::LabelsUnavailable => {
                return Err(CliError::new(
                    ExitStatus::Render,
                    "text_render_failed",
                    "Could not render the grid labels.",
                ));
            }
        }
    }
    png::save(&rendered.image, artifact.path())?;
    Ok(output)
}
