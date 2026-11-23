use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use image::{Pixel, Rgba, RgbaImage, imageops::replace};
use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSGraphicsContext,
    NSStringDrawingDeprecated, NSStringDrawingOptions,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGContext, CGImageAlphaInfo, kCGColorSpaceSRGB};
use objc2_foundation::{NSDictionary, NSString};

use crate::capture::CGBitmapContextCreate;

pub type Point = (u32, u32);
pub const ARROW_COLOR: [u8; 3] = [255, 85, 102];

pub enum Mark {
    Arrow {
        start: Point,
        end: Point,
        stroke: f64,
    },
    Mosaic {
        start: Point,
        end: Point,
        block: u32,
    },
    Text {
        position: Point,
        label: RgbaImage,
    },
}

impl Mark {
    fn apply(&self, image: &mut RgbaImage) {
        match self {
            Self::Arrow { start, end, stroke } => arrow(image, *start, *end, *stroke),
            Self::Mosaic { start, end, block } => mosaic(image, *start, *end, *block),
            Self::Text {
                position: (x, y),
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
    start: (f64, f64),
    end: (f64, f64),
    stroke: f64,
) -> [((f64, f64), (f64, f64)); 3] {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length = dx.hypot(dy);
    if length < f64::EPSILON {
        return [(start, end); 3];
    }
    let head = (stroke * 4.).min(length * 0.63);
    let (along_x, along_y) = (dx / length * head, dy / length * head);
    let (base_x, base_y) = (end.0 - along_x, end.1 - along_y);
    [
        (start, end),
        ((base_x - along_y * 0.6, base_y + along_x * 0.6), end),
        ((base_x + along_y * 0.6, base_y - along_x * 0.6), end),
    ]
}

fn arrow(image: &mut RgbaImage, start: Point, end: Point, stroke: f64) {
    let coords = |(x, y): Point| (f64::from(x) + 0.5, f64::from(y) + 0.5);
    let segments = arrow_segments(coords(start), coords(end), stroke);
    for &(a, b) in &segments {
        line(image, a, b, stroke + 2., [65, 20, 28]);
    }
    for (a, b) in segments {
        line(image, a, b, stroke, ARROW_COLOR);
    }
}

fn line(image: &mut RgbaImage, a: (f64, f64), b: (f64, f64), stroke: f64, color: [u8; 3]) {
    let radius = stroke / 2. + 0.5;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_sq = (dx * dx + dy * dy).max(f64::EPSILON);
    let left = (a.0.min(b.0) - radius).floor() as u32;
    let right = ((a.0.max(b.0) + radius).ceil() as u32).min(image.width());
    let top = (a.1.min(b.1) - radius).floor() as u32;
    let bottom = ((a.1.max(b.1) + radius).ceil() as u32).min(image.height());
    for y in top..bottom {
        for x in left..right {
            let (px, py) = (f64::from(x) + 0.5 - a.0, f64::from(y) + 0.5 - a.1);
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

fn mosaic(image: &mut RgbaImage, (x0, y0): Point, (x1, y1): Point, block: u32) {
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

pub fn text_label(
    value: &str,
    scale: f32,
    (max_width, max_height): (u32, u32),
) -> Result<RgbaImage> {
    let scale = f64::from(scale);
    ensure!(
        f64::from(max_width) >= 22. * scale + 2. * (5. * scale).ceil(),
        "选区太窄，无法放置文字"
    );
    draw_text(
        value,
        20. * scale,
        (5. * scale).ceil(),
        (max_width, max_height),
        0xffffff,
        true,
    )
}

pub(crate) fn interface_text(
    value: &str,
    size: f64,
    max_width: u32,
    color: u32,
) -> Result<RgbaImage> {
    draw_text(value, size, 1., (max_width, 2048), color, false)
}

fn draw_text(
    value: &str,
    size: f64,
    padding: f64,
    (max_width, max_height): (u32, u32),
    color: u32,
    opaque: bool,
) -> Result<RgbaImage> {
    autoreleasepool(|_| {
        let font = if opaque {
            NSFont::boldSystemFontOfSize(size)
        } else {
            NSFont::systemFontOfSize(size)
        };
        let channel = |shift: u32| f64::from((color >> shift) & 255) / 255.;
        let foreground =
            NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.);
        // The dictionary has the types required by AppKit's NSString drawing methods.
        let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
        let attributes = NSDictionary::from_slices(&keys, &[font.as_ref(), foreground.as_ref()]);
        let text = NSString::from_str(value);
        let available = f64::from(max_width) - 2. * padding;
        let options = NSStringDrawingOptions::UsesLineFragmentOrigin
            | NSStringDrawingOptions::UsesFontLeading;
        let extent = unsafe {
            text.boundingRectWithSize_options_attributes(
                CGSize::new(available, f64::MAX),
                options,
                Some(&attributes),
            )
        };
        let text_width = extent.size.width.ceil().min(available);
        let text_height = extent.size.height.ceil();
        let width = (text_width + 2. * padding) as u32;
        let height = (text_height + 2. * padding) as u32;
        ensure!(height <= max_height, "文字超出选区高度，请缩短内容");
        let (w, h) = (width as usize, height as usize);
        let mut rgba = RgbaImage::new(width, height);
        let colors = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .context("无法创建文字色彩空间")?;
        // The graphics context borrows the backing buffer only until after drawing.
        let context = unsafe {
            let raw = CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                w,
                h,
                8,
                w * 4,
                &colors,
                CGImageAlphaInfo::PremultipliedLast.0,
            )
            .context("无法创建文字画布")?;
            CFRetained::from_raw(raw)
        };
        if opaque {
            CGContext::set_rgb_fill_color(Some(&context), 0.063, 0.094, 0.137, 1.);
            CGContext::fill_rect(
                Some(&context),
                CGRect::new(
                    CGPoint::new(0., 0.),
                    CGSize::new(f64::from(width), f64::from(height)),
                ),
            );
        }
        {
            let graphics = NSGraphicsContext::graphicsContextWithCGContext_flipped(&context, false);
            NSGraphicsContext::saveGraphicsState_class();
            NSGraphicsContext::setCurrentContext(Some(&graphics));
            let rect = CGRect::new(
                CGPoint::new(padding, padding),
                CGSize::new(text_width, text_height),
            );
            unsafe { text.drawWithRect_options_attributes(rect, options, Some(&attributes)) };
            NSGraphicsContext::restoreGraphicsState_class();
        }
        drop(context);
        if !opaque {
            // Core Graphics writes premultiplied pixels. GPUI blends its image textures with straight alpha.
            for pixel in rgba.pixels_mut() {
                let alpha = u16::from(pixel[3]).max(1);
                for channel in &mut pixel.0[..3] {
                    *channel = (u16::from(*channel) * 255 / alpha).min(255) as u8;
                }
            }
        }
        Ok(rgba)
    })
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{ARROW_COLOR, Document, Mark, interface_text, text_label};

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
            start: (16, 22),
            end: (95, 22),
            stroke: 5.,
        });
        assert_eq!(&doc.result().get_pixel(82, 22).0[..3], &ARROW_COLOR);
        let label = text_label("箭头 Test", 1., original.dimensions()).unwrap();
        assert!(label.pixels().filter(|pixel| pixel[0] > 200).count() > 25);
        let wrapped = text_label("中文箭头与马赛克一起导出", 1., (120, 120)).unwrap();
        assert!(wrapped.height() > label.height());
        assert!(wrapped.width() <= 120);
        doc.add(Mark::Text {
            position: (100, 55),
            label: label.clone(),
        });
        doc.add(Mark::Mosaic {
            start: (10, 18),
            end: (31, 29),
            block: 8,
        });
        assert_eq!(
            doc.result().get_pixel(10, 18),
            doc.result().get_pixel(17, 25)
        );
        assert_eq!(doc.result().get_pixel(9, 18), original.get_pixel(9, 18));
        assert_eq!(doc.result().get_pixel(100, 55), label.get_pixel(0, 0));
        let png = crate::ui::png_bytes(doc.result()).unwrap();
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
            start: (9, 80),
            end: (2, 72),
            block: 8,
        });
        assert!(!doc.can_redo());
    }
}
