use std::{ffi::c_void, ptr::NonNull, sync::mpsc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
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

pub fn ready() -> Result<()> {
    ensure!(
        NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 14,
        "区域截图需要 macOS 14 或更新版本"
    );
    if !CGPreflightScreenCaptureAccess() && !CGRequestScreenCaptureAccess() {
        bail!("请在「系统设置 → 隐私与安全性 → 屏幕与系统音频录制」中授权，再重新打开 Rshot");
    }
    Ok(())
}

pub fn screen(display_id: u32, scale: f32) -> Result<RgbaImage> {
    let (send, receive) = mpsc::channel();
    autoreleasepool(|_| {
        let completion = RcBlock::new(
            move |content: *mut SCShareableContent, error: *mut NSError| {
                // The framework keeps both arguments alive for the duration of the callback.
                let result = unsafe { content.as_ref() }
                    .ok_or_else(|| native_error(error, "无法列出可录制的显示器"))
                    .and_then(|content| start_capture(content, display_id, scale, send.clone()));
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
        .context("屏幕截图等待超时")?
}

pub fn region(
    display_id: u32,
    scale: f32,
    expected_dimensions: (u32, u32),
    [x, y, width, height]: [u32; 4],
) -> Result<RgbaImage> {
    let frame = screen(display_id, scale)?;
    ensure!(
        frame.dimensions() == expected_dimensions,
        "显示器尺寸发生变化，请重新选区"
    );
    ensure!(
        width > 0
            && height > 0
            && x.saturating_add(width) <= frame.width()
            && y.saturating_add(height) <= frame.height(),
        "截图选区超出范围"
    );
    Ok(image::imageops::crop_imm(&frame, x, y, width, height).to_image())
}

fn start_capture(
    content: &SCShareableContent,
    display_id: u32,
    scale: f32,
    send: mpsc::Sender<Result<RgbaImage>>,
) -> Result<()> {
    // ScreenCaptureKit objects below come from the current shareable content snapshot.
    unsafe {
        let display = content
            .displays()
            .iter()
            .find(|display| display.displayID() == display_id)
            .context("找不到要截图的显示器")?;
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
                .ok_or_else(|| native_error(error, "截取屏幕失败"))
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

fn native_error(error: *mut NSError, fallback: &str) -> anyhow::Error {
    // Only called with a callback argument while the Objective-C object is still alive.
    match unsafe { error.as_ref() } {
        Some(error) => anyhow::anyhow!("{fallback} ({})", error.localizedDescription()),
        None => anyhow::anyhow!("{fallback}"),
    }
}

unsafe extern "C-unwind" {
    pub(crate) fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        color_space: &CGColorSpace,
        bitmap_info: u32,
    ) -> Option<NonNull<CGContext>>;
}

fn to_rgba(image: &CGImage) -> Result<RgbaImage> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let stride = width.checked_mul(4).context("截图过宽")?;
    let len = stride.checked_mul(height).context("截图尺寸过大")?;
    let mut pixels = vec![0; len];
    let color_space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .context("无法建立 sRGB 色彩空间")?;
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
        .context("无法分配截图缓冲区")?;
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
        .context("截图缓冲区尺寸不匹配")
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
