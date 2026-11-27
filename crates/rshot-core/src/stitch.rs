use std::{collections::VecDeque, ops::Range};

use image::{
    RgbaImage,
    imageops::{crop_imm, replace},
};
use thiserror::Error;

const MAX_PIXELS: u64 = 64_000_000;
const ROW_SAMPLES: usize = 16;
const MIN_OVERLAP: usize = 20;

type Row = [u8; ROW_SAMPLES];

#[derive(Debug, Error)]
pub enum StitchError {
    #[error("选区太小，无法匹配长截图")]
    RegionTooSmall,
    #[error("选区超过长截图大小上限")]
    RegionTooLarge,
    #[error("截图尺寸发生变化，请重新选区")]
    FrameResized,
    #[error("长截图已达 6400 万像素，请先导出")]
    PixelLimit,
}

#[derive(Clone, Copy)]
pub enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    fn orient(self, image: RgbaImage) -> RgbaImage {
        match self {
            Self::Vertical => image,
            Self::Horizontal => {
                RgbaImage::from_fn(image.height(), image.width(), |x, y| *image.get_pixel(y, x))
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Extended,
    Revisited,
    Unchanged,
    Unmatched,
}

pub struct Stitcher {
    axis: Axis,
    previous: RgbaImage,
    edges: Option<(RgbaImage, RgbaImage)>,
    chunks: VecDeque<RgbaImage>,
    position: i64,
    start: i64,
    end: i64,
}

impl Stitcher {
    pub fn new(axis: Axis, first: RgbaImage) -> Result<Self, StitchError> {
        let previous = axis.orient(first);
        if previous.width() < 8 || previous.height() < 32 {
            return Err(StitchError::RegionTooSmall);
        }
        if u64::from(previous.width()) * u64::from(previous.height()) > MAX_PIXELS {
            return Err(StitchError::RegionTooLarge);
        }
        Ok(Self {
            axis,
            previous,
            edges: None,
            chunks: VecDeque::new(),
            position: 0,
            start: 0,
            end: 0,
        })
    }

    pub fn push(&mut self, frame: RgbaImage) -> Result<Progress, StitchError> {
        let frame = self.axis.orient(frame);
        if frame.dimensions() != self.previous.dimensions() {
            return Err(StitchError::FrameResized);
        }
        if frame == self.previous {
            return Ok(Progress::Unchanged);
        }
        let a = sample_rows(&self.previous);
        let b = sample_rows(&frame);
        let height = frame.height();
        let fixed = self
            .edges
            .as_ref()
            .map(|(head, tail)| (head.height(), tail.height()));
        let (mut head, mut tail) = fixed.unwrap_or_else(|| fixed_edges(&a, &b));
        let body = head as usize..(height - tail) as usize;
        let mut delta = displacement(&a[body.clone()], &b[body]);
        if delta.is_none() && fixed.is_none() && head + tail > 0 {
            (head, tail) = (0, 0);
            delta = displacement(&a, &b);
        }
        let Some(delta) = delta else {
            return Ok(Progress::Unmatched);
        };
        let body_length = i64::from(height - head - tail);
        let end = if self.edges.is_none() {
            body_length
        } else {
            self.end
        };
        let position = self.position + i64::from(delta);
        let new_start = position.min(self.start);
        let new_end = (position + body_length).max(end);
        let total = new_end - new_start + i64::from(head + tail);
        if total as u64 * u64::from(frame.width()) > MAX_PIXELS {
            return Err(StitchError::PixelLimit);
        }
        if self.edges.is_none() {
            self.edges = Some((
                strip(&self.previous, 0..head),
                strip(&self.previous, height - tail..height),
            ));
            self.chunks
                .push_back(strip(&self.previous, head..height - tail));
        }
        if new_start < self.start {
            let count = (self.start - new_start) as u32;
            self.chunks.push_front(strip(&frame, head..head + count));
        }
        if new_end > end {
            let count = (new_end - end) as u32;
            self.chunks
                .push_back(strip(&frame, height - tail - count..height - tail));
        }
        let extended = new_start < self.start || new_end > end;
        self.previous = frame;
        self.position = position;
        self.start = new_start;
        self.end = new_end;
        Ok(if extended {
            Progress::Extended
        } else {
            Progress::Revisited
        })
    }

    fn strips(&self) -> impl Iterator<Item = &RgbaImage> {
        let (head, tail) = match &self.edges {
            Some((head, tail)) => (head, Some(tail)),
            None => (&self.previous, None),
        };
        std::iter::once(head).chain(&self.chunks).chain(tail)
    }

    fn oriented_dimensions(&self) -> (u32, u32) {
        (
            self.previous.width(),
            self.strips().map(RgbaImage::height).sum(),
        )
    }

    pub fn dimensions(&self) -> (u32, u32) {
        let (width, height) = self.oriented_dimensions();
        match self.axis {
            Axis::Vertical => (width, height),
            Axis::Horizontal => (height, width),
        }
    }

    pub fn image(&self) -> RgbaImage {
        let (width, height) = self.dimensions();
        let mut result = RgbaImage::new(width, height);
        let mut offset = 0;
        for chunk in self.strips() {
            match self.axis {
                Axis::Vertical => replace(&mut result, chunk, 0, i64::from(offset)),
                Axis::Horizontal => {
                    for (x, y, pixel) in chunk.enumerate_pixels() {
                        result.put_pixel(offset + y, x, *pixel);
                    }
                }
            }
            offset += chunk.height();
        }
        result
    }

    pub fn preview(&self, max_side: u32) -> RgbaImage {
        let (width, height) = self.oriented_dimensions();
        let scale = f64::from(max_side) / f64::from(width.max(height).max(max_side));
        let (w, h) = (
            (f64::from(width) * scale).round() as u32,
            (f64::from(height) * scale).round() as u32,
        );
        let mut preview = RgbaImage::new(w.max(1), h.max(1));
        let mut offset = 0;
        let mut strips = self.strips().peekable();
        for y in 0..preview.height() {
            let source_y = (u64::from(y) * u64::from(height) / u64::from(preview.height())) as u32;
            while let Some(strip) = strips.next_if(|strip| source_y >= offset + strip.height()) {
                offset += strip.height();
            }
            if let Some(strip) = strips.peek() {
                for x in 0..preview.width() {
                    let source_x =
                        (u64::from(x) * u64::from(width) / u64::from(preview.width())) as u32;
                    preview.put_pixel(x, y, *strip.get_pixel(source_x, source_y - offset));
                }
            }
        }
        self.axis.orient(preview)
    }
}

fn strip(image: &RgbaImage, rows: Range<u32>) -> RgbaImage {
    crop_imm(image, 0, rows.start, image.width(), rows.end - rows.start).to_image()
}

fn sample_rows(image: &RgbaImage) -> Vec<Row> {
    (0..image.height())
        .map(|y| {
            std::array::from_fn(|i| {
                let width = u64::from(image.width());
                let start = (i as u64 * width / ROW_SAMPLES as u64) as u32;
                let end = (((i + 1) as u64 * width / ROW_SAMPLES as u64) as u32).max(start + 1);
                let samples = (end - start).min(8);
                let sum: u32 = (0..samples)
                    .map(|sample| {
                        let x = start + (2 * sample + 1) * (end - start) / (2 * samples);
                        let [r, g, b, _] = image.get_pixel(x, y).0;
                        u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)
                    })
                    .sum();
                (sum / (samples * 10)) as u8
            })
        })
        .collect()
}

fn difference(a: &Row, b: &Row) -> u32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| u32::from(a.abs_diff(*b)))
        .sum()
}

fn fixed_edges(a: &[Row], b: &[Row]) -> (u32, u32) {
    let same = |(a, b): &(&Row, &Row)| difference(a, b) <= ROW_SAMPLES as u32;
    let cap = a.len() / 3;
    let head = a.iter().zip(b).take(cap + 1).take_while(same).count();
    let tail = a.iter().zip(b).rev().take(cap + 1).take_while(same).count();
    let has_detail = |rows: &[Row]| {
        rows.first().is_some_and(|first| {
            rows.iter()
                .flatten()
                .any(|sample| sample.abs_diff(first[0]) > 3)
        })
    };
    (
        if (3..=cap).contains(&head) && has_detail(&a[..head]) {
            head as u32
        } else {
            0
        },
        if (3..=cap).contains(&tail) && has_detail(&a[a.len() - tail..]) {
            tail as u32
        } else {
            0
        },
    )
}

fn displacement(a: &[Row], b: &[Row]) -> Option<i32> {
    let min_overlap = MIN_OVERLAP.max(a.len() / 3);
    let search = i32::try_from(a.len().checked_sub(min_overlap)?).ok()?;
    let mut best = (f32::INFINITY, 0);
    let mut second = f32::INFINITY;
    for offset in -search..=search {
        let a_start = offset.max(0) as usize;
        let b_start = (-offset).max(0) as usize;
        let overlap = a.len() - offset.unsigned_abs() as usize;
        let rows = overlap.min(120);
        let error: u32 = (0..rows)
            .map(|i| {
                let y = (i * 2 + 1) * overlap / (rows * 2);
                difference(&a[a_start + y], &b[b_start + y])
            })
            .sum();
        let score = error as f32 / (rows * ROW_SAMPLES) as f32;
        if score < best.0 {
            second = best.0;
            best = (score, offset);
        } else {
            second = second.min(score);
        }
    }
    (best.1 != 0 && best.0 <= 5. && second - best.0 >= (best.0 * 0.2).max(0.25)).then_some(best.1)
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{Axis, Progress, Stitcher};

    fn page(y: u32, x: u32) -> Rgba<u8> {
        let value = ((y * 0x54321) ^ ((x + 13) * 0x139)).wrapping_mul(0x45d9f3b);
        Rgba([(value >> 12) as u8, (value >> 22) as u8, value as u8, 255])
    }

    fn frame(offset: u32) -> RgbaImage {
        RgbaImage::from_fn(52, 92, |x, y| match y {
            0..8 => Rgba([18, 18, x as u8, 255]),
            86.. => Rgba([44, 44, x as u8, 255]),
            _ => page(y + offset, x),
        })
    }

    #[test]
    fn fixed_edges_and_reverse_scroll_preserve_distinct_rows() {
        let mut stitch = Stitcher::new(Axis::Vertical, frame(25)).unwrap();
        assert_eq!(stitch.push(frame(45)).unwrap(), Progress::Extended);
        assert_eq!(stitch.push(frame(45)).unwrap(), Progress::Unchanged);
        assert_eq!(stitch.push(frame(25)).unwrap(), Progress::Revisited);
        assert_eq!(stitch.push(frame(5)).unwrap(), Progress::Extended);
        assert_eq!(stitch.dimensions(), (52, 132));
        let result = stitch.image();
        for y in 8..126 {
            assert_eq!(*result.get_pixel(7, y), page(y + 5, 7));
        }
        assert_eq!(*result.get_pixel(7, 7), *frame(25).get_pixel(7, 7));
        assert_eq!(*result.get_pixel(7, 126), *frame(25).get_pixel(7, 86));
        assert_eq!(stitch.preview(132), result);
    }

    #[test]
    fn rejects_ambiguous_repetition_without_losing_last_good_frame() {
        let stripes = |offset| RgbaImage::from_fn(40, 100, |x, y| page((y + offset) % 12, x));
        let mut stitch = Stitcher::new(Axis::Vertical, stripes(0)).unwrap();
        assert_eq!(stitch.push(stripes(5)).unwrap(), Progress::Unmatched);
        assert_eq!(stitch.image(), stripes(0));
        assert_eq!(stitch.push(stripes(0)).unwrap(), Progress::Unchanged);
    }

    #[test]
    fn sparse_document_keeps_blank_margins_and_recovers_after_a_large_jump() {
        let document = |offset| {
            RgbaImage::from_fn(420, 190, |x, y| {
                let y = y + offset;
                let text = (y % 29 > 20 && (25..230 + y % 81).contains(&x))
                    || (y % 57 > 46 && (70..185 + y % 110).contains(&x));
                if text && (x + y) % 7 > 2 {
                    Rgba([30, 30, 30, 255])
                } else {
                    Rgba([255, 255, 255, 255])
                }
            })
        };
        let first = document(0);
        let next = document(32);
        let mut stitch = Stitcher::new(Axis::Vertical, first.clone()).unwrap();
        assert_eq!(stitch.push(document(210)).unwrap(), Progress::Unmatched);
        assert_eq!(stitch.push(next.clone()).unwrap(), Progress::Extended);
        let result = stitch.image();
        assert_eq!(result.dimensions(), (420, 222));
        for y in 0..222 {
            let expected = if y < 190 {
                first.get_pixel(110, y)
            } else {
                next.get_pixel(110, y - 32)
            };
            assert_eq!(result.get_pixel(110, y), expected);
        }
    }

    #[test]
    fn horizontal_assembly_uses_the_same_matcher() {
        let mut stitch =
            Stitcher::new(Axis::Horizontal, Axis::Horizontal.orient(frame(0))).unwrap();
        assert_eq!(
            stitch.push(Axis::Horizontal.orient(frame(18))).unwrap(),
            Progress::Extended
        );
        assert_eq!(stitch.dimensions(), (110, 52));
        assert_eq!(*stitch.image().get_pixel(92, 3), page(92, 3));
        assert_eq!(stitch.preview(56).dimensions(), (56, 26));
    }
}
