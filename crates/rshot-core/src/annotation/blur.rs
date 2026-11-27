use image::{Rgba, RgbaImage};

use crate::geometry::Rect;

const SCRATCH_PIXELS: usize = 128 * 1024;
type Pixel = [f32; 4];

pub(super) fn blur(image: &mut RgbaImage, region: Rect<u32>, radius: u32) {
    let (x, y) = (region.origin.x, region.origin.y);
    let (width, height) = (region.size.width, region.size.height);
    let mut samples = Vec::with_capacity(width as usize * height as usize);
    for row in y..y + height {
        for col in x..x + width {
            let [r, g, b, a] = image.get_pixel(col, row).0.map(|c| f32::from(c) / 255.);
            samples.push([r * a, g * a, b * a, a]);
        }
    }

    let (width, height) = (width as usize, height as usize);
    let mut row = vec![[0.; 4]; width];
    let mut vertical_scratch = Vec::new();
    for radius in box_radii(radius.min(width.max(height) as u32) as f32) {
        let weight = 1. / (2 * radius + 1) as f32;
        for pixels in samples.chunks_exact_mut(width) {
            row.copy_from_slice(pixels);
            let last = width - 1;
            let mut sum = row[0].map(|value| value * (radius + 1) as f32);
            for col in 1..=radius {
                add(&mut sum, row[col.min(last)]);
            }
            for (col, pixel) in pixels.iter_mut().enumerate() {
                *pixel = mean(sum, weight);
                add(&mut sum, row[(col + radius + 1).min(last)]);
                subtract(&mut sum, row[col.saturating_sub(radius)]);
            }
        }
        vertical_pass(
            &mut samples,
            width,
            height,
            radius,
            weight,
            &mut vertical_scratch,
        );
    }

    let byte = |channel: f32| (channel * 255.).round() as u8;
    for (row, pixels) in samples.chunks_exact(width).enumerate() {
        for (col, &[r, g, b, a]) in pixels.iter().enumerate() {
            let alpha = byte(a);
            let pixel = if alpha == 0 {
                Rgba([0; 4])
            } else {
                Rgba([byte(r / a), byte(g / a), byte(b / a), alpha])
            };
            image.put_pixel(x + col as u32, y + row as u32, pixel);
        }
    }
}

fn box_radii(sigma: f32) -> [usize; 3] {
    // Match image::imageops::fast_blur's three box widths, including its f32 rounding.
    let ideal = (12. * sigma.powi(2) / 3. + 1.).sqrt();
    let mut lower = ideal.floor();
    if lower % 2. == 0. {
        lower -= 1.;
    }
    let upper = lower + 2.;
    let count =
        (0.25 * 3. * (lower + 3.) - 3. * sigma.powi(2) * (lower + 1.).recip()).round() as usize;
    std::array::from_fn(|pass| {
        let width = if pass < count { lower } else { upper };
        ((width as usize).saturating_sub(1) / 2) | 1
    })
}

fn vertical_pass(
    samples: &mut [Pixel],
    width: usize,
    height: usize,
    radius: usize,
    weight: f32,
    scratch: &mut Vec<Pixel>,
) {
    let last = height - 1;
    let retained_rows = radius.min(height.saturating_sub(radius)).max(1);
    let stripe_width = (SCRATCH_PIXELS / (retained_rows + 1)).clamp(1, width);
    scratch.resize(stripe_width * (retained_rows + 1), [0.; 4]);
    for x in (0..width).step_by(stripe_width) {
        let cols = stripe_width.min(width - x);
        let (sums, earlier) = scratch.split_at_mut(cols);
        earlier[..cols].copy_from_slice(&samples[x..x + cols]);
        for (sum, pixel) in sums.iter_mut().zip(&earlier[..cols]) {
            *sum = pixel.map(|value| value * (radius + 1) as f32);
        }
        for row in 1..=radius {
            let start = row.min(last) * width + x;
            for (sum, &pixel) in sums.iter_mut().zip(&samples[start..start + cols]) {
                add(sum, pixel);
            }
        }
        for row in 0..last {
            let (written, remaining) = samples.split_at_mut((row + 1) * width);
            let start = row * width + x;
            let current = &mut written[start..start + cols];
            let start = radius.min(last - row - 1) * width + x;
            let next = &remaining[start..start + cols];
            let previous = row.saturating_sub(radius) % retained_rows * cols;
            let retain = row <= last.saturating_sub(radius);
            let slot = row % retained_rows * cols;
            for (i, ((pixel, sum), &next)) in
                current.iter_mut().zip(&mut *sums).zip(next).enumerate()
            {
                let filtered = mean(*sum, weight);
                add(sum, next);
                subtract(sum, earlier[previous + i]);
                if retain {
                    earlier[slot + i] = *pixel;
                }
                *pixel = filtered;
            }
        }
        let start = last * width + x;
        for (pixel, &sum) in samples[start..start + cols].iter_mut().zip(&*sums) {
            *pixel = mean(sum, weight);
        }
    }
}

#[inline]
fn add(sum: &mut Pixel, pixel: Pixel) {
    for (sum, value) in sum.iter_mut().zip(pixel) {
        *sum += value;
    }
}

#[inline]
fn subtract(sum: &mut Pixel, pixel: Pixel) {
    for (sum, value) in sum.iter_mut().zip(pixel) {
        *sum -= value;
    }
}

#[inline]
fn mean(sum: Pixel, weight: f32) -> Pixel {
    sum.map(|value| (value * weight).clamp(0., 1.))
}

#[cfg(test)]
mod tests {
    use image::{Rgba32FImage, imageops};

    use super::*;

    #[test]
    fn blur_matches_the_previous_filter_with_clipped_edges_and_alpha() {
        for (width, height, radius) in [
            (1, 1, 20),
            (1, 51, 32),
            (95, 1, 7),
            (19, 17, 8),
            (22, 15, 99),
            (513, 977, 20),
            (1025, 301, 255),
        ] {
            let source = RgbaImage::from_fn(width + 2, height + 2, |x, y| {
                Rgba([
                    (x * 29 + y * 11) as u8,
                    (x * y + y) as u8,
                    (y * 39) as u8,
                    ((x * 7 + y * 19) % 131) as u8,
                ])
            });
            let input = Rgba32FImage::from_fn(width, height, |x, y| {
                let [r, g, b, a] = source
                    .get_pixel(x + 1, y + 1)
                    .0
                    .map(|c| f32::from(c) / 255.);
                Rgba([r * a, g * a, b * a, a])
            });
            let reference = imageops::fast_blur(&input, radius.min(width.max(height)) as f32);
            let mut expected = source.clone();
            let byte = |c: f32| (c * 255.).round() as u8;
            for (x, y, &Rgba([r, g, b, a])) in reference.enumerate_pixels() {
                let pixel = if byte(a) == 0 {
                    [0; 4]
                } else {
                    [byte(r / a), byte(g / a), byte(b / a), byte(a)]
                };
                expected.put_pixel(x + 1, y + 1, Rgba(pixel));
            }
            let mut actual = source;
            blur(&mut actual, Rect::new(1, 1, width, height), radius);
            assert_eq!(actual, expected, "{width}x{height}, radius {radius}");
        }
    }
}
