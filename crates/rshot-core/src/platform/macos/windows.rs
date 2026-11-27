use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect,
};
use objc2_core_graphics::{
    CGDisplayBounds, CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo,
    CGWindowListOption, kCGMainMenuWindowLevel, kCGWindowBounds, kCGWindowLayer, kCGWindowOwnerPID,
};

use crate::{capture::CaptureTargets, geometry::Rect};

pub(super) fn capture_targets(display_id: u32) -> CaptureTargets {
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

fn local_rect(window: CGRect, display: CGRect) -> Option<Rect<f64>> {
    let x0 = (window.origin.x - display.origin.x).max(0.);
    let y0 = (window.origin.y - display.origin.y).max(0.);
    let x1 = (window.origin.x - display.origin.x + window.size.width).min(display.size.width);
    let y1 = (window.origin.y - display.origin.y + window.size.height).min(display.size.height);
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGSize};

    use super::{CGRect, Rect, local_rect};

    #[test]
    fn clips_a_window_to_a_display_with_negative_origin() {
        let display = CGRect::new(CGPoint::new(-800., -200.), CGSize::new(800., 600.));
        let window = CGRect::new(CGPoint::new(-900., -150.), CGSize::new(300., 250.));
        let clipped = local_rect(window, display).unwrap();
        assert_eq!(clipped, Rect::new(0., 50., 200., 250.));
        assert!(local_rect(CGRect::new(CGPoint::new(0., 0.), window.size), display).is_none());
    }
}
