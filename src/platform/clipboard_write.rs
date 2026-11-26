//! Puts a history entry back on the clipboard.

use std::{os::windows::ffi::OsStrExt as _, path::PathBuf, ptr};

use anyhow::{Context as _, Result};
use image::{ImageFormat, RgbaImage};
use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HWND},
        System::{
            DataExchange::{EmptyClipboard, RegisterClipboardFormatW, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT},
        },
    },
    core::{PCWSTR, w},
};

use super::clipboard::{Clipboard, SELF_MARKER};

pub enum Content {
    Text {
        text: String,
        html: Option<Vec<u8>>,
        rtf: Option<Vec<u8>>,
    },
    Image {
        png: Vec<u8>,
    },
    Files(Vec<PathBuf>),
}

/// Replaces the clipboard. `owner` must be a window of the calling thread.
pub fn write(owner: HWND, content: &Content) -> Result<()> {
    // Prepare everything first, to hold the clipboard open as briefly as possible.
    let mut blocks = blocks(content)?;
    // Tells our watcher that this change is not a new copy.
    blocks.push((register(SELF_MARKER), vec![1]));

    let _open = Clipboard::open(owner)?;
    // SAFETY: the clipboard is open.
    unsafe { EmptyClipboard() }.context("could not empty the clipboard")?;
    for (format, bytes) in blocks {
        set(format, &bytes)?;
    }
    Ok(())
}

fn register(name: PCWSTR) -> u32 {
    // SAFETY: callers pass static, null-terminated strings.
    unsafe { RegisterClipboardFormatW(name) }
}

fn blocks(content: &Content) -> Result<Vec<(u32, Vec<u8>)>> {
    Ok(match content {
        Content::Text { text, html, rtf } => {
            let units = text.encode_utf16().chain([0]);
            let mut blocks = vec![(CF_UNICODETEXT.0.into(), to_bytes(units))];
            if let Some(html) = html {
                blocks.push((register(w!("HTML Format")), html.clone()));
            }
            if let Some(rtf) = rtf {
                blocks.push((register(w!("Rich Text Format")), rtf.clone()));
            }
            blocks
        }
        Content::Image { png } => {
            let pixels = image::load_from_memory_with_format(png, ImageFormat::Png)
                .context("the stored image is damaged")?
                .into_rgba8();
            // Apps that understand transparency take the PNG.
            vec![
                (register(w!("PNG")), png.clone()),
                (CF_DIB.0.into(), dib(&pixels)),
            ]
        }
        Content::Files(paths) => {
            // `DROPEFFECT_COPY`, so pasting never moves the originals.
            vec![
                (CF_HDROP.0.into(), drop_files(paths)),
                (
                    register(w!("Preferred DropEffect")),
                    1u32.to_le_bytes().to_vec(),
                ),
            ]
        }
    })
}

fn to_bytes(units: impl Iterator<Item = u16>) -> Vec<u8> {
    units.flat_map(u16::to_le_bytes).collect()
}

/// A bottom-up 32-bit bitmap without alpha, flattened onto white.
fn dib(pixels: &RgbaImage) -> Vec<u8> {
    let (width, height) = pixels.dimensions();
    let mut bytes = Vec::with_capacity(40 + pixels.as_raw().len());
    bytes.extend(40u32.to_le_bytes()); // header size
    bytes.extend((width as i32).to_le_bytes());
    bytes.extend((height as i32).to_le_bytes());
    bytes.extend(1u16.to_le_bytes()); // planes
    bytes.extend(32u16.to_le_bytes()); // bits per pixel
    bytes.extend(0u32.to_le_bytes()); // uncompressed
    bytes.extend((width * height * 4).to_le_bytes());
    bytes.extend([0; 16]); // resolution and palette
    for row in pixels.rows().rev() {
        for pixel in row {
            let [red, green, blue, alpha] = pixel.0.map(u32::from);
            let flatten = |channel| ((channel * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            bytes.extend([flatten(blue), flatten(green), flatten(red), 255]);
        }
    }
    bytes
}

/// A `DROPFILES` header followed by a double-null-terminated list of paths.
fn drop_files(paths: &[PathBuf]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(20u32.to_le_bytes()); // offset of the list
    bytes.extend([0; 12]); // drop point, unused
    bytes.extend(1u32.to_le_bytes()); // wide characters
    for path in paths {
        bytes.extend(to_bytes(path.as_os_str().encode_wide().chain([0])));
    }
    bytes.extend([0, 0]);
    bytes
}

/// Must be called while the clipboard is open.
fn set(format: u32, bytes: &[u8]) -> Result<()> {
    // SAFETY: the block is exactly as large as the copy. The clipboard owns it
    // once `SetClipboardData` succeeds; until then we free it.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            let _ = GlobalFree(Some(memory));
            anyhow::bail!("could not lock clipboard memory");
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        // Reports an error once the lock count reaches zero.
        let _ = GlobalUnlock(memory);
        if let Err(error) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(error).context("could not set the clipboard");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{capture::RawImage, ingest};

    #[test]
    fn bitmaps_read_back_and_flatten_transparency() {
        let mut pixels = RgbaImage::new(2, 2);
        pixels.put_pixel(0, 0, image::Rgba([10, 20, 30, 255]));
        pixels.put_pixel(1, 1, image::Rgba([0, 0, 0, 0]));
        // Round trip through the decoder the watcher uses.
        let read = ingest::decode(&RawImage::Dib(dib(&pixels))).unwrap();
        assert_eq!(read.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(read.get_pixel(1, 1).0, [255, 255, 255, 255]);
    }

    #[test]
    fn file_lists_are_double_terminated() {
        let bytes = drop_files(&[Path::new("a").into(), Path::new("bc").into()]);
        assert_eq!(bytes[..4], 20u32.to_le_bytes());
        assert_eq!(bytes[20..], to_bytes([97, 0, 98, 99, 0, 0].into_iter())[..]);
    }
}
