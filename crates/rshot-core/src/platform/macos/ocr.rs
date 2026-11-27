use std::{ops::Range, ptr};

use image::RgbaImage;
use objc2::{
    AnyThread,
    rc::{Retained, autoreleasepool},
};
use objc2_core_foundation::{CFData, CFRetained, CGRect};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSProcessInfo, NSRange, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizeTextRequestRevision3,
    VNRecognizedText, VNRequestTextRecognitionLevel,
};

use crate::{
    geometry::Rect,
    ocr::{OcrError, OcrOptions, RecognitionLevel, TextObservation, vision_rect},
};

#[derive(Clone, Debug)]
pub(crate) struct RangeSource {
    text: Retained<VNRecognizedText>,
    dimensions: (u32, u32),
}

impl RangeSource {
    pub(crate) fn rect_for(&self, text: &str, range: &Range<usize>) -> Option<Rect<u32>> {
        autoreleasepool(|_| {
            if self.text.string().to_string() != text {
                return None;
            }
            let before = &text[..range.start];
            let after = &text[range.end..];
            let start = before.encode_utf16().count();
            let length = text[range.clone()].encode_utf16().count();
            let rect = self.bounding_rect(start, length)?;

            // In accurate mode Vision can return the same box for a word and part of that word.
            if let Some(previous) = before.chars().next_back().filter(|ch| !ch.is_whitespace()) {
                let units = previous.len_utf16();
                if self.bounding_rect(start - units, length + units) == Some(rect) {
                    return None;
                }
            }
            if let Some(next) = after.chars().next().filter(|ch| !ch.is_whitespace())
                && self.bounding_rect(start, length + next.len_utf16()) == Some(rect)
            {
                return None;
            }
            Some(rect)
        })
    }

    fn bounding_rect(&self, start: usize, length: usize) -> Option<Rect<u32>> {
        // The range is measured in UTF-16 units of the candidate's NSString.
        let observation = unsafe {
            self.text
                .boundingBoxForRange_error(NSRange::new(start, length))
        }
        .ok()?;
        // Vision returned a rectangle observation for this range.
        let rect = pixel_rect(unsafe { observation.boundingBox() }, self.dimensions);
        (rect.size.width > 0 && rect.size.height > 0).then_some(rect)
    }
}

pub(crate) fn recognize(
    image: &RgbaImage,
    options: &OcrOptions,
) -> Result<Vec<TextObservation>, OcrError> {
    autoreleasepool(|_| {
        if NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            < 14
        {
            return Err(OcrError::UnsupportedPlatform);
        }
        let dimensions = image.dimensions();
        let image = cg_image(image)?;
        let request = VNRecognizeTextRequest::new();
        // Revision 3 is available on macOS 14 and enables automatic language detection.
        unsafe { request.setRevision(VNRecognizeTextRequestRevision3) };
        request.setRecognitionLevel(match options.level {
            RecognitionLevel::Accurate => VNRequestTextRecognitionLevel::Accurate,
            RecognitionLevel::Fast => VNRequestTextRecognitionLevel::Fast,
        });
        request.setAutomaticallyDetectsLanguage(options.languages.is_empty());
        if !options.languages.is_empty() {
            set_languages(&request, options)?;
        }
        // An empty options dictionary is valid for any CGImage.
        let handler = unsafe {
            VNImageRequestHandler::initWithCGImage_options(
                VNImageRequestHandler::alloc(),
                &image,
                &NSDictionary::new(),
            )
        };
        handler
            .performRequests_error(&NSArray::from_slice(&[request.as_ref()]))
            .map_err(vision_error)?;

        let mut observations = Vec::new();
        if let Some(results) = request.results() {
            for observation in results.iter() {
                if let Some(candidate) = observation.topCandidates(1).firstObject() {
                    // The result is a Vision text observation with a bounding box.
                    let bounds = unsafe { observation.boundingBox() };
                    observations.push(TextObservation {
                        text: candidate.string().to_string(),
                        confidence: candidate.confidence(),
                        rect: pixel_rect(bounds, dimensions),
                        range_source: Some(RangeSource {
                            text: candidate,
                            dimensions,
                        }),
                    });
                }
            }
        }
        observations.sort_by_key(|line| (line.rect.origin.y, line.rect.origin.x));
        Ok(observations)
    })
}

fn set_languages(request: &VNRecognizeTextRequest, options: &OcrOptions) -> Result<(), OcrError> {
    // The request is configured before querying the languages for its revision and level.
    let supported =
        unsafe { request.supportedRecognitionLanguagesAndReturnError() }.map_err(vision_error)?;
    let supported: Vec<_> = supported
        .iter()
        .map(|language| language.to_string())
        .collect();
    let mut languages = Vec::with_capacity(options.languages.len());
    for language in &options.languages {
        let code = if language.eq_ignore_ascii_case("en") {
            "en-US"
        } else {
            language
        };
        let canonical = supported
            .iter()
            .find(|supported| supported.eq_ignore_ascii_case(code))
            .ok_or_else(|| OcrError::UnsupportedLanguage {
                language: language.clone(),
                level: options.level,
            })?;
        languages.push(NSString::from_str(canonical));
    }
    request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
    Ok(())
}

fn cg_image(image: &RgbaImage) -> Result<CFRetained<CGImage>, OcrError> {
    // kCGColorSpaceSRGB is a Core Graphics color space name.
    let colors = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or(OcrError::ImagePreparation)?;
    let bytes = CFData::from_bytes(image.as_raw());
    let provider = CGDataProvider::with_cf_data(Some(&bytes)).ok_or(OcrError::ImagePreparation)?;
    // The data provider retains the RGBA bytes and the null decode table selects defaults.
    unsafe {
        CGImage::new(
            image.width() as usize,
            image.height() as usize,
            8,
            32,
            image.width() as usize * 4,
            Some(&colors),
            CGBitmapInfo(CGImageAlphaInfo::Last.0 | CGImageByteOrderInfo::Order32Big.0),
            Some(&provider),
            ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or(OcrError::ImagePreparation)
}

fn pixel_rect(bounds: CGRect, dimensions: (u32, u32)) -> Rect<u32> {
    vision_rect(
        Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bounds.size.height,
        ),
        dimensions,
    )
}

fn vision_error(error: Retained<NSError>) -> OcrError {
    OcrError::Vision(error.localizedDescription().to_string())
}
