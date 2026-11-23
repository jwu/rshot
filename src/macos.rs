use anyhow::{Context, Result, anyhow, ensure};
use gpui::Window;
use objc2_app_kit::{NSView, NSWindowStyleMask};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGDisplayBounds, CGMainDisplayID, CGRectMakeWithDictionaryRepresentation,
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGMainMenuWindowLevel, kCGWindowBounds,
    kCGWindowLayer, kCGWindowOwnerPID,
};

pub fn fit_capture_overlay(window: &Window, display_id: u32) -> Result<()> {
    let display = CGDisplayBounds(display_id);
    ensure!(
        display.size.width > 0. && display.size.height > 0.,
        "截图显示器已断开"
    );
    let main = CGDisplayBounds(CGMainDisplayID());
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("无法取得截图窗口句柄 ({error:?})"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(anyhow!("无法找到截图窗口"));
    };
    // The raw view belongs to the live GPUI window on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let native = view.window().context("截图窗口已关闭")?;
    // A titled window is constrained below the menu bar by AppKit.
    let style =
        native.styleMask() - NSWindowStyleMask::Titled - NSWindowStyleMask::FullSizeContentView;
    native.setStyleMask(style);
    native.setHasShadow(false);
    native.setFrame_display(appkit_frame(display, main.size.height), true);
    Ok(())
}

fn appkit_frame(display: CGRect, main_height: f64) -> CGRect {
    CGRect::new(
        CGPoint::new(
            display.origin.x,
            main_height - display.origin.y - display.size.height,
        ),
        display.size,
    )
}

#[derive(Default)]
pub struct CaptureTargets {
    pub menu_bar: Option<CGRect>,
    pub windows: Vec<CGRect>,
}

pub fn capture_targets(display_id: u32) -> CaptureTargets {
    let Some(windows) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    ) else {
        return CaptureTargets::default();
    };
    // CoreGraphics documents this array as window dictionaries with CF values, in front-to-back order.
    type WindowInfo = CFDictionary<CFString, CFType>;
    let windows = unsafe { CFRetained::cast_unchecked::<CFArray<WindowInfo>>(windows) };
    let display = CGDisplayBounds(display_id);
    let mut targets = CaptureTargets::default();
    for (layer, rect) in windows.iter().filter_map(|window| {
        // These keys are immutable framework constants.
        let (layer_key, owner_key, bounds_key) =
            unsafe { (kCGWindowLayer, kCGWindowOwnerPID, kCGWindowBounds) };
        let number = |key| window.get(key)?.downcast_ref::<CFNumber>()?.as_i32();
        let layer = number(layer_key)?;
        if (layer != 0 && layer != kCGMainMenuWindowLevel)
            || number(owner_key)? == std::process::id() as i32
        {
            return None;
        }
        let value = window.get(bounds_key)?;
        let bounds = value.downcast_ref::<CFDictionary>()?;
        let mut rect = CGRect::default();
        // CoreGraphics initializes the writable rectangle with the dictionary's window bounds.
        if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut rect) } {
            return None;
        }
        local_rect(rect, display).map(|rect| (layer, rect))
    }) {
        if layer == 0 {
            targets.windows.push(rect);
        } else if rect.origin.y == 0. && rect.size.width == display.size.width {
            // The visible server menu bar spans the screen, including on Retina displays.
            targets.menu_bar.get_or_insert(rect);
        }
    }
    targets
}

fn local_rect(window: CGRect, display: CGRect) -> Option<CGRect> {
    let x0 = (window.origin.x - display.origin.x).max(0.);
    let y0 = (window.origin.y - display.origin.y).max(0.);
    let x1 = (window.origin.x - display.origin.x + window.size.width).min(display.size.width);
    let y1 = (window.origin.y - display.origin.y + window.size.height).min(display.size.height);
    (x1 > x0 && y1 > y0).then(|| CGRect::new(CGPoint::new(x0, y0), CGSize::new(x1 - x0, y1 - y0)))
}

#[cfg(test)]
mod tests {
    use super::{CGPoint, CGRect, CGSize, appkit_frame, local_rect};

    #[test]
    fn overlay_frame_uses_full_display_coordinates() {
        let main = CGRect::new(CGPoint::new(0., 0.), CGSize::new(1920., 1080.));
        let secondary = CGRect::new(CGPoint::new(-1280., -180.), CGSize::new(1280., 800.));
        assert_eq!(appkit_frame(main, 1080.), main);
        assert_eq!(
            appkit_frame(secondary, 1080.).origin,
            CGPoint::new(-1280., 460.)
        );
    }

    #[test]
    fn clips_a_window_to_a_display_with_negative_origin() {
        let display = CGRect::new(CGPoint::new(-800., -200.), CGSize::new(800., 600.));
        let window = CGRect::new(CGPoint::new(-900., -150.), CGSize::new(300., 250.));
        let clipped = local_rect(window, display).unwrap();
        assert_eq!(clipped.origin, CGPoint::new(0., 50.));
        assert_eq!(clipped.size, CGSize::new(200., 250.));
        assert!(local_rect(CGRect::new(CGPoint::new(0., 0.), window.size), display).is_none());
    }
}
