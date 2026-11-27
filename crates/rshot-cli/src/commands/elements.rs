use rshot_core::{
    capture::{CaptureMetadata, MacOsCapture},
    elements::{
        DEFAULT_MAX_ELEMENTS, ElementFilter, ElementHit, ElementInfo, ElementList, MAX_ELEMENTS,
        element_at, elements_in_window,
    },
    geometry::{Point, Rect},
};
use serde::Serialize;
use serde_json::{Map, json};

use super::capture::capture_error;
use crate::{error::CliError, output::CommandOutput};

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("target").args(["window", "at"]).required(true)))]
pub struct Args {
    /// List elements intersecting this window.
    #[arg(long, value_name = "ID")]
    window: Option<u32>,
    /// Match this exact accessibility role. Repeat to match multiple roles.
    #[arg(long, value_name = "ROLE", requires = "window", conflicts_with = "at")]
    role: Vec<String>,
    /// Limit the number of returned elements. Defaults to 200.
    #[arg(long, value_name = "N", requires = "window", conflicts_with = "at", value_parser = clap::value_parser!(u32).range(1..=MAX_ELEMENTS as i64))]
    max: Option<u32>,
    /// Hit-test a point in global screen coordinates, measured from the top left.
    #[arg(long, value_name = "X,Y", value_parser = parse_point, allow_hyphen_values = true)]
    at: Option<Point<f64>>,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    if let Some(window_id) = args.window {
        let list = elements_in_window(
            window_id,
            ElementFilter {
                roles: &args.role,
                max_results: args
                    .max
                    .map(|max| max as usize)
                    .unwrap_or(DEFAULT_MAX_ELEMENTS),
            },
        )?;
        let metadata = MacOsCapture
            .window_capture_metadata(window_id)
            .map_err(capture_error)?;
        return Ok(list_output(window_id, list, metadata));
    }
    let point = args.at.ok_or_else(|| {
        CliError::new(
            crate::error::ExitStatus::Usage,
            "invalid_usage",
            "Specify --window or --at.",
        )
    })?;
    Ok(hit_output(element_at(point)?))
}

pub(super) fn parse_point(raw: &str) -> Result<Point<f64>, &'static str> {
    const MESSAGE: &str = "Expected two finite numbers separated by a comma: X,Y";
    let (x, y) = raw.split_once(',').ok_or(MESSAGE)?;
    let parse = |coord: &str| {
        coord
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|coord| coord.is_finite())
            .ok_or(MESSAGE)
    };
    Ok(Point::new(parse(x)?, parse(y)?))
}

#[derive(Serialize)]
struct ImageElement<'a> {
    id: usize,
    #[serde(flatten)]
    element: &'a ElementInfo,
    image_rect: Rect<u32>,
}

fn list_output(window_id: u32, list: ElementList, metadata: CaptureMetadata) -> CommandOutput {
    let elements: Vec<_> = list
        .elements
        .iter()
        .filter_map(|entry| {
            Some(ImageElement {
                id: entry.id,
                element: &entry.element,
                image_rect: image_rect(entry.element.bounds, metadata)?,
            })
        })
        .collect();
    let fields = Map::from_iter([
        ("window_id".into(), json!(window_id)),
        ("window_bounds".into(), json!(metadata.bounds)),
        ("scale_factor".into(), json!(metadata.scale_factor)),
        ("elements".into(), json!(elements)),
        ("truncated".into(), json!(list.truncated)),
    ]);
    let mut lines = elements
        .iter()
        .map(|entry| {
            let title = entry
                .element
                .title
                .as_deref()
                .unwrap_or_default()
                .replace(['\n', '\r', '\t'], " ");
            let rect = entry.image_rect;
            format!(
                "{}\t{}\t{},{},{},{}\t{title}",
                entry.id,
                entry.element.role,
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("No elements found.".into());
    }
    let mut output = CommandOutput::new(fields, lines.join("\n"));
    if list.truncated {
        output = output.with_warning(
            "The accessibility traversal reached a limit. The element list is incomplete.",
        );
    }
    output
}

fn hit_output(hit: ElementHit) -> CommandOutput {
    let fields = Map::from_iter([
        ("element".into(), json!(hit.element)),
        ("ancestors".into(), json!(hit.ancestors)),
    ]);
    let bounds = hit.element.bounds;
    let title = hit
        .element
        .title
        .as_deref()
        .unwrap_or_default()
        .replace(['\n', '\r', '\t'], " ");
    let mut lines = vec![format!(
        "{}\t{},{},{},{}\t{title}",
        hit.element.role, bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height
    )];
    for ancestor in hit.ancestors {
        let role = ancestor.role.as_deref().unwrap_or_default();
        if let Some(bounds) = ancestor.bounds {
            lines.push(format!(
                "  {role}\t{},{},{},{}",
                bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height
            ));
        } else {
            lines.push(format!("  {role}"));
        }
    }
    CommandOutput::new(fields, lines.join("\n"))
}

fn image_rect(bounds: Rect<f64>, metadata: CaptureMetadata) -> Option<Rect<u32>> {
    let clipped = bounds.intersection(metadata.bounds)?;
    let scale = metadata.scale_factor;
    let pixel_span = |start: f64, extent: f64, limit: u32| {
        let first = ((start * scale).floor() as u32).min(limit - 1);
        let end = (((start + extent) * scale).ceil() as u32).clamp(first + 1, limit);
        (first, end - first)
    };
    let (x, w) = pixel_span(
        clipped.origin.x - metadata.bounds.origin.x,
        clipped.size.width,
        metadata.pixel_size.width,
    );
    let (y, h) = pixel_span(
        clipped.origin.y - metadata.bounds.origin.y,
        clipped.size.height,
        metadata.pixel_size.height,
    );
    Some(Rect::new(x, y, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::emit;
    use clap::Parser;
    use rshot_core::{
        capture::CaptureSource,
        elements::{ElementAncestor, ListedElement},
        geometry::Size,
    };
    use serde_json::Value;

    #[derive(Parser)]
    struct TestArgs {
        #[command(flatten)]
        elements: Args,
    }

    fn metadata() -> CaptureMetadata {
        CaptureMetadata {
            source: CaptureSource::Window { window_id: 42 },
            display_id: Some(1),
            scale_factor: 2.,
            bounds: Rect::new(-20., -10., 40.2, 30.),
            pixel_size: Size {
                width: 80,
                height: 60,
            },
        }
    }

    fn button() -> ElementInfo {
        ElementInfo {
            role: "AXButton".into(),
            subrole: None,
            title: Some("Save".into()),
            description: None,
            value: None,
            identifier: Some("save".into()),
            bounds: Rect::new(-20.5, -8.75, 4.75, 5.5),
            enabled: Some(true),
            focused: Some(false),
        }
    }

    #[test]
    fn requires_one_target_and_accepts_repeated_window_roles() {
        let args = TestArgs::try_parse_from([
            "elements", "--window", "42", "--role", "AXButton", "--role", "AXLink", "--max", "10",
        ])
        .unwrap()
        .elements;
        assert_eq!(args.window, Some(42));
        assert_eq!(args.role, ["AXButton", "AXLink"]);
        assert_eq!(args.max, Some(10));
        let args = TestArgs::try_parse_from(["elements", "--at", "-10.5, 8"])
            .unwrap()
            .elements;
        assert_eq!(args.at, Some(Point::new(-10.5, 8.)));
        for args in [
            &["elements"][..],
            &["elements", "--window", "42", "--at", "1,2"],
            &["elements", "--at", "1,2", "--role", "AXButton"],
            &["elements", "--at", "1,2", "--max", "1"],
            &["elements", "--window", "42", "--max", "0"],
            &["elements", "--window", "42", "--max", "1001"],
        ] {
            assert!(TestArgs::try_parse_from(args).is_err(), "{args:?}");
        }
        for point in ["1", "1,2,3", "2,nan", "inf,1"] {
            assert!(parse_point(point).is_err(), "{point}");
        }
    }

    #[test]
    fn rounds_outward_and_clips_to_the_rounded_window_image() {
        let metadata = metadata();
        assert_eq!(
            image_rect(button().bounds, metadata),
            Some(Rect::new(0, 2, 9, 12))
        );
        assert_eq!(
            image_rect(Rect::new(19.4, 18.7, 2., 5.), metadata),
            Some(Rect::new(78, 57, 2, 3))
        );
        assert_eq!(
            image_rect(Rect::new(20.1, 19.8, 1., 1.), metadata),
            Some(Rect::new(79, 59, 1, 1))
        );
        assert_eq!(image_rect(Rect::new(40., 0., 1., 1.), metadata), None);
        let standard = CaptureMetadata {
            scale_factor: 1.,
            pixel_size: Size {
                width: 40,
                height: 30,
            },
            ..metadata
        };
        assert_eq!(
            image_rect(button().bounds, standard),
            Some(Rect::new(0, 1, 5, 6))
        );
    }

    fn response(output: CommandOutput) -> Value {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            emit("elements", true, Ok(output), &mut stdout, &mut stderr).unwrap(),
            0
        );
        serde_json::from_slice(&stdout).unwrap()
    }

    #[test]
    fn serializes_window_elements_and_point_ancestors() {
        let windows = response(list_output(
            42,
            ElementList {
                elements: vec![ListedElement {
                    id: 3,
                    element: button(),
                }],
                truncated: true,
            },
            metadata(),
        ));
        assert_eq!(windows["command"], "elements");
        assert_eq!(windows["window_id"], 42);
        assert_eq!(windows["window_bounds"], json!(metadata().bounds));
        assert_eq!(windows["scale_factor"], 2.);
        assert_eq!(windows["truncated"], true);
        assert_eq!(windows["warnings"].as_array().unwrap().len(), 1);
        assert_eq!(
            windows["elements"],
            json!([{
                "id": 3, "role": "AXButton", "title": "Save", "subrole": null,
                "description": null, "value": null, "identifier": "save", "enabled": true, "focused": false,
                "bounds": {"x": -20.5, "y": -8.75, "w": 4.75, "h": 5.5},
                "image_rect": {"x": 0, "y": 2, "w": 9, "h": 12},
            }])
        );
        let hit = response(hit_output(ElementHit {
            element: button(),
            ancestors: vec![
                ElementAncestor {
                    role: Some("AXWindow".into()),
                    bounds: Some(metadata().bounds),
                },
                ElementAncestor {
                    role: Some("AXApplication".into()),
                    bounds: None,
                },
            ],
        }));
        assert_eq!(hit["element"], json!(button()));
        assert_eq!(
            hit["ancestors"],
            json!([
                {"role": "AXWindow", "bounds": metadata().bounds},
                {"role": "AXApplication", "bounds": null},
            ])
        );
    }
}
