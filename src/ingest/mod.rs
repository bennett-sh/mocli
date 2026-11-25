//! Turns clipboard snapshots into history entries.

mod classify;
pub mod code;
pub mod link;
mod picture;

use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

use self::classify::Primary;
use crate::{
    capture::Snapshot,
    config::{CaptureKind, History, Previews},
    platform::shell_image,
    store::{Format, Image, Kind, NewEntry, SharedStore},
};

#[cfg(test)]
pub use self::picture::decode;

const MAX_TEXT_BYTES: usize = 10 * 1024 * 1024;
const MAX_TITLE_CHARS: usize = 200;
/// Edge length of the icons stored for files and source apps.
const FILE_ICON_EDGE: u32 = 128;
const APP_ICON_EDGE: u32 = 48;

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// A copied link waiting for its preview.
struct LinkRequest {
    id: i64,
    hash: blake3::Hash,
    address: String,
}

/// Stores every snapshot on a background thread until the sender is dropped,
/// calling `changed` after each one.
pub fn spawn(
    store: SharedStore,
    settings: History,
    previews: Previews,
    snapshots: Receiver<Snapshot>,
    changed: impl Fn() + Clone + Send + 'static,
) -> Result<()> {
    let links = spawn_link_fetcher(store.clone(), changed.clone())?;
    thread::Builder::new()
        .name("ingest".into())
        .spawn(move || {
            shell_image::init_thread();
            prune(&store, &settings);
            for snapshot in snapshots {
                let entry = match process(snapshot, &settings) {
                    Ok(Some(entry)) => entry,
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::warn!(error = format!("{error:#}"), "could not read a copy");
                        continue;
                    }
                };
                // Never log the content itself.
                // Bound first, so the lock is released before pruning.
                let recorded = store.lock().record(&entry);
                match recorded {
                    Ok(recorded) => {
                        tracing::info!(
                            id = recorded.id,
                            kind = entry.kind.as_str(),
                            new = recorded.new,
                            "entry stored"
                        );
                        add_icons(&store, &entry);
                        if previews.fetch_links
                            && recorded.new
                            && let Some(request) = link_request(recorded.id, &entry)
                        {
                            // The fetcher lives as long as this thread.
                            let _ = links.send(request);
                        }
                    }
                    Err(error) => {
                        tracing::error!(error = format!("{error:#}"), "could not store a copy")
                    }
                }
                prune(&store, &settings);
                changed();
            }
        })?;
    Ok(())
}

fn link_request(id: i64, entry: &NewEntry) -> Option<LinkRequest> {
    let address = entry.text.as_deref()?.trim();
    (entry.kind == Kind::Link && link::is_fetchable(address)).then(|| LinkRequest {
        id,
        hash: entry.hash,
        address: address.to_owned(),
    })
}

/// Stores the pictures shown for copied files and for source apps.
fn add_icons(store: &SharedStore, entry: &NewEntry) {
    let render = |path: &str, edge| {
        shell_image::render(path.as_ref(), edge).and_then(|pixels| picture::to_png(&pixels).ok())
    };
    // Several files share one generic icon.
    if entry.kind == Kind::Files
        && let Some(path) = entry.text.as_deref().filter(|text| !text.contains('\n'))
        && let Some(icon) = render(path, FILE_ICON_EDGE)
        && let Err(error) = store.lock().set_thumbnail(entry.hash, &icon)
    {
        tracing::warn!(error = format!("{error:#}"), "could not save a file icon");
    }
    if let Some(source) = &entry.source {
        // Bound first: rendering must not hold the lock.
        let missing = !store.lock().app_icon(&source.exe).is_file();
        if missing
            && let Some(icon) = render(&source.exe, APP_ICON_EDGE)
            && let Err(error) = store.lock().set_app_icon(&source.exe, &icon)
        {
            tracing::warn!(error = format!("{error:#}"), "could not save an app icon");
        }
    }
}

/// Fetches previews one at a time, so a slow site never delays capturing.
fn spawn_link_fetcher(
    store: SharedStore,
    changed: impl Fn() + Send + 'static,
) -> Result<Sender<LinkRequest>> {
    let (requests, incoming) = mpsc::channel::<LinkRequest>();
    thread::Builder::new()
        .name("link-previews".into())
        .spawn(move || {
            for request in incoming {
                let saved = link::fetch(&request.address).and_then(|preview| {
                    store
                        .lock()
                        .set_link_preview(request.id, request.hash, &preview)
                });
                // Addresses are clipboard content: log ids only.
                match saved {
                    Ok(()) => changed(),
                    Err(_) => tracing::debug!(id = request.id, "no link preview"),
                }
            }
        })?;
    Ok(requests)
}

fn prune(store: &SharedStore, settings: &History) {
    let removed = store
        .lock()
        .prune(settings.max_entries, settings.max_age_days, now_millis());
    match removed {
        Ok(0) => {}
        Ok(removed) => tracing::debug!(removed, "old entries removed"),
        Err(error) => tracing::warn!(error = format!("{error:#}"), "could not prune the history"),
    }
}

/// Returns `None` for content that is empty, switched off or too large.
pub fn process(mut snapshot: Snapshot, settings: &History) -> Result<Option<NewEntry>> {
    let captures = |kind| settings.capture.contains(&kind);
    if !captures(CaptureKind::Text) {
        snapshot.text = None;
    }
    if !captures(CaptureKind::Image) {
        snapshot.image = None;
    }
    // Do not fall back to the file names some apps offer as text.
    if !captures(CaptureKind::Files) && !snapshot.files.is_empty() {
        return Ok(None);
    }

    let Some(primary) = classify::primary(&snapshot) else {
        return Ok(None);
    };
    let entry = match primary {
        Primary::Files => Some(files_entry(&snapshot.files)),
        Primary::Text => snapshot
            .text
            .take()
            .filter(|text| text.len() <= MAX_TEXT_BYTES)
            .map(|text| {
                let formats = [
                    (Format::Html, snapshot.html.take()),
                    (Format::Rtf, snapshot.rtf.take()),
                ]
                .into_iter()
                .filter_map(|(format, data)| Some((format, data?)))
                .collect();
                NewEntry {
                    formats,
                    ..text_entry(text)
                }
            }),
        Primary::Image => match &snapshot.image {
            Some(raw) => image_entry(picture::encode(raw)?, settings.max_image_mb),
            None => None,
        },
    };

    Ok(entry.map(|entry| NewEntry {
        source: snapshot.source,
        copied_at: snapshot.copied_at,
        ..entry
    }))
}

fn entry(kind: Kind, title: String, hash: blake3::Hash) -> NewEntry {
    NewEntry {
        kind,
        title,
        text: None,
        hash,
        language: None,
        image: None,
        size_bytes: 0,
        formats: Vec::new(),
        source: None,
        copied_at: 0,
    }
}

fn hash(tag: &str, parts: &[&[u8]]) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(tag.as_bytes());
    for part in parts {
        // Length prefixes keep ["ab", "c"] and ["a", "bc"] apart.
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize()
}

fn title_of(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

fn text_entry(text: String) -> NewEntry {
    let (kind, language) = classify::text_kind(text.trim());
    NewEntry {
        language: language.map(|language| language.token),
        size_bytes: text.len() as u64,
        ..entry(kind, title_of(&text), hash("text", &[text.as_bytes()]))
    }
    .with_text(text)
}

fn files_entry(paths: &[PathBuf]) -> NewEntry {
    let names: Vec<String> = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let title = match paths {
        [path] => path
            .file_name()
            .map_or_else(|| names[0].clone(), |name| name.to_string_lossy().into()),
        _ => format!("{} files", paths.len()),
    };
    // Folders are not walked; their contents do not count.
    let size_bytes = paths
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum();
    let parts: Vec<&[u8]> = names.iter().map(String::as_bytes).collect();
    NewEntry {
        size_bytes,
        ..entry(Kind::Files, title, hash("files", &parts))
    }
    .with_text(names.join("\n"))
}

fn image_entry(picture: picture::Picture, max_image_mb: u32) -> Option<NewEntry> {
    let size_bytes = picture.png.len() as u64;
    if size_bytes > u64::from(max_image_mb) * 1024 * 1024 {
        tracing::debug!(size_bytes, "image is over the size limit");
        return None;
    }
    let (width, height) = picture.pixels.dimensions();
    // Hash the pixels, so the same picture matches whatever format it came in.
    let digest = hash(
        "image",
        &[
            &width.to_le_bytes(),
            &height.to_le_bytes(),
            picture.pixels.as_raw(),
        ],
    );
    Some(NewEntry {
        size_bytes,
        image: Some(Image {
            png: picture.png,
            thumbnail: picture.thumbnail,
            width,
            height,
        }),
        ..entry(Kind::Image, format!("Image ({width}×{height})"), digest)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{RawImage, Source};

    fn run(snapshot: Snapshot) -> Option<NewEntry> {
        process(snapshot, &History::default()).unwrap()
    }

    fn text(text: &str) -> Snapshot {
        Snapshot {
            text: Some(text.into()),
            ..Snapshot::default()
        }
    }

    #[test]
    fn text_keeps_whitespace_formats_and_source() {
        let source = Source {
            exe: r"C:\app.exe".into(),
            name: "App".into(),
            window_title: None,
        };
        let entry = run(Snapshot {
            html: Some(b"<b>x</b>".to_vec()),
            source: Some(source.clone()),
            copied_at: 42,
            ..text("\n   first line  \nsecond")
        })
        .unwrap();
        assert_eq!(entry.kind, Kind::Text);
        assert_eq!(entry.title, "first line");
        assert_eq!(entry.text.as_deref(), Some("\n   first line  \nsecond"));
        assert_eq!(entry.formats, [(Format::Html, b"<b>x</b>".to_vec())]);
        assert_eq!(entry.source, Some(source));
        assert_eq!(entry.copied_at, 42);
    }

    #[test]
    fn long_titles_are_cut_on_character_boundaries() {
        let entry = run(text(&"é".repeat(500))).unwrap();
        assert_eq!(entry.title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn same_picture_in_two_formats_has_one_hash() {
        let dib = RawImage::Dib(picture::tests::dib(2, 2, [1, 2, 3, 255]));
        let png = RawImage::Png(picture::encode(&dib).unwrap().png);
        let hashes: Vec<_> = [dib, png]
            .into_iter()
            .map(|image| {
                let entry = run(Snapshot {
                    image: Some(image),
                    ..Snapshot::default()
                })
                .unwrap();
                assert_eq!(entry.title, "Image (2×2)");
                entry.hash
            })
            .collect();
        assert_eq!(hashes[0], hashes[1]);
    }

    #[test]
    fn files_are_named_by_count() {
        let files = |paths: &[&str]| Snapshot {
            files: paths.iter().map(PathBuf::from).collect(),
            ..Snapshot::default()
        };
        let one = run(files(&[r"C:\dir\report.pdf"])).unwrap();
        assert_eq!((one.kind, one.title.as_str()), (Kind::Files, "report.pdf"));
        let two = run(files(&["a", "b"])).unwrap();
        assert_eq!(two.title, "2 files");
        assert_eq!(two.text.as_deref(), Some("a\nb"));
        assert_ne!(run(files(&["ab"])).unwrap().hash, two.hash);
    }

    #[test]
    fn switched_off_kinds_are_skipped() {
        let settings = History {
            capture: vec![CaptureKind::Text],
            ..History::default()
        };
        let image = Snapshot {
            image: Some(RawImage::Dib(picture::tests::dib(2, 2, [0; 4]))),
            ..Snapshot::default()
        };
        let files = Snapshot {
            files: vec!["a".into()],
            ..text("a")
        };
        assert!(process(image, &settings).unwrap().is_none());
        assert!(process(files, &settings).unwrap().is_none());
        assert!(process(text("a"), &settings).unwrap().is_some());
    }

    #[test]
    fn oversized_images_are_skipped() {
        let settings = History {
            max_image_mb: 0,
            ..History::default()
        };
        let image = Snapshot {
            image: Some(RawImage::Dib(picture::tests::dib(2, 2, [0; 4]))),
            ..Snapshot::default()
        };
        assert!(process(image, &settings).unwrap().is_none());
    }
}
