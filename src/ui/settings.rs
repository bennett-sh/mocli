//! The settings window. Every change is saved to the config file at once.

use std::{collections::HashMap, fmt::Display};

use anyhow::Result;
use async_channel::Sender;
use gpui_kit::component::{
    ActiveTheme as _, Root, Sizable as _,
    button::{Button, ButtonVariants as _},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{
    config::{
        Backdrop, CaptureKind, Config, FocusMode, Placement, PopupSize, PrimaryAction, Sensitive,
        SharedConfig, ThemeChoice,
    },
    paths,
    platform::{Event, autostart, hotkey::Hotkey, window},
    store::SharedStore,
};

const REPOSITORY: &str = "https://github.com/bennett-sh/mocli";
const SHORTCUTS: [&str; 6] = [
    "Win+V",
    "Alt+V",
    "Ctrl+Shift+V",
    "Ctrl+Alt+V",
    "Win+Shift+V",
    "Alt+Shift+V",
];

/// What the fields read and write.
struct SettingsContext {
    config: SharedConfig,
    store: SharedStore,
    events: Sender<Event>,
    /// Text being typed, which may not be valid yet.
    drafts: HashMap<&'static str, SharedString>,
    confirm_clear: bool,
}

impl Global for SettingsContext {}

fn read<R>(cx: &App, read: impl FnOnce(&Config) -> R) -> R {
    cx.global::<SettingsContext>().config.read(read)
}

fn save(cx: &mut App, change: impl FnOnce(&mut Config)) {
    if let Err(error) = cx.global::<SettingsContext>().config.update(change) {
        tracing::error!(error = format!("{error:#}"), "could not save the settings");
    }
}

fn switch(
    title: &'static str,
    get: fn(&Config) -> bool,
    set: fn(&mut Config, bool),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx| read(cx, get),
            move |value, cx| save(cx, |config| set(config, value)),
        ),
    )
}

/// A dropdown over common limits. Typing a number would save every digit on
/// the way, and a limit of "1" prunes the history at the next copy.
fn limit(
    title: &'static str,
    presets: &[u32],
    unit: &'static str,
    zero: &'static str,
    get: fn(&Config) -> u32,
    set: fn(&mut Config, u32),
    current: u32,
) -> SettingItem {
    let label = move |value: u32| match value {
        0 => zero.to_owned(),
        value => format!("{value} {unit}"),
    };
    let options = limit_options(presets, current)
        .into_iter()
        .map(|value| (value.to_string().into(), label(value).into()))
        .collect();
    SettingItem::new(
        title,
        SettingField::dropdown(
            options,
            move |cx| read(cx, get).to_string().into(),
            move |chosen: SharedString, cx| {
                if let Ok(value) = chosen.parse() {
                    save(cx, |config| set(config, value));
                }
            },
        ),
    )
}

/// The presets, with a value set in the config file slotted in. Zero, meaning
/// no limit, comes last.
fn limit_options(presets: &[u32], current: u32) -> Vec<u32> {
    let mut options = presets.to_vec();
    if !options.contains(&current) {
        options.push(current);
    }
    options.sort_by_key(|&value| (value == 0, value));
    options
}

/// A dropdown over a few values, each with its label.
fn choice<T: Copy + PartialEq + 'static>(
    title: &'static str,
    options: &'static [(T, &'static str)],
    get: fn(&Config) -> T,
    set: fn(&mut Config, T),
) -> SettingItem {
    let labels = options
        .iter()
        .map(|(_, label)| (SharedString::from(*label), SharedString::from(*label)))
        .collect();
    SettingItem::new(
        title,
        SettingField::dropdown(
            labels,
            move |cx| {
                let current = read(cx, get);
                options
                    .iter()
                    .find(|(value, _)| *value == current)
                    .map_or_else(SharedString::default, |(_, label)| (*label).into())
            },
            move |chosen: SharedString, cx| {
                if let Some((value, _)) =
                    options.iter().find(|(_, label)| *label == chosen.as_ref())
                {
                    save(cx, |config| set(config, *value));
                }
            },
        ),
    )
}

fn shortcut(current: Hotkey) -> SettingItem {
    let mut options: Vec<String> = SHORTCUTS.iter().map(|s| (*s).to_owned()).collect();
    let current = current.to_string();
    if !options.contains(&current) {
        options.push(current);
    }
    SettingItem::new(
        "Shortcut",
        SettingField::dropdown(
            options
                .into_iter()
                .map(|s| (SharedString::from(s.clone()), SharedString::from(s)))
                .collect(),
            |cx| read(cx, |c| c.general.hotkey.to_string()).into(),
            |chosen: SharedString, cx| {
                if let Ok(hotkey) = chosen.parse::<Hotkey>() {
                    save(cx, |config| config.general.hotkey = hotkey);
                }
            },
        ),
    )
    .description("Opens and closes the popup. Other combinations can be set in the config file.")
}

/// A text field for a value with its own syntax. The typed text is kept while
/// it still means what is saved, so separators do not vanish while typing.
fn text_value<T: PartialEq + 'static>(
    key: &'static str,
    title: &'static str,
    show: fn(&Config) -> String,
    parse: fn(&str) -> Option<T>,
    current: fn(&Config) -> T,
    set: fn(&mut Config, T),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::input(
            move |cx| {
                let context = cx.global::<SettingsContext>();
                match context.drafts.get(key) {
                    Some(draft) if parse(draft) != Some(context.config.read(current)) => {
                        // Unfinished text stays; outdated text is replaced.
                        if parse(draft).is_none() {
                            draft.clone()
                        } else {
                            context.config.read(show).into()
                        }
                    }
                    Some(draft) => draft.clone(),
                    None => context.config.read(show).into(),
                }
            },
            move |typed: SharedString, cx| {
                let value = parse(&typed);
                cx.global_mut::<SettingsContext>().drafts.insert(key, typed);
                if let Some(value) = value {
                    save(cx, |config| set(config, value));
                }
            },
        ),
    )
    .layout(Axis::Vertical)
}

fn list_of(text: &str) -> Option<Vec<String>> {
    Some(
        text.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

type PerApp = std::collections::BTreeMap<String, crate::platform::hotkey::PasteKeys>;

fn show_pairs(pairs: &PerApp) -> String {
    join(
        pairs.iter().map(|(app, keys)| format!("{app}={keys}")),
        "; ",
    )
}

fn join<T: Display>(items: impl Iterator<Item = T>, separator: &str) -> String {
    items
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
        .join(separator)
}

/// `app.exe=Ctrl+Shift+V; other.exe=Shift+Insert`
fn pairs_of(text: &str) -> Option<PerApp> {
    text.split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (app, keys) = pair.split_once('=')?;
            let app = app.trim();
            (!app.is_empty()).then_some((app.to_owned(), keys.trim().parse().ok()?))
        })
        .collect()
}

fn button(title: &'static str, label: &'static str, on_click: fn(&mut App)) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::<SharedString>::render(move |options, _, _| {
            Button::new(title)
                .outline()
                .with_size(options.size())
                .label(label)
                .on_click(move |_, _, cx| on_click(cx))
        }),
    )
}

fn open_folder(path: Result<std::path::PathBuf>, cx: &mut App) {
    match path {
        Ok(path) => cx.open_with_system(&path),
        Err(error) => tracing::error!(error = format!("{error:#}"), "no such folder"),
    }
}

fn general(config: &Config) -> SettingPage {
    SettingPage::new("General").default_open(true).group(
        SettingGroup::new().items(vec![
            shortcut(config.general.hotkey),
            choice(
                "Open the popup",
                &[
                    (Placement::Caret, "Next to the text cursor"),
                    (Placement::Centered, "In the middle of the screen"),
                    (Placement::Cursor, "At the mouse pointer"),
                ],
                |c| c.general.placement,
                |c, v| c.general.placement = v,
            ),
            choice(
                "Focus",
                &[
                    (FocusMode::NoActivate, "Stays in your app"),
                    (FocusMode::Activate, "Moves to the popup"),
                ],
                |c| c.general.focus_mode,
                |c, v| c.general.focus_mode = v,
            )
            .description("Move focus to the popup if typing there does not work, as with some input methods."),
            choice(
                "Enter",
                &[
                    (PrimaryAction::Paste, "Pastes"),
                    (PrimaryAction::Copy, "Copies"),
                ],
                |c| c.general.primary_action,
                |c, v| c.general.primary_action = v,
            )
            .description("Ctrl+Enter does the other."),
            switch(
                "Move reused entries to the top",
                |c| c.general.move_to_top_on_reuse,
                |c, v| c.general.move_to_top_on_reuse = v,
            ),
            switch(
                "Restore the clipboard after pasting",
                |c| c.general.restore_clipboard_after_paste,
                |c, v| c.general.restore_clipboard_after_paste = v,
            ),
            SettingItem::new(
                "Start with Windows",
                SettingField::switch(
                    |_| autostart::is_enabled(),
                    |enabled, cx| {
                        if let Err(error) = autostart::set(enabled) {
                            tracing::error!(error = format!("{error:#}"), "autostart failed");
                        }
                        cx.refresh_windows();
                    },
                ),
            ),
        ]),
    )
}

fn appearance() -> SettingPage {
    SettingPage::new("Appearance").group(SettingGroup::new().items(vec![
        choice(
            "Theme",
            &[
                (ThemeChoice::System, "Follow Windows"),
                (ThemeChoice::Light, "Light"),
                (ThemeChoice::Dark, "Dark"),
            ],
            |c| c.appearance.theme,
            |c, v| c.appearance.theme = v,
        ),
        choice(
            "Background",
            &[(Backdrop::Acrylic, "Acrylic"), (Backdrop::Solid, "Solid")],
            |c| c.appearance.backdrop,
            |c, v| c.appearance.backdrop = v,
        ),
        choice(
            "Popup size",
            &[
                (PopupSize::Compact, "Compact"),
                (PopupSize::Medium, "Medium"),
                (PopupSize::Large, "Large"),
            ],
            |c| c.appearance.size,
            |c, v| c.appearance.size = v,
        ),
        switch(
            "Preview pane",
            |c| c.appearance.preview_pane,
            |c, v| c.appearance.preview_pane = v,
        ),
        switch(
            "Source app icons",
            |c| c.appearance.show_source_icon,
            |c, v| c.appearance.show_source_icon = v,
        ),
        switch(
            "Animations",
            |c| c.appearance.animations,
            |c, v| c.appearance.animations = v,
        ),
    ]))
}

fn set_capture(config: &mut Config, kind: CaptureKind, on: bool) {
    config.history.capture.retain(|&k| k != kind);
    if on {
        config.history.capture.push(kind);
    }
}

fn history(config: &Config) -> SettingPage {
    SettingPage::new("History")
        .group(SettingGroup::new().title("Save").items(vec![
            switch(
                "Text",
                |c| c.history.capture.contains(&CaptureKind::Text),
                |c, v| set_capture(c, CaptureKind::Text, v),
            ),
            switch(
                "Images",
                |c| c.history.capture.contains(&CaptureKind::Image),
                |c, v| set_capture(c, CaptureKind::Image, v),
            ),
            switch(
                "Files",
                |c| c.history.capture.contains(&CaptureKind::Files),
                |c, v| set_capture(c, CaptureKind::Files, v),
            ),
            switch(
                "Search text in images",
                |c| c.history.ocr_images,
                |c, v| c.history.ocr_images = v,
            )
            .description("Reads copied images with the text recognition built into Windows."),
            switch(
                "Link previews",
                |c| c.previews.fetch_links,
                |c, v| c.previews.fetch_links = v,
            )
            .description("Contacts a copied link's website to show its title and picture. Addresses that look private or contain secrets are skipped."),
        ]))
        .group(SettingGroup::new().title("Limits").items(vec![
            limit(
                "Entries to keep",
                &[100, 500, 1000, 5000, 10_000, 0],
                "entries",
                "All",
                |c| c.history.max_entries,
                |c, v| c.history.max_entries = v,
                config.history.max_entries,
            ),
            limit(
                "Keep entries for",
                &[7, 30, 90, 180, 365, 0],
                "days",
                "Ever",
                |c| c.history.max_age_days,
                |c, v| c.history.max_age_days = v,
                config.history.max_age_days,
            ),
            limit(
                "Largest image",
                &[5, 10, 25, 50, 100],
                "MB",
                "No images",
                |c| c.history.max_image_mb,
                |c, v| c.history.max_image_mb = v,
                config.history.max_image_mb,
            ),
        ]))
}

fn clear_history() -> SettingItem {
    SettingItem::new(
        "Delete everything except pinned entries",
        SettingField::<SharedString>::render(|options, _, cx| {
            let confirming = cx.global::<SettingsContext>().confirm_clear;
            Button::new("clear-history")
                .with_size(options.size())
                .map(|b| if confirming { b.danger() } else { b.outline() })
                .label(if confirming {
                    "Click again to delete"
                } else {
                    "Clear history"
                })
                .on_click(|_, _, cx| {
                    if cx.global::<SettingsContext>().confirm_clear {
                        let context = cx.global::<SettingsContext>();
                        let cleared = context.store.lock().clear_unpinned();
                        if let Err(error) = cleared {
                            tracing::error!(error = format!("{error:#}"), "could not clear");
                        }
                        let _ = context.events.try_send(Event::HistoryChanged);
                    }
                    let context = cx.global_mut::<SettingsContext>();
                    context.confirm_clear = !context.confirm_clear;
                    cx.refresh_windows();
                })
        }),
    )
}

fn privacy() -> SettingPage {
    SettingPage::new("Privacy").group(SettingGroup::new().items(vec![
        switch(
            "Pause capture",
            |c| c.privacy.private_mode,
            |c, v| c.privacy.private_mode = v,
        ),
        switch(
            "Respect private markers",
            |c| c.privacy.respect_exclusion_formats,
            |c, v| c.privacy.respect_exclusion_formats = v,
        )
        .description("Skip content apps mark as private, such as copied passwords."),
        switch(
            "Skip secrets",
            |c| c.privacy.sensitive == Sensitive::Skip,
            |c, v| {
                c.privacy.sensitive = if v { Sensitive::Skip } else { Sensitive::Store };
            },
        )
        .description("Card numbers, private keys and access tokens."),
        text_value(
            "ignore_apps",
            "Ignored programs (comma separated)",
            |c| c.privacy.ignore_apps.join(", "),
            list_of,
            |c| c.privacy.ignore_apps.clone(),
            |c, v| c.privacy.ignore_apps = v,
        ),
        clear_history(),
    ]))
}

fn shortcuts() -> SettingPage {
    const KEYS: [(&str, &str); 11] = [
        ("Paste or copy", "Enter, Ctrl+Enter"),
        ("Paste as plain text", "Shift+Enter"),
        ("Paste the first nine entries", "Ctrl+1 … Ctrl+9"),
        ("Actions", "Ctrl+K"),
        ("Pin or unpin", "Ctrl+P"),
        ("Delete", "Ctrl+Backspace or Delete, with an empty search"),
        ("Edit and paste", "Ctrl+E"),
        ("Open", "Ctrl+O"),
        ("Type filter", "Tab, Shift+Tab"),
        ("Settings", "Ctrl+,"),
        ("Clear search, then close", "Esc"),
    ];
    let reference = KEYS.into_iter().map(|(action, keys)| {
        SettingItem::new(
            action,
            SettingField::<SharedString>::render(move |_, _, cx| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(keys)
            }),
        )
    });
    SettingPage::new("Shortcuts")
        .group(
            SettingGroup::new()
                .title("In the popup")
                .description("Add your own under [keys] in the config file.")
                .items(reference.collect::<Vec<_>>()),
        )
        .group(
            SettingGroup::new().title("Pasting").item(
                text_value(
                    "per_app",
                    "Programs that paste with other keys",
                    |c| show_pairs(&c.paste.per_app),
                    pairs_of,
                    |c| c.paste.per_app.clone(),
                    |c, v| c.paste.per_app = v,
                )
                .description(
                    "For example: mintty.exe=Shift+Insert; WindowsTerminal.exe=Ctrl+Shift+V",
                ),
            ),
        )
}

fn advanced() -> SettingPage {
    SettingPage::new("Advanced").group(SettingGroup::new().items(vec![
        button("Config file", "Edit", |cx| {
            let _ = cx
                .global::<SettingsContext>()
                .events
                .try_send(Event::OpenConfig);
        }),
        button("History and images", "Open folder", |cx| {
            open_folder(paths::data_dir(), cx)
        }),
        button("Logs", "Open folder", |cx| {
            open_folder(paths::log_dir(), cx)
        }),
        button("All settings", "Reset to defaults", |cx| {
            cx.global_mut::<SettingsContext>().drafts.clear();
            save(cx, |config| *config = Config::default());
        }),
    ]))
}

fn about() -> SettingPage {
    SettingPage::new("About").group(
        SettingGroup::new()
            .title(concat!("mocli ", env!("CARGO_PKG_VERSION")))
            .description("A clipboard history for Windows.")
            .item(button("Source code", "Open on GitHub", |cx| {
                cx.open_url(REPOSITORY)
            })),
    )
}

fn pages(config: &Config) -> Vec<SettingPage> {
    vec![
        general(config),
        appearance(),
        history(config),
        privacy(),
        shortcuts(),
        advanced(),
        about(),
    ]
}

pub struct SettingsView;

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = cx.global::<SettingsContext>().config.get();
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(Settings::new("settings").pages(pages(&config)))
    }
}

/// Opens the settings window, or brings it forward.
pub struct SettingsWindow(Option<AnyWindowHandle>);

impl SettingsWindow {
    pub fn new(
        config: SharedConfig,
        store: SharedStore,
        events: Sender<Event>,
        cx: &mut App,
    ) -> Self {
        init(config, store, events, cx);
        Self(None)
    }

    pub fn open(&mut self, cx: &mut AsyncApp) -> Result<()> {
        // Fails if the window was closed, and a new one is opened.
        if let Some(window) = self.0
            && window::bring_forward(&window, cx).is_ok()
        {
            return Ok(());
        }
        let window = cx.update(|cx| {
            cx.global_mut::<SettingsContext>().confirm_clear = false;
            let bounds = WindowBounds::centered(None, size(px(920.), px(640.)), cx);
            let options = WindowOptions {
                window_bounds: Some(bounds),
                titlebar: Some(TitlebarOptions {
                    title: Some("mocli Settings".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(640.), px(420.))),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                let view = cx.new(|_| SettingsView);
                cx.new(|cx| Root::new(view, window, cx))
            })
        })?;
        let window = AnyWindowHandle::from(window);
        self.0 = Some(window);
        window::bring_forward(&window, cx)
    }

    /// Redraws after the config changed elsewhere.
    pub fn refresh(&self, cx: &mut AsyncApp) {
        cx.update(|cx| cx.refresh_windows());
    }
}

/// Makes the settings available to the window's fields.
pub fn init(config: SharedConfig, store: SharedStore, events: Sender<Event>, cx: &mut App) {
    cx.set_global(SettingsContext {
        config,
        store,
        events,
        drafts: HashMap::new(),
        confirm_clear: false,
    });
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI exports a `test` attribute of its own.
    use super::{limit_options, list_of, pairs_of, show_pairs};
    use crate::config::Paste;

    #[test]
    fn limits_include_a_custom_value_in_order() {
        assert_eq!(limit_options(&[10, 100, 0], 100), [10, 100, 0]);
        assert_eq!(limit_options(&[10, 100, 0], 42), [10, 42, 100, 0]);
        assert_eq!(limit_options(&[5, 10], 0), [5, 10, 0]);
    }

    #[test]
    fn typed_lists_ignore_stray_separators() {
        assert_eq!(list_of(" a.exe, , b.exe,").unwrap(), ["a.exe", "b.exe"]);
        assert!(list_of("").unwrap().is_empty());
    }

    #[test]
    fn paste_keys_read_back_what_they_show() {
        let defaults = Paste::default().per_app;
        assert_eq!(pairs_of(&show_pairs(&defaults)), Some(defaults));
        assert!(pairs_of("").unwrap().is_empty());
        for unfinished in ["a.exe", "a.exe=", "=Ctrl+V", "a.exe=Ctrl+"] {
            assert_eq!(pairs_of(unfinished), None, "{unfinished}");
        }
    }
}
