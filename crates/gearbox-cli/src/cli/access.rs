//! `gearbox access`: identities, the launch allowlist, machine sessions.

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::{IdentitySource, code};
use serde_json::json;

use super::check;
use crate::ctx::{Ctx, cli_key_path, ensure_cli_did};
use crate::error::{CliError, Result};
use crate::out::{self, Table};
use crate::paths;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// The CLI's did and key path
    Whoami,
    /// Allowed peers of the instance and who holds each machine
    List,
    /// Allow a peer on the next launch (live grants need agentio support)
    Grant {
        did: String,
        /// Kept for every future launch (the default; flag kept for scripts)
        #[arg(long, default_value_t = true)]
        persist: bool,
    },
    /// Remove a peer from the launch allowlist
    Revoke { did: String },
    /// Steal a machine's session, then release it
    Take { ns: String },
    /// Force-release whoever holds a machine
    Release { ns: String },
    /// Manage CLI identities under agentio's key dir
    Identity {
        #[command(subcommand)]
        cmd: IdentityCmd,
    },
}

#[derive(Subcommand, Debug)]
enum IdentityCmd {
    /// Create (or show) a named identity
    New { name: String },
    /// Named identities on this machine
    List,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Whoami => {
            let did = ensure_cli_did()?;
            let key = cli_key_path();
            ctx.emit(
                || json!({ "did": did, "key": key.display().to_string(), "did_file": gearbox_api::host::cli_did_path().display().to_string() }),
                || {
                    out::kv(&[
                        ("did", did.clone()),
                        ("key", key.display().to_string()),
                        (
                            "did file",
                            gearbox_api::host::cli_did_path().display().to_string(),
                        ),
                    ])
                },
            );
            Ok(())
        }
        Cmd::List => list(ctx),
        Cmd::Grant { did, persist } => {
            validate_did(&did)?;
            let mut dids = paths::read_allow_file();
            if !dids.contains(&did) {
                dids.push(did.clone());
                paths::write_allow_file(&dids)?;
            }
            let live = ctx.client().is_ok();
            let note = if live {
                "; the running instance keeps its current allowlist until relaunch"
            } else {
                ""
            };
            ctx.done(
                &did,
                &format!("{did} allowed on the next `gearbox run`{note}"),
                || json!({ "did": did, "persist": persist, "live": false }),
            );
            Ok(())
        }
        Cmd::Revoke { did } => {
            let mut dids = paths::read_allow_file();
            let before = dids.len();
            dids.retain(|d| *d != did);
            if dids.len() == before {
                return Err(CliError::new(1, format!("{did} is not in the allowlist")));
            }
            paths::write_allow_file(&dids)?;
            ctx.done(
                &did,
                &format!("{did} removed from the launch allowlist"),
                || json!({ "did": did, "live": false }),
            );
            Ok(())
        }
        Cmd::Take { ns } => take(ctx, &ns, "took"),
        Cmd::Release { ns } => take(ctx, &ns, "released"),
        Cmd::Identity { cmd } => identity(ctx, cmd),
    }
}

fn validate_did(did: &str) -> Result<()> {
    agentio::did_key::did_key_to_endpoint(did)
        .map(|_| ())
        .map_err(|e| CliError::usage(format!("`{did}` is not a did:key: {e}")))
}

fn list(ctx: &Ctx) -> Result<()> {
    let target = ctx.target()?.clone();
    let client = ctx.client()?;
    let info = client.info()?;
    let p = info.props();
    let allowed: Vec<String> = p
        .get("allowed")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let allow_any = p.get("allow_any").unwrap_or_default() == "true";
    let pending = paths::read_allow_file();
    let mut sessions = Vec::new();
    for m in client.machines()? {
        let ns = m.namespace();
        if let Ok(s) = client.machine(&ns).session() {
            sessions.push((ns, s));
        }
    }
    ctx.emit(
        || {
            json!({
                "instance": target.name,
                "allow_any": allow_any,
                "allowed": allowed,
                "next_launch": pending,
                "sessions": sessions.iter().map(|(ns, s)| json!({
                    "namespace": ns, "held": s.held != 0, "holder": s.holder(),
                    "session": s.session, "age_ms": s.age_ms, "idle_ms": s.idle_ms,
                })).collect::<Vec<_>>(),
            })
        },
        || {
            out::kv(&[
                ("instance", target.name.clone()),
                ("allow any", out::yes_no(allow_any).into()),
                (
                    "allowed",
                    if allowed.is_empty() {
                        "none".into()
                    } else {
                        allowed.join("\n         ")
                    },
                ),
                (
                    "next launch",
                    if pending.is_empty() {
                        "no extra peers".into()
                    } else {
                        pending.join("\n         ")
                    },
                ),
            ]);
            println!();
            let mut t = Table::new(&["MACHINE", "HELD BY", "SESSION", "AGE", "IDLE"]);
            for (ns, s) in &sessions {
                if s.held != 0 {
                    t.row(vec![
                        ns.clone(),
                        s.holder(),
                        s.session.to_string(),
                        out::fmt_ms(s.age_ms),
                        out::fmt_ms(s.idle_ms),
                    ]);
                } else {
                    t.row(vec![
                        ns.clone(),
                        "-".into(),
                        "-".into(),
                        "-".into(),
                        "-".into(),
                    ]);
                }
            }
            if t.is_empty() {
                println!("no machines");
            } else {
                t.print();
            }
        },
    );
    Ok(())
}

fn take(ctx: &Ctx, ns: &str, verb: &str) -> Result<()> {
    let client = ctx.client()?;
    let mut mc = client.machine(ns);
    let before = mc.session()?;
    let res = mc.claim(500, true)?;
    if res.code != code::OK {
        return Err(CliError::new(
            crate::error::exit_for_wire_code(res.code),
            format!("could not take `{ns}`: code {}", res.code),
        ));
    }
    let _ = mc.cmd_vel(res.session, 0.0, 0.0);
    check(mc.release(res.session)?, "release")?;
    let holder = before.holder();
    ctx.done(
        ns,
        &if before.held != 0 {
            format!("{verb} `{ns}` from {holder}; machine is free")
        } else {
            format!("`{ns}` was already free")
        },
        || json!({ "namespace": ns, "previous_holder": holder, "held": false }),
    );
    Ok(())
}

fn identity(ctx: &Ctx, cmd: IdentityCmd) -> Result<()> {
    match cmd {
        IdentityCmd::New { name } => {
            let key = agentio::resolve_identity(&IdentitySource::Name(name.clone()))?;
            let did = agentio::did_key::endpoint_to_did_key(&key.public())?;
            let path = agentio::default_keys_dir()
                .join("name")
                .join(format!("{name}.key"));
            ctx.done(
                &did,
                &format!("{name}: {did}\n  {}", path.display()),
                || json!({ "name": name, "did": did, "key": path.display().to_string() }),
            );
            Ok(())
        }
        IdentityCmd::List => {
            let dir = agentio::default_keys_dir().join("name");
            let mut rows = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let mut names: Vec<String> = entries
                    .flatten()
                    .filter_map(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .strip_suffix(".key")
                            .map(str::to_string)
                    })
                    .collect();
                names.sort();
                for name in names {
                    let did = agentio::resolve_identity(&IdentitySource::Name(name.clone()))
                        .ok()
                        .and_then(|k| agentio::did_key::endpoint_to_did_key(&k.public()).ok())
                        .unwrap_or_default();
                    rows.push((name, did));
                }
            }
            ctx.emit(
                || {
                    json!(
                        rows.iter()
                            .map(|(n, d)| json!({ "name": n, "did": d }))
                            .collect::<Vec<_>>()
                    )
                },
                || {
                    if rows.is_empty() {
                        println!("no named identities in {}", dir.display());
                        return;
                    }
                    let mut t = Table::new(&["NAME", "DID"]);
                    for (n, d) in &rows {
                        t.row(vec![n.clone(), d.clone()]);
                    }
                    t.print();
                },
            );
            Ok(())
        }
    }
}
