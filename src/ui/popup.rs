//! The popup: search and type filter, the history list, a preview and the
//! footer. It is driven by keys that arrive while another app keeps focus.

use std::time::{Duration, Instant};

use async_channel::Sender;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    kbd::Kbd,
    list::{List, ListEvent, ListState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    file_facts,
    history_list::{self, HistoryDelegate},
    preview::{self, Selected},
    type_filter::{FILTERS, TypeFilter},
    when,
};
use crate::{
    config::Backdrop,
    platform::{Activation, Event},
    store::{Kind, Results, SharedStore},
};

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
        Dismiss
    ]
);

const CONTEXT: &str = "ClipboardPopup";
const SEARCH_CONTEXT: &str = "ClipboardSearch";
const PAGE: isize = 8;
/// Entries loaded at once; the list only draws the visible ones.
const LIMIT: usize = 5000;
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

pub fn bind_keys(cx: &mut App) {
    // Scoped to the search input so these win over its own bindings, and leave
    // the type dropdown alone while it is open.
    let context = Some("ClipboardSearch > Input");
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("home", SelectFirst, context),
        KeyBinding::new("end", SelectLast, context),
        KeyBinding::new("enter", Paste, context),
        KeyBinding::new("shift-enter", PastePlain, context),
        KeyBinding::new("ctrl-enter", Copy, context),
        KeyBinding::new("tab", NextType, context),
        KeyBinding::new("ctrl-f", NextType, context),
        KeyBinding::new("shift-tab", PreviousType, context),
        KeyBinding::new("escape", Dismiss, context),
    ]);
}

pub struct Popup {
    search: Entity<InputState>,
    filter: Entity<SelectState<Vec<TypeFilter>>>,
    list: Entity<ListState<HistoryDelegate>>,
    selected: Option<Selected>,
    /// Decoded pictures, dropped each time the popup reopens.
    images: Entity<RetainAllImageCache>,
    last_click: Option<(i64, Instant)>,
    store: SharedStore,
    backdrop: Backdrop,
    events: Sender<Event>,
    _subscriptions: [Subscription; 4],
}

impl Popup {
    pub fn new(
        store: SharedStore,
        backdrop: Backdrop,
        events: Sender<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type to filter entries…"));
        let filter = cx.new(|cx| {
            SelectState::new(FILTERS.to_vec(), Some(IndexPath::default()), window, cx)
                .searchable(true)
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
                |this, _, _: &SelectEvent<Vec<TypeFilter>>, window, cx| {
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
            images: RetainAllImageCache::new(cx),
            last_click: None,
            store,
            backdrop,
            events,
            _subscriptions: subscriptions,
        }
    }

    /// Prepares a fresh view for the next time the popup opens.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.last_click = None;
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
            let details = details
                .inspect_err(|error| {
                    tracing::error!(error = format!("{error:#}"), "could not load an entry")
                })
                .ok()
                .flatten()?;
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

    /// Moves by `step` rows. Single steps wrap around the ends.
    fn move_selection(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.selected_id();
        let target = self.list.update(cx, |list, cx| {
            let delegate = list.delegate();
            let count = delegate.len() as isize;
            if count == 0 {
                return None;
            }
            let from = current.and_then(|id| delegate.ordinal(id)).unwrap_or(0) as isize;
            let to = if step.abs() == 1 {
                (from + step).rem_euclid(count)
            } else {
                from.saturating_add(step).clamp(0, count - 1)
            };
            let target = delegate.nth(to as usize)?;
            list.set_selected_index(Some(target.0), window, cx);
            list.scroll_to_item(target.0, ScrollStrategy::Top, window, cx);
            cx.notify();
            Some(target)
        });
        if let Some((_, id)) = target {
            self.load(Some(id), cx);
        }
    }

    /// Home and End move the text cursor while there is a query.
    fn jump(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).value().is_empty() {
            self.move_selection(step, window, cx);
        } else {
            cx.propagate();
        }
    }

    /// A click selects, a second click pastes.
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
        self.focus_search(window, cx);
        let double = self
            .last_click
            .is_some_and(|(last, at)| last == id && at.elapsed() < DOUBLE_CLICK);
        self.last_click = Some((id, Instant::now()));
        self.load(Some(id), cx);
        if double {
            self.activate(Activation::Paste);
        }
    }

    fn activate(&mut self, how: Activation) {
        if let Some(id) = self.selected_id() {
            self.send(Event::Activate { id, how });
        }
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
        // The first Escape clears the search, the second closes.
        if self.search.read(cx).value().is_empty() {
            self.send(Event::Hide);
        } else {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.refresh(false, window, cx);
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

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = |id: &'static str, label: &'static str, keys: &'static str, how| {
            Button::new(id)
                .ghost()
                .small()
                .child(
                    h_flex()
                        .gap_2()
                        .child(label)
                        .children(Keystroke::parse(keys).ok().map(Kbd::new)),
                )
                .on_click(cx.listener(move |this, _, _, _| this.activate(how)))
        };
        h_flex()
            .flex_shrink_0()
            .justify_between()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::ClipboardList).small())
                    .child("Clipboard History"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(hint("paste", "Paste", "enter", Activation::Paste))
                    .child(hint("copy", "Copy", "ctrl-enter", Activation::Copy)),
            )
    }
}

impl Render for Popup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.header(cx).into_any_element();
        let footer = self.footer(cx).into_any_element();
        // Taken out while rendering, which needs the context mutably.
        let selected = self.selected.take();
        let preview = selected
            .as_ref()
            .map(|selected| preview::preview(selected, cx).into_any_element());
        self.selected = selected;
        let theme = cx.theme();
        let background = match self.backdrop {
            Backdrop::Acrylic => theme.background.opacity(0.72),
            Backdrop::Solid => theme.background,
        };

        v_flex()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SelectPrevious, window, cx| {
                this.move_selection(-1, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SelectNext, window, cx| this.move_selection(1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PageUp, window, cx| this.move_selection(-PAGE, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PageDown, window, cx| this.move_selection(PAGE, window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectFirst, window, cx| {
                this.jump(isize::MIN / 2, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SelectLast, window, cx| {
                    this.jump(isize::MAX / 2, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Paste, _, _| this.activate(Activation::Paste)))
            .on_action(
                cx.listener(|this, _: &PastePlain, _, _| this.activate(Activation::PastePlain)),
            )
            .on_action(cx.listener(|this, _: &Copy, _, _| this.activate(Activation::Copy)))
            .on_action(cx.listener(|this, _: &NextType, window, cx| this.cycle_type(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousType, window, cx| this.cycle_type(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .image_cache(self.images.clone())
            .size_full()
            .bg(background)
            .text_color(theme.foreground)
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    // Clicks here must not move focus away from the search.
                    .capture_any_mouse_down(|_, window, _| window.prevent_default())
                    .child(
                        div()
                            .w(relative(0.4))
                            .flex_shrink_0()
                            .px_2()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(List::new(&self.list)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when_some(preview, |pane, preview| pane.child(preview)),
                    ),
            )
            .child(footer)
    }
}
