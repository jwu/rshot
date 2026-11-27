#![cfg(target_os = "macos")]

use std::process::{Command, Output};

use serde_json::{Value, json};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rshot-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn argument_errors_use_stderr_in_text_mode() {
    let output = run(&["unknown"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown")
    );
}

#[test]
fn argument_errors_use_the_json_envelope_in_json_mode() {
    for (args, command) in [
        (&["--json", "unknown"][..], "rshot-cli"),
        (&["--json", "capture", "--unknown"][..], "capture"),
        (&["capture", "--json", "--unknown"][..], "capture"),
        (&["--json"][..], "rshot-cli"),
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let response = serde_json::from_slice::<Value>(&output.stdout).unwrap();
        assert_eq!(response["schema_version"], 1);
        assert_eq!(response["ok"], false);
        assert_eq!(response["command"], command);
        assert_eq!(response["error"]["kind"], "invalid_usage");
        assert!(response["error"]["message"].is_string());
        assert_eq!(response["error"]["hint"], Value::Null);
    }
}

#[test]
fn help_and_version_are_successful_json_responses() {
    let output = run(&["capture", "--help", "--json"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let response = serde_json::from_slice::<Value>(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    assert_eq!(response["ok"], true);
    assert_eq!(response["command"], "capture");
    assert!(response["help"].as_str().unwrap().contains("--json"));
    assert_eq!(response["warnings"], json!([]));

    let output = run(&["--json", "--version"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "schema_version": 1,
            "ok": true,
            "command": "rshot-cli",
            "version": env!("CARGO_PKG_VERSION"),
            "warnings": [],
        })
    );
}
