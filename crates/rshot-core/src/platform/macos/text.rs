use image::RgbaImage;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSGraphicsContext,
    NSStringDrawingDeprecated, NSStringDrawingOptions,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGContext, CGImageAlphaInfo, kCGColorSpaceSRGB};
use objc2_foundation::{NSDictionary, NSString};

use super::bitmap::CGBitmapContextCreate;
use crate::annotation::TextError;
use crate::geometry::{Point, Rect};

pub(crate) struct TextRaster {
    pub bounds: Rect<i64>,
    pub pixels: RgbaImage,
}

enum Placement {
    Wrapped {
        padding: f64,
        limits: (u32, u32),
    },
    Positioned {
        at: Point<i64>,
        centered: bool,
        viewport: (u32, u32),
    },
}

pub(crate) fn draw_text(
    value: &str,
    size: f64,
    padding: f64,
    limits: (u32, u32),
    color: u32,
    opaque: bool,
) -> Result<RgbaImage, TextError> {
    rasterize(
        value,
        size,
        color,
        opaque,
        Placement::Wrapped { padding, limits },
    )
    .map(|raster| raster.pixels)
}

pub(crate) fn draw_positioned_text(
    value: &str,
    size: f64,
    color: u32,
    at: Point<i64>,
    centered: bool,
    viewport: (u32, u32),
) -> Result<TextRaster, TextError> {
    rasterize(
        value,
        size,
        color,
        false,
        Placement::Positioned {
            at,
            centered,
            viewport,
        },
    )
}

fn rasterize(
    value: &str,
    size: f64,
    color: u32,
    opaque: bool,
    placement: Placement,
) -> Result<TextRaster, TextError> {
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
        let (available, padding) = match placement {
            Placement::Wrapped {
                padding,
                limits: (max_width, _),
            } => (f64::from(max_width) - 2. * padding, padding),
            Placement::Positioned { .. } => (f64::MAX, 1.),
        };
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
        let width = i64::from((text_width + 2. * padding) as u32);
        let height = i64::from((text_height + 2. * padding) as u32);
        let (origin, viewport) = match placement {
            Placement::Wrapped { limits, .. } => {
                if height > i64::from(limits.1) {
                    return Err(TextError::TooTall);
                }
                (Point::new(0, 0), limits)
            }
            Placement::Positioned {
                at,
                centered: true,
                viewport,
            } => (Point::new(at.x - width / 2, at.y - height / 2), viewport),
            Placement::Positioned { at, viewport, .. } => (at, viewport),
        };
        let bounds = Rect::new(origin.x, origin.y, width, height);
        let clipped = bounds.clip(viewport);
        let mut rgba = RgbaImage::new(clipped.size.width, clipped.size.height);
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        if w == 0 || h == 0 {
            return Ok(TextRaster {
                bounds,
                pixels: rgba,
            });
        }
        let colors = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .ok_or(TextError::ColorSpace)?;
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
            .ok_or(TextError::BitmapContext)?;
            CFRetained::from_raw(raw)
        };
        if opaque {
            CGContext::set_rgb_fill_color(Some(&context), 0.063, 0.094, 0.137, 1.);
            CGContext::fill_rect(
                Some(&context),
                CGRect::new(CGPoint::new(0., 0.), CGSize::new(w as f64, h as f64)),
            );
        }
        {
            let graphics = NSGraphicsContext::graphicsContextWithCGContext_flipped(&context, false);
            NSGraphicsContext::saveGraphicsState_class();
            NSGraphicsContext::setCurrentContext(Some(&graphics));
            let rect = CGRect::new(
                CGPoint::new(
                    padding + (origin.x - i64::from(clipped.origin.x)) as f64,
                    padding + (i64::from(clipped.origin.y) + h as i64 - origin.y - height) as f64,
                ),
                CGSize::new(text_width, text_height),
            );
            unsafe { text.drawWithRect_options_attributes(rect, options, Some(&attributes)) };
            NSGraphicsContext::restoreGraphicsState_class();
        }
        drop(context);
        if !opaque {
            // Convert Core Graphics' premultiplied pixels to straight alpha.
            for pixel in rgba.pixels_mut() {
                let alpha = u16::from(pixel[3]).max(1);
                for channel in &mut pixel.0[..3] {
                    *channel = (u16::from(*channel) * 255 / alpha).min(255) as u8;
                }
            }
        }
        Ok(TextRaster {
            bounds,
            pixels: rgba,
        })
    })
}
