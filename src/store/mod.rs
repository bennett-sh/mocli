//! The history database: SQLite for entries, plain files for images.

mod blobs;
mod query;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, ToSql, params};

use self::blobs::Blobs;
pub use self::query::{Details, Payload, Results, Summary};
use crate::{capture::Source, ingest::link::Preview};

const SCHEMA_VERSION: i64 = 2;
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Link,
    Email,
    Color,
    Code,
    Svg,
    Image,
    Files,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Link => "link",
            Self::Email => "email",
            Self::Color => "color",
            Self::Code => "code",
            Self::Svg => "svg",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    pub const ALL: [Self; 8] = [
        Self::Text,
        Self::Link,
        Self::Email,
        Self::Color,
        Self::Code,
        Self::Svg,
        Self::Image,
        Self::Files,
    ];

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// The history, shared between the thread that records and the UI that reads.
#[derive(Clone)]
pub struct SharedStore(Arc<Mutex<Store>>);

impl SharedStore {
    pub fn new(store: Store) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    pub fn lock(&self) -> MutexGuard<'_, Store> {
        // SQLite transactions keep the data consistent even after a panic.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A clipboard format stored alongside an entry's plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Html,
    Rtf,
}

impl Format {
    fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Rtf => "rtf",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::Html, Self::Rtf]
            .into_iter()
            .find(|format| format.as_str() == name)
    }
}

pub struct Image {
    pub png: Vec<u8>,
    pub thumbnail: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct NewEntry {
    pub kind: Kind,
    /// One line shown in the list.
    pub title: String,
    /// The text itself, or one path per line for files.
    pub text: Option<String>,
    /// Identifies the content; copying it again updates the existing entry.
    pub hash: blake3::Hash,
    /// The highlighter's name for the language of code.
    pub language: Option<&'static str>,
    pub image: Option<Image>,
    pub size_bytes: u64,
    pub formats: Vec<(Format, Vec<u8>)>,
    pub source: Option<Source>,
    pub copied_at: i64,
}

impl NewEntry {
    pub fn with_text(self, text: String) -> Self {
        Self {
            text: Some(text),
            ..self
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Recorded {
    pub id: i64,
    /// `false` if the same content was already in the history.
    pub new: bool,
}

pub struct Store {
    db: Connection,
    blobs: Blobs,
}

impl Store {
    pub fn open(directory: &Path) -> Result<Self> {
        let blobs = Blobs::open(directory.join("blobs"))?;
        let path = directory.join("history.db");
        let mut db = Connection::open(&path)
            .with_context(|| format!("could not open {}", path.display()))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "journal_mode", "wal")?;
        db.pragma_update(None, "foreign_keys", true)?;
        // Overwrite deleted clipboard content instead of leaving it on disk.
        db.pragma_update(None, "secure_delete", true)?;
        migrate(&mut db)?;

        let store = Self { db, blobs };
        store.blobs.remove_orphans(&store.hashes()?)?;
        Ok(store)
    }

    fn hashes(&self) -> Result<HashSet<String>> {
        let mut query = self.db.prepare("SELECT hash FROM entries")?;
        let hashes = query.query_map([], |row| row.get::<_, [u8; 32]>(0))?;
        hashes.map(|hash| Ok(hex(hash?))).collect::<Result<_>>()
    }

    /// The icon shown for a file entry.
    pub fn set_thumbnail(&self, hash: blake3::Hash, png: &[u8]) -> Result<()> {
        self.blobs
            .write_file(&self.blobs.thumbnail(&hash.to_hex()), png)
    }

    /// Where the icon of the program at `exe` is kept, if it has been saved.
    pub fn app_icon(&self, exe: &str) -> PathBuf {
        self.blobs.app_icon(exe)
    }

    pub fn set_app_icon(&self, exe: &str, png: &[u8]) -> Result<()> {
        self.blobs.write_file(&self.blobs.app_icon(exe), png)
    }

    pub fn set_link_preview(
        &mut self,
        id: i64,
        hash: blake3::Hash,
        preview: &Preview,
    ) -> Result<()> {
        let hash = hash.to_hex();
        if let Some(image) = &preview.image {
            self.blobs
                .write_file(&self.blobs.link_image(&hash), image)?;
        }
        if let Some(icon) = &preview.icon {
            self.blobs.write_file(&self.blobs.favicon(&hash), icon)?;
        }
        let updated = self.db.execute(
            "UPDATE entries SET link_title = ?2, link_description = ?3 WHERE id = ?1",
            params![id, preview.title, preview.description],
        )?;
        if updated == 0 {
            // Deleted while the page was loading.
            self.blobs.remove(&hash);
        }
        Ok(())
    }

    /// Adds an entry, or bumps the existing one with the same content.
    pub fn record(&mut self, entry: &NewEntry) -> Result<Recorded> {
        // Files first, so a row never points at a missing image.
        let hash = entry.hash.to_hex();
        if let Some(image) = &entry.image {
            self.blobs.write(&hash, &image.png, &image.thumbnail)?;
        }
        if entry.kind == Kind::Svg
            && let Some(text) = &entry.text
        {
            self.blobs
                .write_file(&self.blobs.svg(&hash), text.trim().as_bytes())?;
        }

        let source = entry.source.as_ref();
        let transaction = self.db.transaction()?;
        let (id, copies): (i64, i64) = transaction.query_row(
            "INSERT INTO entries (kind, title, text, hash, size_bytes, width, height,
                 source_exe, source_name, source_title, first_copied, last_copied, language)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)
             ON CONFLICT (hash) DO UPDATE SET
                 source_exe = excluded.source_exe,
                 source_name = excluded.source_name,
                 source_title = excluded.source_title,
                 last_copied = excluded.last_copied,
                 copy_count = copy_count + 1
             RETURNING id, copy_count",
            params![
                entry.kind.as_str(),
                entry.title,
                entry.text,
                entry.hash.as_bytes(),
                entry.size_bytes as i64,
                entry.image.as_ref().map(|image| image.width),
                entry.image.as_ref().map(|image| image.height),
                source.map(|source| &source.exe),
                source.map(|source| &source.name),
                source.and_then(|source| source.window_title.as_ref()),
                entry.copied_at,
                entry.language,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        // The latest copy decides which formatting is kept.
        transaction.execute("DELETE FROM formats WHERE entry_id = ?1", [id])?;
        for (format, data) in &entry.formats {
            transaction.execute(
                "INSERT INTO formats (entry_id, name, data) VALUES (?1, ?2, ?3)",
                params![id, format.as_str(), data],
            )?;
        }
        transaction.commit()?;

        Ok(Recorded {
            id,
            new: copies == 1,
        })
    }

    pub fn set_pinned(&mut self, id: i64, pinned: bool) -> Result<()> {
        self.db.execute(
            "UPDATE entries SET pinned = ?2 WHERE id = ?1",
            params![id, pinned],
        )?;
        Ok(())
    }

    pub fn delete(&mut self, id: i64) -> Result<()> {
        self.delete_where("id = ?1", params![id])?;
        Ok(())
    }

    /// How many entries `delete_from_source` would remove.
    pub fn count_from_source(&self, exe: &str) -> Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT count(*) FROM entries WHERE pinned = 0 AND source_exe = ?1 COLLATE NOCASE",
            [exe],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Removes the unpinned entries copied from one program.
    pub fn delete_from_source(&mut self, exe: &str) -> Result<usize> {
        self.delete_where(
            "pinned = 0 AND source_exe = ?1 COLLATE NOCASE",
            params![exe],
        )
    }

    /// Removes everything except pinned entries.
    pub fn clear_unpinned(&mut self) -> Result<usize> {
        self.delete_where("pinned = 0", params![])
    }

    /// Deletes the matching entries and their files.
    fn delete_where(&mut self, condition: &str, values: &[&dyn ToSql]) -> Result<usize> {
        let mut query = self.db.prepare(&format!(
            "DELETE FROM entries WHERE {condition} RETURNING hash"
        ))?;
        let removed = query
            .query_map(values, |row| row.get::<_, [u8; 32]>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for hash in &removed {
            self.blobs.remove(&hex(*hash));
        }
        Ok(removed.len())
    }

    /// Removes unpinned entries beyond the count or age limits; zero means no
    /// limit. Returns how many were removed.
    pub fn prune(&mut self, max_entries: u32, max_age_days: u32, now: i64) -> Result<usize> {
        let cutoff = match max_age_days {
            0 => i64::MIN,
            days => now - i64::from(days) * MILLIS_PER_DAY,
        };
        // SQLite reads a negative `LIMIT` as unlimited.
        let limit = match max_entries {
            0 => -1,
            count => i64::from(count),
        };
        self.delete_where(
            "pinned = 0 AND (
                 last_copied < ?1
                 OR id NOT IN (SELECT id FROM entries WHERE pinned = 0
                               ORDER BY last_copied DESC, id DESC LIMIT ?2))",
            params![cutoff, limit],
        )
    }
}

fn hex(hash: [u8; 32]) -> String {
    blake3::Hash::from_bytes(hash).to_hex().to_string()
}

fn migrate(db: &mut Connection) -> Result<()> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    anyhow::ensure!(
        version <= SCHEMA_VERSION,
        "the history was written by a newer version of mocli"
    );
    const STEPS: [&str; SCHEMA_VERSION as usize] =
        [include_str!("schema.sql"), include_str!("migrate2.sql")];
    for (step, sql) in STEPS.iter().enumerate().skip(version.max(0) as usize) {
        let transaction = db.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", step as i64 + 1)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use std::fs;

    use super::*;

    /// A directory removed when the test ends.
    pub struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("mocli-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }

        fn blob_count(&self) -> usize {
            fs::read_dir(self.0.join("blobs")).unwrap().count()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub fn text(content: &str, at: i64) -> NewEntry {
        NewEntry {
            kind: Kind::Text,
            title: content.into(),
            text: Some(content.into()),
            hash: blake3::hash(content.as_bytes()),
            language: None,
            image: None,
            size_bytes: content.len() as u64,
            formats: Vec::new(),
            source: None,
            copied_at: at,
        }
    }

    pub fn picture(name: &str, at: i64) -> NewEntry {
        NewEntry {
            kind: Kind::Image,
            text: None,
            image: Some(Image {
                png: vec![1],
                thumbnail: vec![2],
                width: 1,
                height: 1,
            }),
            ..text(name, at)
        }
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store.db.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn matches(store: &Store, query: &str) -> i64 {
        store
            .db
            .query_row(
                "SELECT count(*) FROM entries_fts WHERE entries_fts MATCH ?1",
                [query],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn copying_again_bumps_the_entry() {
        let scratch = Scratch::new("dedupe");
        let mut store = Store::open(&scratch.0).unwrap();
        let mut first = text("hello", 1);
        first.formats = vec![(Format::Html, vec![1])];
        let a = store.record(&first).unwrap();

        let mut again = text("hello", 5);
        again.source = Some(Source {
            exe: "b.exe".into(),
            name: "Browser".into(),
            window_title: None,
        });
        let b = store.record(&again).unwrap();

        assert!(a.new && !b.new);
        assert_eq!(a.id, b.id);
        assert_eq!(count(&store, "SELECT count(*) FROM entries"), 1);
        assert_eq!(count(&store, "SELECT first_copied FROM entries"), 1);
        assert_eq!(count(&store, "SELECT last_copied FROM entries"), 5);
        assert_eq!(count(&store, "SELECT copy_count FROM entries"), 2);
        assert_eq!(count(&store, "SELECT count(*) FROM formats"), 0);
        assert_eq!(matches(&store, "Browser"), 1);
    }

    #[test]
    fn search_index_follows_inserts_and_deletes() {
        let scratch = Scratch::new("search");
        let mut store = Store::open(&scratch.0).unwrap();
        store.record(&text("clipboard history", 1)).unwrap();
        assert_eq!(matches(&store, "board"), 1);
        store.prune(0, 1, 10 * MILLIS_PER_DAY).unwrap();
        assert_eq!(matches(&store, "board"), 0);
        store
            .db
            .execute_batch("INSERT INTO entries_fts (entries_fts) VALUES ('integrity-check')")
            .unwrap();
    }

    #[test]
    fn count_limit_keeps_the_newest_and_the_pinned() {
        let scratch = Scratch::new("count");
        let mut store = Store::open(&scratch.0).unwrap();
        for (name, at) in [("pinned", 1), ("old", 2), ("mid", 3), ("new", 4)] {
            store.record(&text(name, at)).unwrap();
        }
        store
            .db
            .execute("UPDATE entries SET pinned = 1 WHERE title = 'pinned'", [])
            .unwrap();

        assert_eq!(store.prune(2, 0, 100).unwrap(), 1);
        assert_eq!(matches(&store, "old"), 0);
        assert_eq!(count(&store, "SELECT count(*) FROM entries"), 3);
        assert_eq!(store.prune(0, 0, i64::MAX).unwrap(), 0);
    }

    #[test]
    fn pruned_images_lose_their_files() {
        let scratch = Scratch::new("images");
        let mut store = Store::open(&scratch.0).unwrap();
        store.record(&picture("one", 1)).unwrap();
        store.record(&picture("two", 2)).unwrap();
        assert_eq!(scratch.blob_count(), 4);
        store.prune(1, 0, 3).unwrap();
        assert_eq!(scratch.blob_count(), 2);
    }

    #[test]
    fn deletes_by_entry_source_or_everything_unpinned() {
        let scratch = Scratch::new("delete");
        let mut store = Store::open(&scratch.0).unwrap();
        let from = |name: &str, exe: &str, at| NewEntry {
            source: Some(Source {
                exe: exe.into(),
                name: exe.into(),
                window_title: None,
            }),
            ..picture(name, at)
        };
        let a = store.record(&from("a", r"C:\A.exe", 1)).unwrap().id;
        let b = store.record(&from("b", r"c:\a.EXE", 2)).unwrap().id;
        let c = store.record(&from("c", r"C:\other.exe", 3)).unwrap().id;
        let d = store.record(&from("d", r"C:\other.exe", 4)).unwrap().id;
        store.set_pinned(b, true).unwrap();
        store.set_pinned(d, true).unwrap();

        assert_eq!(store.count_from_source(r"C:\a.exe").unwrap(), 1);
        assert_eq!(store.delete_from_source(r"C:\a.exe").unwrap(), 1);
        assert!(store.details(a).unwrap().is_none());
        store.delete(d).unwrap();
        assert_eq!(store.clear_unpinned().unwrap(), 1);
        assert!(store.details(c).unwrap().is_none());
        assert!(store.details(b).unwrap().unwrap().pinned);
        assert_eq!(scratch.blob_count(), 2);
        assert_eq!(matches(&store, "other"), 0);
    }

    #[test]
    fn upgrades_a_version_one_history() {
        let scratch = Scratch::new("upgrade");
        fs::create_dir_all(&scratch.0).unwrap();
        let db = Connection::open(scratch.0.join("history.db")).unwrap();
        db.execute_batch(include_str!("schema.sql")).unwrap();
        db.execute(
            "INSERT INTO entries (kind, title, text, hash, size_bytes, first_copied, last_copied)
             VALUES ('text', 'old entry', 'old entry', ?1, 9, 1, 1)",
            [[7u8; 32]],
        )
        .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        drop(db);

        let mut store = Store::open(&scratch.0).unwrap();
        assert_eq!(matches(&store, "old"), 1);
        store.record(&text("new entry", 2)).unwrap();
        assert_eq!(matches(&store, "entry"), 2);
        assert_eq!(count(&store, "PRAGMA user_version"), SCHEMA_VERSION);
        store
            .db
            .execute_batch("INSERT INTO entries_fts (entries_fts) VALUES ('integrity-check')")
            .unwrap();
    }

    #[test]
    fn link_previews_are_searchable_and_cleaned_up() {
        let scratch = Scratch::new("links");
        let mut store = Store::open(&scratch.0).unwrap();
        let entry = text("https://example.com", 1);
        let id = store.record(&entry).unwrap().id;
        let preview = Preview {
            title: Some("Marvellous heading".into()),
            description: None,
            image: Some(vec![1]),
            icon: Some(vec![2]),
        };
        store.set_link_preview(id, entry.hash, &preview).unwrap();
        assert_eq!(matches(&store, "Marvellous"), 1);
        assert_eq!(scratch.blob_count(), 2);
        store.prune(0, 1, 10 * MILLIS_PER_DAY).unwrap();
        assert_eq!(scratch.blob_count(), 0);
        // A preview arriving for a deleted entry leaves nothing behind.
        store.set_link_preview(id, entry.hash, &preview).unwrap();
        assert_eq!(scratch.blob_count(), 0);
    }

    #[test]
    fn reopening_keeps_entries_and_removes_orphans() {
        let scratch = Scratch::new("reopen");
        let mut store = Store::open(&scratch.0).unwrap();
        store.record(&picture("kept", 1)).unwrap();
        drop(store);
        fs::write(scratch.0.join("blobs/deadbeef.png"), [0]).unwrap();
        fs::write(scratch.0.join("blobs/half.tmp"), [0]).unwrap();

        let store = Store::open(&scratch.0).unwrap();
        assert_eq!(count(&store, "SELECT count(*) FROM entries"), 1);
        assert_eq!(scratch.blob_count(), 2);
    }
}
