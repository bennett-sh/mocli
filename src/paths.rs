use std::{env, path::PathBuf};

use anyhow::{Context as _, Result};

const APP_DIR: &str = "mocli";

fn from_env(variable: &str) -> Result<PathBuf> {
    let base = env::var_os(variable).with_context(|| format!("%{variable}% is not set"))?;
    Ok(PathBuf::from(base).join(APP_DIR))
}

/// `%APPDATA%\mocli\config.toml`
pub fn config_file() -> Result<PathBuf> {
    Ok(from_env("APPDATA")?.join("config.toml"))
}

/// Where the history and logs live: `%LOCALAPPDATA%\mocli`, unless
/// `MOCLI_DATA_DIR` points elsewhere.
pub fn data_dir() -> Result<PathBuf> {
    match env::var_os("MOCLI_DATA_DIR") {
        Some(directory) => Ok(directory.into()),
        None => from_env("LOCALAPPDATA"),
    }
}

pub fn log_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("logs"))
}
