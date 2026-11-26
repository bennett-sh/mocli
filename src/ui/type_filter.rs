//! The "All Types" dropdown.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _, h_flex, searchable_list::SearchableListItem};
use gpui_kit::*;

use super::kind_icon;
use crate::store::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    Text,
    Images,
    Files,
    Links,
    Emails,
    Colors,
    Code,
}

pub const FILTERS: [TypeFilter; 8] = [
    TypeFilter::All,
    TypeFilter::Text,
    TypeFilter::Images,
    TypeFilter::Files,
    TypeFilter::Links,
    TypeFilter::Emails,
    TypeFilter::Colors,
    TypeFilter::Code,
];

impl TypeFilter {
    /// The kinds shown; none means all of them.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Self::All => &[],
            Self::Text => &[Kind::Text],
            Self::Images => &[Kind::Image, Kind::Svg],
            Self::Files => &[Kind::Files],
            Self::Links => &[Kind::Link],
            Self::Emails => &[Kind::Email],
            Self::Colors => &[Kind::Color],
            Self::Code => &[Kind::Code],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All Types",
            Self::Text => "Text Only",
            Self::Images => "Images Only",
            Self::Files => "Files Only",
            Self::Links => "Links Only",
            Self::Emails => "Emails Only",
            Self::Colors => "Colors Only",
            Self::Code => "Code Only",
        }
    }

    fn row(self) -> impl IntoElement {
        let icon = self
            .kinds()
            .first()
            .map_or(IconName::List, |&k| kind_icon(k));
        h_flex()
            .gap_2()
            .child(Icon::new(icon).small())
            .child(self.label())
    }
}

impl SearchableListItem for TypeFilter {
    type Value = Self;

    fn title(&self) -> SharedString {
        self.label().into()
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(self.row().into_any_element())
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.row()
    }

    fn value(&self) -> &Self {
        self
    }
}
