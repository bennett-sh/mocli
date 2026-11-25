pub mod assets;
mod history_list;
pub mod popup;
mod preview;
mod type_filter;
mod when;

use gpui_kit::assets::IconName;

use crate::store::Kind;

fn kind_icon(kind: Kind) -> IconName {
    match kind {
        Kind::Text => IconName::FileText,
        Kind::Link => IconName::Link,
        Kind::Email => IconName::AtSign,
        Kind::Color => IconName::Pipette,
        Kind::Image => IconName::Image,
        Kind::Files => IconName::Files,
    }
}
