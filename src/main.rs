#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod color;
mod config;
mod ingest;
mod logging;
mod paths;
mod platform;
mod reuse;
mod store;
mod ui;

fn main() -> anyhow::Result<()> {
    let _log_guard = logging::init()?;

    let Some(_instance) = platform::single_instance::acquire()? else {
        tracing::info!("another instance is already running, exiting");
        return Ok(());
    };

    let config = config::Config::load_or_create(&paths::config_file()?);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    app::run(config)
        .inspect_err(|error| tracing::error!(error = format!("{error:#}"), "fatal error"))
}
