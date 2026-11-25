//! The right pane: the selected entry and what is known about it.

mod content;
mod info;

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;

use super::{file_facts::FileFacts, popup::Popup};
use crate::store::{Details, Kind};

/// The selected entry, loaded once per selection.
pub struct Selected {
    pub id: i64,
    pub details: Details,
    /// Set for entries holding exactly one file.
    pub file: Option<FileFacts>,
    /// An SVG is shown as text instead of a picture.
    pub show_source: bool,
}

pub fn preview(selected: &Selected, cx: &mut Context<Popup>) -> impl IntoElement {
    let content = match selected.details.kind {
        Kind::Text => content::text(selected),
        Kind::Code => content::code(selected),
        Kind::Color => content::color(selected, cx),
        Kind::Link => content::link(selected, cx),
        Kind::Email => content::email(selected, cx),
        Kind::Image => content::picture(selected, cx),
        Kind::Svg => content::svg(selected, cx),
        Kind::Files => content::files(selected, cx),
    };
    let theme = cx.theme();

    v_flex()
        .size_full()
        .child(div().flex_1().min_h_0().child(content))
        .child(
            div()
                // Each entry starts scrolled to the top.
                .id(("information", selected.id as u64))
                .flex_shrink_0()
                .max_h(relative(0.55))
                .overflow_y_scroll()
                .px_4()
                .py_3()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .pb_2()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Information"),
                )
                .child(info::table(selected)),
        )
}
