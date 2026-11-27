use std::ptr;

use objc2_core_graphics::{
    CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode, CGError, CGGetActiveDisplayList,
    CGMainDisplayID,
};

use crate::{
    capture::{CaptureError, DisplayInfo},
    geometry::{Rect, Size},
};

pub(super) fn displays() -> Result<Vec<DisplayInfo>, CaptureError> {
    let mut count = 0;
    // A null buffer asks CoreGraphics for the number of active displays.
    check_display_list(unsafe { CGGetActiveDisplayList(0, ptr::null_mut(), &mut count) })?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![0; count as usize];
    // The buffer has room for the given maximum number of IDs.
    check_display_list(unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) })?;
    ids.truncate(count as usize);
    let primary = CGMainDisplayID();
    ids.into_iter()
        .map(|id| {
            let frame = CGDisplayBounds(id);
            let bounds = Rect::new(
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            );
            let mode = CGDisplayCopyDisplayMode(id).ok_or(CaptureError::DisplayNotFound)?;
            let pixels = Size {
                width: CGDisplayMode::pixel_width(Some(&mode)).try_into()?,
                height: CGDisplayMode::pixel_height(Some(&mode)).try_into()?,
            };
            display_info(id, bounds, pixels, id == primary)
        })
        .collect()
}

fn check_display_list(result: CGError) -> Result<(), CaptureError> {
    if result == CGError::Success {
        Ok(())
    } else {
        Err(CaptureError::DisplayEnumeration(format!(
            "Could not enumerate active displays ({result:?})"
        )))
    }
}

fn display_info(
    id: u32,
    bounds: Rect<f64>,
    mut pixel_size: Size<u32>,
    is_primary: bool,
) -> Result<DisplayInfo, CaptureError> {
    if bounds.size.width <= 0.
        || bounds.size.height <= 0.
        || pixel_size.width == 0
        || pixel_size.height == 0
    {
        return Err(CaptureError::DisplayNotFound);
    }
    // A rotated display can have bounds in a different orientation from its mode.
    if (bounds.size.width > bounds.size.height) != (pixel_size.width > pixel_size.height) {
        std::mem::swap(&mut pixel_size.width, &mut pixel_size.height);
    }
    Ok(DisplayInfo {
        id,
        bounds,
        scale_factor: f64::from(pixel_size.width) / bounds.size.width,
        pixel_size,
        is_primary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_retina_and_rotated_displays_in_points() {
        let retina = display_info(
            1,
            Rect::new(0., 0., 1800., 1169.),
            Size {
                width: 3600,
                height: 2338,
            },
            true,
        )
        .unwrap();
        let portrait = display_info(
            2,
            Rect::new(-1080., -24., 1080., 1920.),
            Size {
                width: 1920,
                height: 1080,
            },
            false,
        )
        .unwrap();
        assert_eq!(retina.scale_factor, 2.);
        assert!(retina.is_primary);
        assert_eq!(portrait.scale_factor, 1.);
        assert_eq!(portrait.bounds, Rect::new(-1080., -24., 1080., 1920.));
        assert_eq!(
            portrait.pixel_size,
            Size {
                width: 1080,
                height: 1920
            }
        );
        assert!(!portrait.is_primary);
    }
}
