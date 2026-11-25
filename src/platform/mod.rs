//! Everything that talks to Win32.

pub mod clipboard;
pub mod clipboard_write;
pub mod hook;
pub mod hotkey;
pub mod input;
pub mod keyboard;
pub mod shell;
pub mod single_instance;
mod source_app;
pub mod tray;
pub mod window;

use gpui_kit::Keystroke;

/// Messages from the platform layer (and the popup view) to the app controller.
#[derive(Debug)]
pub enum Event {
    Show,
    Hide,
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
