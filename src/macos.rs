use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGDisplayBounds, CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo,
    CGWindowListOption, kCGWindowBounds, kCGWindowLayer, kCGWindowOwnerPID,
};

pub fn windows_on_display(display_id: u32) -> Vec<CGRect> {
    let Some(windows) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    ) else {
        return Vec::new();
    };
    // CoreGraphics documents this array as window dictionaries with CF values, in front-to-back order.
    type WindowInfo = CFDictionary<CFString, CFType>;
    let windows = unsafe { CFRetained::cast_unchecked::<CFArray<WindowInfo>>(windows) };
    let display = CGDisplayBounds(display_id);
    windows
        .iter()
        .filter_map(|window| {
            // These keys are immutable framework constants.
            let (layer_key, owner_key, bounds_key) =
                unsafe { (kCGWindowLayer, kCGWindowOwnerPID, kCGWindowBounds) };
            let number = |key| window.get(key)?.downcast_ref::<CFNumber>()?.as_i32();
            if number(layer_key)? != 0 || number(owner_key)? == std::process::id() as i32 {
                return None;
            }
            let value = window.get(bounds_key)?;
            let bounds = value.downcast_ref::<CFDictionary>()?;
            let mut rect = CGRect::default();
            // CoreGraphics initializes the writable rectangle with the dictionary's window bounds.
            if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut rect) } {
                return None;
            }
            local_rect(rect, display)
        })
        .collect()
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
    use super::{CGPoint, CGRect, CGSize, local_rect};

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
