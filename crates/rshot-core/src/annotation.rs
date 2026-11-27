use std::sync::Arc;

use image::{Pixel, Rgba, RgbaImage, imageops::replace};

use crate::geometry::Point;

pub mod spec;
pub use spec::{AnnotateError, Spec};

#[cfg(target_os = "macos")]
use crate::platform::macos::text::draw_text;

#[cfg(target_os = "macos")]
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("选区太窄，无法放置文字")]
    TooNarrow,
    #[error("文字超出选区高度，请缩短内容")]
    TooTall,
    #[error("无法创建文字色彩空间")]
    ColorSpace,
    #[error("无法创建文字画布")]
    BitmapContext,
}

pub const ARROW_COLOR: [u8; 3] = [255, 85, 102];

pub enum Mark {
    Arrow {
        start: Point<u32>,
        end: Point<u32>,
        stroke: f64,
    },
    Mosaic {
        start: Point<u32>,
        end: Point<u32>,
        block: u32,
    },
    Text {
        position: Point<u32>,
        label: RgbaImage,
    },
}

impl Mark {
    fn apply(&self, image: &mut RgbaImage) {
        match self {
            Self::Arrow { start, end, stroke } => arrow(image, *start, *end, *stroke),
            Self::Mosaic { start, end, block } => mosaic(image, *start, *end, *block),
            Self::Text {
                position: Point { x, y },
                label,
            } => {
                let x = (*x).min(image.width().saturating_sub(label.width()));
                let y = (*y).min(image.height().saturating_sub(label.height()));
                replace(image, label, i64::from(x), i64::from(y));
            }
        }
    }
}

pub struct Document {
    source: Arc<RgbaImage>,
    result: Arc<RgbaImage>,
    marks: Vec<Mark>,
    undone: Vec<Mark>,
}

impl Document {
    pub fn new(source: RgbaImage) -> Self {
        let source = Arc::new(source);
        Self {
            result: Arc::clone(&source),
            source,
            marks: Vec::new(),
            undone: Vec::new(),
        }
    }

    pub fn result(&self) -> &Arc<RgbaImage> {
        &self.result
    }

    pub fn can_undo(&self) -> bool {
        !self.marks.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn add(&mut self, mark: Mark) {
        mark.apply(Arc::make_mut(&mut self.result));
        self.marks.push(mark);
        self.undone.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(mark) = self.marks.pop() else {
            return false;
        };
        self.undone.push(mark);
        if self.marks.is_empty() {
            self.result = Arc::clone(&self.source);
        } else {
            let result = Arc::make_mut(&mut self.result);
            let pixels: &mut [u8] = result.as_mut();
            pixels.copy_from_slice(self.source.as_raw());
            for mark in &self.marks {
                mark.apply(result);
            }
        }
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(mark) = self.undone.pop() else {
            return false;
        };
        mark.apply(Arc::make_mut(&mut self.result));
        self.marks.push(mark);
        true
    }
}

pub fn arrow_segments(
    start: Point<f64>,
    end: Point<f64>,
    stroke: f64,
) -> [(Point<f64>, Point<f64>); 3] {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let length = dx.hypot(dy);
    if length < f64::EPSILON {
        return [(start, end); 3];
    }
    let head = (stroke * 4.).min(length * 0.63);
    let (along_x, along_y) = (dx / length * head, dy / length * head);
    let (base_x, base_y) = (end.x - along_x, end.y - along_y);
    [
        (start, end),
        (
            Point::new(base_x - along_y * 0.6, base_y + along_x * 0.6),
            end,
        ),
        (
            Point::new(base_x + along_y * 0.6, base_y - along_x * 0.6),
            end,
        ),
    ]
}

fn arrow(image: &mut RgbaImage, start: Point<u32>, end: Point<u32>, stroke: f64) {
    let coords = |Point { x, y }: Point<u32>| Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
    let segments = arrow_segments(coords(start), coords(end), stroke);
    for &(a, b) in &segments {
        line(image, a, b, stroke + 2., [65, 20, 28]);
    }
    for (a, b) in segments {
        line(image, a, b, stroke, ARROW_COLOR);
    }
}

fn line(image: &mut RgbaImage, a: Point<f64>, b: Point<f64>, stroke: f64, color: [u8; 3]) {
    let radius = stroke / 2. + 0.5;
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length_sq = (dx * dx + dy * dy).max(f64::EPSILON);
    let left = (a.x.min(b.x) - radius).floor() as u32;
    let right = ((a.x.max(b.x) + radius).ceil() as u32).min(image.width());
    let top = (a.y.min(b.y) - radius).floor() as u32;
    let bottom = ((a.y.max(b.y) + radius).ceil() as u32).min(image.height());
    for y in top..bottom {
        for x in left..right {
            let (px, py) = (f64::from(x) + 0.5 - a.x, f64::from(y) + 0.5 - a.y);
            let t = ((px * dx + py * dy) / length_sq).clamp(0., 1.);
            let coverage = (radius - (px - t * dx).hypot(py - t * dy)).clamp(0., 1.);
            if coverage > 0. {
                image.get_pixel_mut(x, y).blend(&Rgba([
                    color[0],
                    color[1],
                    color[2],
                    (coverage * 255.).round() as u8,
                ]));
            }
        }
    }
}

fn mosaic(
    image: &mut RgbaImage,
    Point { x: x0, y: y0 }: Point<u32>,
    Point { x: x1, y: y1 }: Point<u32>,
    block: u32,
) {
    let (left, right) = (x0.min(x1), x0.max(x1).saturating_add(1).min(image.width()));
    let (top, bottom) = (y0.min(y1), y0.max(y1).saturating_add(1).min(image.height()));
    let block = block.max(2);
    for y in (top..bottom).step_by(block as usize) {
        for x in (left..right).step_by(block as usize) {
            let (end_x, end_y) = (
                x.saturating_add(block).min(right),
                y.saturating_add(block).min(bottom),
            );
            let mut sum = [0u64; 3];
            for row in y..end_y {
                for col in x..end_x {
                    for (total, channel) in sum.iter_mut().zip(image.get_pixel(col, row).0) {
                        *total += u64::from(channel);
                    }
                }
            }
            let count = u64::from(end_x - x) * u64::from(end_y - y);
            let color = Rgba([
                (sum[0] / count) as u8,
                (sum[1] / count) as u8,
                (sum[2] / count) as u8,
                255,
            ]);
            for row in y..end_y {
                for col in x..end_x {
                    image.put_pixel(col, row, color);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn text_label(
    value: &str,
    scale: f32,
    (max_width, max_height): (u32, u32),
) -> Result<RgbaImage, TextError> {
    let scale = f64::from(scale);
    let padding = (5. * scale).ceil();
    if f64::from(max_width) >= 22. * scale + 2. * padding {
        draw_text(
            value,
            20. * scale,
            padding,
            (max_width, max_height),
            0xffffff,
            true,
        )
    } else {
        Err(TextError::TooNarrow)
    }
}

#[cfg(target_os = "macos")]
pub fn interface_text(
    value: &str,
    size: f64,
    max_width: u32,
    color: u32,
) -> Result<RgbaImage, TextError> {
    draw_text(value, size, 1., (max_width, 2048), color, false)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{ARROW_COLOR, Document, Mark, Point, interface_text, text_label};
    use crate::png;

    #[test]
    fn interface_labels_have_transparent_backgrounds_and_visible_glyphs() {
        for color in [0x101823, 0xf2f6fa] {
            let label = interface_text("保存 PNG", 28., 360, color).unwrap();
            let rgb = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
            assert!(label.pixels().any(|pixel| pixel[3] == 0));
            assert!(
                label
                    .pixels()
                    .any(|pixel| pixel[3] == 255 && pixel.0[..3] == rgb)
            );
        }
    }

    #[test]
    fn arrow_text_and_mosaic_compose_into_the_export_pixels() {
        let original = RgbaImage::from_fn(240, 100, |x, y| Rgba([x as u8, y as u8, 40, 255]));
        let mut doc = Document::new(original.clone());
        doc.add(Mark::Arrow {
            start: Point::new(16, 22),
            end: Point::new(95, 22),
            stroke: 5.,
        });
        assert_eq!(&doc.result().get_pixel(82, 22).0[..3], &ARROW_COLOR);
        let label = text_label("箭头 Test", 1., original.dimensions()).unwrap();
        assert!(label.pixels().filter(|pixel| pixel[0] > 200).count() > 25);
        let wrapped = text_label("中文箭头与马赛克一起导出", 1., (120, 120)).unwrap();
        assert!(wrapped.height() > label.height());
        assert!(wrapped.width() <= 120);
        doc.add(Mark::Text {
            position: Point::new(100, 55),
            label: label.clone(),
        });
        doc.add(Mark::Mosaic {
            start: Point::new(10, 18),
            end: Point::new(31, 29),
            block: 8,
        });
        assert_eq!(
            doc.result().get_pixel(10, 18),
            doc.result().get_pixel(17, 25)
        );
        assert_eq!(doc.result().get_pixel(9, 18), original.get_pixel(9, 18));
        assert_eq!(doc.result().get_pixel(100, 55), label.get_pixel(0, 0));
        let png = png::encode(doc.result()).unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().to_rgba8(),
            **doc.result()
        );
        assert!(doc.undo());
        assert_eq!(doc.result().get_pixel(10, 18), original.get_pixel(10, 18));
        assert!(doc.redo());
        assert_eq!(
            image::load_from_memory(&png).unwrap().to_rgba8(),
            **doc.result()
        );
        assert!(doc.undo());
        doc.add(Mark::Mosaic {
            start: Point::new(9, 80),
            end: Point::new(2, 72),
            block: 8,
        });
        assert!(!doc.can_redo());
    }
}
