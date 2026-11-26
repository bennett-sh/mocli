//! Choices made in the installer, left in the registry for the app's next
//! start. Once read they are removed, and the config file holds them.

use std::ffi::c_void;

use windows::{
    Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegDeleteKeyValueW, RegGetValueW,
    },
    core::{PCWSTR, w},
};

const KEY: PCWSTR = w!(r"Software\mocli");
const CHECK_FOR_UPDATES: PCWSTR = w!("CheckForUpdates");

/// Whether to check for updates, if the installer asked and it was not read
/// yet. Reading it removes it.
pub fn take_update_choice() -> Option<bool> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the buffer is a u32 and `size` says so.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            CHECK_FOR_UPDATES,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast::<c_void>()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    // SAFETY: static names.
    let removed = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, KEY, CHECK_FOR_UPDATES) };
    if let Err(error) = removed.ok() {
        tracing::warn!(%error, "could not remove the installer's update choice");
    }
    Some(value != 0)
}
