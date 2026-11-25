//! The right pane: the selected entry and what is known about it.

use gpui_kit::component::{
    ActiveTheme as _,
    description_list::{DescriptionItem, DescriptionList},
    v_flex,
};
use gpui_kit::*;

use super::when;
use crate::store::{Details, Kind};

/// Laying out more than this makes moving through the list sluggish.
const MAX_LINES: usize = 200;
const MAX_CHARS: usize = 5000;

pub fn preview(id: i64, details: &Details, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let content = match &details.image {
        // The image's height must come from the flex layout. Given as a
        // percentage, its aspect ratio wins and it gets cut off.
        Some(path) => v_flex()
            .size_full()
            .p_4()
            .child(
                img(path.clone())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .object_fit(ObjectFit::ScaleDown)
                    .rounded(theme.radius_lg),
            )
            .into_any_element(),
        None => div()
            // A new id per entry starts each preview scrolled to the top.
            .id(("preview", id as u64))
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .text_sm()
            .child(excerpt(details.text.as_deref().unwrap_or_default()))
            .into_any_element(),
    };

    v_flex()
        .size_full()
        .child(div().flex_1().min_h_0().child(content))
        .child(
            div()
                .flex_shrink_0()
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
                .child(info(details)),
        )
}

fn excerpt(text: &str) -> String {
    let lines: Vec<&str> = text.lines().take(MAX_LINES).collect();
    let joined = lines.join("\n").replace('\t', "    ");
    match joined.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => format!("{}…", &joined[..end]),
        None => joined,
    }
}

fn info(details: &Details) -> DescriptionList {
    DescriptionList::horizontal()
        .columns(1)
        .bordered(false)
        .label_width(px(110.))
        .children(rows(details).into_iter().map(|(label, value)| {
            DescriptionItem::new(label).value(
                div()
                    .w_full()
                    .text_sm()
                    .text_right()
                    .truncate()
                    .child(value)
                    .into_any_element(),
            )
        }))
}

fn rows(details: &Details) -> Vec<(&'static str, String)> {
    let mut rows = vec![(
        "Source",
        details
            .source_name
            .clone()
            .unwrap_or_else(|| "Unknown".into()),
    )];
    if let Some(title) = &details.source_title {
        rows.push(("Window", title.clone()));
    }
    let paths = details
        .text
        .as_deref()
        .map_or(0, |text| text.lines().count());
    let kind = match details.kind {
        Kind::Text => "Text",
        Kind::Link => "Link",
        Kind::Email => "Email",
        Kind::Color => "Color",
        Kind::Image => "Image",
        Kind::Files if paths == 1 => "File",
        Kind::Files => "Files",
    };
    rows.push(("Type", kind.into()));
    match details.kind {
        Kind::Text => {
            rows.push(("Characters", details.characters.to_string()));
            rows.push(("Words", details.words.to_string()));
        }
        Kind::Image => {
            if let Some((width, height)) = details.dimensions {
                rows.push(("Dimensions", format!("{width}×{height}")));
            }
            rows.push(("Size", human_size(details.size_bytes)));
        }
        Kind::Files => rows.push(("Size", human_size(details.size_bytes))),
        Kind::Link | Kind::Email | Kind::Color => {}
    }
    rows.push((
        "Copied",
        when::describe(when::local(details.last_copied), when::now()),
    ));
    if details.copy_count > 1 {
        rows.push(("Times copied", details.copy_count.to_string()));
    }
    rows
}

/// Decimal units, as Explorer's neighbours in the screenshots use them.
fn human_size(bytes: u64) -> String {
    let mut value = bytes as f64;
    for unit in ["B", "kB", "MB"] {
        if value < 999.5 {
            return format!("{value:.0} {unit}");
        }
        value /= 1000.0;
    }
    format!("{value:.1} GB")
}

#[cfg(test)]
mod tests {
    use super::{MAX_CHARS, MAX_LINES, excerpt, human_size};

    #[test]
    fn sizes_use_the_largest_readable_unit() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(999_700), "1 MB");
        assert_eq!(human_size(171_000), "171 kB");
        assert_eq!(human_size(2_500_000_000), "2.5 GB");
    }

    #[test]
    fn excerpts_are_bounded() {
        assert_eq!(excerpt("a\tb\r\nc"), "a    b\nc");
        assert_eq!(excerpt(&"x\n".repeat(1000)).lines().count(), MAX_LINES);
        assert_eq!(excerpt(&"é".repeat(9000)).chars().count(), MAX_CHARS + 1);
    }
}
