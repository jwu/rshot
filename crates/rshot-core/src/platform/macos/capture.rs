use block2::RcBlock;
use image::RgbaImage;
use objc2::{
    AnyThread,
    rc::{Retained, autoreleasepool},
};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGPreflightScreenCaptureAccess,
    CGRequestScreenCaptureAccess, CGSessionCopyCurrentDictionary, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSArray, NSError, NSProcessInfo};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCScreenshotManager, SCShareableContent,
    SCStreamConfiguration, SCStreamErrorCode, SCStreamErrorDomain,
};
use std::{sync::mpsc, time::Duration};

use super::{bitmap::CGBitmapContextCreate, displays, windows};
use crate::{
    capture::{
        CaptureBackend, CaptureError, CaptureMetadata, CaptureSource, CaptureTargets,
        CapturedImage, DisplayInfo, WindowInfo, crop, window_display_id,
    },
    geometry::{Rect, Size},
};

pub struct MacOsCapture;

impl MacOsCapture {
    /// Checks the calling process's screen recording permission without requesting it.
    pub fn has_screen_recording_permission(&self) -> bool {
        CGPreflightScreenCaptureAccess()
    }

    /// Reports whether the current session is marked as locked by WindowServer.
    pub fn is_screen_locked(&self) -> bool {
        let Some(session) = CGSessionCopyCurrentDictionary() else {
            return false;
        };
        // CoreGraphics returns a dictionary with CFString keys and Core Foundation values.
        let session =
            unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(session) };
        // WindowServer omits this undocumented key in sessions without a reported lock state.
        session
            .get(&CFString::from_static_str("CGSSessionScreenIsLocked"))
            .is_some_and(|value| {
                value
                    .downcast_ref::<CFBoolean>()
                    .is_some_and(CFBoolean::value)
            })
    }

    /// Checks the capture environment without requesting permission or starting a screenshot.
    pub fn check_capture_available(&self) -> Result<(), CaptureError> {
        self.capture_displays().map(|_| ())
    }

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

    /// Captures the entire active display at its backing resolution. `None` selects the primary display.
    pub fn capture_display(&self, display_id: Option<u32>) -> Result<CapturedImage, CaptureError> {
        self.capture_display_area(display_id, None)
    }

    /// Captures a region in display-local points, measured from the top left.
    /// `None` selects the primary display. The captured bounds expand to pixel edges.
    /// Empty and out-of-bounds regions return `CaptureError::RegionOutOfBounds`.
    pub fn capture_region(
        &self,
        display_id: Option<u32>,
        region: Rect<f64>,
    ) -> Result<CapturedImage, CaptureError> {
        self.capture_display_area(display_id, Some(region))
    }

    /// Returns the geometry and pixel scale of a single-window screenshot without taking it.
    pub fn window_capture_metadata(&self, window_id: u32) -> Result<CaptureMetadata, CaptureError> {
        let displays = self.capture_displays()?;
        let (send, receive) = mpsc::channel();
        request_shareable_content(move |content| {
            let result = content
                .and_then(|content| window_capture_plan(content, window_id, &displays))
                .map(|(_, metadata)| metadata);
            let _ = send.send(result);
        });
        receive_capture(receive)
    }

    /// Captures the entire window in isolation, including portions outside the display.
    pub fn capture_window(&self, window_id: u32) -> Result<CapturedImage, CaptureError> {
        let displays = self.capture_displays()?;
        let (image, mut metadata) = capture_image(move |content| {
            let (filter, metadata) = window_capture_plan(content, window_id, &displays)?;
            let Size { width, height } = metadata.pixel_size;
            let config = stream_config(width as usize, height as usize);
            // These macOS 14 properties apply to desktop-independent window filters.
            unsafe {
                config.setIgnoreShadowsSingleWindow(true);
                config.setIgnoreGlobalClipSingleWindow(true);
                config.setShouldBeOpaque(true);
            }
            Ok((filter, config, metadata))
        })?;
        let (width, height) = image.dimensions();
        metadata.pixel_size = Size { width, height };
        Ok(CapturedImage { image, metadata })
    }

    fn capture_display_area(
        &self,
        display_id: Option<u32>,
        region: Option<Rect<f64>>,
    ) -> Result<CapturedImage, CaptureError> {
        let display = select_display(self.capture_displays()?, display_id)?;
        let selection = region
            .map(|region| display_region(&display, region))
            .transpose()?;
        let Size { width, height } = display.pixel_size;
        let expected_size = display.bounds.size;
        let id = display.id;
        let (frame, ()) = capture_image(move |content| {
            let native_display = find_display(content, id)?;
            // ScreenCaptureKit keeps the display alive through this configuration callback.
            if logical_rect(unsafe { native_display.frame() }).size != expected_size {
                return Err(CaptureError::DisplayResized);
            }
            let filter = display_filter(&native_display, &NSArray::new());
            let config = stream_config(width as usize, height as usize);
            Ok((filter, config, ()))
        })?;
        if frame.dimensions() != (width, height) {
            return Err(CaptureError::DisplayResized);
        }
        let (image, bounds, source) = match selection {
            Some((pixels, bounds)) => (crop(&frame, pixels)?, bounds, CaptureSource::Region),
            None => (frame, display.bounds, CaptureSource::Display),
        };
        let (width, height) = image.dimensions();
        Ok(CapturedImage {
            image,
            metadata: CaptureMetadata {
                source,
                display_id: Some(id),
                scale_factor: display.scale_factor,
                bounds,
                pixel_size: Size { width, height },
            },
        })
    }

    fn capture_displays(&self) -> Result<Vec<DisplayInfo>, CaptureError> {
        check_platform()?;
        if self.is_screen_locked() {
            return Err(CaptureError::ScreenLocked);
        }
        if !self.has_screen_recording_permission() {
            return Err(CaptureError::PermissionDenied);
        }
        let displays = self.displays()?;
        if displays.is_empty() {
            return Err(CaptureError::NoActiveDisplay);
        }
        Ok(displays)
    }
}

impl CaptureBackend for MacOsCapture {
    fn ready(&self) -> Result<(), CaptureError> {
        check_platform()?;
        if !self.has_screen_recording_permission() && !CGRequestScreenCaptureAccess() {
            return Err(CaptureError::PermissionDenied);
        }
        Ok(())
    }

    fn screen(&self, display_id: u32, scale: f32) -> Result<RgbaImage, CaptureError> {
        capture_image(move |content| {
            let display = find_display(content, display_id)?;
            // The display and applications come from the current shareable content snapshot.
            let (applications, width, height) = unsafe {
                let applications: Vec<_> = content
                    .applications()
                    .iter()
                    .filter(|app| app.processID() as u32 == std::process::id())
                    .collect();
                let width = (display.width() as f32 * scale).round() as usize;
                let height = (display.height() as f32 * scale).round() as usize;
                (applications, width, height)
            };
            let excluded =
                NSArray::from_slice(&applications.iter().map(|app| &**app).collect::<Vec<_>>());
            Ok((
                display_filter(&display, &excluded),
                stream_config(width, height),
                (),
            ))
        })
        .map(|(image, ())| image)
    }

    fn targets(&self, display_id: u32) -> CaptureTargets {
        windows::capture_targets(display_id)
    }
}

fn check_platform() -> Result<(), CaptureError> {
    if NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        < 14
    {
        return Err(CaptureError::UnsupportedMacOs);
    }
    Ok(())
}

fn select_display(
    displays: Vec<DisplayInfo>,
    id: Option<u32>,
) -> Result<DisplayInfo, CaptureError> {
    displays
        .into_iter()
        .find(|display| id.map_or(display.is_primary, |id| display.id == id))
        .ok_or(CaptureError::DisplayNotFound)
}

fn display_region(
    display: &DisplayInfo,
    region: Rect<f64>,
) -> Result<(Rect<u32>, Rect<f64>), CaptureError> {
    let (x0, x1) = pixel_edges(
        region.origin.x,
        region.size.width,
        display.bounds.size.width,
        display.pixel_size.width,
    )?;
    let (y0, y1) = pixel_edges(
        region.origin.y,
        region.size.height,
        display.bounds.size.height,
        display.pixel_size.height,
    )?;
    let pixels = Rect::new(x0, y0, x1 - x0, y1 - y0);
    let x_scale = display.bounds.size.width / f64::from(display.pixel_size.width);
    let y_scale = display.bounds.size.height / f64::from(display.pixel_size.height);
    let bounds = Rect::new(
        display.bounds.origin.x + f64::from(x0) * x_scale,
        display.bounds.origin.y + f64::from(y0) * y_scale,
        f64::from(pixels.size.width) * x_scale,
        f64::from(pixels.size.height) * y_scale,
    );
    Ok((pixels, bounds))
}

fn pixel_edges(
    start: f64,
    extent: f64,
    points: f64,
    pixels: u32,
) -> Result<(u32, u32), CaptureError> {
    let end = start + extent;
    if !(start >= 0. && end > start && end <= points) {
        return Err(CaptureError::RegionOutOfBounds);
    }
    let pixels = f64::from(pixels);
    Ok((
        (start * pixels / points).floor() as u32,
        (end * pixels / points).ceil() as u32,
    ))
}

fn logical_rect(rect: CGRect) -> Rect<f64> {
    Rect::new(
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    )
}

fn find_display(
    content: &SCShareableContent,
    id: u32,
) -> Result<Retained<SCDisplay>, CaptureError> {
    // ScreenCaptureKit supplies the display list for the current capture request.
    unsafe {
        content
            .displays()
            .iter()
            .find(|display| display.displayID() == id)
    }
    .ok_or(CaptureError::DisplayNotFound)
}

fn display_filter(
    display: &SCDisplay,
    excluded: &NSArray<SCRunningApplication>,
) -> Retained<SCContentFilter> {
    // The filter retains the display and application list as needed.
    unsafe {
        SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
            SCContentFilter::alloc(),
            display,
            excluded,
            &NSArray::new(),
        )
    }
}

fn window_capture_plan(
    content: &SCShareableContent,
    window_id: u32,
    displays: &[DisplayInfo],
) -> Result<(Retained<SCContentFilter>, CaptureMetadata), CaptureError> {
    // The window comes from the current shareable content snapshot.
    let (filter, bounds, scale_factor) = unsafe {
        let window = content
            .windows()
            .iter()
            .find(|window| window.windowID() == window_id)
            .ok_or_else(|| {
                if windows::exists(window_id) {
                    CaptureError::Screenshot(format!(
                        "Window {window_id} is unavailable for capture"
                    ))
                } else {
                    CaptureError::WindowNotFound(window_id)
                }
            })?;
        let filter =
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window);
        let bounds = logical_rect(window.frame());
        let scale_factor = f64::from(filter.pointPixelScale());
        (filter, bounds, scale_factor)
    };
    let width: u32 = ((bounds.size.width * scale_factor).round() as u64).try_into()?;
    let height: u32 = ((bounds.size.height * scale_factor).round() as u64).try_into()?;
    if width == 0 || height == 0 {
        return Err(CaptureError::Screenshot(format!(
            "Window {window_id} has no capturable pixels"
        )));
    }
    let metadata = CaptureMetadata {
        source: CaptureSource::Window { window_id },
        display_id: window_display_id(bounds, displays),
        scale_factor,
        bounds,
        pixel_size: Size { width, height },
    };
    Ok((filter, metadata))
}

fn stream_config(width: usize, height: usize) -> Retained<SCStreamConfiguration> {
    // The configuration is private to this capture request.
    unsafe {
        let config = SCStreamConfiguration::new();
        config.setWidth(width);
        config.setHeight(height);
        config.setShowsCursor(false);
        config
    }
}

type PreparedCapture<T> = (
    Retained<SCContentFilter>,
    Retained<SCStreamConfiguration>,
    T,
);

fn capture_image<T: Copy + Send + Sync + 'static>(
    prepare: impl Fn(&SCShareableContent) -> Result<PreparedCapture<T>, CaptureError>
    + Send
    + Sync
    + 'static,
) -> Result<(RgbaImage, T), CaptureError> {
    let (send, receive) = mpsc::channel();
    request_shareable_content(move |content| {
        let (filter, config, metadata) = match content.and_then(&prepare) {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = send.send(Err(error));
                return;
            }
        };
        let send = send.clone();
        let completion = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            // The CGImage and error stay alive until this callback returns.
            let (image, error) = unsafe { (image.as_ref(), error.as_ref()) };
            let result = image
                .ok_or_else(|| native_error(error, "截取屏幕失败", CaptureError::Screenshot))
                .and_then(to_rgba)
                .map(|frame| (frame, metadata));
            let _ = send.send(result);
        });
        // ScreenCaptureKit copies the block and retains the filter and configuration as needed.
        unsafe {
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                &filter,
                &config,
                Some(&completion),
            );
        }
    });
    receive_capture(receive)
}

fn request_shareable_content(
    reply: impl Fn(Result<&SCShareableContent, CaptureError>) + Send + Sync + 'static,
) {
    autoreleasepool(|_| {
        let completion = RcBlock::new(
            move |content: *mut SCShareableContent, error: *mut NSError| {
                // The framework keeps both arguments alive for the duration of the callback.
                let (content, error) = unsafe { (content.as_ref(), error.as_ref()) };
                reply(content.ok_or_else(|| {
                    native_error(
                        error,
                        "无法列出可录制的显示器",
                        CaptureError::DisplayEnumeration,
                    )
                }));
            },
        );
        // The copied block runs on a ScreenCaptureKit queue, including in processes without a run loop.
        unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&completion) };
    });
}

fn receive_capture<T>(receive: mpsc::Receiver<Result<T, CaptureError>>) -> Result<T, CaptureError> {
    receive
        .recv_timeout(Duration::from_secs(15))
        .map_err(CaptureError::Timeout)?
}

fn native_error(
    error: Option<&NSError>,
    fallback: &str,
    wrap: impl FnOnce(String) -> CaptureError,
) -> CaptureError {
    let Some(error) = error else {
        return wrap(fallback.to_owned());
    };
    // ScreenCaptureKit provides this immutable domain constant.
    if &*error.domain() == unsafe { SCStreamErrorDomain }
        && matches!(
            SCStreamErrorCode(error.code()),
            SCStreamErrorCode::UserDeclined | SCStreamErrorCode::MissingEntitlements
        )
    {
        return CaptureError::PermissionDenied;
    }
    wrap(format!("{fallback} ({})", error.localizedDescription()))
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

    use super::*;

    fn display(id: u32, bounds: Rect<f64>, scale: u32, is_primary: bool) -> DisplayInfo {
        DisplayInfo {
            id,
            bounds,
            scale_factor: f64::from(scale),
            pixel_size: Size {
                width: bounds.size.width as u32 * scale,
                height: bounds.size.height as u32 * scale,
            },
            is_primary,
        }
    }

    #[test]
    fn selects_the_primary_or_requested_display_in_a_mixed_scale_setup() {
        let primary = display(1, Rect::new(0., 0., 1440., 900.), 2, true);
        let secondary = display(2, Rect::new(-1600., -200., 1600., 1000.), 1, false);
        let displays = vec![secondary.clone(), primary.clone()];
        assert_eq!(select_display(displays.clone(), None).unwrap(), primary);
        assert_eq!(
            select_display(displays.clone(), Some(2)).unwrap(),
            secondary
        );
        assert!(matches!(
            select_display(displays, Some(3)),
            Err(CaptureError::DisplayNotFound)
        ));
    }

    #[test]
    fn aligns_a_local_region_to_the_display_pixel_grid() {
        let bounds = Rect::new(-120., -100., 120., 100.);
        let region = Rect::new(0.75, 1.25, 2.25, 3.5);
        let retina = display(1, bounds, 2, false);
        let standard = display(2, bounds, 1, false);
        assert_eq!(
            display_region(&retina, region).unwrap(),
            (Rect::new(1, 2, 5, 8), Rect::new(-119.5, -99., 2.5, 4.)),
        );
        assert_eq!(
            display_region(&standard, region).unwrap(),
            (Rect::new(0, 1, 3, 4), Rect::new(-120., -99., 3., 4.)),
        );
        assert_eq!(
            display_region(&retina, Rect::new(0., 0., 120., 100.)).unwrap(),
            (Rect::new(0, 0, 240, 200), bounds),
        );
    }

    #[test]
    fn rejects_empty_nonfinite_and_out_of_bounds_regions() {
        let display = display(1, Rect::new(-500., 50., 500., 600.), 2, false);
        for region in [
            Rect::new(0., 0., 0., 20.),
            Rect::new(0., 0., 20., -1.),
            Rect::new(0., f64::NAN, 20., 20.),
            Rect::new(0., 0., 20., f64::INFINITY),
            Rect::new(-0.1, 0., 20., 20.),
            Rect::new(490., 0., 10.1, 20.),
            Rect::new(0., 600., 20., 10.),
        ] {
            assert!(matches!(
                display_region(&display, region),
                Err(CaptureError::RegionOutOfBounds)
            ));
        }
    }

    #[test]
    fn crops_the_selected_physical_pixels_in_top_left_order() {
        let display = display(1, Rect::new(-4., 0., 4., 4.), 2, false);
        let (pixels, _) = display_region(&display, Rect::new(0.5, 1.5, 1.5, 1.)).unwrap();
        let frame = RgbaImage::from_fn(8, 8, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        let region = crop(&frame, pixels).unwrap();
        assert_eq!(region.dimensions(), (3, 2));
        assert_eq!(region.get_pixel(0, 0).0, [1, 3, 0, 255]);
        assert_eq!(region.get_pixel(2, 1).0, [3, 4, 0, 255]);
    }

    #[test]
    fn associates_a_cross_display_window_by_its_point_area() {
        let retina = display(1, Rect::new(0., 0., 1440., 900.), 2, true);
        let standard = display(2, Rect::new(-1600., -200., 1600., 1000.), 1, false);
        let displays = [standard, retina];
        assert_eq!(
            window_display_id(Rect::new(-300., 20., 400., 600.), &displays),
            Some(2)
        );
        assert_eq!(
            window_display_id(Rect::new(-200., 20., 400., 600.), &displays),
            Some(1)
        );
        assert_eq!(
            window_display_id(Rect::new(1440., 20., 300., 400.), &displays),
            None
        );
    }

    #[test]
    fn distinguishes_permission_errors_from_other_capture_failures() {
        // Neither error needs a user-info dictionary, and both domains outlive the errors.
        let (denied, failed) = unsafe {
            (
                NSError::errorWithDomain_code_userInfo(
                    SCStreamErrorDomain,
                    SCStreamErrorCode::UserDeclined.0,
                    None,
                ),
                NSError::errorWithDomain_code_userInfo(
                    SCStreamErrorDomain,
                    SCStreamErrorCode::InternalError.0,
                    None,
                ),
            )
        };
        assert!(matches!(
            native_error(Some(&denied), "Capture failed", CaptureError::Screenshot),
            CaptureError::PermissionDenied,
        ));
        assert!(matches!(
            native_error(Some(&failed), "Capture failed", CaptureError::Screenshot),
            CaptureError::Screenshot(_),
        ));
    }

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
