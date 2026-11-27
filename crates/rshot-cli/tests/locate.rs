#![cfg(target_os = "macos")]

use std::{
    env, fs,
    path::PathBuf,
    process::{self, Command, Output},
    sync::atomic::{AtomicU32, Ordering},
};

use image::{Rgba, RgbaImage};
use rshot_core::{
    annotation::{Spec, render},
    geometry::Rect,
};
use serde_json::{Value, json};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = env::temp_dir().join(format!(
            "rshot-cli-locate-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&dir).unwrap();
        let fixture = Self(dir);
        fs::create_dir(fixture.0.join("images")).unwrap();
        RgbaImage::from_fn(100, 80, |x, y| Rgba([x as u8, y as u8, 40, 255]))
            .save(fixture.0.join("images/frame.v1.png"))
            .unwrap();
        fixture
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rshot-cli"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn grid_reports_dimensions_and_saves_beside_the_input() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        "--json",
        "grid",
        "images/frame.v1.png",
        "--step",
        "30",
        "--label-every",
        "2",
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let dir = fixture.0.canonicalize().unwrap();
    let path = dir.join("images/frame.v1.grid.png");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "schema_version": 1,
            "ok": true,
            "command": "grid",
            "image": { "path": path, "width": 100, "height": 80 },
            "source": { "path": dir.join("images/frame.v1.png"), "width": 100, "height": 80 },
            "step": 30,
            "columns": 4,
            "rows": 3,
            "warnings": [],
        })
    );
    assert_eq!(image::image_dimensions(path).unwrap(), (100, 80));
}

#[test]
fn zoom_reports_the_clipped_region_and_its_mapping() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        "zoom",
        "images/frame.v1.png",
        "--region",
        "90,65,30,25",
        "--scale",
        "2.5",
        "--step",
        "4",
        "--out",
        "crop.png",
        "--json",
    ]);
    assert!(output.status.success(), "{output:?}");
    let warning = "The zoom region was clipped to the source image.";
    assert_eq!(output.stderr, format!("Warning: {warning}\n").into_bytes());
    let dir = fixture.0.canonicalize().unwrap();
    let path = dir.join("crop.png");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "schema_version": 1,
            "ok": true,
            "command": "zoom",
            "image": { "path": path, "width": 25, "height": 38 },
            "source": { "path": dir.join("images/frame.v1.png"), "width": 100, "height": 80 },
            "source_region": { "x": 90, "y": 65, "w": 10, "h": 15 },
            "scale": 2.5,
            "mapping": { "offset": { "x": 90, "y": 65 }, "scale": 2.5 },
            "warnings": [warning],
        })
    );
    assert_eq!(image::image_dimensions(path).unwrap(), (25, 38));
}

#[test]
fn zoom_prints_the_default_output_path_in_text_mode() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        "zoom",
        "images/frame.v1.png",
        "--region",
        "20,10,10,6",
        "--max-side",
        "60",
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let path = fixture
        .0
        .canonicalize()
        .unwrap()
        .join("images/frame.v1.zoom.png");
    assert_eq!(output.stdout, format!("{}\n", path.display()).into_bytes());
    assert_eq!(image::image_dimensions(path).unwrap(), (60, 36));
}

#[test]
fn image_commands_report_io_and_usage_errors() {
    let fixture = Fixture::new();
    for (args, code, kind) in [
        (&["--json", "grid", "missing.png"][..], 1, "io_error"),
        (&["--json", "ocr", "missing.png"][..], 1, "io_error"),
        (
            &["--json", "locate", "missing.png", "Save"][..],
            1,
            "io_error",
        ),
        (
            &["--json", "locate", "missing.png", "Save", "--nth", "0"][..],
            2,
            "invalid_usage",
        ),
        (
            &[
                "--json",
                "locate",
                "missing.png",
                "Save",
                "--nth",
                "1",
                "--all",
            ][..],
            2,
            "invalid_usage",
        ),
        (
            &["--json", "locate", "missing.png", ""][..],
            2,
            "invalid_usage",
        ),
        (
            &["--json", "zoom", "missing.png", "--region", "0,0,10,10"][..],
            1,
            "io_error",
        ),
        (
            &[
                "--json",
                "zoom",
                "images/frame.v1.png",
                "--region",
                "0,0,0,10",
            ][..],
            2,
            "empty_region",
        ),
        (
            &[
                "--json",
                "zoom",
                "images/frame.v1.png",
                "--region",
                "100,0,10,10",
            ][..],
            2,
            "region_out_of_bounds",
        ),
        (
            &[
                "--json",
                "zoom",
                "images/frame.v1.png",
                "--region",
                "0,0,10",
            ][..],
            2,
            "invalid_usage",
        ),
        (
            &[
                "--json",
                "zoom",
                "images/frame.v1.png",
                "--region",
                "0,0,10,10",
                "--scale",
                "2",
                "--max-side",
                "40",
            ][..],
            2,
            "invalid_usage",
        ),
    ] {
        let output = fixture.run(args);
        assert_eq!(output.status.code(), Some(code));
        assert!(output.stderr.is_empty());
        let response = serde_json::from_slice::<Value>(&output.stdout).unwrap();
        assert_eq!(response["schema_version"], 1);
        assert_eq!(response["ok"], false);
        assert_eq!(response["command"], args[1]);
        assert_eq!(response["error"]["kind"], kind);
    }
    assert!(!fixture.0.join("images/frame.v1.zoom.png").exists());
}

fn render_text(rows: &[(u32, u32, &str)]) -> RgbaImage {
    let marks: Vec<_> = rows
        .iter()
        .map(|(x, y, text)| {
            json!({"type":"text", "at":{"x":x,"y":y}, "text":text, "color":"#000", "size":42})
        })
        .collect();
    let spec = Spec::from_json(&json!({"schema_version":1,"marks":marks}).to_string()).unwrap();
    let white = RgbaImage::from_pixel(640, 360, Rgba([255; 4]));
    render(&white, &spec).unwrap().image
}

fn ink_bounds(image: &RgbaImage, rows: std::ops::Range<u32>) -> Rect<u32> {
    let pixels = image
        .enumerate_pixels()
        .filter(|(_, y, pixel)| rows.contains(y) && pixel[0] < 128);
    let (left, top, right, bottom) = pixels.fold(
        (image.width(), image.height(), 0, 0),
        |(left, top, right, bottom), (x, y, _)| {
            (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1))
        },
    );
    assert!(
        right > left && bottom > top,
        "No text was drawn in {rows:?}"
    );
    Rect::new(left, top, right - left, bottom - top)
}

fn assert_rect_near(actual: &Value, expected: Rect<u32>) {
    let actual: Rect<u32> = serde_json::from_value(actual.clone()).unwrap();
    const TOLERANCE: u32 = 9;
    for (actual_edge, expected_edge) in [
        (actual.origin.x, expected.origin.x),
        (actual.origin.y, expected.origin.y),
        (
            actual.origin.x + actual.size.width,
            expected.origin.x + expected.size.width,
        ),
        (
            actual.origin.y + actual.size.height,
            expected.origin.y + expected.size.height,
        ),
    ] {
        assert!(
            actual_edge.abs_diff(expected_edge) <= TOLERANCE,
            "{actual:?} vs {expected:?}"
        );
    }
}

#[test]
#[ignore = "requires the local macOS Vision text recognition models"]
fn vision_recognizes_and_locates_english_and_chinese_text() {
    let fixture = Fixture::new();
    let image = render_text(&[
        (48, 30, "Save Changes"),
        (108, 136, "中文识别测试"),
        (180, 242, "Save Draft"),
    ]);
    image.save(fixture.0.join("ocr.png")).unwrap();
    let run = |args: &[&str], code| {
        let output = fixture.run(args);
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let raw = String::from_utf8(output.stdout).unwrap();
        println!("{args:?} -> {raw}");
        serde_json::from_str::<Value>(&raw).unwrap()
    };

    let ocr = run(&["ocr", "ocr.png", "--json"], 0);
    let explicit = run(
        &[
            "ocr", "ocr.png", "--lang", "zh-Hans", "--lang", "en", "--json",
        ],
        0,
    );
    let explicit_lines = explicit["lines"].as_array().unwrap();
    assert_eq!(explicit_lines[1]["text"], "中文识别测试");
    let lines = ocr["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    for (index, (text, rows)) in [
        ("Save Changes", 0..120),
        ("中文识别测试", 120..220),
        ("Save Draft", 220..360),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(lines[index]["text"], text);
        assert_rect_near(&lines[index]["rect"], ink_bounds(&image, rows));
    }

    let fast = run(&["ocr", "ocr.png", "--lang", "en", "--fast", "--json"], 0);
    assert!(
        fast["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["text"] == "Save Changes")
    );

    let chinese = run(&["locate", "ocr.png", "中文", "--json"], 0);
    let chinese = &chinese["matches"][0];
    assert_eq!(chinese["text"], "中文");
    assert_eq!(chinese["estimated"], false);
    assert_rect_near(
        &chinese["rect"],
        ink_bounds(&render_text(&[(108, 136, "中文")]), 120..220),
    );

    let ambiguous = run(&["locate", "ocr.png", "save", "--json"], 5);
    assert_eq!(ambiguous["error"]["kind"], "ambiguous_target");
    let all = run(&["locate", "ocr.png", "save", "--all", "--json"], 0);
    assert_eq!(all["matches"], ambiguous["error"]["matches"]);
    assert_eq!(all["matches"].as_array().unwrap().len(), 2);
    for (index, (x, y)) in [(48, 30), (180, 242)].into_iter().enumerate() {
        let found = &all["matches"][index];
        assert_eq!(found["text"], "Save");
        assert_eq!(found["estimated"], false);
        assert_rect_near(
            &found["rect"],
            ink_bounds(&render_text(&[(x, y, "Save")]), y..y + 80),
        );
    }

    let partial_word = run(&["locate", "ocr.png", "ave", "--nth", "1", "--json"], 0);
    let partial: Rect<u32> =
        serde_json::from_value(partial_word["matches"][0]["rect"].clone()).unwrap();
    let word: Rect<u32> = serde_json::from_value(all["matches"][0]["rect"].clone()).unwrap();
    assert!(partial.origin.x > word.origin.x && partial.size.width < word.size.width);

    let nth = run(&["locate", "ocr.png", "save", "--nth", "2", "--json"], 0);
    assert_eq!(nth["matches"], json!([all["matches"][1]]));
    let missing = run(&["locate", "ocr.png", "saver", "--json"], 6);
    assert_eq!(missing["error"]["kind"], "text_not_found");
    assert!(
        missing["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("Save Changes")
    );
}
