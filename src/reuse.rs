//! Turns a stored entry back into clipboard content.

use std::{fs, path::PathBuf};

use anyhow::{Context as _, Result};

use crate::{
    capture::Snapshot,
    ingest,
    platform::{Activation, clipboard_write::Content},
    store::{Format, Kind, Payload},
};

pub fn content(payload: Payload, how: Activation) -> Result<Content> {
    let plain = how == Activation::PastePlain;

    if let Some(path) = &payload.image {
        let png = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        return Ok(Content::Image { png });
    }

    let text = payload.text.context("the entry has no content")?;
    // As plain text, files become their paths.
    if payload.kind == Kind::Files && !plain {
        return Ok(Content::Files(text.lines().map(PathBuf::from).collect()));
    }

    let mut formats = payload.formats;
    if plain {
        formats.clear();
    }
    let mut take = |wanted| {
        let index = formats.iter().position(|(format, _)| *format == wanted)?;
        Some(formats.swap_remove(index).1)
    };
    Ok(Content::Text {
        html: take(Format::Html),
        rtf: take(Format::Rtf),
        text,
    })
}

/// What was on the clipboard before, as far as mocli can put it back: files,
/// text with its formatting, or a picture.
pub fn restored(snapshot: Snapshot) -> Option<Content> {
    if !snapshot.files.is_empty() {
        return Some(Content::Files(snapshot.files));
    }
    if let Some(text) = snapshot.text {
        return Some(Content::Text {
            text,
            html: snapshot.html,
            rtf: snapshot.rtf,
        });
    }
    let png = ingest::to_png(&snapshot.image?)
        .inspect_err(|error| tracing::warn!(%error, "could not keep the previous picture"))
        .ok()?;
    Some(Content::Image { png })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(kind: Kind) -> Payload {
        Payload {
            kind,
            text: Some("a\nb".into()),
            image: None,
            formats: vec![(Format::Html, vec![1]), (Format::Rtf, vec![2])],
        }
    }

    #[test]
    fn plain_paste_drops_formatting() {
        let Ok(Content::Text { html, rtf, text }) = content(payload(Kind::Text), Activation::Paste)
        else {
            panic!("expected text");
        };
        assert_eq!(
            (html, rtf, text.as_str()),
            (Some(vec![1]), Some(vec![2]), "a\nb")
        );

        let Ok(Content::Text { html, rtf, .. }) =
            content(payload(Kind::Text), Activation::PastePlain)
        else {
            panic!("expected text");
        };
        assert_eq!((html, rtf), (None, None));
    }

    #[test]
    fn files_paste_as_files_or_as_paths() {
        let Ok(Content::Files(paths)) = content(payload(Kind::Files), Activation::Copy) else {
            panic!("expected files");
        };
        assert_eq!(paths, [PathBuf::from("a"), PathBuf::from("b")]);
        assert!(matches!(
            content(payload(Kind::Files), Activation::PastePlain),
            Ok(Content::Text { .. })
        ));
    }

    #[test]
    fn a_missing_image_file_is_an_error() {
        let mut image = payload(Kind::Image);
        image.image = Some("does-not-exist.png".into());
        assert!(content(image, Activation::Paste).is_err());
    }
}
