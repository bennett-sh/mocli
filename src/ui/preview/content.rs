//! One renderer per kind of entry.

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    text::TextView,
    v_flex,
};
use gpui_kit::*;

use super::Selected;
use crate::{
    color::Color,
    platform::Event,
    ui::{file_facts, popup::Popup},
};

/// Laying out more than this makes moving through the list sluggish.
const MAX_LINES: usize = 200;
const MAX_CHARS: usize = 5000;
/// Rows listed for an entry with many files.
const MAX_FILES: usize = 100;

fn excerpt(text: &str) -> String {
    let lines: Vec<&str> = text.lines().take(MAX_LINES).collect();
    let joined = lines.join("\n").replace('\t', "    ");
    match joined.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => format!("{}…", &joined[..end]),
        None => joined,
    }
}

/// A Markdown code block around `text`, with a fence longer than any inside it.
fn fenced(text: &str, language: &str) -> String {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{text}\n{fence}")
}

fn body(selected: &Selected) -> &str {
    selected.details.text.as_deref().unwrap_or_default()
}

fn scrolling(selected: &Selected) -> Stateful<Div> {
    // A new id per entry starts each preview scrolled to the top.
    div()
        .id(("preview", selected.id as u64))
        .size_full()
        .overflow_y_scroll()
        .p_4()
}

pub fn text(selected: &Selected) -> AnyElement {
    scrolling(selected)
        .text_sm()
        .child(excerpt(body(selected)))
        .into_any_element()
}

pub fn code(selected: &Selected) -> AnyElement {
    let language = selected.details.language.as_deref().unwrap_or_default();
    highlighted(selected, language)
}

fn highlighted(selected: &Selected, language: &str) -> AnyElement {
    let source = fenced(&excerpt(body(selected)), language);
    scrolling(selected)
        .text_sm()
        .child(TextView::markdown(("code", selected.id as u64), source))
        .into_any_element()
}

/// A picture as large as fits, never enlarged.
fn framed(path: PathBuf, cx: &App) -> Img {
    // The height must come from the flex layout. Given as a percentage, the
    // aspect ratio wins and the picture gets cut off.
    img(path)
        .flex_1()
        .min_h_0()
        .w_full()
        .object_fit(ObjectFit::ScaleDown)
        .rounded(cx.theme().radius_lg)
}

pub fn picture(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    v_flex()
        .size_full()
        .p_4()
        .children(selected.details.image.clone().map(|path| framed(path, cx)))
        .into_any_element()
}

pub fn svg(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    let source = selected.show_source;
    let toggle = Button::new("svg-source")
        .ghost()
        .xsmall()
        .label(if source {
            "Show picture"
        } else {
            "Show source"
        })
        .on_click(cx.listener(|popup, _, _, cx| popup.toggle_source(cx)));
    let content = if source {
        highlighted(selected, "xml")
    } else {
        picture(selected, cx)
    };
    v_flex()
        .size_full()
        .child(div().flex_1().min_h_0().child(content))
        .child(h_flex().justify_end().px_2().pb_1().child(toggle))
        .into_any_element()
}

pub fn color(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    let Some(color) = Color::parse(body(selected)) else {
        return text(selected);
    };
    let theme = cx.theme();
    let (muted, border, radius) = (theme.muted_foreground, theme.border, theme.radius_lg);
    let notations = [
        ("HEX", color.to_hex()),
        ("RGB", color.to_rgb()),
        ("HSL", color.to_hsl()),
        ("OKLCH", color.to_oklch()),
    ];
    let rows: Vec<_> = notations
        .into_iter()
        .map(|(label, value)| {
            let copied = value.clone();
            Button::new(label)
                .ghost()
                .small()
                .w_full()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(div().text_color(muted).child(label))
                        .child(value),
                )
                .on_click(cx.listener(move |popup, _, _, _| {
                    popup.send(Event::CopyText(copied.clone()));
                }))
        })
        .collect();

    scrolling(selected)
        .child(
            div()
                .h(px(96.))
                .w_full()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(rgba(color.to_rgba_u32())),
        )
        .child(v_flex().pt_2().children(rows))
        .child(div().pt_1().text_xs().text_color(muted).child(format!(
            "Contrast {:.1}:1 on white, {:.1}:1 on black. Click a value to copy it.",
            color.contrast_with_white(),
            color.contrast_with_black()
        )))
        .into_any_element()
}

fn large(icon: IconName) -> AnyElement {
    Icon::new(icon).size_10().into_any_element()
}

/// An icon with a headline, for entries without a preview of their own.
fn card(media: AnyElement, headline: String, caption: String, cx: &App) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_4()
        .child(media)
        .child(div().max_w_full().text_center().text_base().child(headline))
        .child(
            div()
                .max_w_full()
                .text_center()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(caption),
        )
}

pub fn link(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    let address = excerpt(body(selected).trim());
    let link = &selected.details.link;
    let theme = cx.theme();
    let Some(image) = link.image.clone() else {
        let headline = link.title.clone().unwrap_or_else(|| address.clone());
        let caption = match &link.title {
            Some(_) => address,
            None => String::new(),
        };
        let media = match &link.icon {
            Some(icon) => img(icon.clone()).size_12().into_any_element(),
            None => large(IconName::Link),
        };
        return card(media, headline, caption, cx).into_any_element();
    };
    v_flex()
        .size_full()
        .gap_2()
        .p_4()
        .child(framed(image, cx))
        .children(
            link.title
                .clone()
                .map(|title| div().flex_shrink_0().truncate().child(title)),
        )
        .children(link.description.clone().map(|description| {
            div()
                .flex_shrink_0()
                .line_clamp(2)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(description)
        }))
        .into_any_element()
}

pub fn email(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    let address = body(selected).trim().trim_start_matches("mailto:");
    let domain = address.split_once('@').map_or("", |(_, domain)| domain);
    card(
        large(IconName::AtSign),
        excerpt(address),
        domain.to_owned(),
        cx,
    )
    .into_any_element()
}

pub fn files(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    let Some(file) = &selected.file else {
        return file_list(selected, cx);
    };
    let name = file
        .path
        .file_name()
        .map_or_else(
            || file.path.to_string_lossy(),
            |name| name.to_string_lossy(),
        )
        .into_owned();
    if file.exists && !file.is_folder && file_facts::is_picture(&file.path) {
        return v_flex()
            .size_full()
            .p_4()
            .child(framed(file.path.clone(), cx))
            .into_any_element();
    }
    let caption = if !file.exists {
        "This file no longer exists".to_owned()
    } else if let Some(version) = &file.version {
        [&version.product, &version.company]
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ")
    } else if file.is_folder {
        "Folder".to_owned()
    } else {
        String::new()
    };
    // The shell's own picture is better than a generic icon.
    let media = match &selected.details.thumbnail {
        Some(thumbnail) => img(thumbnail.clone())
            .size_24()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        None if file.is_folder => large(IconName::Folder),
        None => large(IconName::File),
    };
    card(media, name, caption, cx).into_any_element()
}

fn file_list(selected: &Selected, cx: &mut Context<Popup>) -> AnyElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let total = body(selected).lines().count();
    let rows = body(selected).lines().take(MAX_FILES).map(|line| {
        let path = Path::new(line);
        let name = path
            .file_name()
            .map_or_else(|| line.to_owned(), |n| n.to_string_lossy().into_owned());
        let folder = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        h_flex()
            .gap_2()
            .py_1()
            .child(Icon::new(IconName::File).small())
            .child(div().flex_shrink_0().text_sm().child(name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(folder),
            )
    });
    scrolling(selected)
        .children(rows)
        .children((total > MAX_FILES).then(|| {
            div()
                .py_1()
                .text_xs()
                .text_color(muted)
                .child(format!("and {} more", total - MAX_FILES))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI exports a `test` attribute of its own.
    use super::{MAX_CHARS, MAX_LINES, excerpt, fenced};

    #[test]
    fn excerpts_are_bounded() {
        assert_eq!(excerpt("a\tb\r\nc"), "a    b\nc");
        assert_eq!(excerpt(&"x\n".repeat(1000)).lines().count(), MAX_LINES);
        assert_eq!(excerpt(&"é".repeat(9000)).chars().count(), MAX_CHARS + 1);
    }

    #[test]
    fn fences_outlast_backticks_in_the_code() {
        assert_eq!(fenced("a", "rs"), "```rs\na\n```");
        assert_eq!(fenced("```\nx\n```", ""), "````\n```\nx\n```\n````");
    }
}
