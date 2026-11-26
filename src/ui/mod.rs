pub mod assets;
mod file_facts;
mod history_list;
pub mod popup;
mod preview;
pub mod settings;
mod stored_image;
#[cfg(test)]
mod tests;
mod type_filter;
mod when;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, ParentElement as _, Styled as _, black, div, rgba, white};

use crate::{color::Color, store::Kind};

/// A color sample. A translucent color is laid over white and black halves,
/// so its alpha shows. Size it with the usual styles.
fn swatch(color: Color) -> Div {
    let backing = || {
        gpui_kit::component::h_flex()
            .absolute()
            .inset_0()
            .child(div().flex_1().h_full().bg(white()))
            .child(div().flex_1().h_full().bg(black()))
    };
    div()
        .relative()
        .overflow_hidden()
        .when(color.alpha < 1.0, |sample| sample.child(backing()))
        .child(div().absolute().inset_0().bg(rgba(color.to_rgba_u32())))
}

fn kind_icon(kind: Kind) -> IconName {
    match kind {
        Kind::Text => IconName::FileText,
        Kind::Link => IconName::Link,
        Kind::Email => IconName::AtSign,
        Kind::Color => IconName::Pipette,
        Kind::Code => IconName::CodeXml,
        Kind::Svg => IconName::PenTool,
        Kind::Image => IconName::Image,
        Kind::Files => IconName::Files,
    }
}
