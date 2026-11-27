use std::path::Path;

use image::{ImageEncoder, ImageFormat, ImageResult, RgbaImage, codecs::png::PngEncoder};

pub fn encode(image: &RgbaImage) -> ImageResult<Vec<u8>> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png)
}

pub fn save(image: &RgbaImage, path: &Path) -> ImageResult<()> {
    image.save_with_format(path, ImageFormat::Png)
}

#[cfg(test)]
mod tests {
    use super::{RgbaImage, encode};

    #[test]
    fn png_preserves_color_and_alpha() {
        let image = RgbaImage::from_raw(1, 1, vec![250, 22, 100, 127]).unwrap();
        assert_eq!(
            image::load_from_memory(&encode(&image).unwrap())
                .unwrap()
                .to_rgba8(),
            image
        );
    }
}
