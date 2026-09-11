//! `gearbox <name> …` runs `gearbox-<name>` from PATH with the target
//! instance in its environment.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::ctx::Ctx;
use crate::error::{CliError, Result};

const CORE: [&str; 10] = [
    "run", "instance", "spawn", "clear", "scene", "select", "machine", "access", "api", "env",
];

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

pub fn installed() -> Vec<String> {
    let mut names = Vec::new();
    for dir in path_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name.strip_prefix("gearbox-")
                && !rest.is_empty()
                && rest != "sim"
                && !CORE.contains(&rest)
                && is_executable(&entry.path())
                && !names.contains(&rest.to_string())
            {
                names.push(rest.to_string());
            }
        }
    }
    names.sort();
    names
}

fn find(name: &str) -> Option<PathBuf> {
    let file = format!("gearbox-{name}");
    path_dirs()
        .into_iter()
        .map(|d| d.join(&file))
        .find(|p| is_executable(p))
}

pub fn run(ctx: &Ctx, argv: Vec<OsString>) -> Result<()> {
    let Some((name, rest)) = argv.split_first() else {
        return Err(CliError::usage("no command given"));
    };
    let name = name.to_string_lossy().into_owned();
    let Some(exe) = find(&name) else {
        return Err(CliError::usage(format!(
            "unknown command `{name}` and no `gearbox-{name}` on PATH"
        )));
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(rest);
    if let Ok(target) = ctx.target() {
        cmd.env("GEARBOX_INSTANCE", &target.name)
            .env("GEARBOX_DID", &target.did)
            .env("GEARBOX_ADDR", &target.addr);
    }
    if let Some(sel) = &ctx.context.selection {
        cmd.env("GEARBOX_SELECTION", format!("{}:{}", sel.kind, sel.id));
    }
    let status = cmd.status()?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => Err(CliError::new(code, "")),
        None => Err(CliError::error(format!(
            "gearbox-{name} was killed by a signal"
        ))),
    }
}
