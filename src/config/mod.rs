//! Settings, as stored in `config.toml`.

mod shared;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use self::shared::SharedConfig;
use crate::platform::hotkey::{Hotkey, PasteKeys};

pub const DEFAULT_FILE: &str = r#"# mocli configuration. Changes apply as soon as the file is saved, except
# [keys], which needs a restart.

[general]
# Opens and closes the popup, for example "Win+V", "Alt+V" or "Ctrl+Shift+V".
# "Win+V" replaces the clipboard history built into Windows.
hotkey = "Win+V"
# Where the popup opens: "caret" (next to the text you are typing, centered if
# there is none), "centered" or "cursor" (at the mouse pointer).
placement = "caret"
# "no_activate" leaves focus in your app. Use "activate" if typing in the popup
# does not work, for example with an input method for Chinese or Japanese.
focus_mode = "no_activate"
# What Enter does, "paste" or "copy". Ctrl+Enter does the other.
primary_action = "paste"
# Pasting or copying an entry moves it to the top of the list.
move_to_top_on_reuse = true
# Put the previous clipboard content back after pasting.
restore_clipboard_after_paste = false

[appearance]
# "system", "light" or "dark"
theme = "system"
# "acrylic" or "solid"
backdrop = "acrylic"
# "compact", "medium" or "large"
size = "medium"
preview_pane = true
show_source_icon = true
animations = true

[history]
# Pinned entries are never removed. 0 means no limit.
max_entries = 1000
max_age_days = 90
# Larger images are not saved.
max_image_mb = 25
capture = ["text", "image", "files"]
# Read the text in copied images, so they can be searched.
ocr_images = true

[previews]
# Download the title and picture of copied web links. This contacts those sites
# as soon as you copy a link, so it is off by default.
fetch_links = false

[privacy]
# Copies made in these programs are not saved.
ignore_apps = ["1Password.exe", "Bitwarden.exe", "KeePass.exe", "KeePassXC.exe"]
# Skip content that apps mark as private, such as copied passwords.
respect_exclusion_formats = true
# Do not save anything while this is on.
private_mode = false
# "skip" leaves out card numbers, private keys and access tokens; "store" keeps
# them.
sensitive = "skip"

# Programs that paste with other keys than Ctrl+V.
[paste.per_app]
"WindowsTerminal.exe" = "Ctrl+Shift+V"
"mintty.exe" = "Shift+Insert"
"putty.exe" = "Shift+Insert"

# Other popup shortcuts, for example `delete = "ctrl-d"`. Actions: paste,
# paste_plain, copy, actions, pin, delete, settings, next_type, previous_type.
[keys]
"#;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub appearance: Appearance,
    pub history: History,
    pub previews: Previews,
    pub privacy: Privacy,
    pub paste: Paste,
    pub keys: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct General {
    pub hotkey: Hotkey,
    pub placement: Placement,
    pub focus_mode: FocusMode,
    pub primary_action: PrimaryAction,
    pub move_to_top_on_reuse: bool,
    pub restore_clipboard_after_paste: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            placement: Placement::default(),
            focus_mode: FocusMode::default(),
            primary_action: PrimaryAction::default(),
            move_to_top_on_reuse: true,
            restore_clipboard_after_paste: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Caret,
    Centered,
    Cursor,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    #[default]
    NoActivate,
    Activate,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryAction {
    #[default]
    Paste,
    Copy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    pub backdrop: Backdrop,
    pub size: PopupSize,
    pub preview_pane: bool,
    pub show_source_icon: bool,
    pub animations: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            backdrop: Backdrop::default(),
            size: PopupSize::default(),
            preview_pane: true,
            show_source_icon: true,
            animations: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

// Mica is left out on purpose: Windows paints it as a solid color on inactive
// windows, and the popup is normally not the active window.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    #[default]
    Acrylic,
    Solid,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PopupSize {
    Compact,
    #[default]
    Medium,
    Large,
}

impl PopupSize {
    /// Width and height in logical pixels.
    pub fn dimensions(self) -> (f32, f32) {
        match self {
            Self::Compact => (660.0, 420.0),
            Self::Medium => (760.0, 480.0),
            Self::Large => (920.0, 580.0),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct History {
    pub max_entries: u32,
    pub max_age_days: u32,
    pub max_image_mb: u32,
    pub capture: Vec<CaptureKind>,
    pub ocr_images: bool,
}

impl Default for History {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            max_age_days: 90,
            max_image_mb: 25,
            capture: vec![CaptureKind::Text, CaptureKind::Image, CaptureKind::Files],
            ocr_images: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Text,
    Image,
    Files,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Previews {
    pub fetch_links: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Privacy {
    pub ignore_apps: Vec<String>,
    pub respect_exclusion_formats: bool,
    pub private_mode: bool,
    pub sensitive: Sensitive,
}

impl Default for Privacy {
    fn default() -> Self {
        let ignore_apps = [
            "1Password.exe",
            "Bitwarden.exe",
            "KeePass.exe",
            "KeePassXC.exe",
        ];
        Self {
            ignore_apps: ignore_apps.map(String::from).to_vec(),
            respect_exclusion_formats: true,
            private_mode: false,
            sensitive: Sensitive::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Sensitive {
    #[default]
    Skip,
    Store,
}

/// The file name of a Windows path.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

impl Privacy {
    /// Whether copies made by the program at `exe` must not be saved.
    pub fn ignores(&self, exe: &str) -> bool {
        let name = file_name(exe);
        self.ignore_apps
            .iter()
            .any(|ignored| ignored.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Paste {
    pub per_app: BTreeMap<String, PasteKeys>,
}

impl Default for Paste {
    fn default() -> Self {
        let per_app = [
            ("WindowsTerminal.exe", "Ctrl+Shift+V"),
            ("mintty.exe", "Shift+Insert"),
            ("putty.exe", "Shift+Insert"),
        ];
        Self {
            per_app: per_app
                .into_iter()
                .filter_map(|(app, keys)| Some((app.to_owned(), keys.parse().ok()?)))
                .collect(),
        }
    }
}

impl Paste {
    /// The keys that paste in the program at `exe`.
    pub fn keys_for(&self, exe: &str) -> PasteKeys {
        let name = file_name(exe);
        self.per_app
            .iter()
            .find(|(app, _)| app.eq_ignore_ascii_case(name))
            .map_or_else(PasteKeys::default, |(_, keys)| *keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_matches_default_values() {
        assert_eq!(
            toml::from_str::<Config>(DEFAULT_FILE).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config: Config = toml::from_str("[appearance]\nbackdrop = \"solid\"").unwrap();
        assert_eq!(config.appearance.backdrop, Backdrop::Solid);
        assert_eq!(config.general, General::default());
    }

    #[test]
    fn ignored_apps_match_the_file_name_in_any_case() {
        let privacy = Privacy::default();
        assert!(privacy.ignores(r"C:\Program Files\KeePassXC\keepassxc.exe"));
        assert!(!privacy.ignores(r"C:\Bitwarden.exe\notepad.exe"));
        assert!(!privacy.ignores("NotBitwarden.exe"));
    }

    #[test]
    fn terminals_paste_with_their_own_keys() {
        let paste = Paste::default();
        assert_eq!(
            paste.keys_for(r"C:\x\windowsterminal.exe").to_string(),
            "Ctrl+Shift+V"
        );
        assert_eq!(paste.keys_for(r"C:\x\notepad.exe").to_string(), "Ctrl+V");
    }

    #[test]
    fn bad_hotkey_is_rejected() {
        assert!(toml::from_str::<Config>("[general]\nhotkey = \"V\"").is_err());
    }
}
