use rshot_core::capture::{DisplayInfo, MacOsCapture, WindowInfo, window_display_id};
use serde_json::{Map, Value, json};

use super::capture::bounds_json;
use crate::{error::CliError, output::CommandOutput};

#[derive(clap::Args)]
pub struct Args {
    /// Include small windows and nonstandard layers, including desktop elements.
    #[arg(long)]
    all: bool,
    /// Match a case-insensitive substring of the owning application's name.
    #[arg(long, value_name = "NAME")]
    app: Option<String>,
}

pub fn run(args: Args) -> Result<CommandOutput, CliError> {
    let capture = MacOsCapture;
    let displays = capture.displays()?;
    let windows = if args.all {
        capture.all_windows()
    } else {
        capture.windows()
    };
    Ok(window_output(
        windows,
        displays,
        args.app.as_deref(),
        capture.has_screen_recording_permission(),
        capture.is_screen_locked(),
    ))
}

fn window_output(
    mut windows: Vec<WindowInfo>,
    displays: Vec<DisplayInfo>,
    app: Option<&str>,
    screen_recording_permission: bool,
    screen_locked: bool,
) -> CommandOutput {
    if let Some(app) = app {
        let app = app.to_lowercase();
        windows.retain(|window| window.owner_name.to_lowercase().contains(&app));
    }

    let text = if windows.is_empty() {
        "No windows found.".to_owned()
    } else {
        windows
            .iter()
            .map(|window| {
                let app = window.owner_name.replace(['\n', '\r', '\t'], " ");
                let title = window
                    .title
                    .as_deref()
                    .unwrap_or_default()
                    .replace(['\n', '\r', '\t'], " ");
                format!("{}\t{app}\t{title}", window.window_id)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let listed_windows = windows
        .into_iter()
        .map(|window| {
            let mut fields = Map::from_iter([
                ("window_id".into(), json!(window.window_id)),
                ("app".into(), json!(window.owner_name)),
                ("pid".into(), json!(window.owner_pid)),
                ("title".into(), json!(window.title)),
                ("bounds".into(), bounds_json(window.bounds)),
                ("layer".into(), json!(window.layer)),
                ("on_screen".into(), json!(window.is_on_screen)),
            ]);
            if let Some(display_id) = window_display_id(window.bounds, &displays) {
                fields.insert("display_id".into(), json!(display_id));
            }
            Value::Object(fields)
        })
        .collect();
    let no_active_display = displays.is_empty();
    let listed_displays = displays
        .into_iter()
        .map(|display| {
            json!({
                "id": display.id,
                "bounds": bounds_json(display.bounds),
                "scale_factor": display.scale_factor,
                "pixel_size": { "width": display.pixel_size.width, "height": display.pixel_size.height },
                "is_primary": display.is_primary,
            })
        })
        .collect();
    let fields = Map::from_iter([
        ("windows".into(), Value::Array(listed_windows)),
        ("displays".into(), Value::Array(listed_displays)),
        (
            "screen_recording_permission".into(),
            json!(screen_recording_permission),
        ),
        ("screen_locked".into(), json!(screen_locked)),
    ]);
    let mut output = CommandOutput::new(fields, text);
    if !screen_recording_permission {
        output = output
            .with_warning("Window titles may be missing without screen recording permission.");
    }
    match (screen_locked, no_active_display) {
        (true, true) => output
            .with_warning("The screen is locked; the display list is empty and capture will fail."),
        (true, false) => output.with_warning("The screen is locked; capture will fail."),
        (false, true) => output.with_warning("No active display is available; capture will fail."),
        (false, false) => output,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use rshot_core::geometry::{Rect, Size};

    use crate::output::emit;

    use super::*;

    #[derive(Parser)]
    struct TestArgs {
        #[command(flatten)]
        windows: Args,
    }

    #[test]
    fn parses_all_and_app_filters() {
        let args = TestArgs::try_parse_from(["windows", "--all", "--app", "My Browser"])
            .unwrap()
            .windows;
        assert!(args.all);
        assert_eq!(args.app.as_deref(), Some("My Browser"));
    }

    fn display() -> DisplayInfo {
        DisplayInfo {
            id: 3,
            bounds: Rect::new(-100., 0., 100., 100.),
            scale_factor: 2.,
            pixel_size: Size {
                width: 200,
                height: 200,
            },
            is_primary: false,
        }
    }

    fn window(id: u32, app: &str, bounds: Rect<f64>) -> WindowInfo {
        WindowInfo {
            window_id: id,
            owner_name: app.to_owned(),
            owner_pid: 21,
            title: None,
            bounds,
            layer: 0,
            is_on_screen: id == 1,
        }
    }

    #[test]
    fn serializes_filtered_windows_and_displays_with_permission_warning() {
        let windows = vec![
            window(1, "My Browser", Rect::new(-90., 10., 50., 60.)),
            window(2, "Console", Rect::new(-60., 0., 100., 100.)),
            window(3, "Browser Helper", Rect::new(5., 10., 50., 60.)),
        ];
        let output = window_output(windows, vec![display()], Some("bRoWsEr"), false, false);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            emit("windows", true, Ok(output), &mut stdout, &mut stderr).unwrap(),
            0
        );
        let json: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(
            json,
            json!({
                "schema_version": 1, "ok": true, "command": "windows",
                "windows": [
                    {
                        "window_id": 1, "app": "My Browser", "pid": 21, "title": null,
                        "bounds": {"x": -90., "y": 10., "w": 50., "h": 60.},
                        "layer": 0, "on_screen": true, "display_id": 3,
                    },
                    {
                        "window_id": 3, "app": "Browser Helper", "pid": 21, "title": null,
                        "bounds": {"x": 5., "y": 10., "w": 50., "h": 60.},
                        "layer": 0, "on_screen": false,
                    },
                ],
                "displays": [{
                    "id": 3, "bounds": {"x": -100., "y": 0., "w": 100., "h": 100.}, "scale_factor": 2.,
                    "pixel_size": {"width": 200, "height": 200}, "is_primary": false,
                }],
                "screen_recording_permission": false,
                "screen_locked": false,
                "warnings": ["Window titles may be missing without screen recording permission."],
            })
        );
        assert_eq!(
            stderr,
            b"Warning: Window titles may be missing without screen recording permission.\n"
        );
    }

    #[test]
    fn warns_when_locked_or_without_active_displays() {
        for (locked, warning) in [
            (
                true,
                "The screen is locked; the display list is empty and capture will fail.",
            ),
            (false, "No active display is available; capture will fail."),
        ] {
            let output = window_output(Vec::new(), Vec::new(), None, true, locked);
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                emit("windows", true, Ok(output), &mut stdout, &mut stderr).unwrap(),
                0
            );
            let json: Value = serde_json::from_slice(&stdout).unwrap();
            assert_eq!(json["windows"], json!([]));
            assert_eq!(json["displays"], json!([]));
            assert_eq!(json["screen_recording_permission"], true);
            assert_eq!(json["screen_locked"], locked);
            assert_eq!(json["warnings"], json!([warning]));
            assert_eq!(stderr, format!("Warning: {warning}\n").into_bytes());
        }
    }
}
