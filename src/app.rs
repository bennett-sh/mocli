use std::{sync::mpsc, time::Duration};

use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender};
use gpui_kit::component::{Root, Theme};
use gpui_kit::*;

use crate::{
    config::{Backdrop, Config},
    ingest, paths,
    platform::{
        Activation, Event, clipboard, clipboard_write, hook, input, shell, tray,
        window::PopupWindow,
    },
    reuse,
    store::{SharedStore, Store},
    ui::{
        assets::Assets,
        popup::{self, Popup},
    },
};

const POPUP_WIDTH: f32 = 760.0;
const POPUP_HEIGHT: f32 = 480.0;
/// Gives other clipboard tools a moment to let go before the app pastes.
const PASTE_DELAY: Duration = Duration::from_millis(50);

pub fn run(config: Config) -> Result<()> {
    let store =
        SharedStore::new(Store::open(&paths::data_dir()?).context("failed to open the history")?);
    let (events, incoming) = async_channel::unbounded();

    let (snapshots, copies) = mpsc::channel();
    let changes = events.clone();
    ingest::spawn(store.clone(), config.history, copies, move || {
        // Fails only during shutdown.
        let _ = changes.try_send(Event::HistoryChanged);
    })?;
    clipboard::spawn(config.privacy, snapshots)?;
    hook::spawn(config.general.hotkey, events.clone())?;

    let settings = Settings {
        backdrop: config.appearance.backdrop,
        move_to_top_on_reuse: config.general.move_to_top_on_reuse,
    };

    gpui_kit::application()
        .with_assets(Assets)
        // The popup is usually hidden; only "Quit" ends the app.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::sync_system_appearance(None, cx);
            popup::bind_keys(cx);

            if let Err(error) = start(settings, store, events, incoming, cx) {
                tracing::error!(error = format!("{error:#}"), "startup failed");
                cx.quit();
            }
        });
    Ok(())
}

#[derive(Clone, Copy)]
struct Settings {
    backdrop: Backdrop,
    move_to_top_on_reuse: bool,
}

/// Owns the popup window and carries out events.
///
/// Its window calls happen between `update`s, while `App` is not borrowed.
struct Controller {
    window: AnyWindowHandle,
    view: Entity<Popup>,
    popup: PopupWindow,
    store: SharedStore,
    settings: Settings,
    visible: bool,
}

impl Controller {
    fn show(&mut self, cx: &mut AsyncApp) -> Result<()> {
        hook::set_capture(true);
        let view = &self.view;
        self.window.update(cx, |_, window, cx| {
            view.update(cx, |view, cx| view.reset(window, cx));
        })?;
        let rect = self.popup.show_centered(POPUP_WIDTH, POPUP_HEIGHT)?;
        hook::set_popup_rect(Some(rect));
        self.visible = true;
        tracing::info!("popup shown");
        Ok(())
    }

    fn hide(&mut self) -> Result<()> {
        hook::set_capture(false);
        hook::set_popup_rect(None);
        if std::mem::take(&mut self.visible) {
            tracing::info!("popup hidden");
            self.popup.hide()?;
        }
        Ok(())
    }

    /// Puts an entry on the clipboard and, unless only copying, pastes it into
    /// the app that kept focus.
    async fn activate(&mut self, id: i64, how: Activation, cx: &mut AsyncApp) -> Result<()> {
        let payload = self
            .store
            .lock()
            .payload(id)?
            .context("the entry no longer exists")?;
        let content = reuse::content(payload, how)?;
        self.hide()?;
        clipboard_write::write(self.popup.handle(), &content)?;
        self.store.lock().mark_used(
            id,
            ingest::now_millis(),
            self.settings.move_to_top_on_reuse,
        )?;
        tracing::info!(id, ?how, "entry used");

        if how != Activation::Copy {
            cx.background_executor().timer(PASTE_DELAY).await;
            input::paste()?;
        }
        Ok(())
    }

    fn with_view(
        &self,
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut Popup, &mut Window, &mut Context<Popup>),
    ) -> Result<()> {
        self.window.update(cx, |_, window, cx| {
            self.view.update(cx, |view, cx| update(view, window, cx));
        })
    }
}

fn start(
    settings: Settings,
    store: SharedStore,
    events: Sender<Event>,
    incoming: Receiver<Event>,
    cx: &mut App,
) -> Result<()> {
    let tray = tray::create(events.clone()).context("failed to create the tray icon")?;
    let (window, view) = open_popup(settings.backdrop, store.clone(), events, cx)?;
    let popup = window.update(cx, |_, window, _| PopupWindow::of(window))??;
    let mut controller = Controller {
        window,
        view,
        popup,
        store,
        settings,
        visible: false,
    };

    cx.spawn(async move |cx| {
        // Keeps the tray icon alive as long as events are handled.
        let _tray = tray;
        if let Err(error) = popup.prepare() {
            tracing::error!(error = format!("{error:#}"), "could not prepare the popup");
            cx.update(|cx| cx.quit());
            return;
        }
        tracing::info!("ready");

        while let Ok(event) = incoming.recv().await {
            let result = match event {
                Event::Show => controller.show(cx),
                Event::Hide => controller.hide(),
                Event::Key(keystroke) if controller.visible => {
                    controller.window.update(cx, |_, window, cx| {
                        window.dispatch_keystroke(keystroke, cx);
                    })
                }
                Event::Activate { id, how } if controller.visible => {
                    controller.activate(id, how, cx).await
                }
                Event::HistoryChanged if controller.visible => {
                    controller.with_view(cx, |view, window, cx| view.refresh(true, window, cx))
                }
                // The popup closed before these arrived, or reloads when shown.
                Event::Key(_) | Event::Activate { .. } | Event::HistoryChanged => Ok(()),
                Event::OpenConfig => {
                    paths::config_file().and_then(|path| shell::edit_text_file(&path))
                }
                Event::Quit => {
                    tracing::info!("quitting");
                    cx.update(|cx| cx.quit());
                    break;
                }
            };
            if let Err(error) = result {
                tracing::error!(error = format!("{error:#}"), "event failed");
                // Never keep the keyboard captured for a broken popup.
                if !controller.visible {
                    hook::set_capture(false);
                }
            }
        }
    })
    .detach();

    Ok(())
}

/// Creates the popup hidden; it is reused every time it opens.
fn open_popup(
    backdrop: Backdrop,
    store: SharedStore,
    events: Sender<Event>,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<Popup>)> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            None,
            size(px(POPUP_WIDTH), px(POPUP_HEIGHT)),
            cx,
        )),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        // GPUI sees the popup as active, but never throttle it either way.
        inactive_frame_interval: None,
        window_background: match backdrop {
            Backdrop::Acrylic => WindowBackgroundAppearance::Blurred,
            Backdrop::Solid => WindowBackgroundAppearance::Opaque,
        },
        ..Default::default()
    };

    let mut view = None;
    let window = cx.open_window(options, |window, cx| {
        let popup = cx.new(|cx| Popup::new(store, backdrop, events, window, cx));
        view = Some(popup.clone());
        // The popup paints its own translucent background over the backdrop.
        cx.new(|cx| Root::new(popup, window, cx).bg(transparent_black()))
    })?;
    Ok((window.into(), view.context("the popup view was not built")?))
}
