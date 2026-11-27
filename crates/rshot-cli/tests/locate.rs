#![cfg(target_os = "macos")]

use std::{
    env, fs,
    path::PathBuf,
    process::{self, Command, Output},
    sync::atomic::{AtomicU32, Ordering},
};

use image::{Rgba, RgbaImage};
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
fn locate_commands_report_io_and_usage_errors() {
    let fixture = Fixture::new();
    for (args, code, kind) in [
        (&["--json", "grid", "missing.png"][..], 1, "io_error"),
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
