use image::{GrayImage, Rgba, RgbaImage, imageops};
use serde::Serialize;
use tiny_skia::{FillRule, Mask, Path, PathBuilder, Transform};

use super::spec::{
    AnnotateError, Color, HighlightBlend, MagnifySpec, MarkKind, MarkSpec, Spec, SpotlightShape,
    SpotlightSpec, step_number,
};
use super::{LineCoverage, arrow_segments, blur, mosaic};
use crate::geometry::{Point, Rect};
use crate::locate::CoordinateMapping;

#[cfg(target_os = "macos")]
use crate::platform::macos::text::draw_positioned_text;

#[derive(Debug)]
pub struct Rendered {
    pub image: RgbaImage,
    pub warnings: Vec<RenderWarning>,
    pub legend: Vec<LegendEntry>,
    pub marks: Vec<RenderedMark>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct RenderWarning {
    pub index: usize,
    pub field: &'static str,
    pub message: &'static str,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct LegendEntry {
    pub n: u32,
    pub label: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct RenderedMark {
    pub index: usize,
    #[serde(rename = "type")]
    pub kind: MarkKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Pixels written by this mark before later marks are applied, or a zero-sized rectangle.
    pub bounds: Rect<u32>,
}

pub fn render(image: &RgbaImage, spec: &Spec) -> Result<Rendered, AnnotateError> {
    spec.validate()?;
    render_with_source(image.clone(), spec, |_| Some((Point::new(0, 0), image)))
}

pub fn render_owned(image: RgbaImage, spec: &Spec) -> Result<Rendered, AnnotateError> {
    spec.validate()?;
    let sources: Vec<_> = spec
        .marks
        .iter()
        .map(|mark| {
            let MarkSpec::Magnify(mark) = mark else {
                return None;
            };
            let sample = widen(mark.source).clip(image.dimensions());
            let area = magnify_area(sample, mark);
            let visible = area.clip(image.dimensions());
            if visible.size.width == 0 || visible.size.height == 0 {
                return None;
            }
            let mapping = CoordinateMapping {
                offset: sample.origin,
                scale: mark.scale,
            };
            let x = (i64::from(visible.origin.x) - area.origin.x) as u32;
            let y = (i64::from(visible.origin.y) - area.origin.y) as u32;
            let first = mapping.source_pixel(Point::new(x, y));
            let last = mapping.source_pixel(Point::new(
                x + (visible.size.width - 1),
                y + (visible.size.height - 1),
            ));
            let pixels = imageops::crop_imm(
                &image,
                first.x,
                first.y,
                last.x - first.x + 1,
                last.y - first.y + 1,
            )
            .to_image();
            Some((first, pixels))
        })
        .collect();
    render_with_source(image, spec, |index| {
        sources[index]
            .as_ref()
            .map(|(origin, pixels)| (*origin, pixels))
    })
}

fn render_with_source<'a>(
    image: RgbaImage,
    spec: &Spec,
    source: impl Fn(usize) -> Option<(Point<u32>, &'a RgbaImage)>,
) -> Result<Rendered, AnnotateError> {
    let dimensions = image.dimensions();
    let mut rendered = Rendered {
        image,
        warnings: Vec::new(),
        legend: Vec::new(),
        marks: Vec::with_capacity(spec.marks.len()),
    };
    let mut highest = 0;
    for (index, mark) in spec.marks.iter().enumerate() {
        let mut canvas = Canvas {
            image: &mut rendered.image,
            bounds: None,
            index,
            warnings: &mut rendered.warnings,
        };
        let mut n = None;
        let clip_check = match mark {
            MarkSpec::Rect(mark) => {
                let area = widen(mark.rect);
                canvas.outline(
                    area,
                    Shape::Rounded(f64::from(mark.radius)),
                    mark.width,
                    mark.stroke,
                    mark.fill,
                )?;
                Some((area, "rect"))
            }
            MarkSpec::Ellipse(mark) => {
                let area = widen(mark.rect);
                canvas.outline(area, Shape::Ellipse, mark.width, mark.stroke, mark.fill)?;
                Some((area, "rect"))
            }
            MarkSpec::Arrow(mark) => Some((
                canvas.arrow(mark.from, mark.to, mark.width, mark.stroke),
                "from/to",
            )),
            MarkSpec::Text(mark) => Some((
                canvas.text(
                    &mark.text,
                    f64::from(mark.size),
                    mark.color,
                    mark.background,
                    TextAnchor::TopLeft(Point::new(i64::from(mark.at.x), i64::from(mark.at.y))),
                )?,
                "at",
            )),
            MarkSpec::Step(mark) => {
                let number = step_number(index, mark.n, &mut highest)?;
                let value = number.to_string();
                let (x, y, size) = (
                    i64::from(mark.at.x),
                    i64::from(mark.at.y),
                    i64::from(mark.size),
                );
                let area = Rect::new(x - size / 2, y - size / 2, size, size);
                let circle = Shape::Ellipse.path(area, 0);
                canvas.mask(area, circle.as_ref(), FillRule::Winding, mark.fill, "size")?;
                let visible = area.clip(dimensions).size;
                if mark.color.0[3] != 0 && visible.width != 0 && visible.height != 0 {
                    canvas.text(
                        &value,
                        (1.2 / value.len() as f64).min(0.54) * f64::from(mark.size),
                        mark.color,
                        None,
                        TextAnchor::Center(Point::new(x, y)),
                    )?;
                }
                n = Some(number);
                rendered.legend.push(LegendEntry {
                    n: number,
                    label: mark.label.clone(),
                });
                Some((area, "at"))
            }
            MarkSpec::Highlight(mark) => {
                let area = widen(mark.rect);
                canvas.highlight(area, mark.color, mark.opacity, mark.blend);
                Some((area, "rect"))
            }
            MarkSpec::Redact(mark) => {
                let area = widen(mark.rect);
                canvas.solid(area, mark.fill, true);
                Some((area, "rect"))
            }
            MarkSpec::Mosaic(mark) => {
                let area = widen(mark.rect);
                if mark.block < 8 {
                    canvas.warn(
                        "block",
                        "a block size below 8 pixels produces a weak mosaic",
                    );
                }
                canvas.mosaic(area, mark.block);
                Some((area, "rect"))
            }
            MarkSpec::Blur(mark) => {
                let area = widen(mark.rect);
                if mark.radius < 8 {
                    canvas.warn("radius", "a radius below 8 pixels produces a weak blur");
                }
                canvas.blur(area, mark.radius);
                Some((area, "rect"))
            }
            MarkSpec::Spotlight(mark) => {
                canvas.spotlight(mark);
                mark.rects
                    .iter()
                    .map(|&rect| widen(rect))
                    .find(|&area| outside(area, dimensions))
                    .map(|area| (area, "rects"))
            }
            MarkSpec::Magnify(mark) => {
                canvas.warn_if_clipped(widen(mark.source), "source");
                Some((canvas.magnify(source(index), mark), "at"))
            }
        };
        if let Some((area, field)) = clip_check {
            canvas.warn_if_clipped(area, field);
        }
        rendered.marks.push(RenderedMark {
            index,
            kind: mark.kind(),
            n,
            bounds: canvas.bounds(),
        });
    }
    Ok(rendered)
}

fn widen(rect: Rect<i32>) -> Rect<i64> {
    Rect::new(
        i64::from(rect.origin.x),
        i64::from(rect.origin.y),
        i64::from(rect.size.width),
        i64::from(rect.size.height),
    )
}

fn magnify_area(sample: Rect<u32>, mark: &MagnifySpec) -> Rect<i64> {
    Rect::new(
        i64::from(mark.at.x),
        i64::from(mark.at.y),
        (f64::from(sample.size.width) * mark.scale).round() as i64,
        (f64::from(sample.size.height) * mark.scale).round() as i64,
    )
}

fn outside(area: Rect<i64>, (width, height): (u32, u32)) -> bool {
    let Point { x, y } = area.origin;
    x < 0
        || y < 0
        || x + area.size.width > i64::from(width)
        || y + area.size.height > i64::from(height)
}

#[derive(Clone, Copy)]
enum Shape {
    Rounded(f64),
    Ellipse,
}

impl Shape {
    fn path(self, area: Rect<i64>, inset: u32) -> Option<Path> {
        let inset = i64::from(inset);
        let (x, y) = (area.origin.x + inset, area.origin.y + inset);
        let (w, h) = (area.size.width - 2 * inset, area.size.height - 2 * inset);
        if w <= 0 || h <= 0 {
            return None;
        }
        let (l, t, r, b) = (x as f32, y as f32, (x + w) as f32, (y + h) as f32);
        let rect = tiny_skia::Rect::from_ltrb(l, t, r, b)?;
        let mut path = PathBuilder::new();
        match self {
            Self::Ellipse => path.push_oval(rect),
            Self::Rounded(radius) => {
                let radius = radius.min(area.size.width.min(area.size.height) as f64 / 2.);
                let rad = (radius - inset as f64).max(0.) as f32;
                if rad == 0. {
                    path.push_rect(rect);
                } else {
                    let control = rad * 0.552_284_8;
                    path.move_to(l + rad, t);
                    path.line_to(r - rad, t);
                    path.cubic_to(r - rad + control, t, r, t + rad - control, r, t + rad);
                    path.line_to(r, b - rad);
                    path.cubic_to(r, b - rad + control, r - rad + control, b, r - rad, b);
                    path.line_to(l + rad, b);
                    path.cubic_to(l + rad - control, b, l, b - rad + control, l, b - rad);
                    path.line_to(l, t + rad);
                    path.cubic_to(l, t + rad - control, l + rad - control, t, l + rad, t);
                    path.close();
                }
            }
        }
        path.finish()
    }
}

#[derive(Clone, Copy)]
enum TextAnchor {
    TopLeft(Point<i64>),
    Center(Point<i64>),
}

struct Canvas<'a> {
    image: &'a mut RgbaImage,
    bounds: Option<[u32; 4]>,
    index: usize,
    warnings: &'a mut Vec<RenderWarning>,
}

impl Canvas<'_> {
    fn warn(&mut self, field: &'static str, message: &'static str) {
        self.warnings.push(RenderWarning {
            index: self.index,
            field,
            message,
        });
    }

    fn warn_if_clipped(&mut self, area: Rect<i64>, field: &'static str) {
        if outside(area, self.image.dimensions()) {
            self.warn(field, "mark extends outside the image and was clipped");
        }
    }

    fn bounds(&self) -> Rect<u32> {
        match self.bounds {
            Some([l, t, r, b]) => Rect::new(l, t, r - l + 1, b - t + 1),
            None => Rect::new(0, 0, 0, 0),
        }
    }

    fn include(&mut self, x: u32, y: u32) {
        self.bounds = Some(match self.bounds {
            Some([l, t, r, b]) => [l.min(x), t.min(y), r.max(x), b.max(y)],
            None => [x, y, x, y],
        });
    }

    fn highlight(&mut self, area: Rect<i64>, color: Color, opacity: f64, mode: HighlightBlend) {
        let color = apply_opacity(color, opacity);
        if color.0[3] == 0 {
            return;
        }
        let region = area.clip(self.image.dimensions());
        let Point { x, y } = region.origin;
        for row in y..y + region.size.height {
            for col in x..x + region.size.width {
                self.blend_with_mode(col, row, Rgba(color.0), mode);
            }
        }
    }

    fn blend(&mut self, x: u32, y: u32, color: Rgba<u8>) {
        self.blend_with_mode(x, y, color, HighlightBlend::Normal);
    }

    fn blend_with_mode(&mut self, x: u32, y: u32, color: Rgba<u8>, mode: HighlightBlend) {
        let alpha = u32::from(color[3]);
        if alpha == 0 {
            return;
        }
        self.include(x, y);
        let destination = self.image.get_pixel_mut(x, y);
        if alpha == 255 && mode == HighlightBlend::Normal {
            *destination = color;
            return;
        }
        let backdrop_alpha = u32::from(destination[3]);
        let source_weight = alpha * 255;
        let destination_weight = backdrop_alpha * (255 - alpha);
        let weight = source_weight + destination_weight;
        let divisor = weight * 255;
        for (out, source) in destination.0[..3].iter_mut().zip(color.0) {
            let backdrop = u32::from(*out);
            let source = u32::from(source);
            let source_contribution = match mode {
                HighlightBlend::Multiply => {
                    source * alpha * ((255 - backdrop_alpha) * 255 + backdrop_alpha * backdrop)
                }
                HighlightBlend::Normal => source * source_weight * 255,
            };
            *out = ((source_contribution + backdrop * destination_weight * 255 + divisor / 2)
                / divisor) as u8;
        }
        destination[3] = ((weight + 127) / 255) as u8;
    }

    fn solid(&mut self, area: Rect<i64>, color: Color, replace: bool) {
        let region = area.clip(self.image.dimensions());
        let Point { x, y } = region.origin;
        let (right, bottom) = (x + region.size.width, y + region.size.height);
        if right == x || bottom == y || (!replace && color.0[3] == 0) {
            return;
        }
        if replace || color.0[3] == 255 {
            for row in y..bottom {
                for col in x..right {
                    self.image.put_pixel(col, row, Rgba(color.0));
                }
            }
            self.include(x, y);
            self.include(right - 1, bottom - 1);
        } else {
            for row in y..bottom {
                for col in x..right {
                    self.blend(col, row, Rgba(color.0));
                }
            }
        }
    }

    fn mosaic(&mut self, area: Rect<i64>, block: u32) {
        let region = area.clip(self.image.dimensions());
        let Point { x, y } = region.origin;
        let (width, height) = (region.size.width, region.size.height);
        if width == 0 || height == 0 {
            return;
        }
        let end = Point::new(x + width - 1, y + height - 1);
        mosaic(self.image, region.origin, end, block);
        self.include(x, y);
        self.include(end.x, end.y);
    }

    fn blur(&mut self, area: Rect<i64>, radius: u32) {
        let region = area.clip(self.image.dimensions());
        let Point { x, y } = region.origin;
        let (width, height) = (region.size.width, region.size.height);
        if width == 0 || height == 0 {
            return;
        }
        blur::blur(self.image, region, radius);
        self.include(x, y);
        self.include(x + width - 1, y + height - 1);
    }

    fn spotlight(&mut self, mark: &SpotlightSpec) {
        let color = apply_opacity(mark.color, mark.opacity);
        if color.0[3] == 0 {
            return;
        }
        let (width, height) = self.image.dimensions();
        let mut focus = GrayImage::new(width, height);
        for &rect in &mark.rects {
            let area = widen(rect);
            let region = area.clip((width, height));
            let Point { x, y } = region.origin;
            let half_width = f64::from(rect.size.width) / 2.;
            let half_height = f64::from(rect.size.height) / 2.;
            let center = center(area);
            for row in y..y + region.size.height {
                let dy = (f64::from(row) + 0.5 - center.y) / half_height;
                for col in x..x + region.size.width {
                    let dx = (f64::from(col) + 0.5 - center.x) / half_width;
                    if mark.shape == SpotlightShape::Rect || dx * dx + dy * dy <= 1. {
                        focus.get_pixel_mut(col, row).0[0] = 1;
                    }
                }
            }
        }
        for (x, y, pixel) in focus.enumerate_pixels() {
            if pixel.0[0] == 0 {
                self.blend(x, y, Rgba(color.0));
            }
        }
    }

    fn magnify(
        &mut self,
        source: Option<(Point<u32>, &RgbaImage)>,
        mark: &MagnifySpec,
    ) -> Rect<i64> {
        let sample = widen(mark.source).clip(self.image.dimensions());
        let (width, height) = (sample.size.width, sample.size.height);
        if width == 0 || height == 0 {
            return Rect::new(0, 0, 0, 0);
        }
        let area = magnify_area(sample, mark);
        if mark.connector {
            let source_area = Rect::new(
                i64::from(sample.origin.x),
                i64::from(sample.origin.y),
                i64::from(width),
                i64::from(height),
            );
            let connector = self.stroke_lines(
                [(center(source_area), center(area))],
                f64::from(mark.width),
                mark.border,
            );
            self.warn_if_clipped(connector, "connector");
        }
        let Some((source_origin, source)) = source else {
            return area;
        };
        let mapping = CoordinateMapping {
            offset: sample.origin,
            scale: mark.scale,
        };
        let border = i64::from(mark.width);
        let region = area.clip(self.image.dimensions());
        let Point { x, y } = region.origin;
        for row in y..y + region.size.height {
            for col in x..x + region.size.width {
                let dx = i64::from(col) - area.origin.x;
                let dy = i64::from(row) - area.origin.y;
                let from = mapping.source_pixel(Point::new(dx as u32, dy as u32));
                let pixel = source.get_pixel(from.x - source_origin.x, from.y - source_origin.y);
                self.blend(col, row, *pixel);
                if dx < border
                    || dy < border
                    || dx >= area.size.width - border
                    || dy >= area.size.height - border
                {
                    self.blend(col, row, Rgba(mark.border.0));
                }
            }
        }
        area
    }

    fn mask(
        &mut self,
        area: Rect<i64>,
        path: Option<&Path>,
        rule: FillRule,
        color: Color,
        field: &str,
    ) -> Result<(), AnnotateError> {
        let region = area.clip(self.image.dimensions());
        let (w, h) = (region.size.width, region.size.height);
        let Some(path) = path.filter(|_| color.0[3] != 0 && w != 0 && h != 0) else {
            return Ok(());
        };
        let mut mask = Mask::new(w, h).ok_or_else(|| {
            AnnotateError::field(self.index, field, "could not allocate raster mask")
        })?;
        let Point { x, y } = region.origin;
        mask.fill_path(
            path,
            rule,
            true,
            Transform::from_translate(-(x as f32), -(y as f32)),
        );
        for (row, coverage) in mask.data().chunks_exact(w as usize).enumerate() {
            for (col, &coverage) in coverage
                .iter()
                .enumerate()
                .filter(|(_, coverage)| **coverage != 0)
            {
                let mut rgba = color.0;
                rgba[3] = multiply_alpha(rgba[3], coverage);
                self.blend(x + col as u32, y + row as u32, Rgba(rgba));
            }
        }
        Ok(())
    }

    fn outline(
        &mut self,
        area: Rect<i64>,
        shape: Shape,
        width: u32,
        stroke: Color,
        fill: Option<Color>,
    ) -> Result<(), AnnotateError> {
        let Some(outer) = shape.path(area, 0) else {
            return Ok(());
        };
        if let Some(fill) = fill {
            self.mask(area, Some(&outer), FillRule::Winding, fill, "rect")?;
        }
        let mut outline = PathBuilder::new();
        outline.push_path(&outer);
        if let Some(inner) = shape.path(area, width) {
            outline.push_path(&inner);
        }
        self.mask(
            area,
            outline.finish().as_ref(),
            FillRule::EvenOdd,
            stroke,
            "rect",
        )
    }

    fn arrow(&mut self, from: Point<i32>, to: Point<i32>, width: u32, color: Color) -> Rect<i64> {
        let coords =
            |point: Point<i32>| Point::new(f64::from(point.x) + 0.5, f64::from(point.y) + 0.5);
        let width = f64::from(width);
        self.stroke_lines(
            arrow_segments(coords(from), coords(to), width),
            width,
            color,
        )
    }

    fn stroke_lines<const N: usize>(
        &mut self,
        segments: [(Point<f64>, Point<f64>); N],
        width: f64,
        color: Color,
    ) -> Rect<i64> {
        let mut ends = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for (a, b) in segments {
            ends[0] = ends[0].min(a.x).min(b.x);
            ends[1] = ends[1].min(a.y).min(b.y);
            ends[2] = ends[2].max(a.x).max(b.x);
            ends[3] = ends[3].max(a.y).max(b.y);
        }
        let [l, t, r, b] = ends;
        let radius = width / 2.;
        let (l, t) = ((l - radius).floor() as i64, (t - radius).floor() as i64);
        let area = Rect::new(
            l,
            t,
            (r + radius).ceil() as i64 - l,
            (b + radius).ceil() as i64 - t,
        );
        if color.0[3] == 0 {
            return area;
        }
        let region = area.clip(self.image.dimensions());
        let Point { x, y } = region.origin;
        let lines = segments.map(|(a, b)| LineCoverage::new(a, b, width));
        for row in y..y + region.size.height {
            for col in x..x + region.size.width {
                let coverage = lines
                    .iter()
                    .map(|line| line.at(col, row))
                    .fold(0.0, f64::max);
                let mut rgba = color.0;
                rgba[3] = (f64::from(rgba[3]) * coverage).round() as u8;
                self.blend(col, row, Rgba(rgba));
            }
        }
        area
    }

    #[cfg(target_os = "macos")]
    fn text(
        &mut self,
        value: &str,
        size: f64,
        color: Color,
        background: Option<Color>,
        anchor: TextAnchor,
    ) -> Result<Rect<i64>, AnnotateError> {
        let (at, centered, field) = match anchor {
            TextAnchor::TopLeft(at) => (at, false, "text"),
            TextAnchor::Center(at) => (at, true, "n"),
        };
        if value.is_empty() {
            return Ok(Rect::new(at.x, at.y, 0, 0));
        }
        let rgb = u32::from_be_bytes([0, color.0[0], color.0[1], color.0[2]]);
        let dimensions = self.image.dimensions();
        let raster =
            draw_positioned_text(value, size, rgb, at, centered, dimensions).map_err(|source| {
                AnnotateError::Text {
                    index: self.index,
                    field,
                    source,
                }
            })?;
        if let Some(background) = background {
            self.solid(raster.bounds, background, false);
        }
        let Point { x, y } = raster.bounds.clip(dimensions).origin;
        for (col, row, &pixel) in raster.pixels.enumerate_pixels() {
            let mut pixel = pixel;
            pixel[3] = multiply_alpha(pixel[3], color.0[3]);
            self.blend(x + col, y + row, pixel);
        }
        Ok(raster.bounds)
    }

    #[cfg(not(target_os = "macos"))]
    fn text(
        &mut self,
        value: &str,
        _size: f64,
        _color: Color,
        _background: Option<Color>,
        anchor: TextAnchor,
    ) -> Result<Rect<i64>, AnnotateError> {
        let (at, field) = match anchor {
            TextAnchor::TopLeft(at) => (at, "text"),
            TextAnchor::Center(at) => (at, "n"),
        };
        if value.is_empty() {
            return Ok(Rect::new(at.x, at.y, 0, 0));
        }
        Err(AnnotateError::field(
            self.index,
            field,
            "text rendering requires macOS",
        ))
    }
}

fn center(area: Rect<i64>) -> Point<f64> {
    Point::new(
        area.origin.x as f64 + area.size.width as f64 / 2.,
        area.origin.y as f64 + area.size.height as f64 / 2.,
    )
}

fn apply_opacity(mut color: Color, opacity: f64) -> Color {
    color.0[3] = (f64::from(color.0[3]) * opacity).round() as u8;
    color
}

fn multiply_alpha(first: u8, second: u8) -> u8 {
    ((u16::from(first) * u16::from(second) + 127) / 255) as u8
}
