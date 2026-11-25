//! Finds the application a clipboard change came from.

use std::path::Path;

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND},
        System::{
            DataExchange::GetClipboardOwner,
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId},
    },
    core::PWSTR,
};

use super::version;
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
            let name = version::read(Path::new(&exe))
                .and_then(|info| info.description)
                .unwrap_or_else(|| {
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

/// The program a window belongs to.
pub fn executable_of(window: HWND) -> Option<String> {
    process_path(process_id(window))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_path_of_this_process() {
        let path = process_path(std::process::id()).unwrap();
        assert!(path.ends_with(".exe"), "{path}");
        assert_eq!(process_path(0), None);
    }
}
