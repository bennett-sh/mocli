use std::sync::Arc;

use anyhow::Result;
use async_channel::Sender;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use super::Event;

const SHOW: &str = "show";
const CONFIG: &str = "config";
const QUIT: &str = "quit";

/// Must be created on the thread that runs the UI message loop. Dropping the
/// returned value removes the icon.
pub fn create(events: Sender<Event>) -> Result<TrayIcon> {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id(SHOW, "Show clipboard history", true, None),
        &MenuItem::with_id(CONFIG, "Edit config", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit mocli", true, None),
    ])?;

    let menu_events = events.clone();
    MenuEvent::set_event_handler(Some(Arc::new(move |event: MenuEvent| {
        let event = match event.id.as_ref() {
            SHOW => Event::Show,
            CONFIG => Event::OpenConfig,
            QUIT => Event::Quit,
            _ => return,
        };
        let _ = menu_events.try_send(event);
    })));

    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = events.try_send(Event::Show);
        }
    }));

    Ok(TrayIconBuilder::new()
        .with_tooltip("mocli: clipboard history")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon()?)
        .build()?)
}

/// A small clipboard, drawn in code until the app has artwork.
fn icon() -> Result<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let clip = (11..21).contains(&x) && (2..8).contains(&y);
            let board = (5..27).contains(&x) && (5..30).contains(&y);
            let line = (9..23).contains(&x) && matches!(y, 13 | 14 | 18 | 19 | 23 | 24);
            let pixel: [u8; 4] = if clip {
                [200, 200, 200, 255]
            } else if line {
                [255, 255, 255, 255]
            } else if board {
                [255, 122, 26, 255]
            } else {
                [0, 0, 0, 0]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    Ok(Icon::from_rgba(rgba, SIZE, SIZE)?)
}
