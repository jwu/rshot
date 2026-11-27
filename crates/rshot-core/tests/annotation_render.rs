use image::{GenericImageView, Rgba, RgbaImage, imageops};
#[cfg(target_os = "macos")]
use rshot_core::annotation::LegendEntry;
use rshot_core::annotation::{Rendered, Spec, render};
use rshot_core::geometry::Rect;
use serde_json::{Value, json};

fn render_marks(image: &RgbaImage, marks: Value) -> Rendered {
    let spec = Spec::from_json(&json!({"schema_version":1,"marks":marks}).to_string()).unwrap();
    render(image, &spec).unwrap()
}

fn pixel_bounds(image: &RgbaImage) -> Rect<u32> {
    let mut bounds = None::<[u32; 4]>;
    for (x, y, _) in image
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[3] > 0)
    {
        bounds = Some(match bounds {
            Some([l, t, r, b]) => [l.min(x), t.min(y), r.max(x), b.max(y)],
            None => [x, y, x, y],
        });
    }
    bounds.map_or(Rect::new(0, 0, 0, 0), |[l, t, r, b]| {
        Rect::new(l, t, r - l + 1, b - t + 1)
    })
}

#[test]
fn redact_replaces_all_channels_and_composes_in_array_order() {
    let source = RgbaImage::from_fn(9, 7, |x, y| Rgba([x as u8, y as u8, 170, 255]));
    let marks = json!([
        {"type":"highlight","rect":{"x":2,"y":1,"w":5,"h":5},"color":"#FFF","opacity":0.6},
        {"type":"redact","rect":{"x":2,"y":1,"w":5,"h":5},"fill":"#2A3B4C00"},
        {"type":"highlight","rect":{"x":2,"y":1,"w":1,"h":1},"color":"#123","opacity":0.4}
    ]);
    let rendered = render_marks(&source, marks.clone());
    for (x, y, pixel) in rendered.image.enumerate_pixels() {
        let expected = match (x, y) {
            (2, 1) => Rgba([17, 34, 51, 102]),
            (2..=6, 1..=5) => Rgba([42, 59, 76, 0]),
            _ => *source.get_pixel(x, y),
        };
        assert_eq!(*pixel, expected, "at ({x}, {y})");
    }
    let alternate = RgbaImage::from_pixel(9, 7, Rgba([80, 90, 100, 200]));
    let alternate = render_marks(&alternate, marks);
    for y in 1..6 {
        for x in 2..7 {
            assert_eq!(
                rendered.image.get_pixel(x, y),
                alternate.image.get_pixel(x, y)
            );
        }
    }
    assert_eq!(rendered.marks[1].bounds, Rect::new(2, 1, 5, 5));
    assert!(rendered.warnings.is_empty());
    assert_eq!(source.get_pixel(2, 1), &Rgba([2, 1, 170, 255]));
}

#[test]
fn rectangle_stroke_is_inside_the_declared_bounds() {
    let original = Rgba([3, 7, 11, 255]);
    let stroke = Rgba([230, 57, 70, 255]);
    let source = RgbaImage::from_pixel(13, 12, original);
    let rendered = render_marks(
        &source,
        json!([
            {"type":"rect","rect":{"x":2,"y":3,"w":8,"h":6},"width":2}
        ]),
    );
    for (x, y, pixel) in rendered.image.enumerate_pixels() {
        let expected = match (x, y) {
            (4..=7, 5..=6) => original,
            (2..=9, 3..=8) => stroke,
            _ => original,
        };
        assert_eq!(*pixel, expected, "at ({x}, {y})");
    }
    assert_eq!(
        serde_json::to_value(&rendered.marks).unwrap(),
        json!([
            {"index":0,"type":"rect","bounds":{"x":2,"y":3,"w":8,"h":6}}
        ])
    );
}

#[test]
fn normal_highlight_multiplies_opacity_by_the_color_alpha() {
    let source = RgbaImage::from_fn(2, 2, |x, _| {
        Rgba([20, 40, 60, if x == 0 { 255 } else { 128 }])
    });
    let rendered = render_marks(
        &source,
        json!([
            {"type":"highlight","rect":{"x":0,"y":0,"w":2,"h":1},"color":"#C88C5080","opacity":0.5,"blend":"normal"}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(0, 0), &Rgba([65, 65, 65, 255]));
    assert_eq!(rendered.image.get_pixel(1, 0), &Rgba([92, 80, 68, 160]));
    assert_eq!(rendered.image.get_pixel(1, 1), source.get_pixel(1, 1));
    assert_eq!(rendered.marks[0].bounds, Rect::new(0, 0, 2, 1));
}

#[test]
fn multiply_highlight_tints_white_and_preserves_dark_pixels() {
    let source = RgbaImage::from_fn(2, 2, |x, _| {
        if x == 0 {
            Rgba([255; 4])
        } else {
            Rgba([18, 18, 18, 255])
        }
    });
    let rendered = render_marks(
        &source,
        json!([
            {"type":"highlight","rect":{"x":0,"y":0,"w":2,"h":1},"opacity":1},
            {"type":"highlight","rect":{"x":0,"y":1,"w":2,"h":1},"blend":"multiply"}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(0, 0), &Rgba([255, 216, 78, 255]));
    assert_eq!(rendered.image.get_pixel(1, 0), &Rgba([18, 15, 6, 255]));
    assert_eq!(rendered.image.get_pixel(0, 1), &Rgba([255, 239, 184, 255]));
    assert_eq!(rendered.image.get_pixel(1, 1), &Rgba([18, 17, 13, 255]));
}

#[test]
fn multiply_highlight_accounts_for_backdrop_and_color_alpha() {
    let source = RgbaImage::from_fn(3, 1, |x, _| Rgba([20, 40, 60, [255, 128, 0][x as usize]]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"highlight","rect":{"x":0,"y":0,"w":3,"h":1},"color":"#C88C5080","opacity":0.5}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(0, 0), &Rgba([19, 35, 50, 255]));
    assert_eq!(rendered.image.get_pixel(1, 0), &Rgba([55, 56, 56, 160]));
    assert_eq!(rendered.image.get_pixel(2, 0), &Rgba([200, 140, 80, 64]));
}

#[test]
fn mosaic_averages_each_visible_block_without_changing_the_surroundings() {
    let source = RgbaImage::from_fn(8, 6, |x, y| Rgba([10 * x as u8, 10 * y as u8, 170, 255]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"mosaic","rect":{"x":-1,"y":1,"w":6,"h":4},"block":3}
        ]),
    );
    for (x, y, pixel) in rendered.image.enumerate_pixels() {
        let expected = match (x, y) {
            (0..=2, 1..=3) => Rgba([10, 20, 170, 255]),
            (3..=4, 1..=3) => Rgba([35, 20, 170, 255]),
            (0..=2, 4) => Rgba([10, 40, 170, 255]),
            (3..=4, 4) => Rgba([35, 40, 170, 255]),
            _ => *source.get_pixel(x, y),
        };
        assert_eq!(*pixel, expected, "at ({x}, {y})");
    }
    assert_eq!(rendered.marks[0].bounds, Rect::new(0, 1, 5, 4));
    assert_eq!(rendered.warnings.len(), 2);
    assert_eq!(
        (rendered.warnings[0].index, rendered.warnings[0].field),
        (0, "block")
    );
    assert_eq!(rendered.warnings[1].field, "rect");
}

#[test]
fn blur_reduces_variance_inside_its_rectangle() {
    let source = RgbaImage::from_fn(40, 38, |x, y| {
        let value = ((x + y) % 2 * 255) as u8;
        Rgba([value, value, value, 255])
    });
    let rendered = render_marks(
        &source,
        json!([
            {"type":"blur","rect":{"x":6,"y":8,"w":24,"h":20},"radius":4}
        ]),
    );
    let variance = |image: &RgbaImage| {
        let pixels: Vec<_> = imageops::crop_imm(image, 6, 8, 24, 20)
            .pixels()
            .map(|(_, _, pixel)| f64::from(pixel[0]))
            .collect();
        let mean = pixels.iter().sum::<f64>() / pixels.len() as f64;
        pixels
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / pixels.len() as f64
    };
    assert!(variance(&rendered.image) < variance(&source) / 10.);
    for (x, y, pixel) in rendered.image.enumerate_pixels() {
        if !(6..30).contains(&x) || !(8..28).contains(&y) {
            assert_eq!(pixel, source.get_pixel(x, y));
        }
        assert_eq!(pixel[3], 255);
    }
    assert_eq!(rendered.marks[0].bounds, Rect::new(6, 8, 24, 20));
    assert_eq!(rendered.warnings.len(), 1);
    assert_eq!(
        (rendered.warnings[0].index, rendered.warnings[0].field),
        (0, "radius")
    );
}

#[test]
fn blur_ignores_color_in_transparent_pixels() {
    let source = RgbaImage::from_fn(9, 4, |x, _| {
        if x < 4 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 0])
        }
    });
    let rendered = render_marks(
        &source,
        json!([
            {"type":"blur","rect":{"x":0,"y":0,"w":9,"h":4},"radius":4294967295u32}
        ]),
    );
    let pixel = rendered.image.get_pixel(4, 1);
    assert_eq!(&pixel.0[..3], &[255, 0, 0]);
    assert!((1..255).contains(&pixel[3]));
}

#[test]
fn spotlight_preserves_the_union_of_its_focus_areas() {
    let base = Rgba([200, 100, 50, 255]);
    let source = RgbaImage::from_pixel(18, 12, base);
    let rendered = render_marks(
        &source,
        json!([
            {"type":"spotlight","rects":[{"x":2,"y":2,"w":8,"h":8},{"x":7,"y":2,"w":8,"h":8}],"shape":"ellipse"}
        ]),
    );
    for (x, y) in [(2, 2), (0, 0), (17, 11), (15, 5)] {
        assert_eq!(rendered.image.get_pixel(x, y), &Rgba([80, 40, 20, 255]));
    }
    for (x, y) in [(5, 5), (8, 5), (11, 5)] {
        assert_eq!(rendered.image.get_pixel(x, y), &base);
    }
    assert_eq!(rendered.marks[0].bounds, Rect::new(0, 0, 18, 12));
    assert!(rendered.warnings.is_empty());
}

#[test]
fn spotlight_reports_only_the_bounds_it_dims() {
    let source = RgbaImage::from_pixel(6, 4, Rgba([200, 100, 50, 255]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"spotlight","rects":[{"x":-1,"y":0,"w":5,"h":4}],"color":"#00000080","opacity":0.5},
            {"type":"spotlight","rects":[{"x":0,"y":0,"w":6,"h":4}]}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(0, 0), source.get_pixel(0, 0));
    assert_eq!(rendered.image.get_pixel(3, 3), source.get_pixel(3, 3));
    assert_eq!(rendered.image.get_pixel(4, 3), &Rgba([150, 75, 37, 255]));
    assert_eq!(rendered.marks[0].bounds, Rect::new(4, 0, 2, 4));
    assert_eq!(rendered.marks[1].bounds, Rect::new(0, 0, 0, 0));
    assert_eq!(rendered.warnings.len(), 1);
    assert_eq!(
        (rendered.warnings[0].index, rendered.warnings[0].field),
        (0, "rects")
    );
}

#[test]
fn magnify_samples_the_original_at_nearest_neighbor_resolution() {
    let source = RgbaImage::from_fn(44, 24, |x, y| Rgba([x as u8, y as u8, 170, 255]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"redact","rect":{"x":2,"y":3,"w":5,"h":4}},
            {"type":"magnify","source":{"x":2,"y":3,"w":5,"h":4},"at":{"x":20,"y":8},"scale":2.5,"width":1,"connector":false}
        ]),
    );
    for (x, y, pixel) in rendered.image.enumerate_pixels() {
        let expected = match (x, y) {
            (2..=6, 3..=6) => Rgba([0, 0, 0, 255]),
            (20 | 32, 8..=17) | (20..=32, 8 | 17) => Rgba([39, 110, 241, 255]),
            (21..=31, 9..=16) => *source.get_pixel(2 + (x - 20) * 2 / 5, 3 + (y - 8) * 2 / 5),
            _ => *source.get_pixel(x, y),
        };
        assert_eq!(*pixel, expected, "at ({x}, {y})");
    }
    assert_eq!(rendered.marks[1].bounds, Rect::new(20, 8, 13, 10));
    assert!(rendered.warnings.is_empty());
}

#[test]
fn magnify_connector_contributes_to_its_bounds() {
    let source = RgbaImage::from_pixel(32, 20, Rgba([255; 4]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"magnify","source":{"x":1,"y":2,"w":5,"h":5},"at":{"x":15,"y":0},"width":1}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(7, 4), &Rgba([65, 128, 243, 255]));
    assert_eq!(rendered.image.get_pixel(8, 6), &Rgba([255; 4]));
    assert_eq!(rendered.marks[0].bounds, Rect::new(3, 0, 22, 10));
    assert!(rendered.warnings.is_empty());

    let clipped = render_marks(
        &source,
        json!([
            {"type":"magnify","source":{"x":1,"y":2,"w":5,"h":5},"at":{"x":15,"y":0},"width":12}
        ]),
    );
    assert_eq!(clipped.warnings.len(), 1);
    assert_eq!(clipped.warnings[0].field, "connector");
}

#[test]
fn magnify_clips_the_source_and_destination() {
    let source = RgbaImage::from_fn(12, 8, |x, y| Rgba([x as u8, y as u8, 100, 255]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"magnify","source":{"x":-2,"y":1,"w":5,"h":3},"at":{"x":-2,"y":4},"border":"#0000FF00","connector":false},
            {"type":"magnify","source":{"x":20,"y":1,"w":5,"h":3},"at":{"x":0,"y":0}}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(0, 4), source.get_pixel(1, 1));
    assert_eq!(rendered.image.get_pixel(3, 7), source.get_pixel(2, 2));
    assert_eq!(rendered.marks[0].bounds, Rect::new(0, 4, 4, 4));
    assert_eq!(rendered.marks[1].bounds, Rect::new(0, 0, 0, 0));
    assert_eq!(
        rendered
            .warnings
            .iter()
            .map(|warning| (warning.index, warning.field))
            .collect::<Vec<_>>(),
        [(0, "source"), (0, "at"), (1, "source")]
    );
}

#[test]
fn out_of_bounds_marks_clip_without_moving_and_invisible_marks_keep_their_index() {
    let source = RgbaImage::new(8, 6);
    let rendered = render_marks(
        &source,
        json!([
            {"type":"redact","rect":{"x":-2,"y":4,"w":4,"h":4}},
            {"type":"ellipse","rect":{"x":100,"y":100,"w":4,"h":4}},
            {"type":"highlight","rect":{"x":2,"y":2,"w":4,"h":4}, "opacity":0},
            {"type":"redact","rect":{"x":0,"y":0,"w":0,"h":4}}
        ]),
    );
    assert_eq!(pixel_bounds(&rendered.image), Rect::new(0, 4, 2, 2));
    assert_eq!(rendered.marks[0].bounds, Rect::new(0, 4, 2, 2));
    for (index, mark) in rendered.marks.iter().enumerate().skip(1) {
        assert_eq!(mark.index, index);
        assert_eq!(mark.bounds, Rect::new(0, 0, 0, 0));
    }
    let warnings: Vec<_> = rendered
        .warnings
        .iter()
        .map(|warning| (warning.index, warning.field))
        .collect();
    assert_eq!(warnings, [(0, "rect"), (1, "rect")]);
}

#[test]
fn stroked_arrow_bounds_include_the_head_and_clip_to_pixels() {
    let source = RgbaImage::new(35, 25);
    let rendered = render_marks(
        &source,
        json!([
            {"type":"arrow","from":{"x":-6,"y":12},"to":{"x":27,"y":12},"width":4,"stroke":"#FF000080"}
        ]),
    );
    let bounds = pixel_bounds(&rendered.image);
    assert_eq!(rendered.marks[0].bounds, bounds);
    assert_eq!(bounds.origin.x, 0);
    assert_eq!(bounds.size.width, 30);
    assert!(bounds.size.height > 4);
    assert_eq!(rendered.image.get_pixel(18, 12), &Rgba([255, 0, 0, 128]));
    assert_eq!(rendered.warnings.len(), 1);
}

#[test]
fn a_distant_arrow_endpoint_leaves_the_visible_shaft_in_place() {
    let source = RgbaImage::new(20, 20);
    let rendered = render_marks(
        &source,
        json!([
            {"type":"arrow","from":{"x":-1000000000,"y":5},"to":{"x":10,"y":5},"width":2}
        ]),
    );
    assert_eq!(rendered.image.get_pixel(0, 5), &Rgba([230, 57, 70, 255]));
    assert_eq!(rendered.warnings.len(), 1);
}

#[test]
fn render_validates_specs_constructed_directly() {
    let source = RgbaImage::new(12, 12);
    let mut spec = Spec::from_json(
        r#"{"schema_version":1,"marks":[{"type":"highlight","rect":{"x":0,"y":0,"w":3,"h":5}}]}"#,
    )
    .unwrap();
    if let rshot_core::annotation::spec::MarkSpec::Highlight(mark) = &mut spec.marks[0] {
        mark.opacity = f64::NAN;
    }
    let error = render(&source, &spec).unwrap_err();
    assert!(error.to_string().starts_with("marks[0].opacity:"));
}

#[cfg(target_os = "macos")]
#[test]
fn step_numbers_follow_the_highest_previous_number_and_labels_stay_in_the_legend() {
    let source = RgbaImage::new(175, 32);
    let marks = json!([
        {"type":"step","at":{"x":16,"y":16},"label":"First action"},
        {"type":"step","at":{"x":51,"y":16},"n":7},
        {"type":"step","at":{"x":86,"y":16},"n":null},
        {"type":"step","at":{"x":121,"y":16},"n":3},
        {"type":"step","at":{"x":156,"y":16}}
    ]);
    let rendered = render_marks(&source, marks.clone());
    assert_eq!(
        rendered.legend,
        vec![
            LegendEntry {
                n: 1,
                label: Some("First action".to_owned())
            },
            LegendEntry { n: 7, label: None },
            LegendEntry { n: 8, label: None },
            LegendEntry { n: 3, label: None },
            LegendEntry { n: 9, label: None },
        ]
    );
    for (index, (mark, n)) in rendered.marks.iter().zip([1, 7, 8, 3, 9]).enumerate() {
        let x = 2 + 35 * index as u32;
        assert_eq!(mark.n, Some(n));
        assert_eq!(mark.bounds, Rect::new(x, 2, 28, 28));
        assert_eq!(rendered.image.get_pixel(x, 2), &Rgba([0; 4]));
        assert!(
            (x..x + 28).any(|col| (2..30).any(|row| rendered.image.get_pixel(col, row)[0] > 220))
        );
    }
    assert!(rendered.warnings.is_empty());
    let mut different_labels = marks;
    different_labels[0]["label"] =
        json!("A very long label that must never be drawn on top of the image");
    assert_eq!(
        rendered.image,
        render_marks(&source, different_labels).image
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_small_step_stays_inside_its_badge_bounds() {
    let source = RgbaImage::new(20, 20);
    for size in 1..10 {
        let rendered = render_marks(
            &source,
            json!([
                {"type":"step","at":{"x":5,"y":5},"n":7,"size":size}
            ]),
        );
        let edge = 5 - size / 2;
        assert_eq!(rendered.marks[0].bounds, Rect::new(edge, edge, size, size));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn text_rasterizes_visible_glyphs_and_clips_at_the_requested_origin() {
    let marks = |x, y| {
        json!([
            {"type":"text","at":{"x":x,"y":y},"text":"Test\n你好","color":"#102030","size":22}
        ])
    };
    let source = RgbaImage::new(120, 90);
    let full = render_marks(&source, marks(0, 0));
    let bounds = pixel_bounds(&full.image);
    assert!(bounds.size.height > 22);
    assert!(full.image.pixels().filter(|pixel| pixel[3] > 120).count() > 40);
    assert_eq!(full.marks[0].bounds, bounds);
    assert!(full.warnings.is_empty());

    let clipped = render_marks(&RgbaImage::new(25, 18), marks(-6, -11));
    assert_eq!(
        clipped.image,
        image::imageops::crop_imm(&full.image, 6, 11, 25, 18).to_image()
    );
    assert_eq!(clipped.marks[0].bounds, pixel_bounds(&clipped.image));
    assert_eq!(clipped.warnings.len(), 1);

    let with_background = render_marks(
        &source,
        json!([
            {"type":"text","at":{"x":3,"y":4},"text":"Test\n你好","background":"#FF880080"}
        ]),
    );
    assert_eq!(
        with_background.image.get_pixel(3, 4),
        &Rgba([255, 136, 0, 128])
    );
    assert_eq!(with_background.marks[0].bounds.origin.x, 3);
    assert_eq!(with_background.marks[0].bounds.origin.y, 4);
}

#[test]
fn geometric_marks_match_the_golden_image() {
    let source = RgbaImage::from_fn(120, 96, |x, y| Rgba([90 + x as u8, 80 + y as u8, 130, 255]));
    let rendered = render_marks(
        &source,
        json!([
            {"type":"rect","rect":{"x":9,"y":6,"w":93,"h":44},"radius":12,"fill":"#FFFFFF66"},
            {"type":"ellipse","rect":{"x":43,"y":26,"w":65,"h":45},"fill":"#276EF144"},
            {"type":"highlight","rect":{"x":15,"y":49,"w":82,"h":39}},
            {"type":"arrow","from":{"x":17,"y":68},"to":{"x":82,"y":17},"width":5},
            {"type":"redact","rect":{"x":71,"y":77,"w":41,"h":10}},
            {"type":"step","at":{"x":110,"y":34},"size":22,"color":"#FFFFFF00"}
        ]),
    );
    assert_golden(&rendered.image, "annotation.png");
}

#[test]
fn effect_marks_match_the_golden_image() {
    let source = RgbaImage::from_fn(128, 96, |x, y| {
        let pattern = ((x * 13) ^ (y * 37)) % 80;
        Rgba([120 + pattern as u8, 50 + x as u8, 60 + y as u8, 255])
    });
    let rendered = render_marks(
        &source,
        json!([
            {"type":"spotlight","rects":[{"x":5,"y":5,"w":56,"h":46},{"x":68,"y":3,"w":48,"h":42}],"shape":"ellipse"},
            {"type":"blur","rect":{"x":12,"y":15,"w":42,"h":25},"radius":8},
            {"type":"mosaic","rect":{"x":71,"y":14,"w":37,"h":25},"block":8},
            {"type":"highlight","rect":{"x":73,"y":73,"w":45,"h":10}},
            {"type":"magnify","source":{"x":79,"y":18,"w":18,"h":14},"at":{"x":25,"y":47},"scale":3.0}
        ]),
    );
    assert!(rendered.warnings.is_empty());
    assert_golden(&rendered.image, "annotation-effects.png");
}

fn assert_golden(image: &RgbaImage, filename: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(filename);
    if std::env::var_os("RSHOT_UPDATE_GOLDEN").is_some_and(|value| value == "1") {
        image.save(&path).unwrap();
    }
    let golden = image::open(&path).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), golden.dimensions());
    for (x, y, pixel) in image.enumerate_pixels() {
        assert_eq!(
            pixel,
            golden.get_pixel(x, y),
            "golden mismatch at ({x}, {y})"
        );
    }
}
