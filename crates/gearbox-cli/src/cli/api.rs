//! `gearbox api`: raw topics, schemas, diagnostics, doctor.

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::wire::json as wire_json;
use gearbox_api::{Env, HostInfo, Ping, topics};
use serde_json::{Value, json};

use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::out::{self, Table};

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Topics of the target and its machines, with types
    Topics,
    /// req/res call with a JSON body
    Call {
        topic: String,
        /// Request body; keys not in the wire type become string props
        body: Option<String>,
        /// Request wire type when it cannot be inferred from the topic
        #[arg(long = "type")]
        type_name: Option<String>,
    },
    /// que/ans query, one line per answer
    Que {
        topic: String,
        body: Option<String>,
        #[arg(long = "type")]
        type_name: Option<String>,
    },
    /// Schema of a wire type, or the list of all
    Schema { type_name: Option<String> },
    /// Transport path to the target (or a peer did)
    Diag { did: Option<String> },
    /// Registry, key and version checks
    Doctor,
}

/// (topic suffix or absolute, exchange, request type, response type)
const HOST_TOPICS: [(&str, &str, &str, &str); 12] = [
    (
        topics::HOST_INFO,
        "req/res",
        "gearbox.ping.v1",
        "gearbox.host_info.v1",
    ),
    (
        topics::SCENE_CLOCK,
        "req/res",
        "gearbox.clock_command.v1",
        "gearbox.clock_state.v1",
    ),
    (
        topics::SCENE_CLOCK_STATE,
        "pub/sub",
        "gearbox.clock_state.v1",
        "",
    ),
    (
        topics::SCENE_CLEAR,
        "req/res",
        "gearbox.clear_request.v1",
        "gearbox.status.v1",
    ),
    (
        topics::SCENE_LIST,
        "que/ans",
        "gearbox.list_query.v1",
        "gearbox.scene_object.v1",
    ),
    (
        topics::SCENE_EVENTS,
        "pub/sub",
        "gearbox.scene_event.v1",
        "",
    ),
    (
        topics::USD_LOAD,
        "req/res",
        "gearbox.usd_load.v1",
        "gearbox.status.v1",
    ),
    (
        topics::USD_DELETE,
        "req/res",
        "gearbox.usd_ref.v1",
        "gearbox.status.v1",
    ),
    (
        topics::MARKER_SET,
        "req/res",
        "gearbox.marker_set.v1",
        "gearbox.status.v1",
    ),
    (
        topics::MARKER_DELETE,
        "req/res",
        "gearbox.marker_ref.v1",
        "gearbox.status.v1",
    ),
    (
        topics::SELECT,
        "req/res",
        "gearbox.selection.v1",
        "gearbox.selection.v1",
    ),
    (
        topics::MACHINES_LIST,
        "que/ans",
        "gearbox.ping.v1",
        "gearbox.machine_ref.v1",
    ),
];

const MACHINE_TOPICS: [(&str, &str, &str, &str); 11] = [
    (
        topics::MACHINE_INFO,
        "req/res",
        "gearbox.ping.v1",
        "gearbox.machine_info.v1",
    ),
    (
        topics::MACHINE_CLAIM,
        "req/res",
        "gearbox.claim_request.v1",
        "gearbox.claim_response.v1",
    ),
    (
        topics::MACHINE_RELEASE,
        "req/res",
        "gearbox.session_ref.v1",
        "gearbox.status.v1",
    ),
    (
        topics::MACHINE_SESSION,
        "req/res",
        "gearbox.ping.v1",
        "gearbox.session_info.v1",
    ),
    (
        topics::MACHINE_CMD_VEL,
        "req/res",
        "gearbox.twist_cmd.v1",
        "gearbox.status.v1",
    ),
    (
        topics::MACHINE_CMD,
        "req/res",
        "gearbox.controller_command.v1",
        "gearbox.status.v1",
    ),
    (
        topics::MACHINE_STATE,
        "pub/sub",
        "gearbox.machine_state.v1",
        "",
    ),
    (topics::MACHINE_ODOM, "pub/sub", "datapod.odom.v1", ""),
    ("sensors/<imu link>", "pub/sub", "datapod.imu.v1", ""),
    (
        "sensors/<lidar link>",
        "pub/sub",
        "gearbox.lidar_scan.v1",
        "",
    ),
    (
        "sensors/<camera link>",
        "pub/sub",
        "gearbox.camera_frame.v1",
        "",
    ),
];

/// The request type a topic takes, for host and machine topics alike.
fn request_type_for(topic: &str) -> Option<&'static str> {
    if let Some(t) = HOST_TOPICS.iter().find(|t| t.0 == topic) {
        return Some(t.2);
    }
    let leaf = topic.rsplit('/').next().unwrap_or(topic);
    if topic.starts_with("/machines/") {
        return MACHINE_TOPICS.iter().find(|t| t.0 == leaf).map(|t| t.2);
    }
    None
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Topics => list_topics(ctx),
        Cmd::Call {
            topic,
            body,
            type_name,
        } => {
            let req = build_request(&topic, body.as_deref(), type_name.as_deref())?;
            let res = ctx.client()?.call_env(&topic, &req)?;
            print_env(ctx, &res);
            Ok(())
        }
        Cmd::Que {
            topic,
            body,
            type_name,
        } => {
            let req = build_request(&topic, body.as_deref(), type_name.as_deref())?;
            for env in ctx.client()?.query_env(&topic, &req)? {
                print_env(ctx, &env);
            }
            Ok(())
        }
        Cmd::Schema { type_name } => schema(ctx, type_name),
        Cmd::Diag { did } => diag(ctx, did),
        Cmd::Doctor => doctor(ctx),
    }
}

fn build_request(topic: &str, body: Option<&str>, type_name: Option<&str>) -> Result<Env> {
    let name = match type_name {
        Some(n) => n.to_string(),
        None => request_type_for(topic)
            .ok_or_else(|| {
                CliError::usage(format!(
                    "cannot infer the request type of `{topic}`; pass --type NAME"
                ))
            })?
            .to_string(),
    };
    let value: Value = match body {
        Some(text) => serde_json::from_str(text)?,
        None => json!({}),
    };
    wire_json::env_from_json(&name, &value).map_err(CliError::usage)
}

pub(crate) fn print_env(ctx: &Ctx, env: &Env) {
    println!("{}", render_env(ctx, env));
}

/// The same decoding `print_env` prints, as a string — for callers that
/// redraw it in place (`subscribe --inline`) instead of just printing it.
pub(crate) fn render_env(ctx: &Ctx, env: &Env) -> String {
    match wire_json::env_to_json(env) {
        Ok(v) => {
            if ctx.json {
                serde_json::to_string(&v).unwrap_or_default()
            } else {
                crate::out::pretty_json(&v)
            }
        }
        Err(err) => format!(
            "{{\"type_hash\": {}, \"bytes\": {}, \"error\": \"{err}\"}}",
            env.type_hash(),
            env.wire().len()
        ),
    }
}

fn list_topics(ctx: &Ctx) -> Result<()> {
    let client = ctx.client()?;
    let target = ctx.target()?.clone();
    let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
    for (topic, mode, req, res) in HOST_TOPICS {
        let owner = client
            .resolve_topic(topic)
            .ok()
            .and_then(|e| agentio::did_key::endpoint_to_did_key(&e.endpoint_id()).ok())
            .unwrap_or_else(|| target.did.clone());
        rows.push((
            topic.to_string(),
            mode.into(),
            req.into(),
            res.into(),
            owner,
        ));
    }
    for m in client.machines()? {
        let machine_id = m.machine_id();
        for (leaf, mode, req, res) in MACHINE_TOPICS {
            rows.push((
                topics::machine_topic(&machine_id, leaf),
                mode.into(),
                req.into(),
                res.into(),
                m.did(),
            ));
        }
    }
    ctx.emit(
        || {
            json!(rows
                .iter()
                .map(|r| json!({ "topic": r.0, "mode": r.1, "request": r.2, "response": r.3, "owner": r.4 }))
                .collect::<Vec<_>>())
        },
        || {
            let mut t = Table::new(&["TOPIC", "MODE", "REQUEST", "RESPONSE", "OWNER"]);
            for r in &rows {
                t.row(vec![
                    r.0.clone(),
                    r.1.clone(),
                    r.2.clone(),
                    if r.3.is_empty() { "-".into() } else { r.3.clone() },
                    r.4.clone(),
                ]);
            }
            t.print();
        },
    );
    Ok(())
}

fn schema(ctx: &Ctx, type_name: Option<String>) -> Result<()> {
    match type_name {
        None => {
            let types = wire_json::types();
            ctx.emit(
                || {
                    json!(
                        types
                            .iter()
                            .map(|t| json!({ "name": t.name, "type_hash": t.type_hash }))
                            .collect::<Vec<_>>()
                    )
                },
                || {
                    let mut tbl = Table::new(&["TYPE", "HASH"]);
                    for t in &types {
                        tbl.row(vec![t.name.to_string(), format!("{:016x}", t.type_hash)]);
                    }
                    tbl.print();
                },
            );
            Ok(())
        }
        Some(name) => {
            let t = wire_json::find_by_name(&name)
                .ok_or_else(|| CliError::usage(format!("unknown wire type `{name}`")))?;
            let s = (t.schema)();
            ctx.emit(
                || {
                    json!({
                        "name": s.canonical_name, "type_hash": s.type_hash, "schema_hash": s.schema_hash,
                        "header_size": s.header_size,
                        "fields": s.fields.iter().map(|f| json!({
                            "name": f.name, "offset": f.offset, "role": format!("{:?}", f.role), "type": format!("{:?}", f.ty),
                        })).collect::<Vec<_>>(),
                    })
                },
                || {
                    out::kv(&[
                        ("type", s.canonical_name.clone()),
                        ("type hash", format!("{:016x}", s.type_hash)),
                        ("schema hash", format!("{:016x}", s.schema_hash)),
                        ("header", format!("{} bytes", s.header_size)),
                    ]);
                    let mut tbl = Table::new(&["FIELD", "OFFSET", "ROLE", "TYPE"]);
                    for f in &s.fields {
                        tbl.row(vec![
                            f.name.to_string(),
                            f.offset.to_string(),
                            format!("{:?}", f.role),
                            format!("{:?}", f.ty),
                        ]);
                    }
                    tbl.print();
                },
            );
            Ok(())
        }
    }
}

fn diag(ctx: &Ctx, did: Option<String>) -> Result<()> {
    let client = ctx.client()?;
    let did = match did {
        Some(d) => d,
        None => ctx.target()?.did.clone(),
    };
    let peer = agentio::did_key::did_key_to_endpoint(&did)
        .map_err(|e| CliError::usage(format!("`{did}` is not a did:key: {e}")))?;
    // A round trip first, so the path cache has something to report.
    let _ = client.info();
    let diag = client.path_diagnostics(peer)?;
    ctx.emit(
        || match &diag {
            None => json!({ "did": did, "paths": [] }),
            Some(d) => json!({
                "did": did,
                "max_datagram_size": d.max_datagram_size,
                "paths": d.paths.iter().map(|p| json!({
                    "path": p.path_id, "remote": p.remote_addr, "selected": p.selected,
                    "relay": p.is_relay, "rtt_ms": p.rtt.as_secs_f64() * 1000.0, "mtu": p.current_mtu,
                })).collect::<Vec<_>>(),
            }),
        },
        || match &diag {
            None => println!("{did}: no QUIC path cached (shared memory only, or never dialed)"),
            Some(d) => {
                println!("{did}");
                let mut t = Table::new(&["PATH", "REMOTE", "SELECTED", "RELAY", "RTT", "MTU"]);
                for p in &d.paths {
                    t.row(vec![
                        p.path_id.clone(),
                        p.remote_addr.clone(),
                        out::yes_no(p.selected).into(),
                        out::yes_no(p.is_relay).into(),
                        format!("{:.2} ms", p.rtt.as_secs_f64() * 1000.0),
                        p.current_mtu.to_string(),
                    ]);
                }
                t.print();
            }
        },
    );
    Ok(())
}

fn doctor(ctx: &Ctx) -> Result<()> {
    let mut checks: Vec<(String, bool, String)> = Vec::new();
    let reg = gearbox_api::registry::registry_dir();
    checks.push((
        "registry dir".into(),
        reg.is_dir(),
        reg.display().to_string(),
    ));
    let mut stale = 0;
    if let Ok(entries) = std::fs::read_dir(&reg) {
        for e in entries.flatten() {
            if let Some(entry) = gearbox_api::registry::read(&e.path())
                && !gearbox_api::registry::pid_alive(entry.pid)
            {
                stale += 1;
            }
        }
    }
    checks.push((
        "stale registry entries".into(),
        stale == 0,
        format!("{stale} (pruned on next `instance list`)"),
    ));
    let did = crate::ctx::ensure_cli_did();
    checks.push((
        "cli identity".into(),
        did.is_ok(),
        did.clone().unwrap_or_else(|e| e.message),
    ));
    let key = crate::ctx::cli_key_path();
    let mode_ok = std::fs::metadata(&key)
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode() & 0o077 == 0
        })
        .unwrap_or(false);
    checks.push((
        "cli key permissions".into(),
        mode_ok,
        format!("{} should be 0600", key.display()),
    ));
    let shm: Vec<_> = std::fs::read_dir("/dev/shm")
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("qb_"))
                .collect()
        })
        .unwrap_or_default();
    checks.push((
        "peerbus shm segments".into(),
        true,
        format!("{} under /dev/shm", shm.len()),
    ));
    match ctx.target() {
        Ok(target) => {
            let target = target.clone();
            match ctx.client() {
                Ok(client) => {
                    let info: std::result::Result<HostInfo, _> = client.info();
                    match info {
                        Ok(info) => {
                            let sim_version = info.props().get("version").unwrap_or_default();
                            let cli_version = env!("CARGO_PKG_VERSION");
                            checks.push((
                                format!("instance `{}` answers", target.name),
                                true,
                                format!("pid {} uptime {}", info.pid, out::fmt_ms(info.uptime_ms)),
                            ));
                            checks.push((
                                "version match".into(),
                                sim_version == cli_version,
                                format!("sim {sim_version}, cli {cli_version}"),
                            ));
                            let _ = client
                                .call_env(topics::HOST_INFO, &gearbox_api::pack(&Ping::default()));
                        }
                        Err(err) => checks.push((
                            format!("instance `{}` answers", target.name),
                            false,
                            err.to_string(),
                        )),
                    }
                }
                Err(err) => checks.push((
                    format!("instance `{}` reachable", target.name),
                    false,
                    err.message,
                )),
            }
        }
        Err(err) => checks.push(("target instance".into(), false, err.message)),
    }
    let all_ok = checks.iter().all(|c| c.1);
    ctx.emit(
        || {
            json!({
                "ok": all_ok,
                "checks": checks.iter().map(|c| json!({ "check": c.0, "ok": c.1, "detail": c.2 })).collect::<Vec<_>>(),
            })
        },
        || {
            for (name, ok, detail) in &checks {
                println!("{} {:<26} {}", if *ok { "ok  " } else { "FAIL" }, name, detail);
            }
        },
    );
    if all_ok {
        Ok(())
    } else {
        Err(CliError::new(1, ""))
    }
}
