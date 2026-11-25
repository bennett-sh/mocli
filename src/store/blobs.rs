//! Image files, named after the entry's hash and encrypted like the database.

use std::{
    collections::HashSet,
    fs,
    hash::{Hash, Hasher},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result};

use super::vault::Cipher;

const IMAGE: &str = "png";
const THUMBNAIL: &str = "thumb.png";
const SVG: &str = "svg";
const LINK_IMAGE: &str = "link.png";
const FAVICON: &str = "icon.png";
/// Every kind of file an entry can have.
const KINDS: [&str; 5] = [IMAGE, THUMBNAIL, SVG, LINK_IMAGE, FAVICON];
/// Icons of source apps are shared by their entries, so they are kept apart.
const APPS: &str = "apps";

/// A stored file. Reading it decrypts it.
#[derive(Clone)]
pub struct Blob {
    path: PathBuf,
    cipher: Option<Arc<Cipher>>,
}

impl Blob {
    /// A file kept as it is.
    #[cfg(test)]
    pub fn plain(path: PathBuf) -> Self {
        Self { path, cipher: None }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub fn read(&self) -> Result<Vec<u8>> {
        let data = fs::read(&self.path)
            .with_context(|| format!("could not read {}", self.path.display()))?;
        match &self.cipher {
            Some(cipher) => cipher.open(&self.file_name(), &data),
            None => Ok(data),
        }
    }

    pub fn write(&self, content: &[u8]) -> Result<()> {
        let sealed;
        let data = match &self.cipher {
            Some(cipher) => {
                sealed = cipher.seal(&self.file_name(), content)?;
                &sealed
            }
            None => content,
        };
        write_atomically(&self.path, data)
            .with_context(|| format!("could not write {}", self.path.display()))
    }
}

impl std::fmt::Debug for Blob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Blob").field(&self.path).finish()
    }
}

// A path is either encrypted or not, never both, so it identifies the content.
impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for Blob {}

impl Hash for Blob {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

pub struct Blobs {
    directory: PathBuf,
    cipher: Option<Arc<Cipher>>,
}

impl Blobs {
    pub fn open(directory: PathBuf, cipher: Option<Cipher>) -> Result<Self> {
        fs::create_dir_all(&directory)
            .with_context(|| format!("could not create {}", directory.display()))?;
        Ok(Self {
            directory,
            cipher: cipher.map(Arc::new),
        })
    }

    /// The file name, without extension, standing for `content`.
    fn stem(&self, content: &str) -> String {
        match &self.cipher {
            Some(cipher) => cipher.name(content),
            None => content.to_owned(),
        }
    }

    fn at(&self, path: PathBuf) -> Blob {
        Blob {
            path,
            cipher: self.cipher.clone(),
        }
    }

    fn entry_file(&self, hash: &str, kind: &str) -> Blob {
        self.at(self.directory.join(format!("{}.{kind}", self.stem(hash))))
    }

    pub fn image(&self, hash: &str) -> Blob {
        self.entry_file(hash, IMAGE)
    }

    pub fn thumbnail(&self, hash: &str) -> Blob {
        self.entry_file(hash, THUMBNAIL)
    }

    pub fn svg(&self, hash: &str) -> Blob {
        self.entry_file(hash, SVG)
    }

    pub fn link_image(&self, hash: &str) -> Blob {
        self.entry_file(hash, LINK_IMAGE)
    }

    pub fn favicon(&self, hash: &str) -> Blob {
        self.entry_file(hash, FAVICON)
    }

    fn app_stem(&self, exe: &str) -> String {
        let name = blake3::hash(exe.to_lowercase().as_bytes()).to_hex();
        self.stem(&name)
    }

    pub fn app_icon(&self, exe: &str) -> Blob {
        let name = format!("{}.png", self.app_stem(exe));
        self.at(self.directory.join(APPS).join(name))
    }

    /// Deletes every file belonging to an entry.
    pub fn remove(&self, hash: &str) {
        for kind in KINDS {
            remove_file(&self.entry_file(hash, kind).path);
        }
    }

    /// Writes the files of these entries and apps again as `to` keeps them. The
    /// originals stay.
    pub fn copy_to(&self, to: &Blobs, hashes: &[String], apps: &[String]) -> Result<()> {
        let entries = hashes
            .iter()
            .flat_map(|hash| KINDS.map(|kind| (self.entry_file(hash, kind), hash, kind)))
            .map(|(from, hash, kind)| (from, to.entry_file(hash, kind)));
        let icons = apps
            .iter()
            .map(|exe| (self.app_icon(exe), to.app_icon(exe)));
        for (from, to) in entries.chain(icons) {
            if !from.exists() {
                continue;
            }
            match from.read() {
                Ok(content) => to.write(&content)?,
                // One damaged picture must not keep the rest from moving.
                Err(error) => {
                    tracing::warn!(error = format!("{error:#}"), "dropped an unreadable file");
                }
            }
        }
        Ok(())
    }

    /// Deletes files nothing refers to, left behind by a crash or by a change
    /// of encryption.
    pub fn remove_orphans(&self, hashes: &[String], apps: &[String]) -> Result<()> {
        let entries: HashSet<String> = hashes.iter().map(|hash| self.stem(hash)).collect();
        remove_unlisted(&self.directory, &entries)?;
        let icons: HashSet<String> = apps.iter().map(|exe| self.app_stem(exe)).collect();
        let apps = self.directory.join(APPS);
        if apps.is_dir() {
            remove_unlisted(&apps, &icons)?;
        }
        Ok(())
    }
}

fn remove_unlisted(directory: &Path, stems: &HashSet<String>) -> Result<()> {
    for file in fs::read_dir(directory)? {
        let file = file?;
        if file.file_type()?.is_dir() {
            continue;
        }
        let name = file.file_name();
        let name = name.to_string_lossy();
        let stem = name.split('.').next().unwrap_or_default();
        if name.ends_with(".tmp") || !stems.contains(stem) {
            remove_file(&file.path());
        }
    }
    Ok(())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

fn remove_file(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), "could not delete a blob");
    }
}
