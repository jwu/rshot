use image::{Pixel, Rgba, RgbaImage};
use thiserror::Error;

use crate::geometry::{Point, Rect, Size};

const GRID_COLOR: Rgba<u8> = Rgba([255, 85, 102, 112]);
const MAX_SCALE: f64 = 16.;

#[derive(Clone, Copy, Debug)]
pub struct GridOptions {
    /// Distance between lines in source pixels, with a minimum of 10.
    pub step: u32,
    pub color: Rgba<u8>,
    /// Label every nth tick. Zero hides labels.
    pub label_every: u32,
}

impl Default for GridOptions {
    fn default() -> Self {
        Self {
            step: 100,
            color: GRID_COLOR,
            label_every: 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ZoomOptions {
    /// Explicit magnification, capped at 16. Takes precedence over `max_side`.
    pub scale: Option<f64>,
    /// Desired length of the longer side when no scale is supplied.
    pub max_side: u32,
    /// Distance between lines in source pixels, with a minimum of 1.
    pub step: u32,
    pub color: Rgba<u8>,
    /// Label every nth tick in the source image. Zero hides labels.
    pub label_every: u32,
}

impl Default for ZoomOptions {
    fn default() -> Self {
        Self {
            scale: None,
            max_side: 1024,
            step: 10,
            color: GRID_COLOR,
            label_every: 1,
        }
    }
}

#[derive(Debug)]
pub struct Rendered<M> {
    pub image: RgbaImage,
    pub metadata: M,
    pub warnings: Vec<LocateWarning>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridMetadata {
    pub step: u32,
    /// Number of grid cells across, including any partial cell at the right edge.
    pub columns: u32,
    pub rows: u32,
    pub source_size: Size<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomMetadata {
    pub source_region: Rect<u32>,
    pub scale: f64,
    pub output_size: Size<u32>,
    pub mapping: CoordinateMapping,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateMapping {
    pub offset: Point<u32>,
    pub scale: f64,
}

impl CoordinateMapping {
    /// Maps an in-bounds output pixel to `offset + floor(output / scale)` on each axis.
    pub fn source_pixel(self, output: Point<u32>) -> Point<u32> {
        Point::new(
            self.offset.x + source_delta(output.x, self.scale),
            self.offset.y + source_delta(output.y, self.scale),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocateWarning {
    RegionClipped {
        requested: Rect<u32>,
        actual: Rect<u32>,
    },
    LabelsUnavailable,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LocateError {
    #[error("zoom region is empty")]
    EmptyRegion,
    #[error("zoom region is outside the image")]
    RegionOutsideImage,
    #[error("zoom scale must be finite and at least 1")]
    InvalidScale,
    #[error("zoom maximum side must be positive")]
    InvalidMaxSide,
    #[error("zoom output is too large")]
    OutputTooLarge,
}

pub fn grid(image: &RgbaImage, options: GridOptions) -> Rendered<GridMetadata> {
    grid_owned(image.clone(), options)
}

pub fn grid_owned(mut image: RgbaImage, mut options: GridOptions) -> Rendered<GridMetadata> {
    options.step = options.step.max(10);
    let region = Rect::new(0, 0, image.width(), image.height());
    let mut warnings = Vec::new();
    if !draw_grid(&mut image, region, 1., options, false) {
        warnings.push(LocateWarning::LabelsUnavailable);
    }
    let metadata = GridMetadata {
        step: options.step,
        columns: image.width().div_ceil(options.step),
        rows: image.height().div_ceil(options.step),
        source_size: region.size,
    };
    Rendered {
        image,
        metadata,
        warnings,
    }
}

pub fn zoom(
    image: &RgbaImage,
    region: Rect<u32>,
    options: ZoomOptions,
) -> Result<Rendered<ZoomMetadata>, LocateError> {
    let source_region = clip_region(image, region)?;
    let Size { width, height } = source_region.size;
    let scale = match options.scale {
        Some(scale) if scale.is_finite() && scale >= 1. => scale,
        Some(_) => return Err(LocateError::InvalidScale),
        None if options.max_side > 0 => f64::from(options.max_side) / f64::from(width.max(height)),
        None => return Err(LocateError::InvalidMaxSide),
    }
    .clamp(1., MAX_SCALE);
    let offset = source_region.origin;
    let mapping = CoordinateMapping { offset, scale };
    let width =
        u32::try_from(output_boundary(width, scale)).map_err(|_| LocateError::OutputTooLarge)?;
    let height =
        u32::try_from(output_boundary(height, scale)).map_err(|_| LocateError::OutputTooLarge)?;
    let mut output = sample_zoom(image, Size { width, height }, mapping)?;
    let mut warnings = Vec::new();
    if source_region != region {
        warnings.push(LocateWarning::RegionClipped {
            requested: region,
            actual: source_region,
        });
    }
    let grid_options = GridOptions {
        step: options.step.max(1),
        color: options.color,
        label_every: options.label_every,
    };
    if !draw_grid(&mut output, source_region, scale, grid_options, true) {
        warnings.push(LocateWarning::LabelsUnavailable);
    }
    Ok(Rendered {
        image: output,
        metadata: ZoomMetadata {
            source_region,
            scale,
            output_size: Size { width, height },
            mapping,
        },
        warnings,
    })
}

fn clip_region(image: &RgbaImage, region: Rect<u32>) -> Result<Rect<u32>, LocateError> {
    let Point { x, y } = region.origin;
    let Size { width, height } = region.size;
    if width == 0 || height == 0 {
        return Err(LocateError::EmptyRegion);
    }
    if x >= image.width() || y >= image.height() {
        return Err(LocateError::RegionOutsideImage);
    }
    Ok(Rect::new(
        x,
        y,
        width.min(image.width() - x),
        height.min(image.height() - y),
    ))
}

fn source_delta(output: u32, scale: f64) -> u32 {
    (f64::from(output) / scale).floor() as u32
}

fn output_boundary(source: u32, scale: f64) -> u64 {
    // Check the inverse so a fractional scale places the line on the first matching output pixel.
    let output = (f64::from(source) * scale).floor() as u64;
    output + u64::from(output as f64 / scale < f64::from(source))
}

fn sample_zoom(
    image: &RgbaImage,
    Size { width, height }: Size<u32>,
    mapping: CoordinateMapping,
) -> Result<RgbaImage, LocateError> {
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(LocateError::OutputTooLarge)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(bytes)
        .map_err(|_| LocateError::OutputTooLarge)?;
    for y in 0..height {
        for x in 0..width {
            let source = mapping.source_pixel(Point::new(x, y));
            pixels.extend_from_slice(&image.get_pixel(source.x, source.y).0);
        }
    }
    RgbaImage::from_raw(width, height, pixels).ok_or(LocateError::OutputTooLarge)
}

fn ticks(
    start: u32,
    length: u32,
    step: u32,
    scale: f64,
) -> impl Iterator<Item = (u32, u32)> + Clone {
    let first = start.saturating_add(step - start % step);
    (first..start + length)
        .step_by(step as usize)
        .map(move |coordinate| {
            let position = output_boundary(coordinate - start, scale) as u32;
            (coordinate, position)
        })
}

fn draw_grid(
    image: &mut RgbaImage,
    region: Rect<u32>,
    scale: f64,
    options: GridOptions,
    show_origin: bool,
) -> bool {
    if image.width() == 0 || image.height() == 0 {
        return true;
    }
    let Point { x, y } = region.origin;
    let Size { width, height } = region.size;
    let columns = ticks(x, width, options.step, scale);
    let rows = ticks(y, height, options.step, scale);
    let mut color = options.color;
    color[3] = color[3].min(128);
    for (_, x) in columns.clone() {
        for y in 0..image.height() {
            image.get_pixel_mut(x, y).blend(&color);
        }
    }
    for (_, y) in rows.clone() {
        for x in 0..image.width() {
            image.get_pixel_mut(x, y).blend(&color);
        }
    }
    if options.label_every == 0 {
        return true;
    }
    let origin = show_origin.then_some(region.origin);
    #[cfg(target_os = "macos")]
    {
        draw_labels(image, [columns, rows], options, origin).is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = origin;
        false
    }
}

#[cfg(target_os = "macos")]
fn draw_labels(
    image: &mut RgbaImage,
    ticks: [impl Iterator<Item = (u32, u32)>; 2],
    options: GridOptions,
    origin: Option<Point<u32>>,
) -> Result<(), crate::annotation::TextError> {
    use crate::annotation::text_label;
    use image::imageops::replace;

    let label = |value: String| text_label(&value, 0.55, (256, 64));
    let fits = |label: &RgbaImage, image: &RgbaImage| {
        label.width() <= image.width() && label.height() <= image.height()
    };
    let mut occupied = [0, 0];
    if let Some(Point { x, y }) = origin {
        let label = label(format!("x={x} y={y}"))?;
        if fits(&label, image) {
            replace(image, &label, 0, 0);
            occupied = [label.width() + 2, label.height() + 2];
        }
    }
    for (index, (ticks, axis)) in ticks.into_iter().zip(['x', 'y']).enumerate() {
        for (coordinate, position) in ticks {
            if position < occupied[index]
                || !(coordinate / options.step).is_multiple_of(options.label_every)
            {
                continue;
            }
            let label = label(format!("{axis}={coordinate}"))?;
            if !fits(&label, image) {
                continue;
            }
            let (span, length) = if axis == 'x' {
                (image.width(), label.width())
            } else {
                (image.height(), label.height())
            };
            let position = position.min(span - length);
            if position < occupied[index] {
                continue;
            }
            let (x, y) = if axis == 'x' {
                occupied[1] = occupied[1].max(label.height() + 2);
                (position, 0)
            } else {
                (0, position)
            };
            replace(image, &label, i64::from(x), i64::from(y));
            occupied[index] = (position + length).saturating_add(2);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| Rgba([x as u8, y as u8, 40, 255]))
    }

    #[test]
    fn grid_draws_translucent_lines_on_the_requested_pixels() {
        let input = source(31, 20);
        let rendered = grid(
            &input,
            GridOptions {
                step: 2,
                color: Rgba([255, 85, 102, 255]),
                label_every: 0,
            },
        );
        assert_eq!(
            rendered.metadata,
            GridMetadata {
                step: 10,
                columns: 4,
                rows: 2,
                source_size: Size {
                    width: 31,
                    height: 20,
                },
            }
        );
        for (x, y, pixel) in rendered.image.enumerate_pixels() {
            let mut expected = *input.get_pixel(x, y);
            for coordinate in [x, y] {
                if coordinate > 0 && coordinate % 10 == 0 {
                    expected.blend(&Rgba([255, 85, 102, 128]));
                }
            }
            assert_eq!(*pixel, expected, "pixel at ({x}, {y})");
        }
    }

    #[test]
    fn owned_grid_reuses_the_input_allocation() {
        let input = source(31, 20);
        let allocation = input.as_raw().as_ptr();
        let options = GridOptions {
            label_every: 0,
            ..GridOptions::default()
        };
        let expected = grid(&input, options);
        let actual = grid_owned(input, options);
        assert_eq!(actual.image.as_raw().as_ptr(), allocation);
        assert_eq!(actual.image, expected.image);
        assert_eq!(actual.metadata, expected.metadata);
        assert_eq!(actual.warnings, expected.warnings);
    }

    #[test]
    fn zoom_maps_each_output_pixel_to_the_nearest_source_pixel() {
        let input = source(40, 40);
        let region = Rect::new(17, 23, 5, 4);
        let rendered = zoom(
            &input,
            region,
            ZoomOptions {
                scale: Some(2.5),
                step: 100,
                label_every: 0,
                ..ZoomOptions::default()
            },
        )
        .unwrap();
        assert_eq!(rendered.image.dimensions(), (13, 10));
        let metadata = rendered.metadata;
        assert_eq!(metadata.scale, 2.5);
        assert_eq!(metadata.source_region, region);
        assert_eq!(
            metadata.output_size,
            Size {
                width: 13,
                height: 10
            }
        );
        assert_eq!(metadata.mapping.offset, region.origin);
        assert_eq!(metadata.mapping.scale, metadata.scale);
        assert!(rendered.warnings.is_empty());
        for (x, y, pixel) in rendered.image.enumerate_pixels() {
            let mapped = metadata.mapping.source_pixel(Point::new(x, y));
            assert_eq!(mapped, Point::new(17 + 2 * x / 5, 23 + 2 * y / 5));
            assert_eq!(pixel, input.get_pixel(mapped.x, mapped.y));
        }
    }

    #[test]
    fn zoom_grid_lines_align_with_global_source_coordinates() {
        let input = source(50, 50);
        let rendered = zoom(
            &input,
            Rect::new(17, 23, 24, 21),
            ZoomOptions {
                scale: Some(2.5),
                label_every: 0,
                ..ZoomOptions::default()
            },
        )
        .unwrap();
        for (x, y, on_tick) in [(7, 12, false), (8, 12, true), (4, 17, false), (4, 18, true)] {
            let source = rendered.metadata.mapping.source_pixel(Point::new(x, y));
            let mut expected = *input.get_pixel(source.x, source.y);
            if on_tick {
                assert!(source.x == 20 || source.y == 30);
                expected.blend(&GRID_COLOR);
            }
            assert_eq!(*rendered.image.get_pixel(x, y), expected);
        }
    }

    #[test]
    fn zoom_targets_the_output_size_and_caps_magnification() {
        let input = source(120, 100);
        let options = ZoomOptions {
            label_every: 0,
            ..ZoomOptions::default()
        };
        let rendered = zoom(&input, Rect::new(0, 0, 100, 75), options).unwrap();
        assert_eq!(rendered.image.dimensions(), (1024, 768));
        assert_eq!(rendered.metadata.scale, 10.24);
        let rendered = zoom(&input, Rect::new(0, 0, 5, 10), options).unwrap();
        assert_eq!(rendered.image.dimensions(), (80, 160));
        assert_eq!(rendered.metadata.scale, 16.);
        let rendered = zoom(
            &input,
            Rect::new(0, 0, 5, 10),
            ZoomOptions {
                scale: Some(20.),
                ..options
            },
        )
        .unwrap();
        assert_eq!(rendered.metadata.scale, 16.);
        let rendered = zoom(
            &input,
            Rect::new(0, 0, 100, 75),
            ZoomOptions {
                max_side: 50,
                ..options
            },
        )
        .unwrap();
        assert_eq!(rendered.image.dimensions(), (100, 75));
        assert_eq!(rendered.metadata.scale, 1.);
    }

    #[test]
    fn zoom_clips_at_image_edges_and_reports_the_actual_region() {
        let input = source(20, 15);
        let requested = Rect::new(17, 13, u32::MAX, 4);
        let actual = Rect::new(17, 13, 3, 2);
        let rendered = zoom(
            &input,
            requested,
            ZoomOptions {
                scale: Some(3.),
                label_every: 0,
                ..ZoomOptions::default()
            },
        )
        .unwrap();
        assert_eq!(rendered.metadata.source_region, actual);
        assert_eq!(rendered.image.dimensions(), (9, 6));
        assert_eq!(rendered.image.get_pixel(8, 5), input.get_pixel(19, 14));
        assert_eq!(
            rendered.warnings,
            [LocateWarning::RegionClipped { requested, actual }]
        );
    }

    #[test]
    fn zoom_rejects_empty_and_outside_regions() {
        let input = source(20, 15);
        for region in [Rect::new(5, 5, 0, 3), Rect::new(5, 5, 3, 0)] {
            assert_eq!(
                zoom(&input, region, ZoomOptions::default()).unwrap_err(),
                LocateError::EmptyRegion
            );
        }
        for region in [Rect::new(20, 5, 3, 3), Rect::new(5, 15, 3, 3)] {
            assert_eq!(
                zoom(&input, region, ZoomOptions::default()).unwrap_err(),
                LocateError::RegionOutsideImage
            );
        }
    }

    #[test]
    fn zoom_rejects_invalid_scale_settings() {
        let input = source(20, 15);
        let region = Rect::new(0, 0, 20, 15);
        for scale in [f64::NAN, f64::INFINITY, 0.5] {
            assert_eq!(
                zoom(
                    &input,
                    region,
                    ZoomOptions {
                        scale: Some(scale),
                        ..ZoomOptions::default()
                    }
                )
                .unwrap_err(),
                LocateError::InvalidScale,
            );
        }
        assert_eq!(
            zoom(
                &input,
                region,
                ZoomOptions {
                    max_side: 0,
                    ..ZoomOptions::default()
                }
            )
            .unwrap_err(),
            LocateError::InvalidMaxSide,
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn zoom_labels_mark_the_crop_origin_and_selected_global_ticks() {
        let input = RgbaImage::from_pixel(60, 60, Rgba([0, 0, 0, 255]));
        let rendered = zoom(
            &input,
            Rect::new(13, 13, 40, 40),
            ZoomOptions {
                scale: Some(10.),
                color: Rgba([0, 0, 0, 0]),
                label_every: 2,
                ..ZoomOptions::default()
            },
        )
        .unwrap();
        let labeled = |left, top| {
            (top..top + 18)
                .any(|y| (left..left + 25).any(|x| rendered.image.get_pixel(x, y)[0] > 0))
        };
        assert!(rendered.warnings.is_empty());
        assert!(labeled(0, 0));
        for (x, y) in [(270, 0), (0, 270)] {
            assert!(labeled(x, y));
        }
        for (x, y) in [(170, 0), (0, 170), (200, 0), (0, 200)] {
            assert!(!labeled(x, y));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn grid_labels_are_visible_on_both_axes_on_light_and_dark_images() {
        for background in [0, 255] {
            let input =
                RgbaImage::from_pixel(250, 250, Rgba([background, background, background, 255]));
            let rendered = grid(&input, GridOptions::default());
            assert!(rendered.warnings.is_empty());
            for (left, top) in [(101, 0), (0, 101)] {
                assert!((top..top + 18).any(|y| {
                    (left..left + 32)
                        .any(|x| rendered.image.get_pixel(x, y) != input.get_pixel(x, y))
                }));
            }
        }
    }
}
