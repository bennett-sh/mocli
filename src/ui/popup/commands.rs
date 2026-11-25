//! What the popup's actions do.

use std::path::PathBuf;

use gpui_kit::component::{command::CommandState, input::TextareaState};
use gpui_kit::*;

use super::{Popup, menu::Pending};
use crate::{
    color::Color,
    platform::{Event, OpenTarget},
    store::Kind,
};

impl Popup {
    pub(super) fn toggle_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.actions.is_some() {
            self.close_actions(window, cx);
        } else if self.editor.is_none() {
            self.open_actions(window, cx);
        }
    }

    /// A fresh panel, so it starts without a query.
    fn open_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        self.actions = Some(state);
        cx.notify();
    }

    pub(super) fn close_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        if self.actions.take().is_some() {
            self.focus_search(window, cx);
            cx.notify();
        }
    }

    pub(super) fn toggle_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        let Some((id, pinned)) = self
            .selected
            .as_ref()
            .map(|selected| (selected.id, selected.details.pinned))
        else {
            return;
        };
        let result = self.store.lock().set_pinned(id, !pinned);
        report("pin", result);
        self.refresh(true, window, cx);
    }

    /// Deletes the selected entry and selects its neighbor.
    pub(super) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        let Some(id) = self.selected_id() else {
            return;
        };
        let position = self.list.read(cx).delegate().ordinal(id).unwrap_or(0);
        let result = self.store.lock().delete(id);
        report("delete", result);
        self.refresh(false, window, cx);
        self.select_nth(position, window, cx);
    }

    pub(super) fn ask_delete_from_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((exe, name)) = self.selected.as_ref().and_then(|selected| {
            Some((
                selected.details.source_exe.clone()?,
                selected.details.source_name.clone()?,
            ))
        }) else {
            return;
        };
        let count = self.store.lock().count_from_source(&exe);
        match count {
            Ok(count) => {
                self.pending = Some(Pending::FromApp { exe, name, count });
                self.open_actions(window, cx);
            }
            Err(error) => report::<()>("count", Err(error)),
        }
    }

    pub(super) fn ask_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = Some(Pending::Everything);
        self.open_actions(window, cx);
    }

    pub(super) fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let result = match pending {
            Pending::FromApp { exe, .. } => self.store.lock().delete_from_source(&exe),
            Pending::Everything => self.store.lock().clear_unpinned(),
        };
        report("delete", result);
        self.close_actions(window, cx);
        self.refresh(false, window, cx);
    }

    /// The selected entry's text, one path per line for files.
    fn selected_text(&self) -> Option<(Kind, String)> {
        let id = self.selected_id()?;
        let payload = self.store.lock().payload(id);
        match payload {
            Ok(payload) => {
                let payload = payload?;
                Some((payload.kind, payload.text?))
            }
            Err(error) => {
                report("read", Err::<(), _>(error));
                None
            }
        }
    }

    pub(super) fn open_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        let Some(selected) = &self.selected else {
            return;
        };
        let text = selected.details.text.as_deref().unwrap_or_default().trim();
        let target = match selected.details.kind {
            // Classified as a web address or an email address when captured.
            Kind::Link if text.starts_with("www.") => {
                Some(OpenTarget::Address(format!("https://{text}")))
            }
            Kind::Link => Some(OpenTarget::Address(text.to_owned())),
            Kind::Email => Some(OpenTarget::Address(format!(
                "mailto:{}",
                text.trim_start_matches("mailto:")
            ))),
            Kind::Image | Kind::Svg => selected.details.image.clone().map(OpenTarget::File),
            Kind::Files => text.lines().next().map(|p| OpenTarget::File(p.into())),
            Kind::Text | Kind::Code | Kind::Color => None,
        };
        if let Some(target) = target {
            self.send(Event::Open(target));
        }
    }

    pub(super) fn reveal_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        if let Some((Kind::Files, paths)) = self.selected_text()
            && let Some(first) = paths.lines().next()
        {
            self.send(Event::Reveal(PathBuf::from(first)));
        }
    }

    pub(super) fn copy_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        if let Some((Kind::Files, paths)) = self.selected_text() {
            self.send(Event::UseText {
                text: paths,
                paste: false,
            });
        }
    }

    pub(super) fn copy_color(
        &mut self,
        notation: fn(Color) -> String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_actions(window, cx);
        if let Some((Kind::Color, text)) = self.selected_text()
            && let Some(color) = Color::parse(&text)
        {
            self.send(Event::UseText {
                text: notation(color),
                paste: false,
            });
        }
    }

    pub(super) fn copy_image_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        if let Some(text) = self
            .selected
            .as_ref()
            .and_then(|selected| selected.details.ocr_text.clone())
        {
            self.send(Event::UseText { text, paste: false });
        }
    }

    pub(super) fn save_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        if let Some(selected) = &self.selected
            && selected.details.kind == Kind::Image
            && let Some(path) = selected.details.image.clone()
        {
            self.send(Event::SaveImage(path));
        }
    }

    pub(super) fn edit_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        let Some((kind, text)) = self.selected_text() else {
            return;
        };
        if kind == Kind::Files {
            return;
        }
        let editor = cx.new(|cx| TextareaState::new(window, cx).default_value(text));
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        self.editor = Some(editor);
        cx.notify();
    }

    pub(super) fn paste_edited(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &self.editor {
            let text = editor.read(cx).value().to_string();
            self.send(Event::UseText { text, paste: true });
        }
    }

    pub(super) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.take().is_some() {
            self.focus_search(window, cx);
            cx.notify();
        }
    }

    pub(super) fn ignore_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        if let Some(exe) = self
            .selected
            .as_ref()
            .and_then(|selected| selected.details.source_exe.clone())
        {
            self.send(Event::IgnoreApp(exe));
        }
    }

    /// Closes the panel and hands an app-wide request on.
    pub(super) fn request(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        self.send(event);
    }
}

fn report<T>(operation: &str, result: anyhow::Result<T>) {
    if let Err(error) = result {
        tracing::error!(
            error = format!("{error:#}"),
            operation,
            "history change failed"
        );
    }
}
