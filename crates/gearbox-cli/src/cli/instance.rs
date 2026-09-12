//! `gearbox instance`: the host-local registry and `/gearbox/info`.

use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::registry::{self, RegistryEntry};
use gearbox_api::{Client, IdentitySource, clock_op};
use serde_json::json;

use crate::ctx::{CLI_IDENTITY, Ctx};
use crate::error::{CliError, Result};
use crate::out::{self, Table};

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Registry entries with liveness; the default is marked
    List,
    /// `/gearbox/info` of an instance
    Info { id: Option<String> },
    /// Make an instance the default for later commands
    Use { id: String },
    /// Ask an instance to shut down, then signal it
    Stop {
        id: Option<String>,
        /// SIGKILL if it does not exit
        #[arg(long)]
        force: bool,
    },
    /// Round-trip time of `/gearbox/info`
    Ping {
        id: Option<String>,
        #[arg(short = 'n', long, default_value_t = 3)]
        count: u32,
    },
    /// Block until the instance answers
    Wait {
        id: Option<String>,
        /// Seconds to wait
        #[arg(long, default_value_t = 30.0)]
        timeout: f64,
    },
    /// Save a screenshot of the viewer window to a PNG
    Screenshot {
        out: String,
        #[arg(long)]
        id: Option<String>,
        /// Seconds to wait for the file
        #[arg(long, default_value_t = 10.0)]
        timeout: f64,
    },
    /// Move the viewer camera: `fly MACHINE` flies behind a machine like a
    /// double-click in the Agents pane, `follow MACHINE` pins the camera to
    /// it, `unfollow` releases it
    Camera {
        /// fly | follow | unfollow
        action: String,
        /// Machine id (`gearbox:machine:id`), needed by fly and follow
        machine: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// Show the instance's log file
    Logs {
        id: Option<String>,
        /// Keep following
        #[arg(short, long)]
        follow: bool,
        /// Lines from the end to start with
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
    },
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::List => list(ctx),
        Cmd::Info { id } => info(ctx, id),
        Cmd::Use { id } => use_instance(ctx, id),
        Cmd::Stop { id, force } => stop(ctx, id, force),
        Cmd::Ping { id, count } => ping(ctx, id, count),
        Cmd::Wait { id, timeout } => wait(ctx, id, timeout),
        Cmd::Logs { id, follow, lines } => logs(ctx, id, follow, lines),
        Cmd::Screenshot { out, id, timeout } => screenshot(ctx, id, &out, timeout),
        Cmd::Camera {
            action,
            machine,
            id,
        } => camera(ctx, id, &action, machine.as_deref()),
    }
}

fn camera(ctx: &Ctx, id: Option<String>, action: &str, machine: Option<&str>) -> Result<()> {
    let ctx = with_target(ctx, id)?;
    let target = ctx.target()?.clone();
    if target.pid == 0 {
        return Err(CliError::error(
            "instance addressed by did only; camera needs a registry entry",
        ));
    }
    let line = match (action, machine) {
        ("fly", Some(m)) | ("follow", Some(m)) => format!("{action} {m}"),
        ("unfollow", _) => "unfollow".to_string(),
        ("fly", None) | ("follow", None) => {
            return Err(CliError::error(format!("`{action}` needs a machine id")));
        }
        _ => {
            return Err(CliError::error(
                "camera action must be fly, follow or unfollow",
            ));
        }
    };
    let request = registry::registry_dir().join(format!("{}.camera", target.name));
    std::fs::write(&request, line.as_bytes())?;
    ctx.done(
        &line,
        &format!("camera request `{line}` sent to `{}`", target.name),
        || json!({ "instance": target.name, "request": line }),
    );
    Ok(())
}

fn with_target(ctx: &Ctx, id: Option<String>) -> Result<Ctx> {
    match id {
        Some(id) => Ctx::new(Some(id), ctx.json, ctx.quiet, ctx.timeout.as_secs_f64()),
        None => Ctx::new(
            ctx.instance.clone(),
            ctx.json,
            ctx.quiet,
            ctx.timeout.as_secs_f64(),
        ),
    }
}

fn entry_json(e: &RegistryEntry, default: bool) -> serde_json::Value {
    let mut v = serde_json::to_value(e).unwrap_or(json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("alive".into(), json!(registry::pid_alive(e.pid)));
        obj.insert("default".into(), json!(default));
    }
    v
}

fn list(ctx: &Ctx) -> Result<()> {
    let entries = registry::list();
    let default = ctx.target().ok().map(|t| t.name.clone());
    ctx.emit(
        || {
            json!(
                entries
                    .iter()
                    .map(|e| entry_json(e, default.as_deref() == Some(e.name.as_str())))
                    .collect::<Vec<_>>()
            )
        },
        || {
            if entries.is_empty() {
                println!(
                    "no running instances in {}",
                    registry::registry_dir().display()
                );
                return;
            }
            let mut t = Table::new(&["", "NAME", "PID", "VERSION", "STARTED", "DID"]);
            for e in &entries {
                let mark = if default.as_deref() == Some(e.name.as_str()) {
                    "*"
                } else {
                    ""
                };
                t.row(vec![
                    mark.into(),
                    e.name.clone(),
                    e.pid.to_string(),
                    e.version.clone(),
                    e.started.clone(),
                    e.did.clone(),
                ]);
            }
            t.print();
        },
    );
    Ok(())
}

fn info(ctx: &Ctx, id: Option<String>) -> Result<()> {
    let ctx = with_target(ctx, id)?;
    let target = ctx.target()?.clone();
    let client = ctx.client()?;
    let info = client.info()?;
    let props = info.props();
    let clock = client.clock(clock_op::GET)?;
    ctx.emit(
        || {
            let mut v = gearbox_api::wire::json::types()
                .into_iter()
                .find(|t| t.name == gearbox_api::HostInfo::CANONICAL_NAME)
                .and_then(|t| (t.to_json)(gearbox_api::pack(&info).wire()).ok())
                .unwrap_or(json!({}));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("name".into(), json!(target.name));
                obj.insert("did".into(), json!(target.did));
                obj.insert("addr".into(), json!(target.addr));
                obj.insert("step".into(), json!(clock.step));
            }
            v
        },
        || {
            out::kv(&[
                ("instance", target.name.clone()),
                ("did", target.did.clone()),
                ("version", props.get("version").unwrap_or_default()),
                ("pid", info.pid.to_string()),
                ("uptime", out::fmt_ms(info.uptime_ms)),
                (
                    "clock",
                    format!(
                        "{} (step {})",
                        if info.paused != 0 {
                            "paused"
                        } else {
                            "running"
                        },
                        clock.step
                    ),
                ),
                ("machines", info.machine_count.to_string()),
                ("objects", info.object_count.to_string()),
                ("allow any", props.get("allow_any").unwrap_or_default()),
                ("allowed", props.get("allowed").unwrap_or_default()),
                ("log", props.get("log").unwrap_or_default()),
                ("addr", target.addr.clone()),
            ]);
        },
    );
    Ok(())
}

fn use_instance(ctx: &Ctx, id: String) -> Result<()> {
    let entry = registry::find(&id)
        .ok_or_else(|| CliError::no_instance(format!("no running instance `{id}`")))?;
    let mut context = ctx.context.clone();
    context.instance = Some(entry.name.clone());
    context.save()?;
    ctx.done(
        &entry.name,
        &format!("default instance is now `{}`", entry.name),
        || json!({ "instance": entry.name, "did": entry.did }),
    );
    Ok(())
}

fn stop(ctx: &Ctx, id: Option<String>, force: bool) -> Result<()> {
    let ctx = with_target(ctx, id)?;
    let target = ctx.target()?.clone();
    if target.pid == 0 {
        return Err(CliError::error(
            "instance addressed by did only; stop needs a registry entry with a pid",
        ));
    }
    if let Ok(client) = ctx.client() {
        let _ = client.clock(clock_op::SHUTDOWN);
    }
    let mut how = "shutdown request";
    if !wait_dead(target.pid, Duration::from_secs(3)) {
        signal(target.pid, libc::SIGTERM);
        how = "SIGTERM";
        if !wait_dead(target.pid, Duration::from_secs(3)) {
            if !force {
                return Err(CliError::error(format!(
                    "instance `{}` (pid {}) ignored shutdown and SIGTERM; use --force",
                    target.name, target.pid
                )));
            }
            signal(target.pid, libc::SIGKILL);
            how = "SIGKILL";
            wait_dead(target.pid, Duration::from_secs(2));
        }
    }
    registry::remove(&target.name);
    ctx.done(
        &target.name,
        &format!("stopped `{}` (pid {}) via {how}", target.name, target.pid),
        || json!({ "instance": target.name, "pid": target.pid, "how": how }),
    );
    Ok(())
}

fn signal(pid: u32, sig: i32) {
    // SAFETY: kill() has no memory preconditions; a wrong pid only errors.
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

fn wait_dead(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !registry::pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !registry::pid_alive(pid)
}

fn ping(ctx: &Ctx, id: Option<String>, count: u32) -> Result<()> {
    let ctx = with_target(ctx, id)?;
    let target = ctx.target()?.clone();
    let client = ctx.client()?;
    let mut samples = Vec::new();
    for i in 0..count.max(1) {
        let t0 = Instant::now();
        client.info()?;
        let rtt = t0.elapsed();
        samples.push(rtt.as_secs_f64() * 1000.0);
        if !ctx.json && !ctx.quiet {
            println!(
                "{}: seq {} time {:.2} ms",
                target.did,
                i,
                rtt.as_secs_f64() * 1000.0
            );
        }
        if i + 1 < count {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(0.0, f64::max);
    let avg = samples.iter().sum::<f64>() / samples.len() as f64;
    ctx.emit(
        || json!({ "did": target.did, "count": samples.len(), "min_ms": min, "avg_ms": avg, "max_ms": max }),
        || println!("rtt min/avg/max = {min:.2}/{avg:.2}/{max:.2} ms"),
    );
    Ok(())
}

fn wait(ctx: &Ctx, id: Option<String>, timeout: f64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout.max(0.0));
    loop {
        let attempt = with_target(ctx, id.clone()).and_then(|c| {
            let target = c.target()?.clone();
            let client = Client::connect(
                &target.did,
                IdentitySource::Name(CLI_IDENTITY.to_string()),
                CLI_IDENTITY,
            )?;
            client.wait_ready(Duration::from_secs(2))?;
            Ok(target)
        });
        match attempt {
            Ok(target) => {
                ctx.done(
                    &target.name,
                    &format!("instance `{}` is answering", target.name),
                    || json!({ "instance": target.name, "did": target.did }),
                );
                return Ok(());
            }
            Err(err) if Instant::now() >= deadline => {
                return Err(CliError::timeout(format!(
                    "no instance answered within {timeout:.0}s: {}",
                    err.message
                )));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

fn logs(ctx: &Ctx, id: Option<String>, follow: bool, lines: usize) -> Result<()> {
    let ctx = with_target(ctx, id)?;
    let target = ctx.target()?.clone();
    let path = target
        .log
        .clone()
        .or_else(|| {
            let p = crate::paths::logs_dir().join(format!("{}.log", target.name));
            p.exists().then(|| p.to_string_lossy().into_owned())
        })
        .ok_or_else(|| {
            CliError::error(format!(
                "instance `{}` has no log file; it was not started with `gearbox run --detach`",
                target.name
            ))
        })?;
    let text = std::fs::read_to_string(&path)?;
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    for line in &all[start..] {
        println!("{line}");
    }
    if !follow {
        return Ok(());
    }
    let mut offset = text.len() as u64;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if len < offset {
            offset = 0;
        }
        if len > offset {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(&path)?;
            f.seek(SeekFrom::Start(offset))?;
            let mut chunk = String::new();
            f.read_to_string(&mut chunk)?;
            print!("{chunk}");
            offset = len;
        }
        if !registry::pid_alive(target.pid) {
            return Ok(());
        }
    }
}

fn screenshot(ctx: &Ctx, id: Option<String>, out: &str, timeout: f64) -> Result<()> {
    let ctx = with_target(ctx, id)?;
    let target = ctx.target()?.clone();
    if target.pid == 0 {
        return Err(CliError::error(
            "instance addressed by did only; screenshot needs a registry entry",
        ));
    }
    let out = std::path::absolute(out)?;
    let _ = std::fs::remove_file(&out);
    let request = registry::registry_dir().join(format!("{}.shot", target.name));
    std::fs::write(&request, out.to_string_lossy().as_bytes())?;
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    while !out.exists() {
        if Instant::now() >= deadline {
            let _ = std::fs::remove_file(&request);
            return Err(CliError::timeout(format!(
                "instance `{}` did not write {} within {timeout}s",
                target.name,
                out.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ctx.done(
        &out.to_string_lossy(),
        &format!("saved screenshot of `{}` to {}", target.name, out.display()),
        || json!({ "instance": target.name, "path": out.to_string_lossy() }),
    );
    Ok(())
}
