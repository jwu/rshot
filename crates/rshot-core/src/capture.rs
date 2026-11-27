use image::RgbaImage;
use thiserror::Error;

use crate::geometry::{Point, Rect, Size};

#[cfg(target_os = "macos")]
pub use crate::platform::macos::capture::MacOsCapture;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("区域截图需要 macOS 14 或更新版本")]
    UnsupportedMacOs,
    #[error("请在「系统设置 → 隐私与安全性 → 屏幕与系统音频录制」中授权，再重新打开 Rshot")]
    PermissionDenied,
    #[error("{0}")]
    DisplayEnumeration(String),
    #[error("找不到要截图的显示器")]
    DisplayNotFound,
    #[error("Window {0} was not found")]
    WindowNotFound(u32),
    #[error("{0}")]
    Screenshot(String),
    #[error("屏幕截图等待超时")]
    Timeout(#[source] std::sync::mpsc::RecvTimeoutError),
    #[error("截图过宽")]
    RowTooWide,
    #[error("截图尺寸过大")]
    ImageTooLarge,
    #[error("无法建立 sRGB 色彩空间")]
    ColorSpace,
    #[error("无法分配截图缓冲区")]
    BitmapContext,
    #[error(transparent)]
    DimensionOverflow(#[from] std::num::TryFromIntError),
    #[error("截图缓冲区尺寸不匹配")]
    BufferSize,
    #[error("显示器尺寸发生变化，请重新选区")]
    DisplayResized,
    #[error("截图选区超出范围")]
    RegionOutOfBounds,
}

/// An active display in the global top-left coordinate space, measured in points.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    pub id: u32,
    pub bounds: Rect<f64>,
    pub scale_factor: f64,
    pub pixel_size: Size<u32>,
    pub is_primary: bool,
}

/// A WindowServer window. Bounds are in global points with a top-left origin.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub window_id: u32,
    pub owner_name: String,
    pub owner_pid: i32,
    /// Absent for untitled windows and when WindowServer withholds the title.
    pub title: Option<String>,
    pub bounds: Rect<f64>,
    pub layer: i32,
    pub is_on_screen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    Display,
    Region,
    Window { window_id: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureMetadata {
    pub source: CaptureSource,
    /// The captured display, or the display with the largest visible portion of the window.
    /// A window with no intersecting display has no display ID.
    pub display_id: Option<u32>,
    pub scale_factor: f64,
    /// Captured bounds in global points, with a top-left origin.
    pub bounds: Rect<f64>,
    pub pixel_size: Size<u32>,
}

#[derive(Debug)]
pub struct CapturedImage {
    pub image: RgbaImage,
    pub metadata: CaptureMetadata,
}

/// Display-local bounds in logical coordinates, with windows ordered front to back.
#[derive(Default)]
pub struct CaptureTargets {
    pub menu_bar: Option<Rect<f64>>,
    pub windows: Vec<Rect<f64>>,
}

pub trait CaptureBackend {
    /// Checks platform support and requests capture permission if needed.
    fn ready(&self) -> Result<(), CaptureError>;

    /// Captures a display at the requested pixel scale, blocking until a frame is available.
    fn screen(&self, display_id: u32, scale: f32) -> Result<RgbaImage, CaptureError>;

    fn targets(&self, display_id: u32) -> CaptureTargets;

    fn region(
        &self,
        display_id: u32,
        scale: f32,
        expected_dimensions: (u32, u32),
        region: Rect<u32>,
    ) -> Result<RgbaImage, CaptureError> {
        let frame = self.screen(display_id, scale)?;
        if frame.dimensions() != expected_dimensions {
            return Err(CaptureError::DisplayResized);
        }
        crop(&frame, region)
    }
}

/// Crops a nonempty rectangle in image pixels with a top-left origin.
pub fn crop(frame: &RgbaImage, region: Rect<u32>) -> Result<RgbaImage, CaptureError> {
    let Point { x, y } = region.origin;
    let Size { width, height } = region.size;
    if width == 0
        || height == 0
        || x.saturating_add(width) > frame.width()
        || y.saturating_add(height) > frame.height()
    {
        return Err(CaptureError::RegionOutOfBounds);
    }
    Ok(image::imageops::crop_imm(frame, x, y, width, height).to_image())
}
