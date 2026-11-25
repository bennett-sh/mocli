use std::{fs, io::ErrorKind, path::Path};

use serde::Deserialize;

use crate::platform::hotkey::Hotkey;

const DEFAULT_FILE: &str = r#"# mocli configuration. Changes apply after a restart.

[general]
# Opens and closes the popup, for example "Alt+V", "Win+V" or "Ctrl+Shift+V".
hotkey = "Alt+V"
# Pasting or copying an entry moves it to the top of the list.
move_to_top_on_reuse = true

[appearance]
# "acrylic" or "solid"
backdrop = "acrylic"

[history]
# Pinned entries are never removed. 0 means no limit.
max_entries = 1000
max_age_days = 90
# Larger images are not saved.
max_image_mb = 25
capture = ["text", "image", "files"]

[privacy]
# Copies made in these programs are not saved.
ignore_apps = ["1Password.exe", "Bitwarden.exe", "KeePass.exe", "KeePassXC.exe"]
# Skip content that apps mark as private, such as copied passwords.
respect_exclusion_formats = true
"#;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub appearance: Appearance,
    pub history: History,
    pub privacy: Privacy,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct History {
    pub max_entries: u32,
    pub max_age_days: u32,
    pub max_image_mb: u32,
    pub capture: Vec<CaptureKind>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            max_age_days: 90,
            max_image_mb: 25,
            capture: vec![CaptureKind::Text, CaptureKind::Image, CaptureKind::Files],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Text,
    Image,
    Files,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Privacy {
    pub ignore_apps: Vec<String>,
    pub respect_exclusion_formats: bool,
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
        }
    }
}

impl Privacy {
    /// Whether copies made by the program at `exe` must not be saved.
    pub fn ignores(&self, exe: &str) -> bool {
        let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
        self.ignore_apps
            .iter()
            .any(|ignored| ignored.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct General {
    pub hotkey: Hotkey,
    pub move_to_top_on_reuse: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            move_to_top_on_reuse: true,
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub backdrop: Backdrop,
}

// Mica is left out on purpose: Windows paints it as a solid color on inactive
// windows, and the popup is never the active window.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    #[default]
    Acrylic,
    Solid,
}

impl Config {
    /// Reads the config, writing the commented default file on first start. A
    /// broken file is reported and left untouched.
    pub fn load_or_create(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|error| {
                tracing::warn!(%error, path = %path.display(), "invalid config, using defaults");
                Self::default()
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if let Err(error) = write_default(path) {
                    tracing::warn!(%error, "could not write the default config");
                }
                Self::default()
            }
            Err(error) => {
                tracing::warn!(%error, "could not read the config, using defaults");
                Self::default()
            }
        }
    }
}

fn write_default(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, DEFAULT_FILE)
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
    fn bad_hotkey_is_rejected() {
        assert!(toml::from_str::<Config>("[general]\nhotkey = \"V\"").is_err());
    }
}
