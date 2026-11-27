use std::path::PathBuf;

use rshot_core::{
    geometry::Rect,
    locate::{LocateError, ZoomOptions, zoom},
};
use serde_json::{Map, json};

use super::grid::{open_image, save_rendered};
use crate::{
    error::{CliError, ExitStatus},
    output::CommandOutput,
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    /// Crop rectangle in source pixels, with a top-left origin.
    #[arg(long, value_name = "X,Y,W,H", value_parser = parse_region)]
    region: Rect<u32>,
    /// Output file. Defaults to <input stem>.zoom.png beside the input.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
    /// Explicit magnification, with a minimum of 1 and a cap of 16.
    #[arg(long, value_name = "F", conflicts_with = "max_side")]
    scale: Option<f64>,
    /// Target length of the longer side in pixels when no scale is supplied.
    #[arg(long, value_name = "N", default_value_t = ZoomOptions::default().max_side)]
    max_side: u32,
    /// Grid spacing in source pixels, with a minimum of 1.
    #[arg(long, value_name = "N", default_value_t = ZoomOptions::default().step)]
    step: u32,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let input = open_image(&args.input)?;
    let rendered = zoom(
        &input,
        args.region,
        ZoomOptions {
            scale: args.scale,
            max_side: args.max_side,
            step: args.step,
            ..ZoomOptions::default()
        },
    )
    .map_err(|error| {
        let kind = match error {
            LocateError::EmptyRegion => "empty_region",
            LocateError::RegionOutsideImage => "region_out_of_bounds",
            LocateError::InvalidScale => "invalid_scale",
            LocateError::InvalidMaxSide => "invalid_max_side",
            LocateError::OutputTooLarge => "output_too_large",
        };
        CliError::new(ExitStatus::Usage, kind, error.to_string())
    })?;
    let metadata = rendered.metadata;
    let region = metadata.source_region;
    let mapping = metadata.mapping;
    let fields = Map::from_iter([
        (
            "source_region".into(),
            json!({ "x": region.origin.x, "y": region.origin.y, "w": region.size.width, "h": region.size.height }),
        ),
        ("scale".into(), json!(metadata.scale)),
        (
            "mapping".into(),
            json!({ "offset": { "x": mapping.offset.x, "y": mapping.offset.y }, "scale": mapping.scale }),
        ),
    ]);
    save_rendered(
        &args.input,
        input.dimensions(),
        args.out,
        "zoom",
        rendered,
        fields,
    )
}

fn parse_region(value: &str) -> Result<Rect<u32>, &'static str> {
    let parts = value
        .split(',')
        .map(|part| part.trim().parse::<u32>())
        .collect::<Result<Vec<_>, _>>();
    match parts.as_deref() {
        Ok(&[x, y, w, h]) => Ok(Rect::new(x, y, w, h)),
        _ => Err("Use X,Y,W,H with four unsigned pixel coordinates."),
    }
}
