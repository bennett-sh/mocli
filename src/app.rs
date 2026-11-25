use std::{env, fs, path::PathBuf, sync::mpsc, time::Duration};

use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;

use crate::{
    config::{self, Backdrop, Config, FocusMode, Placement, SharedConfig, ThemeChoice},
    ingest, paths,
    platform::{
        Activation, Event, OpenTarget, caret, clipboard,
        clipboard_write::{self, Content},
        cursor, foreground, hook, input, shell, source_app,
        tray::{self, Tray},
        window::PopupWindow,
    },
    reuse,
    store::{SharedStore, Store},
    ui::{
        assets::Assets,
        popup::{self, Popup},
        settings::SettingsWindow,
    },
};

/// Gives other clipboard tools a moment to let go before the app pastes.
const PASTE_DELAY: Duration = Duration::from_millis(50);
/// How long the pasting app has to read the clipboard before the previous
/// content returns.
const RESTORE_DELAY: Duration = Duration::from_millis(500);

pub fn run(config: SharedConfig) -> Result<()> {
    let store =
        SharedStore::new(Store::open(&paths::data_dir()?).context("failed to open the history")?);
    let (events, incoming) = async_channel::unbounded();

    let changes = events.clone();
    config.on_change(move || {
        // Fails only during shutdown.
        let _ = changes.try_send(Event::ConfigChanged);
    });
    config.watch()?;

    let (snapshots, copies) = mpsc::channel();
    let changes = events.clone();
    ingest::spawn(store.clone(), config.clone(), copies, move || {
        let _ = changes.try_send(Event::HistoryChanged);
    })?;
    clipboard::spawn(config.clone(), snapshots)?;

    let initial = config.get();
    hook::spawn(
        initial.general.hotkey,
        !takes_focus(&initial),
        events.clone(),
    )?;
    let locator = caret::Locator::spawn()?;

    gpui_kit::application()
        .with_assets(Assets)
        // The popup is usually hidden; only "Quit" ends the app.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_theme(initial.appearance.theme, cx);
            popup::bind_keys(&initial.keys, cx);

            if let Err(error) = start(config, store, locator, events, incoming, cx) {
                tracing::error!(error = format!("{error:#}"), "startup failed");
                cx.quit();
            }
        });
    Ok(())
}

fn takes_focus(config: &Config) -> bool {
    config.general.focus_mode == FocusMode::Activate
}

fn apply_theme(choice: ThemeChoice, cx: &mut App) {
    match choice {
        ThemeChoice::System => Theme::sync_system_appearance(None, cx),
        ThemeChoice::Light => Theme::change(ThemeMode::Light, None, cx),
        ThemeChoice::Dark => Theme::change(ThemeMode::Dark, None, cx),
    }
}

fn window_background(backdrop: Backdrop) -> WindowBackgroundAppearance {
    match backdrop {
        Backdrop::Acrylic => WindowBackgroundAppearance::Blurred,
        Backdrop::Solid => WindowBackgroundAppearance::Opaque,
    }
}

/// Owns the popup window and carries out events.
///
/// Its window calls happen between `update`s, while `App` is not borrowed.
struct Controller {
    window: AnyWindowHandle,
    view: Entity<Popup>,
    popup: PopupWindow,
    store: SharedStore,
    config: SharedConfig,
    /// The settings in effect, to see what a change touches.
    applied: Config,
    locator: caret::Locator,
    tray: Tray,
    settings: SettingsWindow,
    visible: bool,
}

impl Controller {
    fn show(&mut self, cx: &mut AsyncApp) -> Result<()> {
        // First, so keys typed while the popup appears are not lost.
        hook::open();
        // Asked before the popup shows and perhaps takes focus.
        let anchor = match self.applied.general.placement {
            Placement::Caret => self.locator.locate(hook::target()),
            Placement::Cursor => cursor(),
            Placement::Centered => None,
        };
        let theme = self.applied.appearance.theme;
        if theme == ThemeChoice::System {
            // The system may have switched since the last time.
            cx.update(|cx| apply_theme(theme, cx));
        }
        self.with_view(cx, |view, window, cx| view.reset(window, cx))?;
        let (width, height) = self.applied.appearance.size.dimensions();
        let rect = self.popup.show(anchor, width, height)?;
        hook::set_popup_rect(Some(rect));
        self.visible = true;
        tracing::info!("popup shown");
        Ok(())
    }

    /// With `restore`, a popup that holds focus gives it back to the app.
    fn hide(&mut self, restore: bool) -> Result<()> {
        hook::close();
        hook::set_popup_rect(None);
        if std::mem::take(&mut self.visible) {
            tracing::info!("popup hidden");
            self.popup.hide(restore.then(hook::target))?;
        }
        Ok(())
    }

    /// Puts an entry on the clipboard and, unless only copying, pastes it into
    /// the app the popup was opened from.
    async fn activate(&mut self, id: i64, how: Activation, cx: &mut AsyncApp) -> Result<()> {
        let payload = self
            .store
            .lock()
            .payload(id)?
            .context("the entry no longer exists")?;
        let content = reuse::content(payload, how)?;
        self.deliver(content, how != Activation::Copy, Some(id), cx)
            .await?;
        tracing::info!(id, ?how, "entry used");
        Ok(())
    }

    async fn deliver(
        &mut self,
        content: Content,
        paste: bool,
        id: Option<i64>,
        cx: &mut AsyncApp,
    ) -> Result<()> {
        let target = hook::target();
        let previous = (paste && self.applied.general.restore_clipboard_after_paste)
            .then(|| clipboard::current(self.popup.handle()))
            .flatten()
            .and_then(reuse::restored);
        self.hide(true)?;
        clipboard_write::write(self.popup.handle(), &content)?;
        if let Some(id) = id {
            self.store.lock().mark_used(
                id,
                ingest::now_millis(),
                self.applied.general.move_to_top_on_reuse,
            )?;
        }
        if !paste {
            return Ok(());
        }

        cx.background_executor().timer(PASTE_DELAY).await;
        // Never paste into a window the user did not open the popup from.
        anyhow::ensure!(
            foreground() == target,
            "focus moved elsewhere, so the entry was only copied"
        );
        let keys = source_app::executable_of(target)
            .map(|exe| self.applied.paste.keys_for(&exe))
            .unwrap_or_default();
        input::paste(keys)?;

        if let Some(previous) = previous {
            cx.background_executor().timer(RESTORE_DELAY).await;
            clipboard_write::write(self.popup.handle(), &previous)?;
        }
        Ok(())
    }

    /// Brings the running app in line with changed settings.
    fn apply_config(&mut self, cx: &mut AsyncApp) -> Result<()> {
        let config = self.config.get();
        let before = std::mem::replace(&mut self.applied, config.clone());
        hook::configure(config.general.hotkey, !takes_focus(&config));
        self.tray.set_paused(config.privacy.private_mode);

        if takes_focus(&config) != takes_focus(&before)
            || config.appearance.size != before.appearance.size
        {
            self.hide(true)?;
            self.popup.set_takes_focus(takes_focus(&config))?;
        }
        if config.appearance.theme != before.appearance.theme {
            cx.update(|cx| apply_theme(config.appearance.theme, cx));
        }
        let visible = self.visible;
        self.with_view(cx, |view, window, cx| {
            window.set_background_appearance(window_background(config.appearance.backdrop));
            view.apply(&config, cx);
            if visible {
                view.refresh(true, window, cx);
            }
        })?;
        self.settings.refresh(cx);
        tracing::debug!("settings applied");
        Ok(())
    }

    /// Asks where to save a copy of a stored image.
    fn save_image(&mut self, source: PathBuf, cx: &mut AsyncApp) -> Result<()> {
        self.hide(false)?;
        let downloads = env::var_os("USERPROFILE")
            .map(|home| PathBuf::from(home).join("Downloads"))
            .context("no home directory")?;
        let chosen = cx.update(|cx| cx.prompt_for_new_path(&downloads, Some("image.png")));
        // The dialog stays open as long as the user likes.
        cx.spawn(async move |_| {
            let Ok(Ok(Some(destination))) = chosen.await else {
                return;
            };
            if let Err(error) = fs::copy(&source, &destination) {
                tracing::error!(%error, "could not save the image");
            }
        })
        .detach();
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

    /// Returns whether the app should keep running.
    async fn handle(&mut self, event: Event, cx: &mut AsyncApp) -> Result<bool> {
        match event {
            Event::Show => self.show(cx)?,
            Event::Hide => self.hide(true)?,
            Event::Dismiss => self.hide(false)?,
            Event::Key(keystroke) if self.visible => {
                self.window.update(cx, |_, window, cx| {
                    window.dispatch_keystroke(keystroke, cx);
                })?;
            }
            Event::Activate { id, how } if self.visible => self.activate(id, how, cx).await?,
            Event::UseText { text, paste } if self.visible => {
                let content = Content::Text {
                    text,
                    html: None,
                    rtf: None,
                };
                self.deliver(content, paste, None, cx).await?;
            }
            Event::HistoryChanged if self.visible => {
                self.with_view(cx, |view, window, cx| view.refresh(true, window, cx))?;
            }
            // The popup closed before these arrived, or reloads when shown.
            Event::Key(_)
            | Event::Activate { .. }
            | Event::UseText { .. }
            | Event::HistoryChanged => {}
            Event::Open(target) => {
                self.hide(false)?;
                cx.update(|cx| match target {
                    OpenTarget::Address(address) => cx.open_url(&address),
                    OpenTarget::File(path) => cx.open_with_system(&path),
                });
            }
            Event::Reveal(path) => {
                self.hide(false)?;
                cx.update(|cx| cx.reveal_path(&path));
            }
            Event::SaveImage(path) => self.save_image(path, cx)?,
            Event::IgnoreApp(exe) => {
                let name = config::file_name(&exe).to_owned();
                self.config.update(|config| {
                    if !config.privacy.ignores(&name) {
                        config.privacy.ignore_apps.push(name);
                    }
                })?;
            }
            Event::TogglePrivate => self.config.update(|config| {
                config.privacy.private_mode = !config.privacy.private_mode;
            })?,
            Event::ConfigChanged => self.apply_config(cx)?,
            Event::OpenSettings => {
                self.hide(false)?;
                self.settings.open(cx)?;
            }
            Event::OpenConfig => shell::edit_text_file(self.config.path())?,
            Event::Quit => {
                tracing::info!("quitting");
                cx.update(|cx| cx.quit());
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn start(
    config: SharedConfig,
    store: SharedStore,
    locator: caret::Locator,
    events: Sender<Event>,
    incoming: Receiver<Event>,
    cx: &mut App,
) -> Result<()> {
    let applied = config.get();
    let tray = tray::create(events.clone()).context("failed to create the tray icon")?;
    tray.set_paused(applied.privacy.private_mode);
    let settings = SettingsWindow::new(config.clone(), store.clone(), events.clone(), cx);
    let (window, view) = open_popup(&applied, store.clone(), events, cx)?;
    let popup = window.update(cx, |_, window, _| {
        PopupWindow::of(window, takes_focus(&applied))
    })??;
    let mut controller = Controller {
        window,
        view,
        popup,
        store,
        config,
        applied,
        locator,
        tray,
        settings,
        visible: false,
    };

    cx.spawn(async move |cx| {
        if let Err(error) = controller.popup.prepare() {
            tracing::error!(error = format!("{error:#}"), "could not prepare the popup");
            cx.update(|cx| cx.quit());
            return;
        }
        tracing::info!("ready");

        while let Ok(event) = incoming.recv().await {
            match controller.handle(event, cx).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::error!(error = format!("{error:#}"), "event failed");
                    // Never keep the keyboard captured for a broken popup.
                    if !controller.visible {
                        hook::close();
                    }
                }
            }
        }
    })
    .detach();

    Ok(())
}

/// Creates the popup hidden; it is reused every time it opens.
fn open_popup(
    config: &Config,
    store: SharedStore,
    events: Sender<Event>,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<Popup>)> {
    let (width, height) = config.appearance.size.dimensions();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            None,
            size(px(width), px(height)),
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
        window_background: window_background(config.appearance.backdrop),
        ..Default::default()
    };

    let mut view = None;
    let window = cx.open_window(options, |window, cx| {
        let popup = cx.new(|cx| Popup::new(store, config, events, window, cx));
        view = Some(popup.clone());
        // The popup paints its own translucent background over the backdrop.
        cx.new(|cx| Root::new(popup, window, cx).bg(transparent_black()))
    })?;
    Ok((window.into(), view.context("the popup view was not built")?))
}
