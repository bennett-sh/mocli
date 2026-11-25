//! Opening the history encrypted or not, and converting it between the two.

use std::{
    error, fmt,
    fs::{self, File},
    io::{ErrorKind, Read as _},
    path::Path,
};

use anyhow::{Context as _, Result};
use rusqlite::Connection;

use super::{
    DATABASE, Store,
    blobs::Blobs,
    connect, migrate,
    vault::{self, MasterKey, Protector},
};

/// The start of every database that is not encrypted.
const PLAIN_HEADER: &[u8] = b"SQLite format 3\0";
const CONVERTING: &str = "history.db.converting";

/// The history is encrypted and its key cannot be used. Nothing was changed.
#[derive(Debug)]
pub struct Locked;

impl fmt::Display for Locked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the history cannot be unlocked")
    }
}

impl error::Error for Locked {}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Missing,
    Plain,
    Encrypted,
}

fn state(database: &Path) -> Result<State> {
    let file = match File::open(database) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(State::Missing),
        Err(error) => return Err(error).context("could not open the history"),
    };
    let mut header = Vec::new();
    file.take(PLAIN_HEADER.len() as u64)
        .read_to_end(&mut header)?;
    Ok(if header.is_empty() {
        State::Missing
    } else if header == PLAIN_HEADER {
        State::Plain
    } else {
        State::Encrypted
    })
}

impl Store {
    /// Opens the history in `directory`, first encrypting or decrypting it if
    /// `encrypt` asks for the other form. Fails with [`Locked`] if it is
    /// encrypted and the key is unusable.
    pub fn open_protected(directory: &Path, encrypt: bool, tpm_key: &str) -> Result<Self> {
        fs::create_dir_all(directory)
            .with_context(|| format!("could not create {}", directory.display()))?;
        let database = directory.join(DATABASE);
        // Left over from a conversion that did not finish.
        remove_if_present(&directory.join(CONVERTING))?;

        let state = state(&database)?;
        let stored = vault::load(directory, tpm_key);
        let key: Option<(MasterKey, Protector)> = match state {
            State::Encrypted => {
                let found = stored
                    .context(Locked)?
                    .context("the key file is missing")
                    .context(Locked)?;
                connect(&database, Some(&found.0)).context(Locked)?;
                if encrypt {
                    Some(found)
                } else {
                    convert(directory, Some(&found.0), None)?;
                    None
                }
            }
            State::Plain | State::Missing if encrypt => {
                // A key from an interrupted conversion is used again. A broken
                // one protects nothing yet and is replaced.
                let key = match stored {
                    Ok(Some(key)) => key,
                    _ => vault::create(directory, tpm_key)?,
                };
                if state == State::Plain {
                    convert(directory, None, Some(&key.0))?;
                }
                Some(key)
            }
            State::Plain | State::Missing => None,
        };
        if key.is_none() {
            vault::remove(directory)?;
        }

        let (key, protector) = key.unzip();
        let mut store = Self::with_key(directory, key.as_ref())?;
        store.protector = protector;
        Ok(store)
    }
}

/// Moves an unreadable history out of the way, into a folder of its own.
pub fn set_aside(directory: &Path, now: i64) -> Result<()> {
    let destination = directory.join(format!("locked-{now}"));
    fs::create_dir_all(&destination)?;
    for name in [
        DATABASE,
        "history.db-wal",
        "history.db-shm",
        "history.key",
        "blobs",
    ] {
        match fs::rename(directory.join(name), destination.join(name)) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(error).with_context(|| format!("could not move {name}"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).with_context(|| format!("could not delete {}", path.display()))
        }
        _ => Ok(()),
    }
}

fn count(db: &Connection) -> Result<i64> {
    Ok(db.query_row("SELECT count(*) FROM entries", [], |row| row.get(0))?)
}

/// Rewrites the history from one key to another. The database is replaced in a
/// single step at the end, so stopping part way loses nothing.
fn convert(directory: &Path, from: Option<&MasterKey>, to: Option<&MasterKey>) -> Result<()> {
    let database = directory.join(DATABASE);
    let converted = directory.join(CONVERTING);
    let mut db = connect(&database, from)?;
    migrate(&mut db)?;

    let old = Blobs::open(directory.join("blobs"), from.map(MasterKey::files))?;
    let new = Blobs::open(directory.join("blobs"), to.map(MasterKey::files))?;
    let (hashes, apps) = super::references(&db)?;
    old.copy_to(&new, &hashes, &apps)?;

    // An empty key means no encryption.
    let key = to.map(MasterKey::database).unwrap_or_default();
    let path = converted
        .to_str()
        .context("the data folder's path is not text")?;
    db.execute("ATTACH DATABASE ?1 AS converted KEY ?2", [path, &key])?;
    db.query_row("SELECT sqlcipher_export('converted')", [], |_| Ok(()))?;
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    db.pragma_update(Some("converted"), "user_version", version)?;
    db.execute("DETACH DATABASE converted", [])?;
    let expected = count(&db)?;
    // Closing the only connection folds the log into the old database.
    db.close().map_err(|(_, error)| error)?;
    anyhow::ensure!(
        !directory.join("history.db-wal").exists(),
        "the history is in use by another program"
    );

    let check = connect(&converted, to)?;
    let result: String = check.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    anyhow::ensure!(
        result == "ok" && count(&check)? == expected,
        "the converted history is incomplete"
    );
    check.close().map_err(|(_, error)| error)?;
    fs::rename(&converted, &database).context("could not replace the history")?;
    tracing::info!(
        encrypted = to.is_some(),
        entries = expected,
        "converted the history"
    );
    Ok(())
}
