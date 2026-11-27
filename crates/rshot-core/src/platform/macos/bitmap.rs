use std::{ffi::c_void, ptr::NonNull};

use objc2_core_graphics::{CGColorSpace, CGContext};

unsafe extern "C-unwind" {
    pub(super) fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        color_space: &CGColorSpace,
        bitmap_info: u32,
    ) -> Option<NonNull<CGContext>>;
}
