use block2::RcBlock;
use image::RgbaImage;
use objc2::{AnyThread, rc::autoreleasepool};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGPreflightScreenCaptureAccess,
    CGRequestScreenCaptureAccess, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSArray, NSError, NSProcessInfo};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
};
use std::{sync::mpsc, time::Duration};

use super::{bitmap::CGBitmapContextCreate, displays, windows};
use crate::capture::{CaptureBackend, CaptureError, CaptureTargets, DisplayInfo, WindowInfo};

pub struct MacOsCapture;

impl MacOsCapture {
    /// Lists active displays without requesting screen recording permission.
    pub fn displays(&self) -> Result<Vec<DisplayInfo>, CaptureError> {
        displays::displays()
    }

    /// Lists layer-zero windows at least 32 points wide and tall.
    /// On-screen windows appear front to back, followed by off-screen windows.
    pub fn windows(&self) -> Vec<WindowInfo> {
        windows::windows(false)
    }

    /// Lists all WindowServer windows, including desktop elements and small windows.
    /// On-screen windows appear front to back, followed by off-screen windows.
    pub fn all_windows(&self) -> Vec<WindowInfo> {
        windows::windows(true)
    }
}

impl CaptureBackend for MacOsCapture {
    fn ready(&self) -> Result<(), CaptureError> {
        if NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            < 14
        {
            return Err(CaptureError::UnsupportedMacOs);
        }
        if !CGPreflightScreenCaptureAccess() && !CGRequestScreenCaptureAccess() {
            return Err(CaptureError::PermissionDenied);
        }
        Ok(())
    }

    fn screen(&self, display_id: u32, scale: f32) -> Result<RgbaImage, CaptureError> {
        let (send, receive) = mpsc::channel();
        autoreleasepool(|_| {
            let completion = RcBlock::new(
                move |content: *mut SCShareableContent, error: *mut NSError| {
                    // The framework keeps both arguments alive for the duration of the callback.
                    let result = unsafe { content.as_ref() }
                        .ok_or_else(|| {
                            CaptureError::DisplayEnumeration(native_message(
                                error,
                                "无法列出可录制的显示器",
                            ))
                        })
                        .and_then(|content| {
                            start_capture(content, display_id, scale, send.clone())
                        });
                    if let Err(error) = result {
                        let _ = send.send(Err(error));
                    }
                },
            );
            // ScreenCaptureKit copies the block and invokes it asynchronously.
            unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&completion) };
        });
        receive
            .recv_timeout(Duration::from_secs(15))
            .map_err(CaptureError::Timeout)?
    }

    fn targets(&self, display_id: u32) -> CaptureTargets {
        windows::capture_targets(display_id)
    }
}

fn start_capture(
    content: &SCShareableContent,
    display_id: u32,
    scale: f32,
    send: mpsc::Sender<Result<RgbaImage, CaptureError>>,
) -> Result<(), CaptureError> {
    // ScreenCaptureKit objects below come from the current shareable content snapshot.
    unsafe {
        let display = content
            .displays()
            .iter()
            .find(|display| display.displayID() == display_id)
            .ok_or(CaptureError::DisplayNotFound)?;
        let applications: Vec<_> = content
            .applications()
            .iter()
            .filter(|app| app.processID() as u32 == std::process::id())
            .collect();
        let excluded =
            NSArray::from_slice(&applications.iter().map(|app| &**app).collect::<Vec<_>>());
        let filter = SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
            SCContentFilter::alloc(),
            &display,
            &excluded,
            &NSArray::new(),
        );
        let config = SCStreamConfiguration::new();
        config.setWidth((display.width() as f32 * scale).round() as usize);
        config.setHeight((display.height() as f32 * scale).round() as usize);
        config.setShowsCursor(false);
        let completion = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            let result = image
                .as_ref()
                .ok_or_else(|| CaptureError::Screenshot(native_message(error, "截取屏幕失败")))
                .and_then(to_rgba);
            let _ = send.send(result);
        });
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&completion),
        );
    }
    Ok(())
}

fn native_message(error: *mut NSError, fallback: &str) -> String {
    // Only called with a callback argument while the Objective-C object is still alive.
    match unsafe { error.as_ref() } {
        Some(error) => format!("{fallback} ({})", error.localizedDescription()),
        None => fallback.to_owned(),
    }
}

fn to_rgba(image: &CGImage) -> Result<RgbaImage, CaptureError> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let stride = width.checked_mul(4).ok_or(CaptureError::RowTooWide)?;
    let len = stride
        .checked_mul(height)
        .ok_or(CaptureError::ImageTooLarge)?;
    let mut pixels = vec![0; len];
    let color_space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or(CaptureError::ColorSpace)?;
    // The context borrows the byte buffer. Drop it before reading or moving the buffer.
    let context = unsafe {
        let ptr = CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            stride,
            &color_space,
            CGImageAlphaInfo::NoneSkipLast.0,
        )
        .ok_or(CaptureError::BitmapContext)?;
        CFRetained::from_raw(ptr)
    };
    let rect = CGRect::new(
        CGPoint::new(0., 0.),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&context), rect, Some(image));
    drop(context);
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    RgbaImage::from_raw(width.try_into()?, height.try_into()?, pixels)
        .ok_or(CaptureError::BufferSize)
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{CGBitmapInfo, CGColorRenderingIntent, CGDataProvider};

    use super::{CGColorSpace, CGImage, CGImageAlphaInfo, kCGColorSpaceSRGB, to_rgba};

    #[test]
    fn core_graphics_preserves_color_and_row_order() {
        let packed_rgb = [255, 0, 0, 0, 0, 0, 255, 0];
        let provider =
            CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&packed_rgb))).unwrap();
        let color_space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).unwrap();
        // The data provider contains exactly two RGBX pixels. Null uses the default decode table.
        let image = unsafe {
            CGImage::new(
                1,
                2,
                8,
                32,
                4,
                Some(&color_space),
                CGBitmapInfo(CGImageAlphaInfo::NoneSkipLast.0),
                Some(&provider),
                std::ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .unwrap();
        let rgba = to_rgba(&image).unwrap();
        assert_eq!(rgba.dimensions(), (1, 2));
        assert_eq!(rgba.as_raw(), &[255, 0, 0, 255, 0, 0, 255, 255]);
    }
}
