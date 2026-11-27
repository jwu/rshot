use std::path::{Path, PathBuf};

use clap::builder::NonEmptyStringValueParser;
use rshot_core::{
    geometry::Rect,
    ocr::{OcrOptions, RecognitionLevel, TextObservation, recognize_owned},
};
use serde_json::{Map, json};

use super::grid::open_image;
use crate::{error::CliError, output::CommandOutput};

#[derive(clap::Args)]
pub struct Args {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    /// Language code, such as en or zh-Hans. Repeat to set multiple languages.
    #[arg(long = "lang", value_name = "L", value_parser = NonEmptyStringValueParser::new())]
    languages: Vec<String>,
    /// Prefer speed over recognition accuracy.
    #[arg(long)]
    fast: bool,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let options = OcrOptions {
        languages: args.languages,
        level: if args.fast {
            RecognitionLevel::Fast
        } else {
            RecognitionLevel::Accurate
        },
    };
    let lines = read_text(&args.input, &options)?;
    let text = if lines.is_empty() {
        "No text found.".to_owned()
    } else {
        lines
            .iter()
            .map(|line| text_row(line.rect, line.confidence, &line.text))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let fields = Map::from_iter([("lines".into(), json!(lines))]);
    Ok(CommandOutput::new(fields, text))
}

pub(super) fn read_text(
    path: &Path,
    options: &OcrOptions,
) -> Result<Vec<TextObservation>, CliError> {
    let image = open_image(path)?;
    Ok(recognize_owned(image, options)?)
}

pub(super) fn text_row(rect: Rect<u32>, confidence: f32, text: &str) -> String {
    let text = text.replace(['\n', '\r', '\t'], " ");
    format!(
        "{},{},{},{}\t{confidence}\t{text}",
        rect.origin.x, rect.origin.y, rect.size.width, rect.size.height
    )
}
