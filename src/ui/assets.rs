//! Embedded icons: GPUI Component's defaults plus the ones only mocli uses,
//! without carrying the whole Lucide catalog.

use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString, assets::icon_assets};

// Every icon named in `ui` must be here or among the defaults.
icon_assets!(
    OwnIcons,
    [
        AtSign,
        ClipboardList,
        ClipboardPaste,
        CodeXml,
        Download,
        Eraser,
        ExternalLink,
        EyeOff,
        File,
        FileText,
        Files,
        Folder,
        FolderOpen,
        Hash,
        Image,
        Link,
        ListX,
        List,
        Mail,
        Palette,
        Pause,
        PenTool,
        Pencil,
        Pin,
        PinOff,
        Pipette,
        Play,
        Power,
        Route,
        ScanText,
        Settings,
        Trash,
        Type,
        X
    ]
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match OwnIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(OwnIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AssetSource as _, assets::IconName};

    use super::Assets;
    use crate::{store::Kind, ui::kind_icon};

    #[test]
    fn every_icon_in_use_is_embedded() {
        let others = [
            IconName::ClipboardList,
            IconName::ClipboardPaste,
            IconName::CodeXml,
            IconName::Download,
            IconName::Eraser,
            IconName::ExternalLink,
            IconName::EyeOff,
            IconName::File,
            IconName::FileText,
            IconName::Files,
            IconName::Folder,
            IconName::FolderOpen,
            IconName::Hash,
            IconName::Image,
            IconName::Link,
            IconName::List,
            IconName::ListX,
            IconName::Mail,
            IconName::Palette,
            IconName::Pause,
            IconName::PenTool,
            IconName::Pencil,
            IconName::Pin,
            IconName::PinOff,
            IconName::Pipette,
            IconName::Play,
            IconName::Power,
            IconName::Route,
            IconName::ScanText,
            IconName::Settings,
            IconName::Trash,
            IconName::Type,
            IconName::X,
        ];
        for icon in Kind::ALL.map(kind_icon).into_iter().chain(others) {
            let path = icon.path();
            assert!(matches!(Assets.load(&path), Ok(Some(_))), "{path}");
        }
    }
}
