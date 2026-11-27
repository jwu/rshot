#![cfg(target_os = "macos")]

use std::{
    env, fs,
    io::Write,
    path::PathBuf,
    process::{self, Command, Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
};

use image::{Rgba, RgbaImage};
use serde_json::{Value, json};

const INPUT: &str = "images/frame.v1.png";
const DEFAULT_OUTPUT: &str = "images/frame.v1.annotated.png";
const EMPTY_SPEC: &str = r#"{"schema_version":1,"marks":[]}"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = env::temp_dir().join(format!(
            "rshot-cli-annotate-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&dir).unwrap();
        let fixture = Self(dir.canonicalize().unwrap());
        fs::create_dir(fixture.0.join("images")).unwrap();
        RgbaImage::from_pixel(100, 80, Rgba([10, 20, 30, 255]))
            .save(fixture.0.join(INPUT))
            .unwrap();
        fixture
    }

    fn command(&self, input: &str, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rshot-cli"));
        command
            .current_dir(&self.0)
            .args(["annotate", input])
            .args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(INPUT, args).output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn file_spec_reports_metadata_and_writes_a_png_beside_the_input() {
    let fixture = Fixture::new();
    let spec = json!({"schema_version": 1, "marks": [
        {"type":"step","at":{"x":20,"y":30},"size":20,"n":4,"label":"Open settings"},
        {"type":"step","at":{"x":70,"y":50},"size":20,"label":"Save"},
        {"type":"redact","rect":{"x":5,"y":60,"w":15,"h":10}}
    ]});
    fs::write(fixture.0.join("spec.json"), spec.to_string()).unwrap();
    let output = fixture.run(&["--spec", "spec.json", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let path = fixture.0.join(DEFAULT_OUTPUT);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "schema_version":1, "ok":true, "command":"annotate",
            "image":{"path":path,"width":100,"height":80},
            "source":{"path":fixture.0.join(INPUT),"width":100,"height":80},
            "legend":[{"n":4,"label":"Open settings"},{"n":5,"label":"Save"}],
            "marks":[
                {"index":0,"type":"step","n":4,"bounds":{"x":10,"y":20,"w":20,"h":20}},
                {"index":1,"type":"step","n":5,"bounds":{"x":60,"y":40,"w":20,"h":20}},
                {"index":2,"type":"redact","bounds":{"x":5,"y":60,"w":15,"h":10}}
            ],
            "warnings":[]
        })
    );
    let image = image::open(path).unwrap().into_rgba8();
    assert_eq!(image.dimensions(), (100, 80));
    assert_eq!(image.get_pixel(10, 61), &Rgba([0, 0, 0, 255]));
    assert_eq!(image.get_pixel(10, 59), &Rgba([10, 20, 30, 255]));
}

#[test]
fn stdin_spec_uses_the_explicit_output_path() {
    let fixture = Fixture::new();
    let mut child = fixture
        .command(INPUT, &["--spec", "-", "--out", "result.png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(EMPTY_SPEC.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let path = fixture.0.join("result.png");
    assert_eq!(output.stdout, format!("{}\n", path.display()).into_bytes());
    assert_eq!(image::image_dimensions(path).unwrap(), (100, 80));
    assert!(!fixture.0.join(DEFAULT_OUTPUT).exists());
}

#[test]
fn inline_spec_warnings_appear_in_json_and_on_stderr() {
    let fixture = Fixture::new();
    let spec = json!({"schema_version":1,"marks":[
        {"type":"highlight","rect":{"x":95,"y":75,"w":10,"h":10}}
    ]});
    let output = fixture.run(&["--spec", &spec.to_string(), "--json"]);
    assert!(output.status.success(), "{output:?}");
    let warning = "marks[0].rect: mark extends outside the image and was clipped";
    assert_eq!(output.stderr, format!("Warning: {warning}\n").into_bytes());
    let response = serde_json::from_slice::<Value>(&output.stdout).unwrap();
    assert_eq!(response["warnings"], json!([warning]));
    assert_eq!(response["legend"], json!([]));
    assert_eq!(
        response["marks"][0]["bounds"],
        json!({"x":95,"y":75,"w":5,"h":5})
    );
}

#[test]
fn invalid_specs_report_usage_errors_with_a_field_path_and_hint() {
    let fixture = Fixture::new();
    let spec = json!({"schema_version":1,"marks":[
        {"type":"step","at":{"x":10,"y":12}},
        {"type":"rect","rect":{"x":0,"y":0,"w":-1,"h":5}}
    ]});
    let spec = spec.to_string();
    for (spec, message) in [(spec.as_str(), "marks[1].rect.w"), ("{", "line 1")] {
        let output = fixture.run(&["--spec", spec, "--json"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let response = serde_json::from_slice::<Value>(&output.stdout).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["command"], "annotate");
        assert_eq!(response["error"]["kind"], "invalid_spec");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains(message)
        );
        assert!(
            response["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("schema_version 1")
        );
    }
    fs::write(fixture.0.join("not-utf8.json"), [0xff]).unwrap();
    let output = fixture.run(&["--spec", "not-utf8.json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("utf-8"));
    assert!(stderr.contains("Hint:"));
    assert!(!fixture.0.join(DEFAULT_OUTPUT).exists());
}

#[test]
fn missing_inputs_and_unwritable_outputs_use_the_io_exit_code() {
    let fixture = Fixture::new();
    for (input, args) in [
        ("missing.png", &["--spec", EMPTY_SPEC, "--json"][..]),
        (INPUT, &["--spec", "missing.json", "--json"][..]),
        (
            INPUT,
            &[
                "--spec",
                EMPTY_SPEC,
                "--out",
                "missing/result.png",
                "--json",
            ][..],
        ),
    ] {
        let output = fixture.command(input, args).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        let response = serde_json::from_slice::<Value>(&output.stdout).unwrap();
        assert_eq!(response["error"]["kind"], "io_error");
    }
}
