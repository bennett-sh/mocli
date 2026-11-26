//! Looks for a newer release on GitHub, once a day while the setting is on.
//!
//! Every failure is only logged. GitHub answers 404 while the repository is
//! private or has no release yet, and the PC may be offline: none of that is
//! worth interrupting anyone for.

use std::{
    sync::{Mutex, PoisonError},
    thread,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::config::SharedConfig;

pub const RELEASES_PAGE: &str = "https://github.com/bennett-sh/mocli/releases/latest";
const LATEST: &str = "https://api.github.com/repos/bennett-sh/mocli/releases/latest";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Leaves the network alone while the app starts.
const FIRST_DELAY: Duration = Duration::from_secs(60);
/// How often the background thread looks at the clock and the setting.
const POLL: Duration = Duration::from_secs(60 * 60);
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_BYTES: u64 = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotChecked,
    Checking,
    Current,
    /// The newer version, without a leading "v".
    Available(String),
    /// No release can be seen, as while the repository is private.
    NoRelease,
    Failed,
}

static STATUS: Mutex<Status> = Mutex::new(Status::NotChecked);

pub fn status() -> Status {
    STATUS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The newer version, if one was found.
pub fn available() -> Option<String> {
    match status() {
        Status::Available(version) => Some(version),
        _ => None,
    }
}

/// Checks a minute after start and then daily, while the setting is on.
/// `changed` is called whenever the status changes.
pub fn spawn(config: SharedConfig, changed: impl Fn() + Send + 'static) -> Result<()> {
    thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            thread::sleep(FIRST_DELAY);
            // The wall clock, so time spent asleep counts.
            let mut last: Option<SystemTime> = None;
            loop {
                let due = last.is_none_or(|last| last.elapsed().is_ok_and(|age| age >= INTERVAL));
                if due && config.read(|config| config.updates.check) {
                    last = Some(SystemTime::now());
                    run(&changed);
                }
                thread::sleep(POLL);
            }
        })?;
    Ok(())
}

/// Checks right away, unless a check is already running.
pub fn check_now(changed: impl Fn() + Send + 'static) {
    let spawned = thread::Builder::new()
        .name("update-check-now".into())
        .spawn(move || run(&changed));
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the update check");
    }
}

fn run(changed: &dyn Fn()) {
    let previous = {
        let mut status = STATUS.lock().unwrap_or_else(PoisonError::into_inner);
        if *status == Status::Checking {
            return;
        }
        std::mem::replace(&mut *status, Status::Checking)
    };
    changed();

    let found = match latest() {
        Ok(Some(tag)) => match version(&tag) {
            Some(latest) if version(CURRENT).is_some_and(|current| latest > current) => {
                tracing::info!(%tag, "an update is available");
                Status::Available(tag.trim_start_matches(['v', 'V']).to_owned())
            }
            Some(_) => {
                tracing::debug!(%tag, "up to date");
                Status::Current
            }
            None => {
                tracing::warn!(%tag, "the latest release has no version number");
                Status::Failed
            }
        },
        Ok(None) => {
            tracing::info!("no release to check for updates against");
            Status::NoRelease
        }
        Err(error) => {
            tracing::info!(error = format!("{error:#}"), "could not check for updates");
            Status::Failed
        }
    };
    let status = match (found, previous) {
        // A failed check says nothing new about an update found before.
        (Status::NoRelease | Status::Failed, Status::Available(version)) => {
            Status::Available(version)
        }
        (found, _) => found,
    };
    *STATUS.lock().unwrap_or_else(PoisonError::into_inner) = status;
    changed();
}

/// The tag of the latest release, or `None` if GitHub shows none.
fn latest() -> Result<Option<String>> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("mocli/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("could not reach GitHub")?;
    let status = response.status();
    // Private repositories and ones without releases look the same.
    if status == 404 {
        return Ok(None);
    }
    anyhow::ensure!(status.is_success(), "GitHub answered {status}");
    let release: Release = serde_json::from_reader(
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .reader(),
    )
    .context("unexpected answer from GitHub")?;
    Ok(Some(release.tag_name))
}

/// "v1.2.3" or "1.2.3-beta" as numbers. A missing part counts as 0.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().transpose().ok()?.unwrap_or(0);
    let patch = parts.next().transpose().ok()?.unwrap_or(0);
    parts.next().is_none().then_some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(version("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(version("1.2"), Some((1, 2, 0)));
        assert_eq!(version("V2.0.1-beta.1"), Some((2, 0, 1)));
        assert!(version("v0.10.0") > version("v0.9.9"));
        for bad in ["", "v", "latest", "1.x", "1.2.3.4"] {
            assert_eq!(version(bad), None, "{bad}");
        }
    }

    #[test]
    fn this_build_has_a_version() {
        assert!(version(super::CURRENT).is_some());
    }
}
