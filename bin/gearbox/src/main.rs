//! gearbox — USD tractor simulator. mara owns the window, ribbons and panes;
//! Bevy renders the world into mara's viewport. The simulator surface
//! (planet world, multi-USD loader, machines, physics, tool API) is wired in
//! `app`, the panes in `host`, the per-frame viewer state in `viewer`.

#![allow(
    dead_code,
    clippy::collapsible_if,
    clippy::field_reassign_with_default,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_cast,
    clippy::unnecessary_map_or,
    clippy::useless_conversion
)]

mod app;
mod attach;
mod controller;
mod host;
mod links;
mod load;
mod physics;
mod physics_debug;
mod services;
mod terrain;
mod usd_ext;
mod viewer;
mod world;

use viewer::log::{LoaderLog, LoaderLogLayer};

/// Trace, panics and backtraces are mirrored here so a crash that takes the
/// window is still readable afterwards.
const LOG_FILE: &str = "/tmp/gearbox-sim.log";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli_paths: Vec<std::path::PathBuf> = std::env::args()
        .skip(1)
        .map(|s| {
            let p = std::path::PathBuf::from(&s);
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            }
        })
        .collect();
    let log = LoaderLog::default();
    init_tracing(&log);
    install_panic_logger();
    tracing::info!(target: "gearbox", "gearbox-sim starting — full log at {LOG_FILE}");
    host::run(cli_paths, log)
}

/// Tracing to stderr, the log file and the Log pane. The embedded Bevy app
/// has no `LogPlugin`, so this is the only subscriber. `RUST_LOG` overrides
/// the filter.
fn init_tracing(log: &LoaderLog) {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, fmt};

    let _ = std::fs::write(LOG_FILE, "");
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "warn,gearbox=info,gearbox_sim=info,gearbox_api=info,usd_bevy=info,agentio=error",
        )
    });
    let to_file = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
            .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap())
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(LoaderLogLayer::new(log))
        .with(
            fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(std::io::stderr.and(to_file)),
        )
        .try_init();
}

/// Panics (message and backtrace) go to the log file and stderr; a panic in
/// the embedded app would otherwise vanish with the window.
fn install_panic_logger() {
    unsafe {
        if std::env::var_os("RUST_BACKTRACE").is_none() {
            std::env::set_var("RUST_BACKTRACE", "full");
        }
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = format!("\n==== gearbox-sim PANIC ====\n{info}\n{backtrace}\n");
        tracing::error!(target: "gearbox", "PANIC: {info}");
        eprint!("{msg}");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
        {
            let _ = f.write_all(msg.as_bytes());
        }
        prev(info);
    }));
}
