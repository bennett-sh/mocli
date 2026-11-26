use std::{os::windows::ffi::OsStrExt as _, path::Path};

use anyhow::Result;
use windows::{
    Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    core::{PCWSTR, w},
};

/// Opens a text file in Notepad, which exists on every Windows install, unlike
/// an association for `.toml`.
pub fn edit_text_file(path: &Path) -> Result<()> {
    let quoted: Vec<u16> = std::iter::once(u16::from(b'"'))
        .chain(path.as_os_str().encode_wide())
        .chain([u16::from(b'"'), 0])
        .collect();
    // SAFETY: all strings are null-terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("notepad.exe"),
            PCWSTR(quoted.as_ptr()),
            None,
            SW_SHOWNORMAL,
        )
    };
    // Values of 32 and below are error codes.
    anyhow::ensure!(result.0 as isize > 32, "failed to open {}", path.display());
    Ok(())
}
