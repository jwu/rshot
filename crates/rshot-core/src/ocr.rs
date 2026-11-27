use std::{num::NonZeroUsize, ops::Range};

use caseless::{Caseless, default_case_fold_str};
use image::RgbaImage;
use serde::Serialize;
use thiserror::Error;

use crate::geometry::Rect;
#[cfg(target_os = "macos")]
use crate::platform::macos::ocr as vision;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecognitionLevel {
    #[default]
    Accurate,
    Fast,
}

#[derive(Clone, Debug, Default)]
pub struct OcrOptions {
    pub languages: Vec<String>,
    pub level: RecognitionLevel,
}

#[derive(Clone, Debug, Serialize)]
pub struct TextObservation {
    pub text: String,
    pub confidence: f32,
    pub rect: Rect<u32>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    pub(crate) range_source: Option<vision::RangeSource>,
}

impl TextObservation {
    pub fn new(text: impl Into<String>, confidence: f32, rect: Rect<u32>) -> Self {
        Self {
            text: text.into(),
            confidence,
            rect,
            #[cfg(target_os = "macos")]
            range_source: None,
        }
    }

    fn match_rect(&self, range: &Range<usize>) -> (Rect<u32>, bool) {
        if *range == (0..self.text.len()) {
            return (self.rect, false);
        }
        #[cfg(target_os = "macos")]
        if let Some(rect) = self
            .range_source
            .as_ref()
            .and_then(|source| source.rect_for(&self.text, range))
        {
            return (rect, false);
        }

        let characters = self.text.chars().count() as u128;
        let start = self.text[..range.start].chars().count() as u128;
        let end = start + self.text[range.clone()].chars().count() as u128;
        let width = u128::from(self.rect.size.width);
        let left = (start * width / characters) as u32;
        let right = (end * width).div_ceil(characters) as u32;
        (
            Rect::new(
                self.rect.origin.x.saturating_add(left),
                self.rect.origin.y,
                right - left,
                self.rect.size.height,
            ),
            true,
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LocateOptions {
    pub exact: bool,
    pub case_sensitive: bool,
    pub nth: Option<NonZeroUsize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Match {
    pub text: String,
    pub rect: Rect<u32>,
    pub confidence: f32,
    pub estimated: bool,
}

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("Text recognition requires macOS 14 or later.")]
    UnsupportedPlatform,
    #[error("The image must have a nonzero width and height.")]
    EmptyImage,
    #[error("Could not prepare the image for text recognition.")]
    ImagePreparation,
    #[error("Vision does not support language {language:?} in {level:?} mode.")]
    UnsupportedLanguage {
        language: String,
        level: RecognitionLevel,
    },
    #[error("Vision text recognition failed ({0}).")]
    Vision(String),
}

pub fn recognize(
    image: &RgbaImage,
    options: &OcrOptions,
) -> Result<Vec<TextObservation>, OcrError> {
    #[cfg(target_os = "macos")]
    {
        if image.is_empty() {
            return Err(OcrError::EmptyImage);
        }
        vision::recognize(image, options)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (image, options);
        Err(OcrError::UnsupportedPlatform)
    }
}

pub fn locate(observations: &[TextObservation], query: &str, options: LocateOptions) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let folded_query = if options.case_sensitive {
        query.to_owned()
    } else {
        default_case_fold_str(query)
    };
    let mut matches = observations
        .iter()
        .flat_map(|line| {
            match_ranges(&line.text, &folded_query, options)
                .into_iter()
                .map(move |range| (line, range))
        })
        .map(|(line, range)| {
            let (rect, estimated) = line.match_rect(&range);
            Match {
                text: line.text[range].to_owned(),
                rect,
                confidence: line.confidence,
                estimated,
            }
        });
    match options.nth {
        Some(nth) => matches.nth(nth.get() - 1).into_iter().collect(),
        None => matches.collect(),
    }
}

fn match_ranges(text: &str, query: &str, options: LocateOptions) -> Vec<Range<usize>> {
    if options.case_sensitive {
        if options.exact && text != query {
            return Vec::new();
        }
        return text
            .match_indices(query)
            .map(|(start, found)| start..start + found.len())
            .collect();
    }
    if options.exact {
        return (default_case_fold_str(text) == query)
            .then_some(0..text.len())
            .into_iter()
            .collect();
    }

    // Track original byte offsets for case folds that change the length of a character.
    let mut folded = String::with_capacity(text.len());
    let mut boundaries = Vec::new();
    for (offset, ch) in text.char_indices() {
        boundaries.push((folded.len(), offset));
        folded.extend(std::iter::once(ch).default_case_fold());
    }
    boundaries.push((folded.len(), text.len()));

    let mut ranges = Vec::new();
    let mut previous_end = 0;
    for (start, original_start) in boundaries.iter().copied() {
        if start < previous_end || !folded[start..].starts_with(query) {
            continue;
        }
        let end = start + query.len();
        if let Ok(index) = boundaries.binary_search_by_key(&end, |&(offset, _)| offset) {
            ranges.push(original_start..boundaries[index].1);
            previous_end = end;
        }
    }
    ranges
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn vision_rect(bounds: Rect<f64>, (width, height): (u32, u32)) -> Rect<u32> {
    let x = bounds.origin.x;
    let y = bounds.origin.y;
    let w = bounds.size.width;
    let h = bounds.size.height;
    if ![x, y, w, h].into_iter().all(f64::is_finite) || w <= 0. || h <= 0. {
        return Rect::new(0, 0, 0, 0);
    }
    let pixel_x = |x: f64| x.clamp(0., 1.) * f64::from(width);
    let pixel_y = |y: f64| (1. - y).clamp(0., 1.) * f64::from(height);
    let left = pixel_x(x).floor() as u32;
    let right = pixel_x(x + w).ceil() as u32;
    let top = pixel_y(y + h).floor() as u32;
    let bottom = pixel_y(y).ceil() as u32;
    Rect::new(left, top, right - left, bottom - top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_images_with_an_empty_dimension() {
        let image = RgbaImage::new(0, 10);
        assert!(matches!(
            recognize(&image, &OcrOptions::default()),
            Err(OcrError::EmptyImage)
        ));
    }

    #[test]
    fn flips_vision_coordinates_and_rounds_outward() {
        assert_eq!(
            vision_rect(Rect::new(0.125, 0.5, 0.25, 0.125), (200, 100)),
            Rect::new(25, 37, 50, 13)
        );
    }

    #[test]
    fn clips_vision_boxes_to_the_image() {
        assert_eq!(
            vision_rect(Rect::new(-0.125, 0.75, 0.625, 0.5), (200, 100)),
            Rect::new(0, 0, 100, 25)
        );
        assert_eq!(
            vision_rect(Rect::new(0.75, -0.125, 0.5, 0.25), (200, 100)),
            Rect::new(150, 87, 50, 13)
        );
        assert_eq!(
            vision_rect(Rect::new(0.5, f64::NAN, 0.5, 0.5), (200, 100)),
            Rect::new(0, 0, 0, 0)
        );
    }

    #[test]
    fn finds_case_insensitive_substrings_and_estimates_by_character_count() {
        let observations = [TextObservation::new(
            "猫Open门",
            0.9,
            Rect::new(10, 20, 61, 15),
        )];
        let options = LocateOptions::default();
        assert_eq!(
            locate(&observations, "open", options),
            [Match {
                text: "Open".into(),
                rect: Rect::new(20, 20, 41, 15),
                confidence: 0.9,
                estimated: true,
            }]
        );
        assert!(locate(&observations, "", options).is_empty());
        assert!(locate(&observations, "close", options).is_empty());
    }

    #[test]
    fn case_folding_preserves_original_character_boundaries() {
        let observations = [TextObservation::new("sß Σς", 1., Rect::new(0, 0, 50, 10))];
        let options = LocateOptions::default();
        assert_eq!(locate(&observations, "ss", options)[0].text, "ß");
        assert_eq!(locate(&observations, "s", options).len(), 1);
        assert_eq!(locate(&observations, "σσ", options)[0].text, "Σς");
    }

    #[test]
    fn exact_matches_require_a_full_line_and_use_its_rect() {
        let rect = Rect::new(10, 20, 33, 12);
        let observations = [TextObservation::new("Save", 1., rect)];
        let mut options = LocateOptions {
            exact: true,
            ..LocateOptions::default()
        };
        assert!(locate(&observations, "Sav", options).is_empty());
        let results = locate(&observations, "save", options);
        assert_eq!(results[0].rect, rect);
        assert!(!results[0].estimated);
        options.case_sensitive = true;
        assert!(locate(&observations, "save", options).is_empty());
        assert_eq!(locate(&observations, "Save", options), results);
    }

    #[test]
    fn returns_every_occurrence_or_selects_the_one_based_index() {
        let observations = [
            TextObservation::new("Tap tap", 1., Rect::new(10, 20, 70, 12)),
            TextObservation::new("TAP", 1., Rect::new(30, 40, 30, 11)),
        ];
        let mut options = LocateOptions::default();
        let results = locate(&observations, "Tap", options);
        assert_eq!(
            results.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["Tap", "tap", "TAP"]
        );
        assert_eq!(results[1].rect, Rect::new(50, 20, 30, 12));
        options.case_sensitive = true;
        assert_eq!(locate(&observations, "Tap", options), results[..1]);
        options.case_sensitive = false;
        options.nth = NonZeroUsize::new(2);
        assert_eq!(locate(&observations, "Tap", options), results[1..2]);
        options.nth = NonZeroUsize::new(4);
        assert!(locate(&observations, "Tap", options).is_empty());
    }
}
