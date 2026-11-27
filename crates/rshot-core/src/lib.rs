pub mod annotation;
pub mod capture;
#[cfg(target_os = "macos")]
pub mod elements;
pub mod geometry;
pub mod locate;
pub mod ocr;
pub mod png;
pub mod stitch;

#[cfg(target_os = "macos")]
mod platform;
