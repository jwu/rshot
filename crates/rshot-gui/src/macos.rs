use anyhow::{Context, Result, anyhow, ensure};
use gpui::Window;
use objc2_app_kit::{NSView, NSWindowStyleMask};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_core_graphics::{CGDisplayBounds, CGMainDisplayID};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CGSize;

    use super::{CGPoint, CGRect, appkit_frame};

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
}
