//! Watches the clipboard and reads each change on a dedicated thread.
//!
//! Content marked as private by its app, or copied from an ignored app, is
//! dropped before it is read.

use std::{
    ffi::c_void,
    path::PathBuf,
    slice,
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result};
use windows::{
    Win32::{
        Foundation::{HGLOBAL, HWND},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, GetClipboardData,
                GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT},
        },
        UI::{
            Shell::{DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DispatchMessageW, GetMessageW, HWND_MESSAGE, KillTimer, MSG,
                SetTimer, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WM_TIMER,
            },
        },
    },
    core::{PCWSTR, w},
};

use super::source_app;
use crate::{
    capture::{RawImage, Snapshot, Source},
    config::Privacy,
    ingest::now_millis,
};

/// A format only mocli sets, on content it puts back from the history.
pub const SELF_MARKER: PCWSTR = w!("mocli.self");
/// Apps fill the clipboard in several steps, so wait for it to settle.
const SETTLE_MS: u32 = 80;
const TIMER: usize = 1;
const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;
const MAX_FORMAT_BYTES: usize = 5 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// Ids of the formats Windows only knows by name.
struct Registered {
    png: u32,
    html: u32,
    rtf: u32,
    own: u32,
    /// Presence means clipboard tools must ignore the content.
    exclude: [u32; 2],
    /// A zero value means the content must stay out of histories.
    include_in_history: u32,
}

impl Registered {
    fn new() -> Self {
        // SAFETY: the names are static, null-terminated strings.
        let register = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) };
        Self {
            png: register(w!("PNG")),
            html: register(w!("HTML Format")),
            rtf: register(w!("Rich Text Format")),
            own: register(SELF_MARKER),
            exclude: [
                register(w!("ExcludeClipboardContentFromMonitorProcessing")),
                register(w!("Clipboard Viewer Ignore")),
            ],
            include_in_history: register(w!("CanIncludeInClipboardHistory")),
        }
    }
}

struct Watcher {
    window: HWND,
    formats: Registered,
    privacy: Privacy,
    snapshots: Sender<Snapshot>,
    last_sequence: u32,
}

/// Starts watching and returns once changes are being observed.
pub fn spawn(privacy: Privacy, snapshots: Sender<Snapshot>) -> Result<()> {
    let (ready_tx, ready_rx) = mpsc::channel();

    thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || match Watcher::new(privacy, snapshots) {
            Ok(watcher) => {
                let _ = ready_tx.send(Ok(()));
                watcher.run();
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })?;

    ready_rx
        .recv()
        .context("the clipboard thread exited early")?
}

impl Watcher {
    fn new(privacy: Privacy, snapshots: Sender<Snapshot>) -> Result<Self> {
        // SAFETY: a message-only window of a system class, owned by this
        // thread until the process exits.
        let window = unsafe {
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("mocli clipboard"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )?;
            AddClipboardFormatListener(window).context("failed to watch the clipboard")?;
            window
        };
        Ok(Self {
            window,
            formats: Registered::new(),
            privacy,
            snapshots,
            // Whatever is already on the clipboard was not copied while we ran.
            last_sequence: unsafe { GetClipboardSequenceNumber() },
        })
    }

    fn run(mut self) {
        let mut pending: Option<(Option<Source>, i64)> = None;
        let mut message = MSG::default();
        // SAFETY: `message` is a valid out pointer.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            match message.message {
                WM_CLIPBOARDUPDATE => {
                    // Ask now: the owner may be gone once the timer fires.
                    pending = Some((source_app::current(), now_millis()));
                    // SAFETY: restarting a timer on our own window.
                    if unsafe { SetTimer(Some(self.window), TIMER, SETTLE_MS, None) } == 0 {
                        tracing::warn!("could not schedule a clipboard read");
                    }
                }
                WM_TIMER if message.wParam.0 == TIMER => {
                    // SAFETY: stopping a timer on our own window.
                    let _ = unsafe { KillTimer(Some(self.window), TIMER) };
                    if let Some((source, copied_at)) = pending.take() {
                        self.capture(source, copied_at);
                    }
                }
                // SAFETY: `message` was filled in by `GetMessageW`.
                _ => unsafe {
                    DispatchMessageW(&message);
                },
            }
        }
    }

    fn capture(&mut self, source: Option<Source>, copied_at: i64) {
        // SAFETY: plain query.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence == self.last_sequence {
            return;
        }
        self.last_sequence = sequence;

        if let Some(source) = &source
            && self.privacy.ignores(&source.exe)
        {
            tracing::debug!(app = %source.name, "skipped a copy from an ignored app");
            return;
        }

        let _open = match Clipboard::open(self.window) {
            Ok(open) => open,
            Err(error) => {
                tracing::warn!(error = format!("{error:#}"), "could not open the clipboard");
                return;
            }
        };
        if is_available(self.formats.own) {
            return;
        }
        if self.privacy.respect_exclusion_formats && self.is_excluded() {
            tracing::debug!("skipped content marked as private");
            return;
        }

        let snapshot = Snapshot {
            source,
            copied_at,
            ..self.read()
        };
        // Asking for delayed formats can itself move the sequence on.
        // SAFETY: plain query.
        self.last_sequence = unsafe { GetClipboardSequenceNumber() };
        // Fails only during shutdown.
        let _ = self.snapshots.send(snapshot);
    }

    /// Must be called while the clipboard is open.
    fn is_excluded(&self) -> bool {
        self.formats.exclude.into_iter().any(is_available)
            || data(self.formats.include_in_history, MAX_FORMAT_BYTES)
                .is_some_and(|bytes| bytes.iter().take(4).all(|&byte| byte == 0))
    }

    /// Must be called while the clipboard is open.
    fn read(&self) -> Snapshot {
        let files = files();
        // File copies carry no other content worth keeping.
        if !files.is_empty() {
            return Snapshot {
                files,
                ..Snapshot::default()
            };
        }

        let text = data(CF_UNICODETEXT.0.into(), MAX_TEXT_BYTES).map(|bytes| utf16(&bytes));
        let has_text = text.is_some();
        // PNG keeps transparency. `CF_DIB` decodes more reliably than the V5
        // variant Windows synthesizes from it.
        let image = data(self.formats.png, MAX_IMAGE_BYTES)
            .map(RawImage::Png)
            .or_else(|| data(CF_DIB.0.into(), MAX_IMAGE_BYTES).map(RawImage::Dib))
            .or_else(|| data(CF_DIBV5.0.into(), MAX_IMAGE_BYTES).map(RawImage::Dib));
        let rich = |format| has_text.then(|| data(format, MAX_FORMAT_BYTES)).flatten();

        Snapshot {
            html: rich(self.formats.html),
            rtf: rich(self.formats.rtf),
            text,
            image,
            ..Snapshot::default()
        }
    }
}

/// Holds the clipboard open; other apps cannot use it until this drops.
pub struct Clipboard;

impl Clipboard {
    pub fn open(window: HWND) -> Result<Self> {
        let mut attempt = 0;
        loop {
            // SAFETY: `window` belongs to this thread.
            match unsafe { OpenClipboard(Some(window)) } {
                Ok(()) => return Ok(Self),
                Err(error) if attempt == 20 => return Err(error.into()),
                // Another app has it open, usually only briefly.
                Err(_) => thread::sleep(Duration::from_millis(15)),
            }
            attempt += 1;
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        // SAFETY: we opened it.
        let _ = unsafe { CloseClipboard() };
    }
}

fn is_available(format: u32) -> bool {
    // SAFETY: plain query.
    unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

/// Copies one format out of the open clipboard, up to `limit` bytes.
fn data(format: u32, limit: usize) -> Option<Vec<u8>> {
    if !is_available(format) {
        return None;
    }
    // SAFETY: the handle stays valid while the clipboard is open, and the
    // slice is copied before the memory is unlocked.
    unsafe {
        let memory = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(memory);
        if size == 0 || size > limit {
            return None;
        }
        let pointer: *const c_void = GlobalLock(memory);
        if pointer.is_null() {
            return None;
        }
        let bytes = slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec();
        // Reports an error once the lock count reaches zero.
        let _ = GlobalUnlock(memory);
        Some(bytes)
    }
}

/// Decodes null-terminated UTF-16.
fn utf16(bytes: &[u8]) -> String {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&unit| unit != 0);
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn files() -> Vec<PathBuf> {
    let format = u32::from(CF_HDROP.0);
    if !is_available(format) {
        return Vec::new();
    }
    // SAFETY: the drop handle stays valid while the clipboard is open, and
    // each buffer is sized from the length Windows reports.
    unsafe {
        let Ok(handle) = GetClipboardData(format) else {
            return Vec::new();
        };
        let drop = HDROP(handle.0);
        (0..DragQueryFileW(drop, u32::MAX, None))
            .map(|index| {
                let length = DragQueryFileW(drop, index, None) as usize;
                let mut buffer = vec![0; length + 1];
                let copied = DragQueryFileW(drop, index, Some(&mut buffer)) as usize;
                PathBuf::from(String::from_utf16_lossy(&buffer[..copied]))
            })
            .collect()
    }
}
