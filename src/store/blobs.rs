//! Image files, named after the entry's hash.

use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};

pub struct Blobs(PathBuf);

impl Blobs {
    pub fn open(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)
            .with_context(|| format!("could not create {}", directory.display()))?;
        Ok(Self(directory))
    }

    pub fn image(&self, hash: &str) -> PathBuf {
        self.0.join(format!("{hash}.png"))
    }

    pub fn thumbnail(&self, hash: &str) -> PathBuf {
        self.0.join(format!("{hash}.thumb.png"))
    }

    fn paths(&self, hash: &str) -> [PathBuf; 2] {
        [self.image(hash), self.thumbnail(hash)]
    }

    pub fn write(&self, hash: &str, png: &[u8], thumbnail: &[u8]) -> Result<()> {
        for (path, bytes) in self.paths(hash).iter().zip([png, thumbnail]) {
            write_atomically(path, bytes)
                .with_context(|| format!("could not write {}", path.display()))?;
        }
        Ok(())
    }

    pub fn remove(&self, hash: &str) {
        for path in self.paths(hash) {
            remove_file(&path);
        }
    }

    /// Deletes files no entry refers to, left behind by a crash.
    pub fn remove_orphans(&self, referenced: &HashSet<String>) -> Result<()> {
        for file in fs::read_dir(&self.0)? {
            let file = file?;
            let name = file.file_name();
            let hash = name.to_string_lossy();
            let hash = hash.split('.').next().unwrap_or_default();
            let temporary = name.to_string_lossy().ends_with(".tmp");
            if temporary || !referenced.contains(hash) {
                remove_file(&file.path());
            }
        }
        Ok(())
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
