//! Headless tests of the popup: the real components, driven by keys, without a
//! desktop.

// Not `gpui_kit::*`: GPUI exports a `test` attribute of its own.
use async_channel::Receiver;
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use image::{Rgba, RgbaImage};

use super::popup::{self, Popup};
use crate::{
    config::Backdrop,
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
        let scratch = Scratch::new(name);
        let store = SharedStore::new(Store::open(&scratch.0).unwrap());
        let ids = entries
            .iter()
            .map(|entry| store.lock().record(entry).unwrap().id)
            .collect();
        cx.update(|cx| {
            gpui_kit::init(cx);
            popup::bind_keys(cx);
        });

        let (sender, events) = async_channel::unbounded();
        let mut view = None;
        let shared = store.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let popup = cx.new(|cx| Popup::new(shared, Backdrop::Solid, sender, window, cx));
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
