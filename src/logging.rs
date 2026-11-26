use anyhow::Result;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::paths;

/// Logs to a daily file, and to stderr in debug builds. `MOCLI_LOG` sets the
/// filter, for example `MOCLI_LOG=mocli=debug`.
///
/// The returned guard flushes buffered lines when dropped.
pub fn init() -> Result<WorkerGuard> {
    // The appender complains on stderr if it has to prune a missing directory.
    let directory = paths::log_dir()?;
    std::fs::create_dir_all(&directory)?;
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("mocli")
        .filename_suffix("log")
        .max_log_files(7)
        .build(directory)?;
    let (file, guard) = tracing_appender::non_blocking(appender);

    let filter =
        EnvFilter::try_from_env("MOCLI_LOG").unwrap_or_else(|_| EnvFilter::new("warn,mocli=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_ansi(false).with_writer(file))
        .with(cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr)))
        .init();

    Ok(guard)
}
