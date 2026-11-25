//! The "All Types" dropdown.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _, h_flex, searchable_list::SearchableListItem};
use gpui_kit::*;

use super::kind_icon;
use crate::store::Kind;

/// `None` shows every kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeFilter(pub Option<Kind>);

pub const FILTERS: [TypeFilter; 7] = [
    TypeFilter(None),
    TypeFilter(Some(Kind::Text)),
    TypeFilter(Some(Kind::Image)),
    TypeFilter(Some(Kind::Files)),
    TypeFilter(Some(Kind::Link)),
    TypeFilter(Some(Kind::Email)),
    TypeFilter(Some(Kind::Color)),
];

impl TypeFilter {
    fn label(self) -> &'static str {
        match self.0 {
            None => "All Types",
            Some(Kind::Text) => "Text Only",
            Some(Kind::Image) => "Images Only",
            Some(Kind::Files) => "Files Only",
            Some(Kind::Link) => "Links Only",
            Some(Kind::Email) => "Emails Only",
            Some(Kind::Color) => "Colors Only",
        }
    }

    fn row(self) -> impl IntoElement {
        let icon = self.0.map_or(IconName::List, kind_icon);
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
