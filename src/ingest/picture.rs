//! Turns clipboard bitmaps into the PNG files that get stored.

use std::io::Cursor;

use anyhow::Result;
use image::{DynamicImage, ImageFormat, RgbaImage, codecs::bmp::BmpDecoder};

use crate::capture::RawImage;

const THUMBNAIL_SIZE: u32 = 128;

pub struct Picture {
    pub png: Vec<u8>,
    pub thumbnail: Vec<u8>,
    pub pixels: RgbaImage,
}

pub fn decode(raw: &RawImage) -> Result<RgbaImage> {
    match raw {
        RawImage::Png(bytes) => {
            Ok(image::load_from_memory_with_format(bytes, ImageFormat::Png)?.into_rgba8())
        }
        RawImage::Dib(bytes) => decode_dib(bytes),
    }
}

pub fn encode(raw: &RawImage) -> Result<Picture> {
    let pixels = decode(raw)?;
    let png = match raw {
        // Keep the original bytes: they are already what we store.
        RawImage::Png(bytes) => bytes.clone(),
        RawImage::Dib(_) => to_png(&pixels)?,
    };

    let thumbnail = if pixels.width().max(pixels.height()) > THUMBNAIL_SIZE {
        let image = DynamicImage::ImageRgba8(pixels.clone());
        to_png(&image.thumbnail(THUMBNAIL_SIZE, THUMBNAIL_SIZE).into_rgba8())?
    } else {
        png.clone()
    };

    Ok(Picture {
        png,
        thumbnail,
        pixels,
    })
}

fn decode_dib(bytes: &[u8]) -> Result<RgbaImage> {
    let decoder = BmpDecoder::new_without_file_header(Cursor::new(bytes))?;
    let mut pixels = DynamicImage::from_decoder(decoder)?.into_rgba8();
    // Many apps leave the alpha channel zeroed, which would hide the image.
    if pixels.pixels().all(|pixel| pixel[3] == 0) {
        pixels.pixels_mut().for_each(|pixel| pixel[3] = 255);
    }
    Ok(pixels)
}

fn to_png(pixels: &RgbaImage) -> Result<Vec<u8>> {
    let mut png = Cursor::new(Vec::new());
    pixels.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A bottom-up 32-bit `BITMAPINFOHEADER` bitmap filled with one color.
    pub fn dib(width: i32, height: i32, bgra: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(40u32.to_le_bytes());
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend(1u16.to_le_bytes()); // planes
        bytes.extend(32u16.to_le_bytes()); // bits per pixel
        bytes.extend([0; 24]); // BI_RGB and unused sizes
        for _ in 0..width * height {
            bytes.extend(bgra);
        }
        bytes
    }

    #[test]
    fn decodes_an_opaque_dib() {
        let picture = encode(&RawImage::Dib(dib(3, 2, [10, 20, 30, 0]))).unwrap();
        assert_eq!(picture.pixels.dimensions(), (3, 2));
        assert_eq!(picture.pixels.get_pixel(0, 0).0, [30, 20, 10, 255]);
        let stored = image::load_from_memory(&picture.png).unwrap();
        assert_eq!(stored.into_rgba8(), picture.pixels);
    }

    #[test]
    fn shrinks_large_thumbnails_only() {
        let large = encode(&RawImage::Dib(dib(400, 200, [1, 2, 3, 255]))).unwrap();
        let thumbnail = image::load_from_memory(&large.thumbnail).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (128, 64));

        let small = encode(&RawImage::Dib(dib(4, 4, [1, 2, 3, 255]))).unwrap();
        assert_eq!(small.thumbnail, small.png);
    }

    #[test]
    fn rejects_garbage() {
        assert!(encode(&RawImage::Dib(vec![1, 2, 3])).is_err());
        assert!(encode(&RawImage::Png(vec![1, 2, 3])).is_err());
    }
}
