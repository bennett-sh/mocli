//! Text recognition built into Windows, in the languages the user has set up.

use anyhow::{Context as _, Result};
use image::{RgbaImage, imageops};
use windows::{
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
};

/// Prepares the calling thread, which must not be a UI thread.
pub fn init_thread() {
    // SAFETY: fails harmlessly if COM is already set up on this thread.
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if result.is_err() {
        tracing::debug!(?result, "COM was already initialized differently");
    }
}

pub struct Recognizer(OcrEngine);

impl Recognizer {
    /// Fails if no installed language supports recognition.
    pub fn new() -> Result<Self> {
        OcrEngine::TryCreateFromUserProfileLanguages()
            .map(Self)
            .context("text recognition is not available")
    }

    /// The text in `picture`, one line per line, possibly empty.
    pub fn read(&self, picture: &RgbaImage) -> Result<String> {
        let limit = OcrEngine::MaxImageDimension()?;
        let longest = picture.width().max(picture.height());
        let resized;
        let picture = if longest > limit {
            let scale =
                |edge: u32| (u64::from(edge) * u64::from(limit) / u64::from(longest)) as u32;
            resized = imageops::resize(
                picture,
                scale(picture.width()).max(1),
                scale(picture.height()).max(1),
                imageops::FilterType::Triangle,
            );
            &resized
        } else {
            picture
        };

        let writer = DataWriter::new()?;
        writer.WriteBytes(&on_white(picture))?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &writer.DetachBuffer()?,
            BitmapPixelFormat::Bgra8,
            picture.width() as i32,
            picture.height() as i32,
        )?;
        let lines = self.0.RecognizeAsync(&bitmap)?.join()?.Lines()?;
        let text = lines
            .into_iter()
            .map(|line| Ok(line.Text()?.to_string()))
            .collect::<Result<Vec<_>>>()?;
        Ok(text.join("\n"))
    }
}

/// Opaque BGRA pixels, with transparency shown as white.
fn on_white(picture: &RgbaImage) -> Vec<u8> {
    picture
        .pixels()
        .flat_map(|pixel| {
            let [red, green, blue, alpha] = pixel.0.map(u32::from);
            let flatten = |channel| ((channel * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            [flatten(blue), flatten(green), flatten(red), 255]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    /// "HELLO" in a 5×7 pixel font, scaled up.
    fn picture() -> RgbaImage {
        const GLYPHS: [[u8; 7]; 5] = [
            [17, 17, 17, 31, 17, 17, 17],
            [31, 16, 16, 30, 16, 16, 31],
            [16, 16, 16, 16, 16, 16, 31],
            [16, 16, 16, 16, 16, 16, 31],
            [14, 17, 17, 17, 17, 17, 14],
        ];
        const SCALE: u32 = 10;
        let white = Rgba([255, 255, 255, 255]);
        let mut image = RgbaImage::from_pixel(34 * SCALE, 15 * SCALE, white);
        for (index, glyph) in GLYPHS.iter().enumerate() {
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..5 {
                    if bits & (16 >> column) != 0 {
                        let left = (2 + index as u32 * 6 + column) * SCALE;
                        let top = (4 + row as u32) * SCALE;
                        for x in left..left + SCALE {
                            for y in top..top + SCALE {
                                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                            }
                        }
                    }
                }
            }
        }
        image
    }

    #[test]
    fn reads_large_text() {
        init_thread();
        // Server editions may come without any recognition language.
        let Ok(recognizer) = Recognizer::new() else {
            eprintln!("skipped: no recognition language installed");
            return;
        };
        let text = recognizer.read(&picture()).unwrap();
        assert!(text.to_uppercase().contains("HELL"), "read {text:?}");
    }

    #[test]
    fn flattens_transparency() {
        let image = RgbaImage::from_pixel(1, 1, Rgba([10, 20, 30, 0]));
        assert_eq!(on_white(&image), [255, 255, 255, 255]);
    }
}
