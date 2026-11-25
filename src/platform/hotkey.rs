//! Key combinations written as "Ctrl+Shift+V".

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

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

/// The keys sent to an app to make it paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteKeys(pub Hotkey);

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

impl Default for PasteKeys {
    fn default() -> Self {
        Self(Hotkey {
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
            vk: u16::from(b'V'),
        })
    }
}

const VK_SPACE: u16 = 0x20;
pub const VK_INSERT: u16 = 0x2D;
const VK_F1: u16 = 0x70;

/// Any modifiers plus exactly one key.
fn parse_combination(text: &str) -> Result<Hotkey, String> {
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
    Ok(Hotkey { mods, vk })
}

impl FromStr for Hotkey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let hotkey = parse_combination(text)?;
        let mods = hotkey.mods;
        // Shift alone would swallow ordinary typing.
        if !(mods.ctrl || mods.alt || mods.win) && hotkey.vk < VK_F1 {
            return Err(format!("`{text}` needs Ctrl, Alt or Win"));
        }
        Ok(hotkey)
    }
}

impl FromStr for PasteKeys {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_combination(text).map(Self)
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

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Mods {
            ctrl,
            alt,
            shift,
            win,
        } = self.mods;
        for (held, name) in [(win, "Win"), (ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match self.vk {
            VK_SPACE => f.write_str("Space"),
            VK_INSERT => f.write_str("Insert"),
            key @ VK_F1..=0x87 => write!(f, "F{}", key - VK_F1 + 1),
            key => write!(f, "{}", char::from(key as u8)),
        }
    }
}

impl fmt::Display for PasteKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

fn deserialize_text<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: FromStr<Err = String>,
    D: Deserializer<'de>,
{
    String::deserialize(deserializer)?
        .parse()
        .map_err(D::Error::custom)
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_text(deserializer)
    }
}

impl<'de> Deserialize<'de> for PasteKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_text(deserializer)
    }
}

impl Serialize for Hotkey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for PasteKeys {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
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
    fn writes_shortcuts_the_way_it_reads_them() {
        for text in [
            "Win+V",
            "Ctrl+Alt+Shift+9",
            "Ctrl+Space",
            "Alt+Insert",
            "F1",
            "F24",
        ] {
            assert_eq!(text.parse::<Hotkey>().unwrap().to_string(), text);
        }
        assert_eq!(
            "shift+insert".parse::<PasteKeys>().unwrap().to_string(),
            "Shift+Insert"
        );
    }

    #[test]
    fn rejects_unusable_shortcuts() {
        for text in ["", "V", "Shift+V", "Alt", "Alt+V+B", "Alt+F25", "Alt+Foo"] {
            assert!(text.parse::<Hotkey>().is_err(), "{text:?} was accepted");
        }
    }
}
