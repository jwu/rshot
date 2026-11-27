use std::collections::HashSet;

use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect,
};
use objc2_core_graphics::{
    CGDisplayBounds, CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo,
    CGWindowListOption, kCGMainMenuWindowLevel, kCGWindowBounds, kCGWindowIsOnscreen,
    kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
};

use crate::{
    capture::{CaptureTargets, WindowInfo},
    geometry::Rect,
};

type WindowDictionary = CFDictionary<CFString, CFType>;

fn window_list(
    options: CGWindowListOption,
    relative_to: u32,
) -> Option<CFRetained<CFArray<WindowDictionary>>> {
    let windows = CGWindowListCopyWindowInfo(options, relative_to)?;
    // CoreGraphics documents the array's elements as dictionaries with CF values.
    Some(unsafe { CFRetained::cast_unchecked(windows) })
}

pub(super) fn exists(window_id: u32) -> bool {
    window_list(CGWindowListOption::OptionIncludingWindow, window_id)
        .is_some_and(|windows| !windows.is_empty())
}

pub(super) fn windows(include_all: bool) -> Vec<WindowInfo> {
    let mut windows: Vec<_> = window_list(CGWindowListOption::OptionOnScreenOnly, 0)
        .iter()
        .flat_map(|list| list.iter())
        .filter_map(|window| window_info(&window))
        .collect();
    let seen: HashSet<_> = windows.iter().map(|window| window.window_id).collect();
    windows.extend(
        window_list(CGWindowListOption::OptionAll, 0)
            .iter()
            .flat_map(|list| list.iter())
            .filter_map(|window| window_info(&window))
            .filter(|window| !window.is_on_screen && !seen.contains(&window.window_id)),
    );
    if !include_all {
        windows.retain(is_standard_window);
    }
    windows
}

fn is_standard_window(window: &WindowInfo) -> bool {
    const MIN_WINDOW_POINTS: f64 = 32.;
    window.layer == 0
        && window.bounds.size.width >= MIN_WINDOW_POINTS
        && window.bounds.size.height >= MIN_WINDOW_POINTS
}

fn window_info(window: &WindowDictionary) -> Option<WindowInfo> {
    // These keys are immutable framework constants.
    let (id, owner_name, owner_pid, title, layer, on_screen) = unsafe {
        (
            kCGWindowNumber,
            kCGWindowOwnerName,
            kCGWindowOwnerPID,
            kCGWindowName,
            kCGWindowLayer,
            kCGWindowIsOnscreen,
        )
    };
    let number = |key| window.get(key)?.downcast_ref::<CFNumber>()?.as_i64();
    let text = |key| {
        let value = window.get(key)?;
        let value = value.downcast_ref::<CFString>()?.to_string();
        (!value.is_empty()).then_some(value)
    };
    let bounds = window_bounds(window)?;
    Some(WindowInfo {
        window_id: number(id)?.try_into().ok()?,
        owner_name: text(owner_name).unwrap_or_default(),
        owner_pid: number(owner_pid)?.try_into().ok()?,
        title: text(title),
        bounds: Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bounds.size.height,
        ),
        layer: number(layer)?.try_into().ok()?,
        is_on_screen: window.get(on_screen).is_some_and(|value| {
            value
                .downcast_ref::<CFBoolean>()
                .is_some_and(CFBoolean::value)
        }),
    })
}

pub(super) fn capture_targets(display_id: u32) -> CaptureTargets {
    let Some(windows) = window_list(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    ) else {
        return CaptureTargets::default();
    };
    let display = CGDisplayBounds(display_id);
    let mut targets = CaptureTargets::default();
    for (layer, rect) in windows.iter().filter_map(|window| {
        // These keys are immutable framework constants.
        let (layer_key, owner_key) = unsafe { (kCGWindowLayer, kCGWindowOwnerPID) };
        let number = |key| window.get(key)?.downcast_ref::<CFNumber>()?.as_i32();
        let layer = number(layer_key)?;
        if (layer != 0 && layer != kCGMainMenuWindowLevel)
            || number(owner_key)? == std::process::id() as i32
        {
            return None;
        }
        local_rect(window_bounds(&window)?, display).map(|rect| (layer, rect))
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

fn window_bounds(window: &WindowDictionary) -> Option<CGRect> {
    let value = window.get(unsafe { kCGWindowBounds })?;
    let bounds = value.downcast_ref::<CFDictionary>()?;
    let mut rect = CGRect::default();
    // CoreGraphics initializes the writable rectangle with the dictionary's window bounds.
    unsafe { CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut rect) }.then_some(rect)
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

    use super::{CGRect, Rect, WindowInfo, is_standard_window, local_rect};

    #[test]
    fn filters_nonstandard_layers_and_tiny_windows() {
        let mut window = WindowInfo {
            window_id: 42,
            owner_name: "Example".into(),
            owner_pid: 2,
            title: None,
            bounds: Rect::new(-300., 20., 32., 32.),
            layer: 0,
            is_on_screen: false,
        };
        assert!(is_standard_window(&window));
        window.layer = 1;
        assert!(!is_standard_window(&window));
        window.layer = 0;
        window.bounds.size.width = 31.;
        assert!(!is_standard_window(&window));
        window.bounds.size.width = 100.;
        window.bounds.size.height = 0.;
        assert!(!is_standard_window(&window));
    }

    #[test]
    fn clips_a_window_to_a_display_with_negative_origin() {
        let display = CGRect::new(CGPoint::new(-800., -200.), CGSize::new(800., 600.));
        let window = CGRect::new(CGPoint::new(-900., -150.), CGSize::new(300., 250.));
        let clipped = local_rect(window, display).unwrap();
        assert_eq!(clipped, Rect::new(0., 50., 200., 250.));
        assert!(local_rect(CGRect::new(CGPoint::new(0., 0.), window.size), display).is_none());
    }
}
