use std::sync::Arc;

use anyhow::Result;
use async_channel::Sender;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use super::{Event, OpenTarget};
use crate::update;

const SHOW: &str = "show";
const PAUSE: &str = "pause";
const SETTINGS: &str = "settings";
const CONFIG: &str = "config";
const QUIT: &str = "quit";
const UPDATE: &str = "update";
const TOOLTIP: &str = "mocli: clipboard history";

/// The tray icon, which disappears when this is dropped.
pub struct Tray {
    icon: TrayIcon,
    menu: Menu,
    pause: CheckMenuItem,
    /// Shown at the top of the menu while a newer version is out.
    update: MenuItem,
    update_separator: PredefinedMenuItem,
    /// The version `update` offers.
    offered: Option<String>,
}

impl Tray {
    pub fn set_paused(&self, paused: bool) {
        self.pause.set_checked(paused);
    }

    /// Offers `version` in the menu, or nothing with `None`.
    pub fn set_update(&mut self, version: Option<&str>) {
        if self.offered.as_deref() == version {
            return;
        }
        let result = (|| -> Result<()> {
            if self.offered.is_some() {
                self.menu.remove(&self.update)?;
                self.menu.remove(&self.update_separator)?;
            }
            if let Some(version) = version {
                self.update.set_text(format!("Download mocli {version}…"));
                self.menu
                    .insert_items(&[&self.update, &self.update_separator], 0)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(
                error = format!("{error:#}"),
                "could not show the update in the tray"
            );
        }
        let tooltip = match version {
            Some(version) => format!("{TOOLTIP} (mocli {version} is available)"),
            None => TOOLTIP.to_owned(),
        };
        let _ = self.icon.set_tooltip(Some(tooltip));
        self.offered = version.map(str::to_owned);
    }
}

/// Must be created on the thread that runs the UI message loop.
pub fn create(events: Sender<Event>) -> Result<Tray> {
    let pause = CheckMenuItem::with_id(PAUSE, "Pause capture", true, false, None);
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id(SHOW, "Show clipboard history", true, None),
        &pause,
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(SETTINGS, "Settings…", true, None),
        &MenuItem::with_id(CONFIG, "Edit config file", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit mocli", true, None),
    ])?;

    let menu_events = events.clone();
    MenuEvent::set_event_handler(Some(Arc::new(move |event: MenuEvent| {
        let event = match event.id.as_ref() {
            SHOW => Event::Show,
            PAUSE => Event::TogglePrivate,
            SETTINGS => Event::OpenSettings,
            CONFIG => Event::OpenConfig,
            QUIT => Event::Quit,
            UPDATE => Event::Open(OpenTarget::Address(update::RELEASES_PAGE.into())),
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

    let icon = TrayIconBuilder::new()
        .with_tooltip(TOOLTIP)
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(false)
        .with_icon(icon()?)
        .build()?;
    Ok(Tray {
        icon,
        menu,
        pause,
        update: MenuItem::with_id(UPDATE, "", true, None),
        update_separator: PredefinedMenuItem::separator(),
        offered: None,
    })
}

/// A small clipboard, drawn in code like the application icon.
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
