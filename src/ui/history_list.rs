//! The left pane: entries in dated sections.

use chrono::NaiveDate;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _, h_flex,
    list::{ListDelegate, ListItem, ListState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{kind_icon, stored_image, swatch, when};
use crate::{
    color::Color,
    store::{Kind, Results, Summary},
};

const ROW_HEIGHT: f32 = 40.0;

pub struct Section {
    pub title: &'static str,
    pub entries: Vec<Summary>,
}

#[derive(Default)]
pub struct HistoryDelegate {
    pub sections: Vec<Section>,
    pub filtered: bool,
}

/// Matches grouped by age, then loose matches.
pub fn sections(results: Results, today: NaiveDate) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for entry in results.matches {
        let title = when::Age::of(when::local(entry.last_copied).date(), today).title();
        match sections.last_mut() {
            Some(section) if section.title == title => section.entries.push(entry),
            _ => sections.push(Section {
                title,
                entries: vec![entry],
            }),
        }
    }
    if !results.similar.is_empty() {
        sections.push(Section {
            title: "Similar",
            entries: results.similar,
        });
    }
    sections
}

impl HistoryDelegate {
    pub fn len(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.entries.len())
            .sum()
    }

    pub fn entry(&self, index: IndexPath) -> Option<&Summary> {
        self.sections.get(index.section)?.entries.get(index.row)
    }

    /// Every position, in the order shown.
    fn positions(&self) -> impl Iterator<Item = (IndexPath, &Summary)> {
        self.sections
            .iter()
            .enumerate()
            .flat_map(|(section, content)| {
                content.entries.iter().enumerate().map(move |(row, entry)| {
                    (IndexPath::default().section(section).row(row), entry)
                })
            })
    }

    pub fn position_of(&self, id: i64) -> Option<IndexPath> {
        self.positions()
            .find_map(|(index, entry)| (entry.id == id).then_some(index))
    }

    /// The n-th position, counted across sections.
    pub fn nth(&self, n: usize) -> Option<(IndexPath, i64)> {
        self.positions()
            .nth(n)
            .map(|(index, entry)| (index, entry.id))
    }

    pub fn ordinal(&self, id: i64) -> Option<usize> {
        self.positions().position(|(_, entry)| entry.id == id)
    }
}

impl ListDelegate for HistoryDelegate {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.sections.len()
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.sections
            .get(section)
            .map_or(0, |section| section.entries.len())
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let entry = self.entry(index)?;
        let theme = cx.theme();
        let tile = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size_6()
            .rounded(theme.radius)
            .bg(theme.muted)
            .overflow_hidden();
        let color = (entry.kind == Kind::Color)
            .then(|| Color::parse(&entry.title))
            .flatten();
        let tile = match (&entry.thumbnail, color) {
            (Some(thumbnail), _) => {
                // Photos fill the tile; icons must not be cropped.
                let fit = match entry.kind {
                    Kind::Image => ObjectFit::Cover,
                    _ => ObjectFit::Contain,
                };
                tile.child(
                    stored_image::img(thumbnail.clone())
                        .size_full()
                        .object_fit(fit),
                )
            }
            (None, Some(color)) => tile.child(swatch(color).size_full()),
            (None, None) => tile.child(Icon::new(kind_icon(entry.kind)).small()),
        };

        Some(
            ListItem::new(("entry", entry.id as u64))
                .h(px(ROW_HEIGHT))
                .px_2()
                .rounded(theme.radius_lg)
                .accessibility_label(entry.title.clone())
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(tile)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(entry.title.clone()),
                        )
                        .when(entry.pinned, |row| {
                            row.child(Icon::new(IconName::Pin).xsmall())
                        }),
                ),
        )
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let title = self.sections.get(section)?.title;
        Some(
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let message = if self.filtered {
            "No matching entries"
        } else {
            "Nothing copied yet"
        };
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        // The popup reads the selection from the list state.
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI exports a `test` attribute of its own.
    use super::{HistoryDelegate, sections};
    use crate::{
        store::{Kind, Results, Summary},
        ui::when,
    };

    fn entry(id: i64, pinned: bool, last_copied: i64) -> Summary {
        Summary {
            id,
            kind: Kind::Text,
            title: id.to_string(),
            pinned,
            last_copied,
            thumbnail: None,
        }
    }

    #[test]
    fn builds_sections_and_walks_them_in_order() {
        let now = 1_795_000_000_000;
        let day = 24 * 60 * 60 * 1000;
        let results = Results {
            matches: vec![
                entry(1, false, now),
                entry(2, false, now),
                // A pinned entry stays with those of its age.
                entry(3, true, now - 400 * day),
                entry(4, false, now - 400 * day),
            ],
            similar: vec![entry(5, false, now)],
        };
        let delegate = HistoryDelegate {
            sections: sections(results, when::local(now).date()),
            filtered: true,
        };
        let shape: Vec<_> = delegate
            .sections
            .iter()
            .map(|section| (section.title, section.entries.len()))
            .collect();
        assert_eq!(shape, [("Today", 2), ("Older", 2), ("Similar", 1)]);
        assert_eq!(delegate.len(), 5);
        for n in 0..5 {
            let (index, id) = delegate.nth(n).unwrap();
            assert_eq!(delegate.ordinal(id), Some(n));
            assert_eq!(delegate.position_of(id), Some(index));
            assert_eq!(delegate.entry(index).unwrap().id, id);
        }
        assert!(delegate.nth(5).is_none() && delegate.position_of(9).is_none());
    }
}
