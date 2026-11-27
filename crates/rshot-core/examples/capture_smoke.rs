#[cfg(target_os = "macos")]
use rshot_core::{
    capture::{CaptureError, CaptureSource, CapturedImage, MacOsCapture},
    geometry::Rect,
    png,
};
#[cfg(target_os = "macos")]
use std::{error::Error, fs::DirBuilder, os::unix::fs::DirBuilderExt, path::Path};

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn Error>> {
    let capture = MacOsCapture;
    let displays = capture.displays()?;
    let primary = displays.iter().find(|display| display.is_primary);
    let windows = capture.windows();
    let all_windows = capture.all_windows();
    println!(
        "{} active displays, {} standard windows, {} total windows, {} standard windows with titles",
        displays.len(),
        windows.len(),
        all_windows.len(),
        windows
            .iter()
            .filter(|window| window.title.is_some())
            .count(),
    );
    let output = Path::new("/tmp").join(format!("rshot-capture-{}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&output)?;

    let display = capture.capture_display(None);
    let region = capture.capture_region(
        primary.map(|display| display.id),
        Rect::new(20., 30., 100., 80.),
    );
    let target = windows.iter().find(|window| window.is_on_screen);
    let window = target.map(|window| capture.capture_window(window.window_id));
    for (name, result) in [
        ("display", Some(&display)),
        ("region", Some(&region)),
        ("window", window.as_ref()),
    ] {
        match result {
            Some(Ok(result)) => save_checked(result, &output.join(format!("{name}.png")))?,
            Some(Err(error)) => eprintln!("{name}: {error:?} ({error})"),
            None => eprintln!("{name}: no on-screen standard window"),
        }
    }

    let display = display?;
    assert!(matches!(
        capture.capture_display(Some(u32::MAX)),
        Err(CaptureError::DisplayNotFound)
    ));
    assert!(matches!(
        capture.capture_window(u32::MAX),
        Err(CaptureError::WindowNotFound(u32::MAX))
    ));
    assert!(matches!(
        capture.capture_region(None, Rect::new(0., 0., 0., 1.)),
        Err(CaptureError::RegionOutOfBounds)
    ));
    println!("Verified missing display, empty region and missing window errors");
    let region = region?;
    let window = window.ok_or("No on-screen standard window")??;
    let primary = primary.ok_or("No primary display")?;
    let target = target.ok_or("No on-screen standard window")?;
    assert_eq!(display.metadata.source, CaptureSource::Display);
    assert_eq!(display.metadata.display_id, Some(primary.id));
    assert_eq!(display.metadata.bounds, primary.bounds);
    assert_eq!(display.metadata.pixel_size, primary.pixel_size);

    assert_eq!(region.metadata.source, CaptureSource::Region);
    assert_eq!(region.metadata.display_id, Some(primary.id));
    let x_scale = f64::from(primary.pixel_size.width) / primary.bounds.size.width;
    let y_scale = f64::from(primary.pixel_size.height) / primary.bounds.size.height;
    assert_eq!(
        region.metadata.pixel_size.width,
        ((120. * x_scale).ceil() - (20. * x_scale).floor()) as u32
    );
    assert_eq!(
        region.metadata.pixel_size.height,
        ((110. * y_scale).ceil() - (30. * y_scale).floor()) as u32
    );

    assert_eq!(
        window.metadata.source,
        CaptureSource::Window {
            window_id: target.window_id
        }
    );
    assert_eq!(window.metadata.bounds, target.bounds);
    assert_eq!(
        window.metadata.pixel_size.width,
        (target.bounds.size.width * window.metadata.scale_factor).round() as u32
    );
    assert_eq!(
        window.metadata.pixel_size.height,
        (target.bounds.size.height * window.metadata.scale_factor).round() as u32
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn save_checked(capture: &CapturedImage, path: &Path) -> Result<(), Box<dyn Error>> {
    let expected = (
        capture.metadata.pixel_size.width,
        capture.metadata.pixel_size.height,
    );
    png::save(&capture.image, path)?;
    let actual = image::image_dimensions(path)?;
    assert_eq!(actual, expected, "{}", path.display());
    println!(
        "{}: {}x{} pixels, scale {}, bounds {:?}, display {:?}",
        path.display(),
        actual.0,
        actual.1,
        capture.metadata.scale_factor,
        capture.metadata.bounds,
        capture.metadata.display_id,
    );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This example requires macOS 14 or newer");
}
