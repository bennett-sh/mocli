//! Where settings, history and logs are kept.
//!
//! Normally in the user's profile. A portable copy, marked by a file named
//! `portable` next to the executable, keeps them in a `data` folder there.

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};

const APP_DIR: &str = "mocli";
const MARKER: &str = "portable";

/// The data folder of a portable copy installed in `directory`.
fn portable_data(directory: &Path) -> Option<PathBuf> {
    directory
        .join(MARKER)
        .is_file()
        .then(|| directory.join("data"))
}

fn portable() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    portable_data(exe.parent()?)
}

/// Whether this copy keeps its data beside the executable.
pub fn is_portable() -> bool {
    portable().is_some()
}

fn from_env(variable: &str) -> Result<PathBuf> {
    let base = env::var_os(variable).with_context(|| format!("%{variable}% is not set"))?;
    Ok(PathBuf::from(base).join(APP_DIR))
}

/// `%APPDATA%\mocli\config.toml`, or `data\config.toml` when portable.
pub fn config_file() -> Result<PathBuf> {
    let directory = match portable() {
        Some(data) => data,
        None => from_env("APPDATA")?,
    };
    Ok(directory.join("config.toml"))
}

/// Where the history and logs live: `%LOCALAPPDATA%\mocli`, or `data` when
/// portable. `MOCLI_DATA_DIR` overrides both.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(directory) = env::var_os("MOCLI_DATA_DIR") {
        return Ok(directory.into());
    }
    match portable() {
        Some(data) => Ok(data),
        None => from_env("LOCALAPPDATA"),
    }
}

pub fn log_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("logs"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::store::tests::Scratch;

    #[test]
    fn a_marker_file_makes_a_copy_portable() {
        let scratch = Scratch::new("portable");
        fs::create_dir_all(&scratch.0).unwrap();
        assert_eq!(portable_data(&scratch.0), None);
        fs::write(scratch.0.join(MARKER), "").unwrap();
        assert_eq!(portable_data(&scratch.0), Some(scratch.0.join("data")));
    }
}
