//! One module per top-level group. Each exposes `Args` and `run(ctx, args)`.

pub mod access;
pub mod api;
pub mod clear;
pub mod env;
pub mod extension;
pub mod instance;
pub mod machine;
pub mod peer;
pub mod run;
pub mod scene;
pub mod select;
pub mod spawn;

use gearbox_api::Status;

use crate::error::{CliError, Result};

/// Turn a wire status into a CLI result.
pub fn check(status: Status, what: &str) -> Result<Status> {
    if status.is_ok() {
        Ok(status)
    } else {
        Err(CliError::from_status(&status, what))
    }
}

/// `X Y Z` triples from the command line.
pub fn xyz(at: &Option<Vec<f32>>) -> (f32, f32, f32) {
    match at.as_deref() {
        Some([x, y, z]) => (*x, *y, *z),
        Some([x, z]) => (*x, 0.0, *z),
        _ => (0.0, 0.0, 0.0),
    }
}

/// Resolve a USD path the way the sim does: a file that exists here goes
/// as an absolute path, anything else is handed to the sim's asset root.
pub fn usd_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    if p.exists() {
        if let Ok(abs) = std::fs::canonicalize(p) {
            return abs.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

pub fn default_id(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// A flag that flips on `SIGINT`/`SIGTERM`, for commands that stream until
/// interrupted.
pub fn install_ctrlc() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    let flag = FLAG
        .get_or_init(|| {
            let flag = Arc::new(AtomicBool::new(false));
            extern "C" fn handler(_: libc::c_int) {
                if let Some(f) = FLAG.get() {
                    f.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            // SAFETY: installing a plain signal handler that only stores a flag.
            unsafe {
                libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
                libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
            }
            flag
        })
        .clone();
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    flag
}

pub fn parse_duration(text: &str) -> Result<std::time::Duration> {
    let t = text.trim();
    let (num, unit) = t
        .find(|c: char| c.is_ascii_alphabetic())
        .map(|i| t.split_at(i))
        .unwrap_or((t, "s"));
    let value: f64 = num
        .trim()
        .parse()
        .map_err(|_| CliError::usage(format!("bad duration `{text}`")))?;
    let secs = match unit.trim() {
        "ms" => value / 1000.0,
        "s" | "" => value,
        "m" | "min" => value * 60.0,
        "h" => value * 3600.0,
        other => return Err(CliError::usage(format!("bad duration unit `{other}`"))),
    };
    Ok(std::time::Duration::from_secs_f64(secs.max(0.0)))
}
