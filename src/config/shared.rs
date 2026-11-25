//! The configuration while the app runs: read by every thread, saved from the
//! settings window, and reloaded when the file is edited.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, PoisonError, RwLock},
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result};
use toml_edit::{DocumentMut, Item, Table};

use super::{Config, DEFAULT_FILE};

const POLL: Duration = Duration::from_secs(1);
/// Tables whose keys are data: saving replaces them whole.
const MAPS: [&str; 2] = ["per_app", "keys"];

struct Inner {
    path: PathBuf,
    current: RwLock<Config>,
    changed: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

#[derive(Clone)]
pub struct SharedConfig(Arc<Inner>);

impl SharedConfig {
    /// Reads the file, writing the commented default on first start. A broken
    /// file is reported and left untouched.
    pub fn load(path: PathBuf) -> Self {
        let current = match read(&path) {
            Ok(Some(config)) => config,
            Ok(None) => {
                if let Err(error) = write(&path, DEFAULT_FILE) {
                    tracing::warn!(
                        error = format!("{error:#}"),
                        "could not write the default config"
                    );
                }
                Config::default()
            }
            Err(error) => {
                tracing::warn!(error = format!("{error:#}"), "using the default config");
                Config::default()
            }
        };
        Self(Arc::new(Inner {
            path,
            current: RwLock::new(current),
            changed: OnceLock::new(),
        }))
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn get(&self) -> Config {
        self.read(Config::clone)
    }

    pub fn read<R>(&self, read: impl FnOnce(&Config) -> R) -> R {
        read(
            &self
                .0
                .current
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Calls `changed` after every later change. Only the first call counts.
    pub fn on_change(&self, changed: impl Fn() + Send + Sync + 'static) {
        let _ = self.0.changed.set(Box::new(changed));
    }

    /// Changes settings and saves them, keeping the file's comments.
    pub fn update(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut config = self.get();
        change(&mut config);
        if !self.replace(config.clone()) {
            return Ok(());
        }
        let existing = fs::read_to_string(&self.0.path).unwrap_or_else(|_| DEFAULT_FILE.into());
        write(&self.0.path, &merged(&existing, &config)?)
    }

    /// Takes over the file's content if it is valid and differs.
    fn reload(&self) {
        match read(&self.0.path) {
            Ok(Some(config)) => {
                if self.replace(config) {
                    tracing::info!("config reloaded");
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = format!("{error:#}"), "keeping the previous config");
            }
        }
    }

    /// Returns whether anything changed.
    fn replace(&self, config: Config) -> bool {
        {
            let mut current = self
                .0
                .current
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if *current == config {
                return false;
            }
            *current = config;
        }
        if let Some(changed) = self.0.changed.get() {
            changed();
        }
        true
    }

    /// Reloads the file whenever it is saved.
    pub fn watch(&self) -> Result<()> {
        let shared = self.clone();
        thread::Builder::new()
            .name("config-watch".into())
            .spawn(move || {
                let modified = |path: &Path| fs::metadata(path).and_then(|m| m.modified()).ok();
                let mut seen = modified(&shared.0.path);
                loop {
                    thread::sleep(POLL);
                    let now = modified(&shared.0.path);
                    if now != seen {
                        seen = now;
                        shared.reload();
                    }
                }
            })?;
        Ok(())
    }
}

/// `None` if there is no file.
fn read(path: &Path) -> Result<Option<Config>> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .with_context(|| format!("invalid config {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Replaced in one step, so the watcher never reads half a file.
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))
}

/// The existing file with the values of `config`.
fn merged(existing: &str, config: &Config) -> Result<String> {
    let mut document: DocumentMut = existing
        .parse()
        .or_else(|_| DEFAULT_FILE.parse())
        .context("could not parse the config")?;
    let values: DocumentMut = toml::to_string(config)?.parse()?;
    merge(document.as_table_mut(), values.as_table());
    Ok(document.to_string())
}

fn merge(target: &mut Table, source: &Table) {
    for (key, item) in source {
        match (target.get_mut(key), item) {
            (Some(Item::Table(old)), Item::Table(new)) if !MAPS.contains(&key) => merge(old, new),
            (Some(Item::Table(old)), Item::Table(new)) => {
                old.clear();
                for (name, value) in new {
                    old.insert(name, value.clone());
                }
            }
            (Some(Item::Value(old)), Item::Value(new)) => {
                let mut value = new.clone();
                // Keeps comments written after the value.
                *value.decor_mut() = old.decor().clone();
                *old = value;
            }
            _ => {
                target.insert(key, item.clone());
            }
        }
    }
    // An emptied map is left out when serializing.
    for name in MAPS {
        if !source.contains_key(name)
            && let Some(Item::Table(table)) = target.get_mut(name)
        {
            table.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{config::PopupSize, store::tests::Scratch};

    #[test]
    fn saving_keeps_comments_and_unknown_keys() {
        let existing = "# mine\n[general]\n# about the key\nhotkey = \"Alt+V\" # trailing\ncustom = 1\n\n[paste.per_app]\n\"old.exe\" = \"Ctrl+V\"\n";
        let mut config = Config::default();
        config.appearance.size = PopupSize::Large;
        config.paste.per_app.clear();
        config
            .paste
            .per_app
            .insert("new.exe".into(), "Shift+Insert".parse().unwrap());
        let text = merged(existing, &config).unwrap();
        for kept in ["# mine", "# about the key", "# trailing", "custom = 1"] {
            assert!(text.contains(kept), "{kept} missing in:\n{text}");
        }
        assert!(!text.contains("old.exe"), "{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn updates_are_saved_and_announced_once() {
        let scratch = Scratch::new("config");
        let path = scratch.0.join("config.toml");
        let shared = SharedConfig::load(path.clone());
        assert!(path.is_file());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        shared.on_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });

        shared.update(|c| c.history.max_entries = 5).unwrap();
        shared.update(|c| c.history.max_entries = 5).unwrap();
        shared.reload();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            SharedConfig::load(path.clone()).get().history.max_entries,
            5
        );

        fs::write(&path, "not toml [").unwrap();
        shared.reload();
        assert_eq!(shared.get().history.max_entries, 5);
    }
}
