//! "Start with Windows", through the current user's `Run` key. The registry is
//! the only record of the setting, so the installer and the app agree.

use std::{env, os::windows::ffi::OsStrExt as _};

use anyhow::{Context as _, Result};
use windows::{
    Win32::{
        Foundation::ERROR_FILE_NOT_FOUND,
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    },
    core::{PCWSTR, w},
};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("mocli");

pub fn is_enabled() -> bool {
    // SAFETY: only the size is asked for.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    }
    .is_ok()
}

pub fn set(enabled: bool) -> Result<()> {
    if !enabled {
        // SAFETY: static names.
        let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        return result.ok().context("could not remove the startup entry");
    }

    let exe = env::current_exe()?;
    let command: Vec<u16> = std::iter::once(u16::from(b'"'))
        .chain(exe.as_os_str().encode_wide())
        .chain([u16::from(b'"'), 0])
        .collect();
    // SAFETY: the length covers exactly the null-terminated string.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        )
    }
    .ok()
    .context("could not add the startup entry")
}
