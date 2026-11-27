use rshot_core::annotation::{AnnotateError, Spec};
use serde_json::json;

#[test]
fn spec_round_trips_with_defaults_and_canonical_colors() {
    let input = json!({
        "schema_version": 1,
        "marks": [
            {"type":"rect", "rect":{"x":20,"y":30,"w":280,"h":80}},
            {"type":"ellipse", "rect":{"x":0,"y":0,"w":20,"h":10}, "fill":"#abc"},
            {"type":"arrow", "from":{"x":180,"y":180}, "to":{"x":290,"y":100}},
            {"type":"text", "at":{"x":52,"y":155}, "background":"#ffffffcc"},
            {"type":"step", "at":{"x":28,"y":170}, "n":null, "label":null},
            {"type":"highlight", "rect":{"x":20,"y":30,"w":280,"h":80}},
            {"type":"redact", "rect":{"x":-4,"y":7,"w":8,"h":9}},
            {"type":"mosaic", "rect":{"x":20,"y":30,"w":280,"h":80}},
            {"type":"blur", "rect":{"x":20,"y":30,"w":280,"h":80}},
            {"type":"spotlight", "rects":[{"x":20,"y":30,"w":280,"h":80}]},
            {"type":"magnify", "source":{"x":20,"y":30,"w":28,"h":8}, "at":{"x":20,"y":60}}
        ]
    });
    let spec: Spec = serde_json::from_value(input).unwrap();
    let wire = serde_json::to_value(&spec).unwrap();
    let expected = json!([
        {"type":"rect", "rect":{"x":20,"y":30,"w":280,"h":80}, "stroke":"#E63946", "width":3, "radius":0, "fill":null},
        {"type":"ellipse", "rect":{"x":0,"y":0,"w":20,"h":10}, "stroke":"#276EF1", "width":3, "fill":"#AABBCC"},
        {"type":"arrow", "from":{"x":180,"y":180}, "to":{"x":290,"y":100}, "stroke":"#E63946", "width":4},
        {"type":"text", "at":{"x":52,"y":155}, "text":"", "size":22, "color":"#111111", "background":"#FFFFFFCC"},
        {"type":"step", "at":{"x":28,"y":170}, "n":null, "size":28, "fill":"#276EF1", "color":"#FFFFFF", "label":null},
        {"type":"highlight", "rect":{"x":20,"y":30,"w":280,"h":80}, "color":"#FFD84E", "opacity":0.4, "blend":"multiply"},
        {"type":"redact", "rect":{"x":-4,"y":7,"w":8,"h":9}, "fill":"#000000"},
        {"type":"mosaic", "rect":{"x":20,"y":30,"w":280,"h":80}, "block":12},
        {"type":"blur", "rect":{"x":20,"y":30,"w":280,"h":80}, "radius":12},
        {"type":"spotlight", "rects":[{"x":20,"y":30,"w":280,"h":80}], "shape":"rect", "opacity":0.6, "color":"#000000"},
        {"type":"magnify", "source":{"x":20,"y":30,"w":28,"h":8}, "at":{"x":20,"y":60}, "scale":2.0, "border":"#276EF1", "width":3, "connector":true}
    ]);
    assert_eq!(wire, json!({"schema_version":1,"marks":expected}));
    assert_eq!(Spec::from_json(&wire.to_string()).unwrap(), spec);
}

#[test]
fn unsupported_schema_version_has_a_specific_error() {
    let input = r#"{"schema_version":2,"marks":[]}"#;
    let error = Spec::from_json(input).unwrap_err();
    assert!(matches!(error, AnnotateError::SchemaVersion(2)));
    assert_eq!(error.to_string(), "schema_version: expected 1, got 2");
    assert!(serde_json::from_str::<Spec>(input).is_err());
    assert!(Spec::from_json("{").is_err());
}

#[test]
fn invalid_marks_identify_their_index_and_field() {
    let cases = [
        (json!({"type":"triangle"}), "type", "unknown variant"),
        (json!({"type":3}), "type", "invalid type"),
        (json!({}), "type", "missing field"),
        (json!({"type":"text"}), "at", "missing field"),
        (
            json!({"type":"arrow", "from":{"x":0}, "to":{"x":1,"y":1}}),
            "from.y",
            "missing field",
        ),
        (
            json!({"type":"ellipse","rect":{"x":0,"y":0,"w":-2,"h":1}}),
            "rect.w",
            "must be nonnegative",
        ),
        (
            json!({"type":"redact","rect":{"x":0,"y":0,"w":2,"h":-1}}),
            "rect.h",
            "must be nonnegative",
        ),
        (
            json!({"type":"redact","rect":{"x":0,"y":0,"w":2,"h":"1"}}),
            "rect.h",
            "invalid type",
        ),
        (
            json!({"type":"step","at":{"x":0,"y":0}, "n":0}),
            "n",
            "greater than zero",
        ),
        (
            json!({"type":"text","at":{"x":0,"y":0}, "size":0}),
            "size",
            "greater than zero",
        ),
        (
            json!({"type":"rect","rect":{"x":0,"y":0,"w":2,"h":1}, "width":0}),
            "width",
            "greater than zero",
        ),
        (
            json!({"type":"highlight","rect":{"x":0,"y":0,"w":2,"h":1}, "opacity":1.1}),
            "opacity",
            "between 0 and 1",
        ),
        (
            json!({"type":"highlight","rect":{"x":0,"y":0,"w":2,"h":1}, "blend":"screen"}),
            "blend",
            "unknown variant",
        ),
        (
            json!({"type":"ellipse","rect":{"x":0,"y":0,"w":2,"h":1}, "stroke":"red"}),
            "stroke",
            "expected a hex color",
        ),
        (
            json!({"type":"text","at":{"x":0,"y":0}, "background":"#hélo"}),
            "background",
            "expected a hex color",
        ),
        (
            json!({"type":"mosaic","rect":{"x":0,"y":0,"w":2,"h":1},"block":0}),
            "block",
            "greater than zero",
        ),
        (
            json!({"type":"blur","rect":{"x":0,"y":0,"w":2,"h":1},"radius":0}),
            "radius",
            "greater than zero",
        ),
        (
            json!({"type":"spotlight","rects":[]}),
            "rects",
            "must contain a rectangle",
        ),
        (
            json!({"type":"spotlight","rects":[{"x":0,"y":0,"w":1,"h":1},{"x":0,"y":0,"w":1,"h":-2}]}),
            "rects[1].h",
            "must be nonnegative",
        ),
        (
            json!({"type":"spotlight","rects":[{"x":0,"y":0,"w":1}],"shape":"rect"}),
            "rects[0].h",
            "missing field",
        ),
        (
            json!({"type":"spotlight","rects":[{"x":0,"y":0,"w":1,"h":1}],"shape":"oval"}),
            "shape",
            "unknown variant",
        ),
        (
            json!({"type":"spotlight","rects":[{"x":0,"y":0,"w":1,"h":1}],"opacity":-0.1}),
            "opacity",
            "between 0 and 1",
        ),
        (
            json!({"type":"magnify","source":{"x":1,"y":2,"w":-3,"h":4},"at":{"x":0,"y":0}}),
            "source.w",
            "must be nonnegative",
        ),
        (
            json!({"type":"magnify","source":{"x":1,"y":2,"w":3,"h":4},"at":{"x":0,"y":0},"scale":0.5}),
            "scale",
            "at least 1",
        ),
        (
            json!({"type":"magnify","source":{"x":1,"y":2,"w":3,"h":4},"at":{"x":0,"y":0},"scale":1e10}),
            "scale",
            "scaled dimensions",
        ),
        (
            json!({"type":"magnify","source":{"x":1,"y":2,"w":3,"h":4},"at":{"x":0,"y":0},"width":0}),
            "width",
            "greater than zero",
        ),
    ];
    for (mark, field, message) in cases {
        let input = json!({
            "schema_version": 1,
            "marks": [{"type":"text", "at":{"x":0,"y":0}}, mark]
        });
        let error = Spec::from_json(&input.to_string()).unwrap_err();
        assert!(
            error.to_string().starts_with(&format!("marks[1].{field}:")),
            "{error}"
        );
        assert!(error.to_string().contains(message), "{error}");
        assert!(
            serde_json::from_value::<Spec>(input)
                .unwrap_err()
                .to_string()
                .contains(&format!("marks[1].{field}:"))
        );
    }
}

#[test]
fn automatic_number_overflow_identifies_the_step() {
    let input = json!({"schema_version":1,"marks":[
        {"type":"step","at":{"x":0,"y":0},"n":u32::MAX},
        {"type":"step","at":{"x":1,"y":1}}
    ]});
    let error = Spec::from_json(&input.to_string()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "marks[1].n: the next automatic number exceeds u32::MAX"
    );
}

#[test]
fn invalid_color_formats_are_rejected() {
    for color in ["", "aabbcc", "#", "#abcd", "#GGG", "#1122334455"] {
        let input = json!({"schema_version":1,"marks":[
            {"type":"redact","rect":{"x":0,"y":0,"w":2,"h":1}, "fill":color}
        ]});
        let error = serde_json::from_value::<Spec>(input).unwrap_err();
        assert!(error.to_string().starts_with("marks[0].fill:"), "{error}");
    }
}
