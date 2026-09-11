//! `gearbox run`: launch `gearbox-sim`, wait for it to register, apply the
//! initial loads, then either follow it or detach.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Args as ClapArgs;
use gearbox_api::registry::{self, RegistryEntry};
use gearbox_api::{Client, IdentitySource, UsdLoad, category};
use serde_json::json;

use super::{check, default_id, usd_path};
use crate::ctx::{CLI_IDENTITY, Ctx, ensure_cli_did};
use crate::error::{CliError, Result};
use crate::paths;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Instance name; also the persistent identity
    #[arg(long, default_value = "gearbox")]
    name: String,
    /// Throwaway identity, no registry entry kept after exit
    #[arg(long)]
    ephemeral: bool,
    /// Extra peers allowed to connect (did:key)
    #[arg(long = "allow", value_name = "DID")]
    allow: Vec<String>,
    /// Accept any peer (LAN robots); logged as a warning by the sim
    #[arg(long)]
    allow_any: bool,
    /// Use iroh relays for off-LAN peers
    #[arg(long)]
    relay: bool,
    /// World USD to load once the sim answers
    #[arg(long, value_name = "USD")]
    world: Option<String>,
    /// Machine USDs to load once the sim answers (namespace = file stem)
    #[arg(long = "load", value_name = "USD")]
    load: Vec<String>,
    /// Daemonize and print the registry entry as JSON
    #[arg(long)]
    detach: bool,
    /// Path to gearbox-sim (else GEARBOX_SIM, config `sim`, next to this binary, PATH)
    #[arg(long, value_name = "PATH")]
    sim: Option<PathBuf>,
    /// Arguments passed through to gearbox-sim
    #[arg(last = true)]
    sim_args: Vec<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    let sim = find_sim(ctx, args.sim.as_deref())?;
    if let Some(live) = registry::find(&args.name) {
        return Err(CliError::error(format!(
            "instance `{}` is already running (pid {}, {})",
            live.name, live.pid, live.did
        )));
    }
    let cli_did = ensure_cli_did()?;
    let mut allow = args.allow.clone();
    allow.extend(paths::read_allow_file());
    allow.push(cli_did);

    let mut cmd = Command::new(&sim);
    cmd.args(&args.sim_args)
        .env("GEARBOX_NAME", &args.name)
        .env("GEARBOX_ALLOW", allow.join(":"));
    if args.ephemeral {
        cmd.env("GEARBOX_EPHEMERAL", "1");
    }
    if args.allow_any {
        cmd.env("GEARBOX_ALLOW_ANY", "1");
    }
    if args.relay {
        cmd.env("GEARBOX_RELAY", "1");
    }
    let log_path = if args.detach {
        let dir = paths::logs_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.log", args.name));
        let file = std::fs::File::create(&path)?;
        let err = file.try_clone()?;
        cmd.stdin(Stdio::null()).stdout(file).stderr(err);
        cmd.env("GEARBOX_LOG", &path);
        detach_from_terminal(&mut cmd);
        Some(path)
    } else {
        None
    };

    let mut child = cmd
        .spawn()
        .map_err(|e| CliError::error(format!("cannot start {}: {e}", sim.display())))?;

    let entry = match wait_registered(&args.name, child.id(), Duration::from_secs(60), || {
        child.try_wait().ok().flatten()
    }) {
        Ok(entry) => entry,
        Err(err) => {
            if !args.detach {
                let _ = child.kill();
            }
            return Err(err);
        }
    };

    if args.world.is_some() || !args.load.is_empty() {
        let client = Client::connect(
            &entry.did,
            IdentitySource::Name(CLI_IDENTITY.to_string()),
            CLI_IDENTITY,
        )?;
        client.wait_ready(Duration::from_secs(30))?;
        if let Some(world) = &args.world {
            let req = UsdLoad::new(&default_id(world), &usd_path(world)).category(category::WORLD);
            check(client.load(&req)?, "load world")?;
            let mut context = ctx.context.clone();
            context.world = Some(usd_path(world));
            context.save()?;
        }
        for path in &args.load {
            let ns = default_id(path);
            let req = UsdLoad::new(&ns, &usd_path(path))
                .category(category::MACHINE)
                .with_prop("namespace", &ns);
            check(client.load(&req)?, &format!("load machine {ns}"))?;
        }
    }

    if args.detach {
        let mut value = serde_json::to_value(&entry).unwrap_or(json!({}));
        if let (Some(obj), Some(log)) = (value.as_object_mut(), &log_path) {
            obj.insert("log".into(), json!(log.to_string_lossy()));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return Ok(());
    }

    if !ctx.quiet {
        eprintln!(
            "gearbox: instance `{}` up, did {} (pid {})",
            entry.name, entry.did, entry.pid
        );
    }
    let status = child.wait()?;
    registry::remove(&args.name);
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => Err(CliError::new(
            code,
            format!("gearbox-sim exited with {code}"),
        )),
        None => Err(CliError::error("gearbox-sim was killed by a signal")),
    }
}

fn find_sim(ctx: &Ctx, flag: Option<&Path>) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = flag {
        candidates.push(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("GEARBOX_SIM") {
        candidates.push(PathBuf::from(p));
    }
    if let Some(p) = &ctx.config.sim {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("gearbox-sim"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join("gearbox-sim"));
        }
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        CliError::error(
            "gearbox-sim not found: pass --sim, set GEARBOX_SIM, or `gearbox env set sim PATH`",
        )
    })
}

fn wait_registered(
    name: &str,
    pid: u32,
    timeout: Duration,
    mut exited: impl FnMut() -> Option<std::process::ExitStatus>,
) -> Result<RegistryEntry> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(entry) = registry::find(name)
            && entry.pid == pid
        {
            return Ok(entry);
        }
        if let Some(status) = exited() {
            return Err(CliError::error(format!(
                "gearbox-sim exited before registering ({status})"
            )));
        }
        if Instant::now() >= deadline {
            return Err(CliError::timeout(format!(
                "gearbox-sim did not register `{name}` within {}s",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn detach_from_terminal(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid only touches the child's own session and has no
    // preconditions in a freshly forked process.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}
