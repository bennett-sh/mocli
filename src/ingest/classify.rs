//! Decides what a piece of text or a whole clipboard is.

use url::Url;

use super::color::is_color;
use crate::{capture::Snapshot, store::Kind};

/// The one representation an entry is built from.
#[derive(Debug, PartialEq, Eq)]
pub enum Primary {
    Files,
    Image,
    Text,
}

pub fn primary(snapshot: &Snapshot) -> Option<Primary> {
    if !snapshot.files.is_empty() {
        return Some(Primary::Files);
    }
    let text = snapshot
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    match (text, snapshot.image.is_some()) {
        // "Copy image" in some apps adds the image's address as text. Office
        // apps instead add a picture of copied text, where the text matters.
        (Some(text), true) if is_link(text) || is_path(text) => Some(Primary::Image),
        (Some(_), _) => Some(Primary::Text),
        (None, true) => Some(Primary::Image),
        (None, false) => None,
    }
}

/// Classifies text that has already been trimmed.
pub fn text_kind(text: &str) -> Kind {
    if is_color(text) {
        Kind::Color
    } else if is_email(text) {
        Kind::Email
    } else if is_link(text) {
        Kind::Link
    } else {
        Kind::Text
    }
}

fn is_single_word(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn is_link(text: &str) -> bool {
    if !is_single_word(text) {
        return false;
    }
    if let Some(host) = text.strip_prefix("www.") {
        return host.contains('.') && Url::parse(&format!("https://{text}")).is_ok();
    }
    // `Url` also accepts `C:\path` and `note:this`, so require a web scheme.
    Url::parse(text)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "ftp") && url.host().is_some())
        && text.contains("://")
}

fn is_email(text: &str) -> bool {
    let address = text.strip_prefix("mailto:").unwrap_or(text);
    if !is_single_word(address) {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    let valid_local = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_alphanumeric() || "._%+-'".contains(c));
    let mut labels = domain.split('.');
    let top = domain.rsplit('.').next().unwrap_or_default();
    valid_local
        && domain.contains('.')
        && labels.all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
        && top.chars().count() >= 2
        && top.chars().all(char::is_alphabetic)
}

fn is_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    !text.contains('\n')
        && (text.starts_with(r"\\")
            || text.starts_with("file://")
            || matches!(bytes, [drive, b':', b'\\' | b'/', ..] if drive.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::RawImage;

    #[test]
    fn classifies_text() {
        for (text, kind) in [
            ("https://github.com/longbridge/gpui-component", Kind::Link),
            ("http://localhost:3000/a?b=c", Kind::Link),
            ("www.example.com", Kind::Link),
            ("hello@example.com", Kind::Email),
            ("mailto:first.last+tag@sub.example.co", Kind::Email),
            ("#ff7a1a", Kind::Color),
            ("see https://example.com", Kind::Text),
            ("example.com", Kind::Text),
            (r"C:\Windows\notepad.exe", Kind::Text),
            ("Note:this", Kind::Text),
            ("@types/node", Kind::Text),
            ("user@localhost", Kind::Text),
            ("a@b.c", Kind::Text),
            ("pkg@1.2.3", Kind::Text),
            ("two words", Kind::Text),
        ] {
            assert_eq!(text_kind(text), kind, "{text:?}");
        }
    }

    fn snapshot(text: Option<&str>, image: bool) -> Snapshot {
        Snapshot {
            text: text.map(str::to_owned),
            image: image.then(|| RawImage::Png(Vec::new())),
            ..Snapshot::default()
        }
    }

    #[test]
    fn picks_the_representation_that_matters() {
        assert_eq!(primary(&snapshot(None, true)), Some(Primary::Image));
        assert_eq!(primary(&snapshot(Some("  "), true)), Some(Primary::Image));
        assert_eq!(
            primary(&snapshot(Some("https://example.com/cat.png"), true)),
            Some(Primary::Image)
        );
        // A spreadsheet cell range comes with a rendered picture.
        assert_eq!(
            primary(&snapshot(Some("A\tB\n1\t2"), true)),
            Some(Primary::Text)
        );
        assert_eq!(primary(&snapshot(Some("hi"), false)), Some(Primary::Text));
        assert_eq!(primary(&snapshot(Some(" \n "), false)), None);
        assert_eq!(primary(&snapshot(None, false)), None);

        let mut files = snapshot(Some("a.txt"), true);
        files.files.push("a.txt".into());
        assert_eq!(primary(&files), Some(Primary::Files));
    }
}
