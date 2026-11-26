//! The popup's actions and the keys that trigger them.

use std::collections::BTreeMap;

use gpui_kit::*;

actions!(
    popup,
    [
        SelectPrevious,
        SelectNext,
        PageUp,
        PageDown,
        SelectFirst,
        SelectLast,
        Paste,
        PastePlain,
        Copy,
        NextType,
        PreviousType,
        Dismiss,
        ToggleActions,
        TogglePin,
        DeleteEntry,
        DeleteFromApp,
        ClearHistory,
        ConfirmDelete,
        CancelDelete,
        OpenEntry,
        RevealEntry,
        CopyPath,
        CopyHex,
        CopyRgb,
        CopyHsl,
        CopyImageText,
        SaveImage,
        EditEntry,
        RenameEntry,
        ClearName,
        SaveName,
        CancelRename,
        PasteEdited,
        CancelEdit,
        IgnoreApp,
        TogglePrivate,
        OpenSettings,
        QuitApp
    ]
);

/// Pastes the n-th entry of the list, counted from zero.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = popup, no_json)]
pub struct PasteNth(pub usize);

/// Key contexts. The search one is scoped to its input, so these win over the
/// input's own bindings and leave the type dropdown alone while it is open.
const SEARCH: &str = "ClipboardSearch > Input";
const POPUP: &str = "ClipboardPopup";
const PANEL: &str = "ClipboardActions > Input";
const EDITOR: &str = "ClipboardEditor > Input";
const RENAME: &str = "ClipboardRename > Input";

/// Binds the popup's keys. `overrides` maps action names from the config to
/// additional keystrokes.
pub fn bind_keys(overrides: &BTreeMap<String, String>, cx: &mut App) {
    let search = Some(SEARCH);
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, search),
        KeyBinding::new("down", SelectNext, search),
        KeyBinding::new("pageup", PageUp, search),
        KeyBinding::new("pagedown", PageDown, search),
        KeyBinding::new("home", SelectFirst, search),
        KeyBinding::new("end", SelectLast, search),
        KeyBinding::new("enter", Paste, search),
        KeyBinding::new("shift-enter", PastePlain, search),
        KeyBinding::new("ctrl-enter", Copy, search),
        KeyBinding::new("tab", NextType, search),
        KeyBinding::new("ctrl-f", NextType, search),
        KeyBinding::new("shift-tab", PreviousType, search),
        KeyBinding::new("escape", Dismiss, search),
        KeyBinding::new("ctrl-k", ToggleActions, search),
        KeyBinding::new("ctrl-p", TogglePin, search),
        KeyBinding::new("ctrl-shift-p", TogglePin, search),
        KeyBinding::new("ctrl-backspace", DeleteEntry, search),
        KeyBinding::new("delete", DeleteEntry, search),
        KeyBinding::new("ctrl-o", OpenEntry, search),
        KeyBinding::new("ctrl-e", EditEntry, search),
        KeyBinding::new("f2", RenameEntry, search),
        KeyBinding::new("ctrl-,", OpenSettings, search),
        // Wherever focus is within the popup.
        KeyBinding::new("escape", Dismiss, Some(POPUP)),
        KeyBinding::new("ctrl-k", ToggleActions, Some(POPUP)),
        KeyBinding::new("ctrl-k", ToggleActions, Some(PANEL)),
        // Otherwise Escape would close the panel and then the popup.
        KeyBinding::new("escape", Dismiss, Some(PANEL)),
        // The entry's shortcuts also work while the actions panel is open,
        // which is what makes the panel show them as hints. Plain Enter
        // stays the panel's own.
        KeyBinding::new("shift-enter", PastePlain, Some(PANEL)),
        KeyBinding::new("ctrl-enter", Copy, Some(PANEL)),
        KeyBinding::new("ctrl-p", TogglePin, Some(PANEL)),
        KeyBinding::new("delete", DeleteEntry, Some(PANEL)),
        KeyBinding::new("ctrl-o", OpenEntry, Some(PANEL)),
        KeyBinding::new("ctrl-e", EditEntry, Some(PANEL)),
        KeyBinding::new("f2", RenameEntry, Some(PANEL)),
        KeyBinding::new("ctrl-,", OpenSettings, Some(PANEL)),
        KeyBinding::new("ctrl-enter", PasteEdited, Some(EDITOR)),
        KeyBinding::new("escape", CancelEdit, Some(EDITOR)),
        KeyBinding::new("enter", SaveName, Some(RENAME)),
        KeyBinding::new("escape", CancelRename, Some(RENAME)),
    ]);
    let slots = [
        "ctrl-1", "ctrl-2", "ctrl-3", "ctrl-4", "ctrl-5", "ctrl-6", "ctrl-7", "ctrl-8", "ctrl-9",
    ];
    cx.bind_keys(
        slots
            .into_iter()
            .enumerate()
            .map(|(n, keys)| KeyBinding::new(keys, PasteNth(n), search)),
    );

    for (name, keys) in overrides {
        // `KeyBinding::new` panics on a keystroke it cannot parse, and keeps
        // the text for as long as the app runs. This runs once, at startup.
        let keys: &'static str = String::leak(keys.clone());
        let valid = !keys.trim().is_empty()
            && keys
                .split_whitespace()
                .all(|stroke| Keystroke::parse(stroke).is_ok());
        let Some(search_binding) = valid.then(|| binding(name, keys, SEARCH)).flatten() else {
            tracing::warn!(action = %name, keys = %keys, "ignoring a key setting");
            continue;
        };
        cx.bind_keys([search_binding]);
        // Keys without a modifier would take typing or Enter away from the
        // panel's own search. Function keys type nothing.
        let modified = keys
            .split_whitespace()
            .next()
            .and_then(|stroke| Keystroke::parse(stroke).ok())
            .is_some_and(|stroke| stroke.modifiers.modified() || is_function_key(&stroke.key));
        if modified && name != "actions" {
            cx.bind_keys(binding(name, keys, PANEL));
        }
    }
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number))
}

/// The binding for an action named in the config, in the given key context.
fn binding(name: &str, keys: &'static str, context: &'static str) -> Option<KeyBinding> {
    let context = Some(context);
    Some(match name {
        "paste" => KeyBinding::new(keys, Paste, context),
        "paste_plain" => KeyBinding::new(keys, PastePlain, context),
        "copy" => KeyBinding::new(keys, Copy, context),
        "actions" => KeyBinding::new(keys, ToggleActions, context),
        "pin" => KeyBinding::new(keys, TogglePin, context),
        "rename" => KeyBinding::new(keys, RenameEntry, context),
        "delete" => KeyBinding::new(keys, DeleteEntry, context),
        "settings" => KeyBinding::new(keys, OpenSettings, context),
        "next_type" => KeyBinding::new(keys, NextType, context),
        "previous_type" => KeyBinding::new(keys, PreviousType, context),
        _ => return None,
    })
}
