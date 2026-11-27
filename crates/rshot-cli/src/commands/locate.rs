use std::{collections::HashSet, num::NonZeroUsize, path::PathBuf};

use clap::builder::NonEmptyStringValueParser;
use rshot_core::ocr::{LocateOptions, TextObservation, locate};
use serde_json::{Map, json};
use strsim::jaro_winkler;

use super::ocr::{read_text, text_row};
use crate::{
    error::{CliError, ExitStatus},
    output::CommandOutput,
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    #[arg(value_name = "TEXT", value_parser = NonEmptyStringValueParser::new())]
    query: String,
    /// Match a full line of text.
    #[arg(long)]
    exact: bool,
    /// Match uppercase and lowercase characters as written.
    #[arg(long)]
    case_sensitive: bool,
    /// Select the Nth match, starting at 1.
    #[arg(long, value_name = "N", conflicts_with = "all")]
    nth: Option<NonZeroUsize>,
    /// Return every matching occurrence.
    #[arg(long)]
    all: bool,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let lines = read_text(&args.input, &Default::default())?;
    locate_output(
        &lines,
        &args.query,
        LocateOptions {
            exact: args.exact,
            case_sensitive: args.case_sensitive,
            nth: args.nth,
        },
        args.all,
    )
}

fn locate_output(
    lines: &[TextObservation],
    query: &str,
    options: LocateOptions,
    all: bool,
) -> Result<CommandOutput, CliError> {
    let matches = locate(lines, query, options);
    if matches.is_empty() {
        let message = match options.nth {
            Some(nth) => format!("No occurrence {nth} of {query:?} was found."),
            None => format!("No text matched {query:?}."),
        };
        return Err(
            CliError::new(ExitStatus::TargetNotFound, "text_not_found", message)
                .with_hint(closest_lines(lines, query, options.case_sensitive)),
        );
    }
    if matches.len() > 1 && !all {
        return Err(CliError::new(
            ExitStatus::AmbiguousTarget,
            "ambiguous_target",
            format!("{} occurrences matched {query:?}.", matches.len()),
        )
        .with_hint("Use --nth N to select one occurrence or --all to list every occurrence.")
        .with_matches(matches));
    }
    let text = matches
        .iter()
        .map(|found| {
            let row = text_row(found.rect, found.confidence, &found.text);
            format!("{row}\t{}", found.estimated)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let fields = Map::from_iter([("matches".into(), json!(matches))]);
    Ok(CommandOutput::new(fields, text))
}

fn closest_lines(lines: &[TextObservation], query: &str, case_sensitive: bool) -> String {
    if lines.is_empty() {
        return "OCR did not detect any text in the image.".to_owned();
    }
    let query = if case_sensitive {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    let mut ranked: Vec<_> = lines
        .iter()
        .map(|line| {
            let text = if case_sensitive {
                line.text.clone()
            } else {
                line.text.to_lowercase()
            };
            let score = std::iter::once(text.as_str())
                .chain(text.split_whitespace())
                .map(|part| jaro_winkler(&query, part))
                .fold(0., f64::max);
            (score, &line.text)
        })
        .collect();
    ranked.sort_by(|(a, _), (b, _)| b.total_cmp(a));
    let mut seen = HashSet::new();
    let candidates = ranked
        .into_iter()
        .map(|(_, text)| text)
        .filter(|text| seen.insert(*text))
        .take(3)
        .map(|text| format!("{text:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Closest recognized lines are {candidates}.")
}

#[cfg(test)]
mod tests {
    use rshot_core::geometry::Rect;
    use serde_json::Value;

    use super::*;
    use crate::output::emit;

    fn response(
        lines: &[TextObservation],
        query: &str,
        options: LocateOptions,
        all: bool,
    ) -> (u8, Value) {
        let result = locate_output(lines, query, options, all);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = emit("locate", true, result, &mut stdout, &mut stderr).unwrap();
        assert!(stderr.is_empty());
        (code, serde_json::from_slice(&stdout).unwrap())
    }

    #[test]
    fn ambiguous_json_includes_all_matches_and_flags_resolve_it() {
        let lines = [
            TextObservation::new("Save", 1., Rect::new(10, 20, 35, 15)),
            TextObservation::new("Discard or save", 0.8, Rect::new(50, 70, 150, 20)),
        ];
        let options = LocateOptions::default();
        let (code, ambiguous) = response(&lines, "save", options, false);
        assert_eq!(code, 5);
        assert_eq!(ambiguous["error"]["kind"], "ambiguous_target");
        assert_eq!(
            ambiguous["error"]["matches"],
            json!([
                {"text":"Save", "rect":{"x":10,"y":20,"w":35,"h":15}, "confidence":1., "estimated":false},
                {"text":"save", "rect":{"x":160,"y":70,"w":40,"h":20}, "confidence":f64::from(0.8_f32), "estimated":true}
            ])
        );

        let (code, all) = response(&lines, "save", options, true);
        assert_eq!(code, 0);
        assert_eq!(all["matches"], ambiguous["error"]["matches"]);
        let (code, nth) = response(
            &lines,
            "save",
            LocateOptions {
                nth: NonZeroUsize::new(2),
                ..options
            },
            false,
        );
        assert_eq!(code, 0);
        assert_eq!(nth["matches"], json!([all["matches"][1]]));
    }

    #[test]
    fn missing_text_includes_the_closest_distinct_lines() {
        let rect = Rect::new(0, 0, 100, 20);
        let lines = ["Quit", "Save Changes", "Save Changes", "Undo", "Try Again"]
            .map(|text| TextObservation::new(text, 1., rect));
        let (code, missing) = response(&lines, "savr", LocateOptions::default(), false);
        assert_eq!(code, 6);
        assert_eq!(missing["error"]["kind"], "text_not_found");
        let hint = missing["error"]["hint"].as_str().unwrap();
        assert!(
            hint.starts_with("Closest recognized lines are \"Save Changes\""),
            "{hint}"
        );
        assert_eq!(hint.matches('"').count(), 6);
        assert_eq!(hint.matches("Save Changes").count(), 1);
    }
}
