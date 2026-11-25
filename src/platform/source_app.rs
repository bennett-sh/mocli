//! Finds the application a clipboard change came from.

use std::{ffi::c_void, path::Path, ptr, slice};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND},
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::{
            DataExchange::GetClipboardOwner,
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId},
    },
    core::{HSTRING, PWSTR},
};

use crate::capture::Source;

/// The clipboard owner, or the focused app when the owner is unknown. Many apps
/// leave no owner behind.
pub fn current() -> Option<Source> {
    // SAFETY: plain queries.
    let (owner, foreground) = unsafe { (GetClipboardOwner().ok(), GetForegroundWindow()) };
    let focused_process = process_id(foreground);

    [owner, Some(foreground)]
        .into_iter()
        .flatten()
        .find_map(|window| {
            let process = process_id(window);
            let exe = process_path(process)?;
            let window_title = (process == focused_process)
                .then(|| window_text(foreground))
                .flatten();
            let name = file_description(&exe).unwrap_or_else(|| {
                Path::new(&exe)
                    .file_stem()
                    .map_or_else(|| exe.clone(), |stem| stem.to_string_lossy().into())
            });
            Some(Source {
                exe,
                name,
                window_title,
            })
        })
}

fn process_id(window: HWND) -> u32 {
    let mut id = 0;
    // SAFETY: `id` is a valid out pointer; a null window leaves it zero.
    unsafe { GetWindowThreadProcessId(window, Some(&mut id)) };
    id
}

fn process_path(id: u32) -> Option<String> {
    if id == 0 {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    // SAFETY: `length` matches the buffer, and the handle is closed once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id).ok()?;
        let queried = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        queried.ok()?;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn window_text(window: HWND) -> Option<String> {
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer length is passed along with it.
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

/// The friendly name from the executable's version resource.
fn file_description(exe: &str) -> Option<String> {
    let path = HSTRING::from(exe);
    // SAFETY: the block is as large as Windows asks for, and every pointer
    // `VerQueryValueW` returns points into it.
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, block.as_mut_ptr().cast()).ok()?;

        let query = |key: &str| {
            let mut value: *mut c_void = ptr::null_mut();
            let mut length = 0;
            let found = VerQueryValueW(
                block.as_ptr().cast(),
                &HSTRING::from(key),
                &mut value,
                &mut length,
            );
            (found.as_bool() && !value.is_null() && length > 0)
                .then_some((value.cast::<u16>(), length as usize))
        };

        // The first language and code page pair; its length is in bytes.
        let (translation, bytes) = query(r"\VarFileInfo\Translation")?;
        if bytes < 4 {
            return None;
        }
        let language = translation.read_unaligned();
        let code_page = translation.add(1).read_unaligned();
        // The length of a string value is in characters.
        let (text, length) = query(&format!(
            r"\StringFileInfo\{language:04x}{code_page:04x}\FileDescription"
        ))?;
        let text = String::from_utf16_lossy(slice::from_raw_parts(text, length));
        let text = text.trim_matches(['\0', ' ']);
        (!text.is_empty()).then(|| text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_friendly_name_of_a_system_program() {
        let windows = std::env::var("SystemRoot").unwrap();
        let name = file_description(&format!(r"{windows}\explorer.exe")).unwrap();
        assert!(!name.is_empty() && !name.contains('\0'), "{name:?}");
        assert_eq!(file_description(r"C:\does\not\exist.exe"), None);
    }

    #[test]
    fn finds_the_path_of_this_process() {
        let path = process_path(std::process::id()).unwrap();
        assert!(path.ends_with(".exe"), "{path}");
        assert_eq!(process_path(0), None);
    }
}
