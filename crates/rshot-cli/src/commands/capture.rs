use std::{
    env, fs, io,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rshot_core::{
    capture::{CaptureError, CaptureSource, CapturedImage, DisplayInfo, MacOsCapture},
    elements::{ElementError, ElementInfo, element_at},
    geometry::{Point, Rect},
    png,
};
use serde_json::{Map, Value, json};

use super::elements::parse_point;
use crate::{
    error::{CliError, ExitStatus, UNLOCK_HINT},
    output::{CommandOutput, ImageArtifact},
};

#[derive(clap::Args)]
pub struct Args {
    /// Display ID. Defaults to the primary display.
    #[arg(long, value_name = "ID")]
    display: Option<u32>,
    /// Rectangle in display-local points, measured from the top left.
    #[arg(long, value_name = "X,Y,W,H", value_parser = parse_region, allow_hyphen_values = true)]
    region: Option<Rect<f64>>,
    /// Window ID from the windows command.
    #[arg(long, value_name = "ID", conflicts_with_all = ["display", "region"])]
    window: Option<u32>,
    /// Capture the accessibility element at a point in global screen coordinates.
    #[arg(long, value_name = "X,Y", value_parser = parse_point, allow_hyphen_values = true, conflicts_with_all = ["display", "region", "window"])]
    element_at: Option<Point<f64>>,
    /// Wait this many milliseconds before capturing.
    #[arg(long, value_name = "N", default_value_t = 0)]
    delay_ms: u64,
    /// PNG output path. Defaults to a timestamped file in $TMPDIR/rshot.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let capture = MacOsCapture;
    if args.delay_ms > 0 {
        if args.element_at.is_none() {
            capture.check_capture_available().map_err(capture_error)?;
        }
        thread::sleep(Duration::from_millis(args.delay_ms));
    }
    if let Some(point) = args.element_at {
        let hit = element_at(point)?;
        let displays = capture.displays().map_err(capture_error)?;
        let (display_id, region) = element_region(point, hit.element.bounds, &displays)?;
        let captured = capture
            .capture_region(Some(display_id), region)
            .map_err(capture_error)?;
        return save_capture(captured, args.out, Some(&hit.element));
    }
    let captured = match (args.window, args.region) {
        (Some(window_id), _) => capture.capture_window(window_id),
        (None, Some(region)) => capture.capture_region(args.display, region),
        (None, None) => capture.capture_display(args.display),
    }
    .map_err(capture_error)?;
    save_capture(captured, args.out, None)
}

fn element_region(
    point: Point<f64>,
    bounds: Rect<f64>,
    displays: &[DisplayInfo],
) -> Result<(u32, Rect<f64>), CliError> {
    let display = displays
        .iter()
        .find(|display| {
            let frame = display.bounds;
            (frame.origin.x..frame.origin.x + frame.size.width).contains(&point.x)
                && (frame.origin.y..frame.origin.y + frame.size.height).contains(&point.y)
        })
        .ok_or(CaptureError::DisplayNotFound)?;
    let clipped = bounds
        .intersection(display.bounds)
        .ok_or(ElementError::ElementNotFound)?;
    Ok((
        display.id,
        Rect::new(
            clipped.origin.x - display.bounds.origin.x,
            clipped.origin.y - display.bounds.origin.y,
            clipped.size.width,
            clipped.size.height,
        ),
    ))
}

fn parse_region(value: &str) -> Result<Rect<f64>, &'static str> {
    let coords = value
        .split(',')
        .map(|coord| coord.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Expected four numbers separated by commas: X,Y,W,H")?;
    let [x, y, w, h] = coords.as_slice() else {
        return Err("Expected four numbers separated by commas: X,Y,W,H");
    };
    if !coords.iter().all(|coord| coord.is_finite()) || *x < 0. || *y < 0. || *w <= 0. || *h <= 0. {
        return Err(
            "Region coordinates must be finite, with a nonnegative origin and positive size",
        );
    }
    Ok(Rect::new(*x, *y, *w, *h))
}

pub(super) fn capture_error(error: CaptureError) -> CliError {
    match error {
        CaptureError::PermissionDenied => CliError::from(error).with_hint(
            "Grant screen recording permission to your terminal or calling app in System Settings > Privacy & Security > Screen & System Audio Recording, then relaunch it.",
        ),
        CaptureError::NoActiveDisplay => CliError::new(ExitStatus::Capture, "no_active_display", "No active display is available.")
            .with_hint(UNLOCK_HINT),
        CaptureError::WindowNotFound(window_id) => CliError::new(
            ExitStatus::TargetNotFound,
            "window_not_found",
            format!("Window {window_id} was not found."),
        ),
        _ => error.into(),
    }
}

fn save_capture(
    captured: CapturedImage,
    output: Option<PathBuf>,
    element: Option<&ElementInfo>,
) -> Result<CommandOutput, CliError> {
    let (path, temp_dir) = match output {
        Some(path) => (path, None),
        None => {
            let dir = env::temp_dir().join("rshot");
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos();
            (dir.join(format!("capture-{timestamp}.png")), Some(dir))
        }
    };
    let metadata = captured.metadata;
    let artifact = ImageArtifact::new(path, metadata.pixel_size.width, metadata.pixel_size.height)?;
    if let Some(dir) = temp_dir {
        fs::create_dir_all(dir)?;
        png::save_new(&captured.image, artifact.path())?;
    } else {
        png::save(&captured.image, artifact.path())?;
    }

    let source = match metadata.source {
        CaptureSource::Display => "display",
        CaptureSource::Region => "region",
        CaptureSource::Window { .. } => "window",
    };
    let mut capture = Map::from_iter([
        ("source".into(), json!(source)),
        ("display_id".into(), json!(metadata.display_id)),
        ("scale_factor".into(), json!(metadata.scale_factor)),
        ("bounds".into(), bounds_json(metadata.bounds)),
    ]);
    if let CaptureSource::Window { window_id } = metadata.source {
        capture.insert("window_id".into(), json!(window_id));
    }
    if let Some(element) = element {
        capture.insert("element".into(), json!(element));
    }
    let fields = Map::from_iter([
        ("image".into(), json!(artifact)),
        ("capture".into(), Value::Object(capture)),
    ]);
    Ok(CommandOutput::new(fields, "").with_artifact(&artifact))
}

pub(super) fn bounds_json(bounds: Rect<f64>) -> Value {
    json!({
        "x": bounds.origin.x,
        "y": bounds.origin.y,
        "w": bounds.size.width,
        "h": bounds.size.height,
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use rshot_core::{capture::CaptureMetadata, geometry::Size};

    use crate::output::emit;

    use super::*;

    #[derive(Parser)]
    struct TestArgs {
        #[command(flatten)]
        capture: Args,
    }

    #[test]
    fn parses_default_primary_display_and_a_delayed_region() {
        let args = TestArgs::try_parse_from(["capture"]).unwrap().capture;
        assert_eq!(args.display, None);
        assert_eq!(args.region, None);
        assert_eq!(args.window, None);
        assert_eq!(args.delay_ms, 0);
        assert_eq!(args.out, None);

        let args = TestArgs::try_parse_from([
            "capture",
            "--display",
            "7",
            "--region",
            "12.5, 8,120,100.5",
            "--delay-ms",
            "175",
            "--out",
            "image.png",
        ])
        .unwrap()
        .capture;
        assert_eq!(args.display, Some(7));
        assert_eq!(args.region, Some(Rect::new(12.5, 8., 120., 100.5)));
        assert_eq!(args.delay_ms, 175);
        assert_eq!(args.out, Some(PathBuf::from("image.png")));
    }

    #[test]
    fn rejects_malformed_or_empty_regions_and_conflicting_window_targets() {
        for region in [
            "0,0,1",
            "0,0,1,2,3",
            "0,0,x,2",
            "NaN,0,1,2",
            "0,0,inf,2",
            "-1,0,1,2",
            "0,0,0,2",
        ] {
            let error = TestArgs::try_parse_from(["capture", "--region", region])
                .err()
                .unwrap();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{region}"
            );
        }
        for (flag, value) in [("--display", "1"), ("--region", "0,0,10,10")] {
            let error = TestArgs::try_parse_from(["capture", "--window", "2", flag, value])
                .err()
                .unwrap();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn maps_capture_errors_to_exit_codes_and_actionable_hints() {
        for (error, code, kind) in [
            (CaptureError::PermissionDenied, 4, "permission_denied"),
            (CaptureError::ScreenLocked, 4, "screen_locked"),
            (CaptureError::NoActiveDisplay, 4, "no_active_display"),
            (CaptureError::DisplayNotFound, 6, "display_not_found"),
            (CaptureError::WindowNotFound(2), 6, "window_not_found"),
            (CaptureError::RegionOutOfBounds, 2, "region_out_of_bounds"),
            (
                CaptureError::Screenshot("Native failure".into()),
                4,
                "capture_failed",
            ),
        ] {
            let error = capture_error(error);
            assert_eq!((error.exit_code(), error.kind), (code, kind));
            match kind {
                "permission_denied" => {
                    assert!(error.hint.unwrap().contains("your terminal or calling app"))
                }
                "screen_locked" | "no_active_display" => assert_eq!(
                    error.hint.as_deref(),
                    Some("The screen may be locked or asleep; unlock it and retry")
                ),
                _ => assert!(error.hint.is_none()),
            }
        }
    }

    fn fixture(source: CaptureSource, display_id: Option<u32>) -> CapturedImage {
        CapturedImage {
            image: image::RgbaImage::new(4, 2),
            metadata: CaptureMetadata {
                source,
                display_id,
                scale_factor: 2.,
                bounds: Rect::new(-10., -5., 2., 1.),
                pixel_size: Size {
                    width: 4,
                    height: 2,
                },
            },
        }
    }

    #[test]
    fn saves_png_and_emits_global_bounds_with_physical_image_dimensions() {
        let (window, display, region) = (
            fixture(CaptureSource::Window { window_id: 42 }, None),
            fixture(CaptureSource::Display, Some(1)),
            fixture(CaptureSource::Region, Some(1)),
        );
        for (captured, expected) in [
            (
                window,
                json!({"source":"window", "window_id":42, "display_id":null}),
            ),
            (display, json!({"source":"display", "display_id":1})),
            (region, json!({"source":"region", "display_id":1})),
        ] {
            let output = save_capture(captured, None, None).unwrap();
            let mut stdout = Vec::new();
            let code = emit("capture", true, Ok(output), &mut stdout, &mut Vec::new()).unwrap();
            assert_eq!(code, 0);
            let json: Value = serde_json::from_slice(&stdout).unwrap();
            let mut expected = expected.as_object().unwrap().clone();
            expected.insert("scale_factor".into(), json!(2.));
            expected.insert("bounds".into(), json!({"x":-10., "y":-5., "w":2., "h":1.}));
            assert_eq!(json["capture"], Value::Object(expected));
            assert_eq!(json["image"]["width"], 4);
            assert_eq!(json["image"]["height"], 2);
            let path = PathBuf::from(json["image"]["path"].as_str().unwrap());
            assert!(path.is_absolute());
            assert!(path.starts_with(env::temp_dir().join("rshot")));
            assert_eq!(image::image_dimensions(&path).unwrap(), (4, 2));
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn accepts_negative_element_points_and_rejects_other_targets() {
        let args = TestArgs::try_parse_from(["capture", "--element-at", "-1,2"])
            .unwrap()
            .capture;
        assert_eq!(args.element_at, Some(Point::new(-1., 2.)));
        for (flag, value) in [
            ("--display", "1"),
            ("--region", "0,0,10,10"),
            ("--window", "2"),
        ] {
            let error = TestArgs::try_parse_from(["capture", "--element-at", "-1,2", flag, value])
                .err()
                .unwrap();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn clips_element_capture_to_the_display_containing_the_hit() {
        let display = |id, x, is_primary| DisplayInfo {
            id,
            bounds: Rect::new(x, 0., 100., 100.),
            scale_factor: 2.,
            pixel_size: Size {
                width: 200,
                height: 200,
            },
            is_primary,
        };
        let displays = [display(1, 0., true), display(2, -100., false)];
        let bounds = Rect::new(-10.5, 10., 30., 15.);
        assert_eq!(
            element_region(Point::new(-1., 11.), bounds, &displays).unwrap(),
            (2, Rect::new(89.5, 10., 10.5, 15.))
        );
        assert_eq!(
            element_region(Point::new(0., 11.), bounds, &displays).unwrap(),
            (1, Rect::new(0., 10., 19.5, 15.))
        );
    }

    #[test]
    fn includes_the_matched_element_in_region_capture_metadata() {
        let captured = fixture(CaptureSource::Region, Some(1));
        let element = ElementInfo {
            role: "AXButton".into(),
            subrole: None,
            title: None,
            description: None,
            value: None,
            identifier: None,
            bounds: Rect::new(-9.75, -5., 1.5, 1.),
            enabled: Some(true),
            focused: None,
        };
        let output = save_capture(captured, None, Some(&element)).unwrap();
        let mut stdout = Vec::new();
        assert_eq!(
            emit("capture", true, Ok(output), &mut stdout, &mut Vec::new()).unwrap(),
            0
        );
        let json: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(json["capture"]["source"], "region");
        assert_eq!(
            json["capture"]["bounds"],
            json!(Rect::new(-10., -5., 2., 1.))
        );
        assert_eq!(json["capture"]["element"], json!(element));
        fs::remove_file(json["image"]["path"].as_str().unwrap()).unwrap();
    }

    #[test]
    fn a_save_failure_exits_with_io_status() {
        let captured = fixture(CaptureSource::Display, Some(1));
        let error = save_capture(captured, Some(env::temp_dir()), None).unwrap_err();
        assert_eq!((error.exit_code(), error.kind), (1, "io_error"));
    }
}
