//! The "Information" table under a preview.

use gpui_kit::component::{
    description_list::{DescriptionItem, DescriptionList},
    h_flex,
};
use gpui_kit::*;

use super::Selected;
use crate::{ingest::code, store::Kind, ui::when};

pub fn table(selected: &Selected) -> DescriptionList {
    let source_icon = selected.details.source_icon.clone();
    DescriptionList::horizontal()
        .columns(1)
        .bordered(false)
        .label_width(px(110.))
        .children(rows(selected).into_iter().map(move |(label, value)| {
            let icon = (label == "Source").then(|| source_icon.clone()).flatten();
            DescriptionItem::new(label).value(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .text_sm()
                    .children(icon.map(|path| img(path).size_4().flex_shrink_0()))
                    .child(div().min_w_0().truncate().child(value))
                    .into_any_element(),
            )
        }))
}

fn rows(selected: &Selected) -> Vec<(&'static str, String)> {
    let details = &selected.details;
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
    let single = selected.file.is_some();
    let kind = match details.kind {
        Kind::Text => "Text",
        Kind::Link => "Link",
        Kind::Email => "Email",
        Kind::Color => "Color",
        Kind::Code => "Code",
        Kind::Svg => "SVG",
        Kind::Image => "Image",
        Kind::Files if single => "File",
        Kind::Files => "Files",
    };
    rows.push(("Type", kind.into()));

    let text = details.text.as_deref().unwrap_or_default();
    match details.kind {
        Kind::Text => {
            rows.push(("Characters", details.characters.to_string()));
            rows.push(("Words", details.words.to_string()));
            rows.push(("Lines", details.lines.to_string()));
        }
        Kind::Code => {
            let language = details.language.as_deref().and_then(code::by_token);
            if let Some(language) = language {
                rows.push(("Language", language.name.into()));
            }
            rows.push(("Characters", details.characters.to_string()));
            rows.push(("Lines", details.lines.to_string()));
        }
        Kind::Link => {
            if let Some(title) = &details.link.title {
                rows.push(("Title", title.clone()));
            }
            if let Some(domain) = domain(text) {
                rows.push(("Domain", domain));
            }
        }
        Kind::Image => {
            if let Some((width, height)) = details.dimensions {
                rows.push(("Dimensions", format!("{width}×{height}")));
            }
            rows.push(("Format", "PNG".into()));
            rows.push(("Size", human_size(details.size_bytes)));
        }
        Kind::Svg => {
            if let Some(dimensions) = svg_dimensions(text) {
                rows.push(("Dimensions", dimensions));
            }
            rows.push(("Size", human_size(details.size_bytes)));
        }
        Kind::Files => file_rows(selected, &mut rows),
        Kind::Email | Kind::Color => {}
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

fn file_rows(selected: &Selected, rows: &mut Vec<(&'static str, String)>) {
    let details = &selected.details;
    let Some(file) = &selected.file else {
        rows.push(("Count", details.lines.to_string()));
        rows.push(("Total size", human_size(details.size_bytes)));
        return;
    };
    rows.push(("Path", file.path.to_string_lossy().into_owned()));
    if let Some(version) = &file.version {
        for (label, value) in [
            ("Product", &version.product),
            ("Version", &version.version),
            ("Publisher", &version.company),
        ] {
            if let Some(value) = value {
                rows.push((label, value.clone()));
            }
        }
    }
    if let Some((width, height)) = file.dimensions {
        rows.push(("Dimensions", format!("{width}×{height}")));
    }
    if !file.is_folder {
        rows.push(("Size", human_size(details.size_bytes)));
    }
    if let Some(modified) = file.modified {
        rows.push((
            "Modified",
            when::describe(when::local(modified), when::now()),
        ));
    }
}

fn domain(address: &str) -> Option<String> {
    let address = address.trim();
    let url = url::Url::parse(address)
        .or_else(|_| url::Url::parse(&format!("https://{address}")))
        .ok()?;
    url.host_str().map(str::to_owned)
}

/// The size an SVG declares: its `width` and `height`, else its `viewBox`.
fn svg_dimensions(source: &str) -> Option<String> {
    let start = source.find("<svg")?;
    let tag = &source[start..start + source[start..].find('>')?];
    let attribute = |name: &str| {
        let marker = format!("{name}=");
        let (position, _) = tag
            .match_indices(&marker)
            // `stroke-width` is not `width`.
            .find(|(i, _)| tag[..*i].ends_with(char::is_whitespace))?;
        let rest = &tag[position + marker.len()..];
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        rest[1..].split(quote).next()
    };
    if let (Some(width), Some(height)) = (attribute("width"), attribute("height")) {
        return Some(format!("{width} × {height}"));
    }
    let parts: Vec<&str> = attribute("viewBox")?
        .split([' ', ','])
        .filter(|part| !part.is_empty())
        .collect();
    match parts[..] {
        [_, _, width, height] => Some(format!("{width} × {height}")),
        _ => None,
    }
}

/// Decimal units, as in the reference design.
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
    // Not `super::*`: GPUI exports a `test` attribute of its own.
    use super::{domain, human_size, svg_dimensions};

    #[test]
    fn sizes_use_the_largest_readable_unit() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(999_700), "1 MB");
        assert_eq!(human_size(171_000), "171 kB");
        assert_eq!(human_size(2_500_000_000), "2.5 GB");
    }

    #[test]
    fn reads_svg_sizes() {
        let sized = r#"<svg stroke-width="2" width="24" height='12'></svg>"#;
        assert_eq!(svg_dimensions(sized).as_deref(), Some("24 × 12"));
        let boxed = r#"<?xml version="1.0"?><svg viewBox="0,0, 100 50"/>"#;
        assert_eq!(svg_dimensions(boxed).as_deref(), Some("100 × 50"));
        for source in ["", "<svg", "<svg>", "<svg viewBox=\"1\">", "<svg width=>"] {
            assert_eq!(svg_dimensions(source), None, "{source}");
        }
    }

    #[test]
    fn finds_the_domain_of_links() {
        assert_eq!(
            domain("https://github.com/a").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            domain("www.example.com").as_deref(),
            Some("www.example.com")
        );
    }
}
