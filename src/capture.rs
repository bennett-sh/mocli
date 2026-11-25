//! What one clipboard change looked like, before it is classified.

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub text: Option<String>,
    pub image: Option<RawImage>,
    pub files: Vec<PathBuf>,
    /// The `HTML Format` and `Rich Text Format` payloads, kept for pasting.
    pub html: Option<Vec<u8>>,
    pub rtf: Option<Vec<u8>>,
    pub source: Option<Source>,
    /// Milliseconds since the Unix epoch.
    pub copied_at: i64,
}

#[derive(Clone, Debug)]
pub enum RawImage {
    Png(Vec<u8>),
    /// A `CF_DIB` or `CF_DIBV5` block: a bitmap without its file header.
    Dib(Vec<u8>),
}

/// The application the content came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// Full path of the executable.
    pub exe: String,
    /// Its friendly name, such as "Google Chrome".
    pub name: String,
    pub window_title: Option<String>,
}
