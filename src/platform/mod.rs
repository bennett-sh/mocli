//! Everything that talks to Win32.

pub mod caret;
pub mod clipboard;
pub mod clipboard_write;
pub mod hook;
pub mod hotkey;
pub mod input;
pub mod keyboard;
mod placement;
pub mod shell;
pub mod single_instance;
mod source_app;
pub mod tray;
pub mod window;

use gpui_kit::Keystroke;
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow},
};

/// The window that has keyboard focus.
pub fn foreground() -> HWND {
    // SAFETY: plain query.
    unsafe { GetForegroundWindow() }
}

/// The mouse pointer as an empty rectangle, for placing the popup.
pub fn cursor() -> Option<RECT> {
    let mut point = POINT::default();
    // SAFETY: `point` is a valid out pointer.
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some(RECT {
        left: point.x,
        top: point.y,
        right: point.x,
        bottom: point.y,
    })
}

/// Messages from the platform layer (and the popup view) to the app controller.
#[derive(Debug)]
pub enum Event {
    Show,
    /// Closed from the popup or with the shortcut: focus returns to the app.
    Hide,
    /// The user moved on to something else, which keeps focus.
    Dismiss,
    /// A key captured for the popup while another app keeps keyboard focus.
    Key(Keystroke),
    /// Use a history entry, which also closes the popup.
    Activate {
        id: i64,
        how: Activation,
    },
    HistoryChanged,
    OpenConfig,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// Into the focused app, with formatting.
    Paste,
    /// Into the focused app, as plain text.
    PastePlain,
    /// Onto the clipboard only.
    Copy,
}
