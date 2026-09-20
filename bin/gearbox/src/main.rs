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
mod environment;
mod biomes;
mod globe;
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
fn log_file() -> &'static std::path::Path {
    static PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var_os("GEARBOX_LOG_FILE")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/tmp/gearbox-sim.log".into())
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // No arguments, or an explicit `launch`, boots the sim; anything else is
    // a CLI command handled by `gearbox_cli::run()` in this same process —
    // there is no separate CLI binary any more.
    let launching = args.is_empty() || args.first().map(String::as_str) == Some("launch");
    if !launching {
        std::process::exit(gearbox_cli::run());
    }
    if !args.is_empty() {
        args.remove(0);
    }
    let cli_paths: Vec<std::path::PathBuf> = args
        .iter()
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
    tracing::info!(target: "gearbox", "gearbox-sim starting — full log at {}", log_file().display());
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

    let _ = std::fs::write(log_file(), "");
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "warn,gearbox=info,gearbox_api=info,usd_bevy=info,bevy_diagnostic=info,agentio=error",
        )
    });
    let to_file = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file())
            .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap())
    };
    // A fresh layer per subscriber: its type depends on the layers under it.
    macro_rules! fmt_layer {
        () => {
            fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(std::io::stderr.and(to_file))
        };
    }
    // Profiling: the trace sees every span; the log filter moves onto the logs.
    #[cfg(feature = "profile")]
    if let Ok(path) = std::env::var("GEARBOX_TRACE") {
        use tracing_subscriber::Layer;
        let _ = tracing_subscriber::registry()
            .with(chrome_layer(path))
            .with(LoaderLogLayer::new(log).and_then(fmt_layer!()).with_filter(filter))
            .try_init();
        return;
    }
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(LoaderLogLayer::new(log))
        .with(fmt_layer!())
        .try_init();
}

/// Chrome trace of every Bevy system and render stage (`profile` feature and
/// `GEARBOX_TRACE=path`), written for `GEARBOX_TRACE_SECONDS` (default 60)
/// and then flushed; open it in ui.perfetto.dev.
#[cfg(feature = "profile")]
fn chrome_layer<S>(path: String) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> + Send + Sync,
{
    use tracing_subscriber::Layer;
    let seconds = std::env::var("GEARBOX_TRACE_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);
    let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
        .file(path)
        .include_args(true)
        .build();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        drop(guard);
    });
    // Rapier and parry trace every constraint; recording them costs more than solving.
    layer.with_filter(tracing_subscriber::EnvFilter::new(
        std::env::var("GEARBOX_TRACE_FILTER")
            .unwrap_or_else(|_| "info,rapier3d_f64=warn,parry3d_f64=warn".into()),
    ))
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
            .open(log_file())
        {
            let _ = f.write_all(msg.as_bytes());
        }
        prev(info);
    }));
}
