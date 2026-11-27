use std::path::{self, Path, PathBuf};

use image::{
    ColorType, DynamicImage, ImageDecoder, ImageError, ImageReader, ImageResult, Limits, RgbaImage,
    error::{ParameterError, ParameterErrorKind},
};
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
    let decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    Ok(decode_rgba(decoder)?)
}

fn decode_rgba(mut decoder: impl ImageDecoder) -> ImageResult<RgbaImage> {
    let mut limits = Limits::default();
    let decoded_len = decoder.total_bytes();
    limits.reserve(decoded_len)?;
    decoder.set_limits(limits)?;
    if decoder.color_type() != ColorType::Rgb8 {
        return Ok(DynamicImage::from_decoder(decoder)?.into_rgba8());
    }

    let (width, height) = decoder.dimensions();
    let decoded_len = decoded_len as usize;
    let count = decoded_len / 3;
    let mut pixels = vec![0; count * 4];
    decoder.read_image(&mut pixels[..decoded_len])?;
    for i in (0..count).rev() {
        let src = i * 3;
        let rgba = [pixels[src], pixels[src + 1], pixels[src + 2], 255];
        pixels[i * 4..i * 4 + 4].copy_from_slice(&rgba);
    }
    RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
        ImageError::Parameter(ParameterError::from_kind(
            ParameterErrorKind::DimensionMismatch,
        ))
    })
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{
        ImageEncoder,
        codecs::png::{PngDecoder, PngEncoder},
    };

    use super::*;

    #[test]
    fn decode_rgba_matches_image_reader_for_rgb_and_rgba_pngs() {
        for (color, pixels) in [
            (
                ColorType::Rgb8,
                &[25, 50, 75, 18, 36, 75, 9, 200, 234, 255, 70, 90][..],
            ),
            (
                ColorType::Rgba8,
                &[
                    7, 14, 21, 28, 35, 42, 49, 0, 63, 70, 77, 84, 91, 98, 105, 255,
                ][..],
            ),
            (ColorType::La8, &[25, 50, 75, 18, 36, 75, 9, 200][..]),
        ] {
            let mut png = Vec::new();
            PngEncoder::new(&mut png)
                .write_image(pixels, 2, 2, color.into())
                .unwrap();
            let old = image::load_from_memory(&png).unwrap().into_rgba8();
            let new = decode_rgba(PngDecoder::new(Cursor::new(&png)).unwrap()).unwrap();
            assert_eq!(new, old);
            assert_eq!(new.color_space(), old.color_space());
        }
    }
}
