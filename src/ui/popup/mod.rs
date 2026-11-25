//! The popup: search and type filter, the history list, a preview, the footer
//! and the actions panel. It is driven by keys that arrive while another app
//! keeps focus.

mod commands;
mod keys;
mod menu;

use std::time::{Duration, Instant};

use async_channel::Sender;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    command::{Command, CommandGroup, CommandItem, CommandState},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    kbd::Kbd,
    list::{List, ListEvent, ListState},
    searchable_list::SearchableVec,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub use self::keys::bind_keys;
use self::{
    keys::*,
    menu::{Menu, Pending},
};
use super::{
    file_facts,
    history_list::{self, HistoryDelegate},
    preview::{self, Selected},
    type_filter::{FILTERS, TypeFilter},
    when,
};
use crate::{
    config::{Backdrop, Config, PrimaryAction},
    platform::{Activation, Event, animations_enabled},
    store::{Kind, Results, SharedStore},
};

const CONTEXT: &str = "ClipboardPopup";
const SEARCH_CONTEXT: &str = "ClipboardSearch";
const PAGE: isize = 8;
/// Entries loaded at once; the list only draws the visible ones.
const LIMIT: usize = 5000;
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const APPEAR: Duration = Duration::from_millis(120);

/// The settings the popup draws itself by.
#[derive(Clone, Copy)]
struct Look {
    primary: PrimaryAction,
    preview_pane: bool,
    source_icon: bool,
    backdrop: Backdrop,
    paused: bool,
    animations: bool,
}

impl Look {
    fn of(config: &Config) -> Self {
        Self {
            primary: config.general.primary_action,
            preview_pane: config.appearance.preview_pane,
            source_icon: config.appearance.show_source_icon,
            backdrop: config.appearance.backdrop,
            paused: config.privacy.private_mode,
            animations: config.appearance.animations && animations_enabled(),
        }
    }
}

pub struct Popup {
    search: Entity<InputState>,
    filter: Entity<SelectState<SearchableVec<TypeFilter>>>,
    list: Entity<ListState<HistoryDelegate>>,
    selected: Option<Selected>,
    /// The actions panel, while open.
    actions: Option<Entity<CommandState>>,
    pending: Option<Pending>,
    /// The text being edited before pasting.
    editor: Option<Entity<TextareaState>>,
    /// Decoded pictures, dropped each time the popup reopens.
    images: Entity<RetainAllImageCache>,
    last_click: Option<(i64, Instant)>,
    /// Counts openings, so each one animates anew.
    openings: usize,
    store: SharedStore,
    look: Look,
    events: Sender<Event>,
    _subscriptions: [Subscription; 4],
}

impl Popup {
    pub fn new(
        store: SharedStore,
        config: &Config,
        events: Sender<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type to filter entries…"));
        let filter = cx.new(|cx| {
            // A plain `Vec` would ignore what is typed into the dropdown.
            let filters = SearchableVec::new(FILTERS.to_vec());
            SelectState::new(filters, Some(IndexPath::default()), window, cx).searchable(true)
        });
        let list = cx.new(|cx| ListState::new(HistoryDelegate::default(), window, cx));

        let subscriptions = [
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.refresh(false, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &filter,
                window,
                |this, _, _: &SelectEvent<SearchableVec<TypeFilter>>, window, cx| {
                    this.refresh(false, window, cx);
                },
            ),
            // The dropdown takes focus; typing belongs to the search again.
            cx.subscribe_in(&filter, window, |this, _, _: &DismissEvent, window, cx| {
                this.focus_search(window, cx);
            }),
            cx.subscribe_in(&list, window, |this, _, event: &ListEvent, window, cx| {
                if let ListEvent::Confirm(index) = event {
                    this.clicked(*index, window, cx);
                }
            }),
        ];

        Self {
            search,
            filter,
            list,
            selected: None,
            actions: None,
            pending: None,
            editor: None,
            images: RetainAllImageCache::new(cx),
            last_click: None,
            openings: 0,
            store,
            look: Look::of(config),
            events,
            _subscriptions: subscriptions,
        }
    }

    /// Takes over changed settings.
    pub fn apply(&mut self, config: &Config, cx: &mut Context<Self>) {
        self.look = Look::of(config);
        self.load(self.selected_id(), cx);
    }

    /// Prepares a fresh view for the next time the popup opens.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.openings = self.openings.wrapping_add(1);
        self.last_click = None;
        self.actions = None;
        self.pending = None;
        self.editor = None;
        self.images
            .update(cx, |images, cx| images.clear(window, cx));
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.filter.update(cx, |filter, cx| {
            filter.set_selected_index(Some(IndexPath::default()), window, cx);
        });
        self.focus_search(window, cx);
        self.refresh(false, window, cx);
    }

    /// Opens the way for keys to reach the type dropdown, as a click does.
    #[cfg(test)]
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter
            .update(cx, |filter, cx| filter.focus(window, cx));
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }

    /// Runs the search again. With `keep`, the selected entry stays selected.
    pub fn refresh(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value();
        let kinds = self
            .filter
            .read(cx)
            .selected_value()
            .map_or(&[][..], |filter| filter.kinds());
        let results = self
            .store
            .lock()
            .search(&query, kinds, LIMIT)
            .unwrap_or_else(|error| {
                tracing::error!(error = format!("{error:#}"), "search failed");
                Results::default()
            });
        // Counts only: the query and the titles are clipboard content.
        tracing::debug!(
            matches = results.matches.len(),
            similar = results.similar.len(),
            "results"
        );

        let kept = keep.then(|| self.selected_id()).flatten();
        let target = self.list.update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            delegate.filtered = !query.is_empty() || !kinds.is_empty();
            delegate.sections = history_list::sections(results, when::now().date());
            let target = kept
                .and_then(|id| delegate.position_of(id).map(|index| (index, id)))
                .or_else(|| delegate.nth(0));
            list.set_selected_index(target.map(|(index, _)| index), window, cx);
            // The first index also scrolls its section header into view.
            let scroll_to = target.map_or_else(IndexPath::default, |(index, _)| index);
            list.scroll_to_item(scroll_to, ScrollStrategy::Top, window, cx);
            cx.notify();
            target
        });
        self.load(target.map(|(_, id)| id), cx);
    }

    fn load(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        // Reloading the same entry keeps how it is being viewed.
        let show_source = self
            .selected
            .as_ref()
            .is_some_and(|selected| selected.show_source && Some(selected.id) == id);
        self.selected = id.and_then(|id| {
            // Bound first, so reading the disk happens without the lock.
            let details = self.store.lock().details(id);
            let mut details = details
                .inspect_err(|error| {
                    tracing::error!(error = format!("{error:#}"), "could not load an entry")
                })
                .ok()
                .flatten()?;
            if !self.look.source_icon {
                details.source_icon = None;
            }
            let file = (details.kind == Kind::Files && details.lines == 1)
                .then(|| details.text.as_deref().map(file_facts::read))
                .flatten();
            Some(Selected {
                id,
                details,
                file,
                show_source,
            })
        });
        tracing::debug!(id, "selected");
        cx.notify();
    }

    fn selected_id(&self) -> Option<i64> {
        self.selected.as_ref().map(|selected| selected.id)
    }

    pub fn toggle_source(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = &mut self.selected {
            selected.show_source = !selected.show_source;
            cx.notify();
        }
    }

    /// Asks the app to act on something the popup cannot do itself.
    pub fn send(&self, event: Event) {
        // Fails only during shutdown.
        let _ = self.events.try_send(event);
    }

    /// Selects the n-th row, or the last one if there are fewer.
    fn select_nth(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.list.update(cx, |list, cx| {
            let delegate = list.delegate();
            let last = delegate.len().checked_sub(1)?;
            let target = delegate.nth(n.min(last))?;
            list.set_selected_index(Some(target.0), window, cx);
            list.scroll_to_item(target.0, ScrollStrategy::Top, window, cx);
            cx.notify();
            Some(target)
        });
        if let Some((_, id)) = target {
            self.load(Some(id), cx);
        }
    }

    /// Moves by `step` rows. Single steps wrap around the ends.
    fn move_selection(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.selected_id();
        let list = self.list.read(cx);
        let delegate = list.delegate();
        let count = delegate.len() as isize;
        if count == 0 {
            return;
        }
        let from = current.and_then(|id| delegate.ordinal(id)).unwrap_or(0) as isize;
        let to = if step.abs() == 1 {
            (from + step).rem_euclid(count)
        } else {
            from.saturating_add(step).clamp(0, count - 1)
        };
        self.select_nth(to as usize, window, cx);
    }

    /// Home and End move the text cursor while there is a query.
    fn jump(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).value().is_empty() {
            self.move_selection(step, window, cx);
        } else {
            cx.propagate();
        }
    }

    /// A click selects, a second click uses the entry.
    fn clicked(&mut self, index: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self
            .list
            .read(cx)
            .delegate()
            .entry(index)
            .map(|entry| entry.id)
        else {
            return;
        };
        self.close_actions(window, cx);
        self.focus_search(window, cx);
        let double = self
            .last_click
            .is_some_and(|(last, at)| last == id && at.elapsed() < DOUBLE_CLICK);
        self.last_click = Some((id, Instant::now()));
        self.load(Some(id), cx);
        if double {
            self.activate(self.primary());
        }
    }

    /// What Enter does.
    fn primary(&self) -> Activation {
        match self.look.primary {
            PrimaryAction::Paste => Activation::Paste,
            PrimaryAction::Copy => Activation::Copy,
        }
    }

    /// What Ctrl+Enter does.
    fn secondary(&self) -> Activation {
        match self.look.primary {
            PrimaryAction::Paste => Activation::Copy,
            PrimaryAction::Copy => Activation::Paste,
        }
    }

    fn activate(&mut self, how: Activation) {
        if let Some(id) = self.selected_id() {
            self.send(Event::Activate { id, how });
        }
    }

    /// Runs an activation chosen in the panel or by key.
    fn choose(&mut self, how: Activation, window: &mut Window, cx: &mut Context<Self>) {
        self.close_actions(window, cx);
        self.activate(how);
    }

    fn cycle_type(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |filter, cx| {
            let current = filter.selected_index(cx).map_or(0, |index| index.row) as isize;
            let next = (current + step).rem_euclid(FILTERS.len() as isize) as usize;
            filter.set_selected_index(Some(IndexPath::default().row(next)), window, cx);
            cx.notify();
        });
        self.refresh(false, window, cx);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.actions.is_some() {
            self.close_actions(window, cx);
        } else if self.search.read(cx).value().is_empty() {
            self.send(Event::Hide);
        } else {
            // The first Escape clears the search, the second closes.
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.refresh(false, window, cx);
        }
    }

    /// Deleting keys edit the query while there is one.
    fn delete_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.actions.is_none() && !self.search.read(cx).value().is_empty() {
            cx.propagate();
        } else {
            self.delete_selected(window, cx);
        }
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .key_context(SEARCH_CONTEXT)
                    .flex_1()
                    .text_lg()
                    .child(Input::new(&self.search).appearance(false)),
            )
            .child(div().w(px(180.)).child(Select::new(&self.filter)))
    }

    fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(editor) = &self.editor {
            return div()
                .key_context("ClipboardEditor")
                .flex_1()
                .min_h_0()
                .p_3()
                .child(Textarea::new(editor).h_full())
                .into_any_element();
        }

        // Taken out while rendering, which needs the context mutably.
        let selected = self.selected.take();
        let preview = selected
            .as_ref()
            .filter(|_| self.look.preview_pane)
            .map(|selected| preview::preview(selected, cx).into_any_element());
        self.selected = selected;

        let border = cx.theme().border;
        let split = self.look.preview_pane;
        h_flex()
            .flex_1()
            .min_h_0()
            .items_stretch()
            // Clicks here must not move focus away from the search.
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .child(
                div()
                    .px_2()
                    .map(|pane| {
                        if split {
                            pane.w(relative(0.4))
                                .flex_shrink_0()
                                .border_r_1()
                                .border_color(border)
                        } else {
                            pane.flex_1()
                        }
                    })
                    .child(List::new(&self.list)),
            )
            .when(split, |body| {
                body.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when_some(preview, |pane, preview| pane.child(preview)),
                )
            })
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = |id: &'static str, label: &'static str, keys: &'static str| {
            Button::new(id).ghost().small().child(
                h_flex()
                    .gap_2()
                    .child(label)
                    .children(Keystroke::parse(keys).ok().map(Kbd::new)),
            )
        };
        let buttons = h_flex().gap_2();
        let buttons = if self.editor.is_some() {
            buttons
                .child(
                    hint("paste-edited", "Paste edited", "ctrl-enter")
                        .on_click(cx.listener(|this, _, _, cx| this.paste_edited(cx))),
                )
                .child(
                    hint("cancel-edit", "Cancel", "escape")
                        .on_click(cx.listener(|this, _, window, cx| this.cancel_edit(window, cx))),
                )
        } else {
            let primary = match self.look.primary {
                PrimaryAction::Paste => "Paste",
                PrimaryAction::Copy => "Copy",
            };
            buttons
                .child(
                    hint("primary", primary, "enter")
                        .on_click(cx.listener(|this, _, _, _| this.activate(this.primary()))),
                )
                .child(
                    hint("actions", "Actions", "ctrl-k").on_click(
                        cx.listener(|this, _, window, cx| this.toggle_actions(window, cx)),
                    ),
                )
        };

        let theme = cx.theme();
        h_flex()
            .flex_shrink_0()
            .justify_between()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(theme.border)
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::ClipboardList).small())
                    .child("Clipboard History")
                    .when(self.look.paused, |left| {
                        left.child(div().text_color(theme.warning).child("· Capture paused"))
                    }),
            )
            .child(buttons)
    }

    fn panel(&self, state: Entity<CommandState>, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = menu::items(&Menu {
            selected: self.selected.as_ref(),
            primary: self.look.primary,
            paused: self.look.paused,
            pending: self.pending.as_ref(),
        });
        let popup = cx.entity();
        let mut command = Command::new(&state)
            .placeholder("Search actions…")
            .max_h(px(260.))
            .on_cancel(move |window, cx| {
                popup.update(cx, |popup, cx| popup.close_actions(window, cx));
            });
        for (index, group) in groups.into_iter().enumerate() {
            if index > 0 {
                command = command.separator();
            }
            command = command.group(CommandGroup::new().items(group.into_iter().map(|item| {
                CommandItem::new()
                    .label(item.label)
                    .icon(Icon::new(item.icon))
                    .action(item.action)
            })));
        }

        div()
            .key_context("ClipboardActions")
            .absolute()
            .right_2()
            .bottom(px(48.))
            .w(px(340.))
            .occlude()
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_actions(window, cx)))
            .child(command)
    }
}

impl Render for Popup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.header(cx).into_any_element();
        let body = self.body(cx);
        let footer = self.footer(cx).into_any_element();
        let panel = self
            .actions
            .clone()
            .map(|state| self.panel(state, cx).into_any_element());
        let theme = cx.theme();
        let background = match self.look.backdrop {
            Backdrop::Acrylic => theme.background.opacity(0.72),
            Backdrop::Solid => theme.background,
        };

        macro_rules! on {
            ($view:ident, $action:ty, $handler:expr) => {
                $view.on_action(
                    cx.listener(move |this: &mut Self, _: &$action, window, cx| {
                        let handler: fn(&mut Self, &mut Window, &mut Context<Self>) = $handler;
                        handler(this, window, cx)
                    }),
                )
            };
        }

        let view = v_flex().key_context(CONTEXT).relative();
        let view = on!(view, SelectPrevious, |p, w, cx| p.move_selection(-1, w, cx));
        let view = on!(view, SelectNext, |p, w, cx| p.move_selection(1, w, cx));
        let view = on!(view, PageUp, |p, w, cx| p.move_selection(-PAGE, w, cx));
        let view = on!(view, PageDown, |p, w, cx| p.move_selection(PAGE, w, cx));
        let view = on!(view, SelectFirst, |p, w, cx| p.jump(isize::MIN / 2, w, cx));
        let view = on!(view, SelectLast, |p, w, cx| p.jump(isize::MAX / 2, w, cx));
        let view = on!(view, Paste, |p, w, cx| p.choose(p.primary(), w, cx));
        let view = on!(view, Copy, |p, w, cx| p.choose(p.secondary(), w, cx));
        let view = on!(view, PastePlain, |p, w, cx| p.choose(
            Activation::PastePlain,
            w,
            cx
        ));
        let view = on!(view, NextType, |p, w, cx| p.cycle_type(1, w, cx));
        let view = on!(view, PreviousType, |p, w, cx| p.cycle_type(-1, w, cx));
        let view = on!(view, Dismiss, Self::dismiss);
        let view = on!(view, ToggleActions, Self::toggle_actions);
        let view = on!(view, TogglePin, Self::toggle_pin);
        let view = on!(view, DeleteEntry, Self::delete_key);
        let view = on!(view, DeleteFromApp, Self::ask_delete_from_app);
        let view = on!(view, ClearHistory, Self::ask_clear);
        let view = on!(view, ConfirmDelete, Self::confirm_delete);
        let view = on!(view, CancelDelete, Self::close_actions);
        let view = on!(view, OpenEntry, Self::open_entry);
        let view = on!(view, RevealEntry, Self::reveal_entry);
        let view = on!(view, CopyPath, Self::copy_path);
        let view = on!(view, CopyHex, |p, w, cx| p.copy_color(
            crate::color::Color::to_hex,
            w,
            cx
        ));
        let view = on!(view, CopyRgb, |p, w, cx| p.copy_color(
            crate::color::Color::to_rgb,
            w,
            cx
        ));
        let view = on!(view, CopyHsl, |p, w, cx| p.copy_color(
            crate::color::Color::to_hsl,
            w,
            cx
        ));
        let view = on!(view, SaveImage, Self::save_image);
        let view = on!(view, CopyImageText, Self::copy_image_text);
        let view = on!(view, EditEntry, Self::edit_entry);
        let view = on!(view, PasteEdited, |p, _, cx| p.paste_edited(cx));
        let view = on!(view, CancelEdit, Self::cancel_edit);
        let view = on!(view, IgnoreApp, Self::ignore_app);
        let view = on!(view, TogglePrivate, |p, w, cx| p.request(
            Event::TogglePrivate,
            w,
            cx
        ));
        let view = on!(view, OpenSettings, |p, w, cx| p.request(
            Event::OpenSettings,
            w,
            cx
        ));
        let view = on!(view, QuitApp, |p, w, cx| p.request(Event::Quit, w, cx));

        let animations = self.look.animations;
        let openings = self.openings;
        let view = view
            .on_action(cx.listener(|this, PasteNth(n): &PasteNth, window, cx| {
                let target = this.list.read(cx).delegate().nth(*n);
                if let Some((_, id)) = target {
                    this.select_nth(*n, window, cx);
                    this.send(Event::Activate {
                        id,
                        how: this.primary(),
                    });
                }
            }))
            .image_cache(self.images.clone())
            .size_full()
            .bg(background)
            .text_color(theme.foreground)
            .child(header)
            .child(body)
            .child(footer)
            .children(panel);

        if !animations {
            return view.into_any_element();
        }
        view.with_animation(
            ("appear", openings),
            Animation::new(APPEAR).with_easing(ease_out_quint()),
            |view, progress| view.opacity(progress),
        )
        .into_any_element()
    }
}
