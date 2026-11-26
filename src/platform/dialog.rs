//! A question asked before there is any window to ask it in.

use windows::{
    Win32::UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO, MessageBoxW,
    },
    core::{HSTRING, w},
};

/// Asks a yes-or-no question; "no" is preselected.
pub fn confirm(question: &str) -> bool {
    let text = HSTRING::from(question);
    // SAFETY: both strings outlive the call.
    let answer = unsafe {
        MessageBoxW(
            None,
            &text,
            w!("mocli"),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}
