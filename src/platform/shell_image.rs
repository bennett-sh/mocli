//! The picture Explorer would show for a file: a thumbnail if its type has one,
//! otherwise its icon.

use std::{ffi::c_void, path::Path};

use image::RgbaImage;
use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
            GetDIBits, GetObjectW, HBITMAP, ReleaseDC,
        },
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
        UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT},
    },
    core::HSTRING,
};

/// Prepares the calling thread. The shell requires this once per thread.
pub fn init_thread() {
    // SAFETY: fails harmlessly if COM is already set up on this thread.
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.is_err() {
        tracing::debug!(?result, "COM was already initialized differently");
    }
}

/// Renders up to `size` pixels square. `None` if the file does not exist or the
/// shell has nothing for it.
pub fn render(path: &Path, size: u32) -> Option<RgbaImage> {
    let edge = i32::try_from(size).ok()?;
    // SAFETY: the bitmap is ours and is deleted once its pixels are copied.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let bitmap = factory
            .GetImage(SIZE { cx: edge, cy: edge }, SIIGBF_RESIZETOFIT)
            .ok()?;
        let pixels = pixels(bitmap);
        let _ = DeleteObject(bitmap.into());
        pixels
    }
}

/// # Safety
///
/// `bitmap` must be a live bitmap handle.
unsafe fn pixels(bitmap: HBITMAP) -> Option<RgbaImage> {
    // SAFETY: `info` and `data` are sized for what GDI is asked to write.
    unsafe {
        let mut info = BITMAP::default();
        let read = GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some((&raw mut info).cast::<c_void>()),
        );
        if read == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
            return None;
        }
        let (width, height) = (info.bmWidth as u32, info.bmHeight as u32);
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: info.bmWidth,
                // Negative: rows from the top.
                biHeight: -info.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut data = vec![0u8; width as usize * height as usize * 4];
        let screen = GetDC(None);
        let lines = GetDIBits(
            screen,
            bitmap,
            0,
            height,
            Some(data.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, screen);
        if lines == 0 {
            return None;
        }
        straighten(&mut data);
        RgbaImage::from_raw(width, height, data)
    }
}

/// Turns premultiplied BGRA into straight RGBA.
fn straighten(data: &mut [u8]) {
    // Some handlers leave alpha empty for opaque pictures.
    let (pixels, _) = data.as_chunks_mut::<4>();
    let opaque = pixels.iter().all(|pixel| pixel[3] == 0);
    for pixel in pixels {
        pixel.swap(0, 2);
        let alpha = u32::from(pixel[3]);
        if opaque {
            pixel[3] = 255;
        } else if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_premultiplied_pixels() {
        let mut half_red = [0, 0, 128, 128, 0, 0, 0, 0];
        straighten(&mut half_red);
        assert_eq!(half_red, [255, 0, 0, 128, 0, 0, 0, 0]);
        let mut no_alpha = [10, 20, 30, 0];
        straighten(&mut no_alpha);
        assert_eq!(no_alpha, [30, 20, 10, 255]);
    }

    #[test]
    fn renders_a_system_program_icon() {
        init_thread();
        let windows = std::env::var("SystemRoot").unwrap();
        let icon = render(&Path::new(&windows).join("explorer.exe"), 64).unwrap();
        assert!(icon.width() > 0 && icon.width() <= 64);
        assert!(icon.pixels().any(|pixel| pixel[3] > 0));
        assert!(render(Path::new(r"C:\does\not\exist.exe"), 64).is_none());
    }
}
