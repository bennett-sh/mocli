use std::str::FromStr;

use serde::{Deserialize, Deserializer, de::Error as _};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

/// A global shortcut: exact modifiers plus one Windows virtual-key code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: Mods,
    pub vk: u16,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            mods: Mods {
                win: true,
                ..Mods::default()
            },
            vk: u16::from(b'V'),
        }
    }
}

const VK_SPACE: u16 = 0x20;
const VK_INSERT: u16 = 0x2D;
const VK_F1: u16 = 0x70;

impl FromStr for Hotkey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut mods = Mods::default();
        let mut key = None;

        for part in text.split('+').map(|part| part.trim().to_ascii_lowercase()) {
            match part.as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                "win" | "super" => mods.win = true,
                name => {
                    let vk = key_code(name).ok_or_else(|| format!("unknown key `{name}`"))?;
                    if key.replace(vk).is_some() {
                        return Err(format!("`{text}` has more than one key"));
                    }
                }
            }
        }

        let vk = key.ok_or_else(|| format!("`{text}` has no key"))?;
        // Shift alone would swallow ordinary typing.
        if !(mods.ctrl || mods.alt || mods.win) && vk < VK_F1 {
            return Err(format!("`{text}` needs Ctrl, Alt or Win"));
        }
        Ok(Self { mods, vk })
    }
}

fn key_code(name: &str) -> Option<u16> {
    match name.as_bytes() {
        [c] if c.is_ascii_alphanumeric() => Some(u16::from(c.to_ascii_uppercase())),
        _ => match name {
            "space" => Some(VK_SPACE),
            "insert" => Some(VK_INSERT),
            _ => {
                let number: u16 = name.strip_prefix('f')?.parse().ok()?;
                (1..=24).contains(&number).then(|| VK_F1 + number - 1)
            }
        },
    }
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_shortcuts() {
        assert_eq!(" win + v ".parse(), Ok(Hotkey::default()));
        let alt_v: Hotkey = "Alt+V".parse().unwrap();
        assert!(alt_v.mods.alt && !alt_v.mods.win);
        assert_eq!("F12".parse::<Hotkey>().unwrap().vk, 0x7B);
    }

    #[test]
    fn rejects_unusable_shortcuts() {
        for text in ["", "V", "Shift+V", "Alt", "Alt+V+B", "Alt+F25", "Alt+Foo"] {
            assert!(text.parse::<Hotkey>().is_err(), "{text:?} was accepted");
        }
    }
}
