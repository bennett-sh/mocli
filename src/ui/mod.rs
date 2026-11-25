pub mod assets;
mod file_facts;
mod history_list;
pub mod popup;
mod preview;
#[cfg(test)]
mod tests;
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
        Kind::Code => IconName::CodeXml,
        Kind::Svg => IconName::PenTool,
        Kind::Image => IconName::Image,
        Kind::Files => IconName::Files,
    }
}
