//! What the actions panel offers for the selected entry.

use gpui_kit::Action;
use gpui_kit::assets::IconName;

use super::keys::*;
use crate::{color::Color, config::PrimaryAction, store::Kind, ui::preview::Selected};

pub struct Item {
    pub label: String,
    pub icon: IconName,
    pub action: Box<dyn Action>,
}

fn item(label: impl Into<String>, icon: IconName, action: impl Action) -> Item {
    Item {
        label: label.into(),
        icon,
        action: Box::new(action),
    }
}

/// A deletion waiting to be confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    FromApp {
        exe: String,
        name: String,
        count: usize,
    },
    Everything,
}

pub struct Menu<'a> {
    pub selected: Option<&'a Selected>,
    pub primary: PrimaryAction,
    pub paused: bool,
    pub pending: Option<&'a Pending>,
}

/// Groups of items, in the order shown.
pub fn items(menu: &Menu) -> Vec<Vec<Item>> {
    if let Some(pending) = menu.pending {
        let question = match pending {
            Pending::FromApp { name, count, .. } => {
                format!("Yes, delete {count} entries from {name}")
            }
            Pending::Everything => "Yes, delete everything except pinned entries".to_owned(),
        };
        return vec![vec![
            item(question, IconName::Trash, ConfirmDelete),
            item("Cancel", IconName::X, CancelDelete),
        ]];
    }

    let mut groups = Vec::new();
    if let Some(selected) = menu.selected {
        groups.push(entry_items(selected, menu.primary));

        let mut deletion = vec![item("Delete entry", IconName::Trash, DeleteEntry)];
        if let Some(name) = &selected.details.source_name {
            deletion.push(item(
                format!("Delete all from {name}…"),
                IconName::ListX,
                DeleteFromApp,
            ));
            deletion.push(item(
                format!("Never save copies from {name}"),
                IconName::EyeOff,
                IgnoreApp,
            ));
        }
        groups.push(deletion);
    }

    let capture = if menu.paused {
        item("Resume capture", IconName::Play, TogglePrivate)
    } else {
        item("Pause capture", IconName::Pause, TogglePrivate)
    };
    groups.push(vec![
        capture,
        item("Clear history…", IconName::Eraser, ClearHistory),
        item("Settings…", IconName::Settings, OpenSettings),
        item("Quit mocli", IconName::Power, QuitApp),
    ]);
    groups
}

fn entry_items(selected: &Selected, primary: PrimaryAction) -> Vec<Item> {
    let details = &selected.details;
    let paste = item("Paste", IconName::ClipboardPaste, Paste);
    let copy = item("Copy to clipboard", IconName::Copy, Copy);
    // The `Paste` and `Copy` actions mean "primary" and "secondary".
    let mut items = match primary {
        PrimaryAction::Paste => vec![paste, copy],
        PrimaryAction::Copy => vec![
            item("Copy to clipboard", IconName::Copy, Paste),
            item("Paste", IconName::ClipboardPaste, Copy),
        ],
    };

    let textual = !matches!(details.kind, Kind::Image);
    if textual {
        items.push(item("Paste as plain text", IconName::Type, PastePlain));
    }
    items.push(if details.pinned {
        item("Unpin", IconName::PinOff, TogglePin)
    } else {
        item("Pin to keep", IconName::Pin, TogglePin)
    });
    if textual && details.kind != Kind::Files {
        items.push(item("Edit and paste", IconName::Pencil, EditEntry));
    }

    match details.kind {
        Kind::Link => items.push(item("Open in browser", IconName::ExternalLink, OpenEntry)),
        Kind::Email => items.push(item("Write an email", IconName::Mail, OpenEntry)),
        Kind::Files => {
            items.push(item("Open", IconName::ExternalLink, OpenEntry));
            items.push(item("Show in Explorer", IconName::FolderOpen, RevealEntry));
            items.push(item("Copy path", IconName::Route, CopyPath));
        }
        Kind::Image => {
            items.push(item("Open image", IconName::ExternalLink, OpenEntry));
            items.push(item("Save image as…", IconName::Download, SaveImage));
            if details.ocr_text.is_some() {
                items.push(item(
                    "Copy text from image",
                    IconName::ScanText,
                    CopyImageText,
                ));
            }
        }
        Kind::Color => {
            if details.text.as_deref().and_then(Color::parse).is_some() {
                items.push(item("Copy as HEX", IconName::Hash, CopyHex));
                items.push(item("Copy as RGB", IconName::Palette, CopyRgb));
                items.push(item("Copy as HSL", IconName::Palette, CopyHsl));
            }
        }
        Kind::Text | Kind::Code | Kind::Svg => {}
    }
    items
}
