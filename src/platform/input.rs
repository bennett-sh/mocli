//! Synthetic key presses.

use anyhow::Result;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput,
    VIRTUAL_KEY,
};

use super::{
    hotkey::{PasteKeys, VK_INSERT},
    keyboard::VK_DUMMY,
};

/// Marks input we inject ourselves so the keyboard hook lets it through.
pub const OWN_INPUT: usize = 0x6D6F_636C; // "mocl"

const VK_LSHIFT: u16 = 0xA0;
const VK_LCONTROL: u16 = 0xA2;
const VK_LALT: u16 = 0xA4;
const VK_LWIN: u16 = 0x5B;
/// Left and right Shift, Ctrl, Alt and Win.
const MODIFIERS: [u16; 8] = [0xA0, 0xA1, VK_LCONTROL, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C];

#[derive(Clone, Copy)]
enum Stroke {
    Down,
    Up,
}

fn key(vk: u16, stroke: Stroke) -> INPUT {
    let mut flags = match stroke {
        Stroke::Down => KEYBD_EVENT_FLAGS(0),
        Stroke::Up => KEYEVENTF_KEYUP,
    };
    // Right Ctrl, right Alt, the Win keys and Insert.
    if matches!(vk, 0xA3 | 0xA5 | 0x5B | 0x5C | VK_INSERT) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                // Terminals and some games read the scan code.
                // SAFETY: plain query.
                wScan: unsafe { MapVirtualKeyW(vk.into(), MAPVK_VK_TO_VSC) } as u16,
                dwFlags: flags,
                dwExtraInfo: OWN_INPUT,
                ..Default::default()
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    // SAFETY: the element size matches the slice.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    // Windows blocks input aimed at programs running as administrator.
    anyhow::ensure!(
        sent as usize == inputs.len(),
        "Windows blocked the key presses"
    );
    Ok(())
}

/// Stops a following Alt or Win release from opening a menu.
pub fn tap_dummy_key() {
    if let Err(error) = send(&[key(VK_DUMMY, Stroke::Down), key(VK_DUMMY, Stroke::Up)]) {
        tracing::debug!(%error, "could not tap the dummy key");
    }
}

/// Presses the paste keys, normally Ctrl+V, in the focused app.
pub fn paste(PasteKeys(keys): PasteKeys) -> Result<()> {
    // Keys still held from the shortcut would turn this into something else.
    let held: Vec<u16> = MODIFIERS
        .into_iter()
        // SAFETY: plain query.
        .filter(|&vk| unsafe { GetAsyncKeyState(vk.into()) } < 0)
        .collect();

    let mut inputs = Vec::new();
    if held
        .iter()
        .any(|&vk| matches!(vk, 0xA4 | 0xA5 | 0x5B | 0x5C))
    {
        inputs.extend([key(VK_DUMMY, Stroke::Down), key(VK_DUMMY, Stroke::Up)]);
    }
    inputs.extend(held.into_iter().map(|vk| key(vk, Stroke::Up)));
    let modifiers: Vec<u16> = [
        (keys.mods.win, VK_LWIN),
        (keys.mods.ctrl, VK_LCONTROL),
        (keys.mods.alt, VK_LALT),
        (keys.mods.shift, VK_LSHIFT),
    ]
    .into_iter()
    .filter_map(|(held, vk)| held.then_some(vk))
    .collect();
    inputs.extend(modifiers.iter().map(|&vk| key(vk, Stroke::Down)));
    inputs.extend([key(keys.vk, Stroke::Down), key(keys.vk, Stroke::Up)]);
    inputs.extend(modifiers.iter().rev().map(|&vk| key(vk, Stroke::Up)));
    send(&inputs)
}
