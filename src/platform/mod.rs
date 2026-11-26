//! Everything that talks to Win32.

pub mod autostart;
pub mod caret;
pub mod clipboard;
pub mod clipboard_write;
pub mod dialog;
pub mod hook;
pub mod hotkey;
pub mod input;
pub mod installer;
pub mod keyboard;
pub mod ocr;
mod placement;
pub mod protect;
pub mod shell;
pub mod shell_image;
pub mod single_instance;
pub mod source_app;
pub mod tray;
pub mod version;
pub mod window;

use std::path::PathBuf;

use gpui_kit::Keystroke;
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, SPI_GETCLIENTAREAANIMATION,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    },
};

use crate::store::Blob;

/// Whether Windows is set to animate, which people turn off to reduce motion.
pub fn animations_enabled() -> bool {
    let mut enabled = windows::core::BOOL(1);
    // SAFETY: the setting is a `BOOL`, written to a valid pointer.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_err() || enabled.as_bool()
}

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
    /// Put derived text on the clipboard and close, pasting it if asked.
    UseText {
        text: String,
        paste: bool,
    },
    /// Open an address or a file with its default program.
    Open(OpenTarget),
    /// Show a file in Explorer.
    Reveal(PathBuf),
    /// Open a stored picture with its default program.
    OpenStored(Blob),
    /// Ask where to keep a copy of a stored image.
    SaveImage(Blob),
    /// Stop saving what a program copies.
    IgnoreApp(String),
    TogglePrivate,
    ConfigChanged,
    OpenSettings,
    OpenConfig,
    /// The update check started or finished.
    UpdateChecked,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Address(String),
    File(PathBuf),
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
