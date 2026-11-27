use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};

use image::{ImageResult, RgbaImage};

pub fn encode(image: &RgbaImage) -> ImageResult<Vec<u8>> {
    let mut png = Vec::new();
    write(image, &mut png)?;
    Ok(png)
}

pub fn save(image: &RgbaImage, path: &Path) -> ImageResult<()> {
    write(image, File::create(path)?)
}

pub fn save_new(image: &RgbaImage, path: &Path) -> ImageResult<()> {
    let result = write(image, File::create_new(path)?);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn write(image: &RgbaImage, output: impl Write) -> ImageResult<()> {
    let mut encoder = png::Encoder::new(output, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::Adaptive);
    let mut writer = encoder.write_header().map_err(io::Error::from)?;
    writer
        .write_image_data(image.as_raw())
        .map_err(io::Error::from)?;
    Ok(writer.finish().map_err(io::Error::from)?)
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use image::{ImageEncoder, codecs::png::PngEncoder};

    use super::*;

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

    #[test]
    fn png_preserves_the_previous_fast_encoding_for_noisy_input() {
        let mut seed = 1_u32;
        let image = RgbaImage::from_fn(173, 199, |_, _| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgba(seed.to_le_bytes())
        });
        let mut expected = Vec::new();
        PngEncoder::new(&mut expected)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        assert_eq!(
            expected[43] & 0b110,
            0,
            "incompressible image uses stored blocks"
        );
        assert_eq!(encode(&image).unwrap(), expected);
    }

    #[test]
    fn png_propagates_data_and_end_chunk_write_errors() {
        struct FailChunk([u8; 4]);
        impl Write for FailChunk {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                if data == self.0 || data.get(4..8) == Some(&self.0) {
                    return Err(io::Error::other("chunk write failed"));
                }
                Ok(data.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        for chunk in [*b"IDAT", *b"IEND"] {
            let error = write(&RgbaImage::new(1, 1), FailChunk(chunk)).unwrap_err();
            assert_eq!(error.to_string(), "chunk write failed");
        }
    }

    #[test]
    fn png_save_new_protects_existing_files_and_removes_failed_outputs() {
        let directory = std::env::temp_dir().join(format!("rshot-save-new-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("image.png");
        fs::write(&path, b"original").unwrap();
        let error = save_new(&RgbaImage::new(1, 1), &path).unwrap_err();
        assert!(
            matches!(error, image::ImageError::IoError(error) if error.kind() == ErrorKind::AlreadyExists)
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");
        fs::remove_file(&path).unwrap();
        assert!(save_new(&RgbaImage::new(0, 0), &path).is_err());
        assert!(!path.exists());
        fs::remove_dir(directory).unwrap();
    }
}
