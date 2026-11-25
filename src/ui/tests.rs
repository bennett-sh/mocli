//! Headless tests of the popup: the real components, driven by keys, without a
//! desktop.

// Not `gpui_kit::*`: GPUI exports a `test` attribute of its own.
use async_channel::Receiver;
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use image::{Rgba, RgbaImage};

use super::{
    popup::{self, Popup},
    settings,
};
use crate::{
    capture::Source,
    config::{Config, PrimaryAction, SharedConfig},
    ingest::link::Preview,
    platform::{Activation, Event},
    store::{
        Image, Kind, NewEntry, SharedStore, Store,
        tests::{Scratch, text},
    },
};

fn png() -> Vec<u8> {
    let pixels = RgbaImage::from_pixel(6, 4, Rgba([10, 120, 200, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    pixels
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

pub fn of_kind(kind: Kind, content: &str, at: i64) -> NewEntry {
    NewEntry {
        kind,
        language: (kind == Kind::Code).then_some("rs"),
        ..text(content, at)
    }
}

/// One entry of every kind, oldest first.
fn samples() -> Vec<NewEntry> {
    let windows = std::env::var("SystemRoot").unwrap();
    let mut picture = of_kind(Kind::Image, "Image (6×4)", 7);
    picture.text = None;
    picture.image = Some(Image {
        png: png(),
        thumbnail: png(),
        width: 6,
        height: 4,
    });
    vec![
        of_kind(Kind::Text, "plain words\nand a second line", 1),
        of_kind(Kind::Code, "fn main() {\n    let a = \"```\";\n}", 2),
        of_kind(
            Kind::Svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4"/></svg>"#,
            3,
        ),
        of_kind(Kind::Link, "https://example.com/page", 4),
        of_kind(Kind::Email, "someone@example.com", 5),
        of_kind(Kind::Color, "#ff7a1a", 6),
        picture,
        of_kind(Kind::Files, &format!(r"{windows}\explorer.exe"), 8),
        of_kind(Kind::Files, r"C:\does\not\exist.txt", 9),
        of_kind(Kind::Files, "C:\\a\\one.txt\nC:\\a\\two.txt", 10),
    ]
}

pub struct Harness<'a> {
    pub popup: Entity<Popup>,
    pub cx: &'a mut VisualTestContext,
    pub events: Receiver<Event>,
    pub store: SharedStore,
    /// Entry ids in the order they were recorded.
    pub ids: Vec<i64>,
    _scratch: Scratch,
}

impl Harness<'_> {
    pub fn open<'a>(cx: &'a mut TestAppContext, name: &str, entries: Vec<NewEntry>) -> Harness<'a> {
        Self::configured(cx, name, entries, Config::default())
    }

    pub fn configured<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
        entries: Vec<NewEntry>,
        config: Config,
    ) -> Harness<'a> {
        let scratch = Scratch::new(name);
        let store = SharedStore::new(Store::open(&scratch.0).unwrap());
        let ids = entries
            .iter()
            .map(|entry| store.lock().record(entry).unwrap().id)
            .collect();
        cx.update(|cx| {
            gpui_kit::init(cx);
            popup::bind_keys(&config.keys, cx);
        });

        let (sender, events) = async_channel::unbounded();
        let mut view = None;
        let shared = store.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let popup = cx.new(|cx| Popup::new(shared, &config, sender, window, cx));
            view = Some(popup.clone());
            Root::new(popup, window, cx)
        });
        let popup = view.unwrap();
        popup.update_in(cx, |popup, window, cx| popup.reset(window, cx));
        cx.run_until_parked();
        Harness {
            popup,
            cx,
            events,
            store,
            ids,
            _scratch: scratch,
        }
    }

    pub fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
        self.cx.run_until_parked();
    }

    pub fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(text);
        self.cx.run_until_parked();
    }

    /// Whether an overlay is showing the acrylic.
    pub fn frosted(&mut self) -> bool {
        self.popup
            .update_in(self.cx, |popup, window, cx| popup.is_frosted(window, cx))
    }

    /// The titles in the history, top to bottom.
    pub fn titles(&mut self) -> Vec<String> {
        self.store
            .lock()
            .search("", &[], 100)
            .unwrap()
            .matches
            .into_iter()
            .map(|entry| entry.title)
            .collect()
    }

    pub fn event(&mut self) -> Event {
        self.events.try_recv().expect("an event was sent")
    }

    /// Chooses the action whose label matches `query` in the actions panel.
    pub fn action(&mut self, query: &str) {
        self.press("ctrl-k");
        self.type_text(query);
        self.press("enter");
    }

    /// The entry Enter would paste now.
    pub fn pressing_enter(&mut self) -> Option<i64> {
        self.press("enter");
        match self.events.try_recv() {
            Ok(Event::Activate {
                id,
                how: Activation::Paste,
            }) => Some(id),
            Ok(other) => panic!("unexpected {other:?}"),
            Err(_) => None,
        }
    }
}

#[gpui_kit::test]
fn every_kind_has_a_preview(cx: &mut TestAppContext) {
    let mut entries = samples();
    let link = entries[3].hash;
    let mut ui = Harness::open(cx, "ui-kinds", Vec::new());
    ui.ids = entries
        .drain(..)
        .map(|entry| ui.store.lock().record(&entry).unwrap().id)
        .collect();
    ui.store
        .lock()
        .set_link_preview(
            ui.ids[3],
            link,
            &Preview {
                title: Some("A page".into()),
                description: Some("About things".into()),
                image: Some(png()),
                icon: Some(png()),
            },
        )
        .unwrap();
    ui.popup
        .update_in(ui.cx, |popup, window, cx| popup.reset(window, cx));
    ui.cx.run_until_parked();

    // Selecting an entry draws its preview.
    for id in ui.ids.clone().into_iter().rev() {
        assert_eq!(ui.pressing_enter(), Some(id));
        ui.popup.update(ui.cx, |popup, cx| popup.toggle_source(cx));
        ui.cx.run_until_parked();
        ui.press("down");
    }
}

#[gpui_kit::test]
fn keys_search_filter_and_activate(cx: &mut TestAppContext) {
    let mut ui = Harness::open(cx, "ui-keys", samples());
    let newest = *ui.ids.last().unwrap();
    assert_eq!(ui.pressing_enter(), Some(newest));

    ui.press("up");
    // Wraps to the oldest entry.
    assert_eq!(ui.pressing_enter(), Some(ui.ids[0]));

    ui.type_text("someone@");
    assert_eq!(ui.pressing_enter(), Some(ui.ids[4]));
    ui.press("escape");
    assert!(
        ui.events.try_recv().is_err(),
        "the first Escape only clears"
    );
    assert_eq!(ui.pressing_enter(), Some(newest));

    // All, Text, Images: the newest picture or SVG.
    ui.press("tab tab");
    assert_eq!(ui.pressing_enter(), Some(ui.ids[6]));
    ui.press("shift-tab shift-tab");

    ui.type_text("zzzzzz");
    assert_eq!(ui.pressing_enter(), None);
    ui.press("escape escape");
    assert!(matches!(ui.events.try_recv(), Ok(Event::Hide)));

    ui.press("ctrl-enter");
    assert!(matches!(
        ui.events.try_recv(),
        Ok(Event::Activate {
            how: Activation::Copy,
            ..
        })
    ));
}

fn from_app(content: &str, exe: &str, at: i64) -> NewEntry {
    NewEntry {
        source: Some(Source {
            exe: exe.into(),
            name: exe.trim_end_matches(".exe").into(),
            window_title: None,
        }),
        ..text(content, at)
    }
}

fn three() -> Vec<NewEntry> {
    vec![
        from_app("alpha", "one.exe", 1),
        from_app("bravo", "two.exe", 2),
        from_app("charlie", "two.exe", 3),
    ]
}

#[gpui_kit::test]
fn searching_the_type_dropdown_picks_a_type(cx: &mut TestAppContext) {
    let mut ui = Harness::open(cx, "ui-dropdown", samples());
    ui.popup
        .update_in(ui.cx, |popup, window, cx| popup.focus_filter(window, cx));
    ui.cx.run_until_parked();
    ui.press("enter");
    assert!(ui.frosted(), "the open dropdown shows the acrylic");
    ui.type_text("link");
    ui.press("enter");
    assert!(!ui.frosted());
    // Only the link is listed, and the keys reach the search again.
    assert_eq!(ui.pressing_enter(), Some(ui.ids[3]));
}

#[gpui_kit::test]
fn pin_and_delete_by_key(cx: &mut TestAppContext) {
    let mut ui = Harness::open(cx, "ui-pin", three());
    ui.press("down down ctrl-p");
    // Pinning does not reorder.
    assert_eq!(ui.titles(), ["charlie", "bravo", "alpha"]);
    // The pinned entry stays selected.
    assert_eq!(ui.pressing_enter(), Some(ui.ids[0]));
    ui.press("ctrl-p up ctrl-backspace");
    assert_eq!(ui.titles(), ["charlie", "alpha"]);
    // The neighbor takes its place.
    assert_eq!(ui.pressing_enter(), Some(ui.ids[0]));

    // While searching, the keys edit the query instead.
    ui.type_text("char");
    ui.press("ctrl-backspace");
    assert_eq!(ui.titles().len(), 2);
    ui.press("delete");
    assert_eq!(ui.titles(), ["alpha"]);
}

#[gpui_kit::test]
fn number_keys_paste_that_row(cx: &mut TestAppContext) {
    let mut ui = Harness::open(cx, "ui-slots", three());
    ui.press("ctrl-3");
    let oldest = ui.ids[0];
    assert!(matches!(
        ui.event(),
        Event::Activate { id, how: Activation::Paste } if id == oldest
    ));
    ui.press("ctrl-9");
    assert!(ui.events.try_recv().is_err());
}

#[gpui_kit::test]
fn enter_can_copy_instead(cx: &mut TestAppContext) {
    let mut config = Config::default();
    config.general.primary_action = PrimaryAction::Copy;
    config.keys.insert("pin".into(), "ctrl-j".into());
    config.keys.insert("delete".into(), "not a key+".into());
    let mut ui = Harness::configured(cx, "ui-primary", three(), config);
    ui.press("enter");
    assert!(matches!(
        ui.event(),
        Event::Activate {
            how: Activation::Copy,
            ..
        }
    ));
    ui.press("ctrl-enter");
    assert!(matches!(
        ui.event(),
        Event::Activate {
            how: Activation::Paste,
            ..
        }
    ));
    ui.press("down ctrl-j");
    assert!(ui.store.lock().details(ui.ids[1]).unwrap().unwrap().pinned);
}

#[gpui_kit::test]
fn actions_panel_runs_what_is_chosen(cx: &mut TestAppContext) {
    let mut ui = Harness::open(cx, "ui-actions", three());
    ui.action("pin to");
    let pinned = ui.store.lock().details(ui.ids[2]).unwrap().unwrap().pinned;
    assert!(pinned);

    ui.press("ctrl-k");
    assert!(ui.frosted(), "the open panel shows the acrylic");
    // Escape closes the panel, and typing searches again.
    ui.press("escape");
    assert!(!ui.frosted());
    ui.type_text("brav");
    assert_eq!(ui.pressing_enter(), Some(ui.ids[1]));
    ui.press("escape");

    ui.action("settings");
    assert!(matches!(ui.event(), Event::OpenSettings));
    ui.action("pause");
    assert!(matches!(ui.event(), Event::TogglePrivate));
}

#[gpui_kit::test]
fn deleting_many_asks_first(cx: &mut TestAppContext) {
    let mut ui = Harness::open(cx, "ui-confirm", three());
    ui.action("delete all");
    // Ctrl+K closes the question without deleting.
    ui.press("ctrl-k");
    assert_eq!(ui.titles().len(), 3);

    ui.action("delete all");
    ui.press("enter");
    assert_eq!(ui.titles(), ["alpha"]);

    ui.action("clear history");
    ui.type_text("cancel");
    ui.press("enter");
    assert_eq!(ui.titles(), ["alpha"]);
    ui.action("clear history");
    ui.press("enter");
    assert!(ui.titles().is_empty());
}

#[gpui_kit::test]
fn colors_and_edits_become_text(cx: &mut TestAppContext) {
    let entries = vec![of_kind(Kind::Color, "#ff7a1a", 1)];
    let mut ui = Harness::open(cx, "ui-text", entries);
    ui.action("as rgb");
    assert!(matches!(
        ui.event(),
        Event::UseText { text, paste: false } if text == "rgb(255, 122, 26)"
    ));

    ui.press("ctrl-e");
    ui.type_text("!");
    ui.press("ctrl-enter");
    match ui.event() {
        Event::UseText { text, paste: true } => {
            assert!(text.contains('!') && text.len() == 8, "{text}")
        }
        other => panic!("unexpected {other:?}"),
    }
    ui.press("escape");
    assert!(ui.events.try_recv().is_err(), "Escape only left the editor");
    assert_eq!(ui.pressing_enter(), Some(ui.ids[0]));
}

#[gpui_kit::test]
fn settings_window_draws(cx: &mut TestAppContext) {
    let scratch = Scratch::new("ui-settings");
    let config = SharedConfig::load(scratch.0.join("config.toml"));
    let store = SharedStore::new(Store::open(&scratch.0).unwrap());
    let (events, _received) = async_channel::unbounded();
    cx.update(|cx| {
        gpui_kit::init(cx);
        settings::init(config.clone(), store, events, cx);
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| settings::SettingsView);
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    // Merely drawing the fields must not change anything.
    assert_eq!(config.get(), Config::default());
}
