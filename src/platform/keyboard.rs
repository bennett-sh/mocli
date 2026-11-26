//! Decides what happens to each hooked key, and translates captured keys into
//! GPUI keystrokes.

use gpui_kit::{Keystroke, Modifiers};
use windows::Win32::UI::{
    Input::KeyboardAndMouse::{GetKeyboardLayout, MAPVK_VK_TO_CHAR, MapVirtualKeyExW, ToUnicodeEx},
    WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};

use super::hotkey::{Hotkey, Mods};

const VK_TAB: u16 = 0x09;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_CAPITAL: u16 = 0x14;
/// Unicode text injected by on-screen keyboards and automation tools.
const VK_PACKET: u16 = 0xE7;
/// Unassigned key, tapped to stop a lone Alt or Win release from opening a menu.
pub const VK_DUMMY: u16 = 0xE8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Let the focused app have the key.
    Pass,
    /// Eat the key.
    Swallow,
    /// Eat the key and toggle the popup.
    Toggle,
    /// Eat the key and send it to the popup.
    Forward,
}

/// Tracks which held keys went where, so the focused app never receives half of
/// a key press.
pub struct KeyFilter {
    hotkey: Hotkey,
    swallowed: [bool; 256],
    passed: [bool; 256],
}

impl KeyFilter {
    pub fn new(hotkey: Hotkey) -> Self {
        Self {
            hotkey,
            swallowed: [false; 256],
            passed: [false; 256],
        }
    }

    pub fn set_hotkey(&mut self, hotkey: Hotkey) {
        self.hotkey = hotkey;
    }

    pub fn key_up(&mut self, vk: u16) -> Verdict {
        let index = usize::from(vk & 0xFF);
        self.passed[index] = false;
        if std::mem::take(&mut self.swallowed[index]) {
            Verdict::Swallow
        } else {
            Verdict::Pass
        }
    }

    pub fn key_down(&mut self, vk: u16, mods: Mods, capturing: bool) -> Verdict {
        if is_modifier(vk) || vk == VK_DUMMY {
            return Verdict::Pass;
        }
        let index = usize::from(vk & 0xFF);
        // `VK_PACKET` stands for many different characters, so it never repeats.
        let repeat = self.swallowed[index] && vk != VK_PACKET;

        if vk == self.hotkey.vk && mods == self.hotkey.mods {
            if repeat {
                return Verdict::Swallow;
            }
            self.swallowed[index] = true;
            return Verdict::Toggle;
        }

        if capturing && !is_system_key(vk, mods) {
            self.swallowed[index] |= !self.passed[index];
            return Verdict::Forward;
        }

        // Auto-repeat of a key that closed the popup must not leak through.
        if repeat {
            return Verdict::Swallow;
        }
        self.passed[index] = true;
        Verdict::Pass
    }
}

fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// Keys that keep working while the popup captures the keyboard.
fn is_system_key(vk: u16, mods: Mods) -> bool {
    mods.win
        || (vk == VK_TAB && mods.alt)
        || (vk == VK_ESCAPE && (mods.alt || mods.ctrl))
        // Lock keys, Print Screen, volume and media keys.
        || matches!(vk, VK_CAPITAL | 0x90 | 0x91 | 0x2C | 0xA6..=0xB7)
}

/// Translates a captured key using the focused app's keyboard layout.
///
/// Returns `None` for keys with nothing to deliver, including dead keys, which
/// combine with the next key.
pub fn keystroke(vk: u16, scan: u32, mods: Mods, caps_lock: bool) -> Option<Keystroke> {
    let mut modifiers = Modifiers {
        control: mods.ctrl,
        alt: mods.alt,
        shift: mods.shift,
        platform: mods.win,
        function: false,
    };

    if vk == VK_PACKET {
        let text = char::from_u32(scan)
            .filter(|c| !c.is_control())?
            .to_string();
        return Some(Keystroke {
            modifiers: Modifiers::default(),
            key: text.to_lowercase(),
            key_char: Some(text),
        });
    }

    if let Some(name) = named_key(vk) {
        let types_space = vk == VK_SPACE && !mods.ctrl && !mods.alt;
        return Some(Keystroke {
            modifiers,
            key: name,
            key_char: types_space.then(|| " ".into()),
        });
    }

    // SAFETY: plain queries; a null foreground window yields the system layout.
    let layout = unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        GetKeyboardLayout(thread)
    };

    // Ctrl+Alt is AltGr on many layouts, so it can still produce text.
    let mut key_char = None;
    if mods.ctrl == mods.alt {
        let mut state = [0u8; 256];
        for (held, key) in [(mods.shift, 0x10), (mods.ctrl, 0x11), (mods.alt, 0x12)] {
            state[key] = if held { 0x80 } else { 0 };
        }
        state[usize::from(VK_CAPITAL)] = u8::from(caps_lock);

        let mut buffer = [0u16; 8];
        // SAFETY: both buffers outlive the call.
        let written = unsafe { ToUnicodeEx(vk.into(), scan, &state, &mut buffer, 0, Some(layout)) };
        if written < 0 {
            return None;
        }
        let text = String::from_utf16_lossy(&buffer[..written as usize]);
        if !text.is_empty() && !text.chars().any(char::is_control) {
            key_char = Some(text);
            modifiers.control = false;
            modifiers.alt = false;
        }
    }

    // SAFETY: plain query. The high bit marks dead keys and is masked off.
    let base = unsafe { MapVirtualKeyExW(vk.into(), MAPVK_VK_TO_CHAR, Some(layout)) } & 0xFFFF;
    let key = char::from_u32(base)
        .filter(|&c| c != '\0')
        .map(|c| c.to_lowercase().to_string())
        .or_else(|| key_char.clone())?;

    Some(Keystroke {
        modifiers,
        key,
        key_char,
    })
}

/// GPUI's names for keys that do not depend on the layout.
fn named_key(vk: u16) -> Option<String> {
    let name = match vk {
        0x08 => "backspace",
        VK_TAB => "tab",
        0x0D => "enter",
        VK_ESCAPE => "escape",
        VK_SPACE => "space",
        0x21 => "pageup",
        0x22 => "pagedown",
        0x23 => "end",
        0x24 => "home",
        0x25 => "left",
        0x26 => "up",
        0x27 => "right",
        0x28 => "down",
        0x2D => "insert",
        0x2E => "delete",
        0x70..=0x87 => return Some(format!("f{}", vk - 0x6F)),
        _ => return None,
    };
    Some(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: u16 = b'V' as u16;
    const A: u16 = b'A' as u16;
    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
    };
    const ALT: Mods = Mods { alt: true, ..NONE };

    fn filter() -> KeyFilter {
        KeyFilter::new("Alt+V".parse().unwrap())
    }

    #[test]
    fn hotkey_toggles_once_per_press() {
        let mut filter = filter();
        assert_eq!(filter.key_down(V, ALT, false), Verdict::Toggle);
        assert_eq!(filter.key_down(V, ALT, true), Verdict::Swallow);
        assert_eq!(filter.key_up(V), Verdict::Swallow);
        assert_eq!(filter.key_down(V, ALT, true), Verdict::Toggle);
    }

    #[test]
    fn win_shortcut_is_taken_from_windows() {
        let mut filter = KeyFilter::new(Hotkey::default());
        let win = Mods { win: true, ..NONE };
        assert_eq!(filter.key_down(0x5B, win, false), Verdict::Pass);
        assert_eq!(filter.key_down(V, win, false), Verdict::Toggle);
        assert_eq!(filter.key_up(V), Verdict::Swallow);
        assert_eq!(filter.key_up(0x5B), Verdict::Pass);
        // Other Win shortcuts are left to Windows.
        assert_eq!(filter.key_down(A, win, false), Verdict::Pass);
    }

    #[test]
    fn keys_pass_while_closed_and_are_captured_while_open() {
        let mut filter = filter();
        assert_eq!(filter.key_down(A, NONE, false), Verdict::Pass);
        assert_eq!(filter.key_up(A), Verdict::Pass);
        assert_eq!(filter.key_down(A, NONE, true), Verdict::Forward);
        assert_eq!(filter.key_up(A), Verdict::Swallow);
    }

    #[test]
    fn key_held_before_opening_still_releases_in_the_app() {
        let mut filter = filter();
        assert_eq!(filter.key_down(A, NONE, false), Verdict::Pass);
        assert_eq!(filter.key_down(A, NONE, true), Verdict::Forward);
        assert_eq!(filter.key_up(A), Verdict::Pass);
    }

    #[test]
    fn repeat_after_closing_does_not_leak() {
        let mut filter = filter();
        assert_eq!(filter.key_down(0x0D, NONE, true), Verdict::Forward);
        assert_eq!(filter.key_down(0x0D, NONE, false), Verdict::Swallow);
        assert_eq!(filter.key_up(0x0D), Verdict::Swallow);
    }

    #[test]
    fn modifiers_and_system_keys_pass_while_open() {
        let mut filter = filter();
        assert_eq!(filter.key_down(0xA4, ALT, true), Verdict::Pass);
        assert_eq!(filter.key_down(VK_TAB, ALT, true), Verdict::Pass);
        let win = Mods { win: true, ..NONE };
        assert_eq!(filter.key_down(b'D'.into(), win, true), Verdict::Pass);
    }
}
