//! `gearbox scene`: clock, listing, poses, events, reset.

use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::wire::json as wire_json;
use gearbox_api::{
    ClockCommand, MachineState, SceneEvent, UsdLoad, category, clear_scope, clock_op, event_kind,
    next_sample, object_kind, pack,
};
use serde_json::json;

use super::check;
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
    /// Clock state, counts, world
    Status,
    /// Run the clock
    Play,
    /// Pause the clock
    Pause,
    /// Toggle the clock
    Toggle,
    /// Advance N physics frames while paused
    Step {
        #[arg(default_value_t = 1)]
        n: u32,
    },
    /// Time scale (not supported by the sim yet)
    Speed { factor: f64 },
    /// Objects in the scene
    List {
        /// machine, prop, marker, terrain
        #[arg(long)]
        kind: Option<String>,
    },
    /// Pose of one object; live for a machine
    Pose {
        id: String,
        /// Keep printing as it changes
        #[arg(long)]
        watch: bool,
    },
    /// Tail scene events
    Events {
        /// loaded, pose, harvested, removed, machine_ready
        #[arg(long)]
        kind: Option<String>,
        /// Stop after this many events
        #[arg(short = 'n', long)]
        count: Option<usize>,
    },
    /// Clear, then reload the world and every recorded manifest
    Reset,
    /// One tf-style tree of the scene: machines with their links, props and markers as leaves
    Tree,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Status => status(ctx),
        Cmd::Play => clock(ctx, clock_op::PLAY, 0),
        Cmd::Pause => clock(ctx, clock_op::PAUSE, 0),
        Cmd::Toggle => clock(ctx, clock_op::TOGGLE, 0),
        Cmd::Step { n } => clock(ctx, clock_op::STEP, n),
        Cmd::Speed { factor } => Err(CliError::unsupported(format!(
            "the sim has no time scale yet (asked for {factor}); use `scene step` or `scene pause`"
        ))),
        Cmd::List { kind } => list(ctx, kind),
        Cmd::Pose { id, watch } => pose(ctx, &id, watch),
        Cmd::Events { kind, count } => events(ctx, kind, count),
        Cmd::Reset => reset(ctx),
        Cmd::Tree => tree(ctx),
    }
}

fn status(ctx: &Ctx) -> Result<()> {
    let target = ctx.target()?.clone();
    let client = ctx.client()?;
    let info = client.info()?;
    let clock = client.clock(clock_op::GET)?;
    let world = ctx.context.world.clone().unwrap_or_default();
    ctx.emit(
        || {
            json!({
                "instance": target.name, "paused": clock.paused != 0, "step": clock.step,
                "machines": info.machine_count, "objects": info.object_count, "world": world,
            })
        },
        || {
            out::kv(&[
                ("instance", target.name.clone()),
                (
                    "clock",
                    if clock.paused != 0 {
                        "paused".into()
                    } else {
                        "running".into()
                    },
                ),
                ("step", clock.step.to_string()),
                ("machines", info.machine_count.to_string()),
                ("objects", info.object_count.to_string()),
                ("world", world.clone()),
            ]);
        },
    );
    Ok(())
}

fn clock(ctx: &Ctx, op: u32, steps: u32) -> Result<()> {
    let client = ctx.client()?;
    let state = client
        .call_env(
            gearbox_api::topics::SCENE_CLOCK,
            &pack(&ClockCommand { op, steps }),
        )
        .and_then(|env| {
            gearbox_api::unpack::<gearbox_api::ClockState>(env.type_hash(), env.wire())
                .map_err(|e| agentio::Error::Format(e.to_string()))
        })?;
    let name = clock_op::name(op);
    ctx.done(
        name,
        &format!(
            "clock {} (step {})",
            if state.paused != 0 {
                "paused"
            } else {
                "running"
            },
            state.step
        ),
        || json!({ "op": name, "paused": state.paused != 0, "step": state.step }),
    );
    Ok(())
}

fn list(ctx: &Ctx, kind: Option<String>) -> Result<()> {
    let kind_code = match &kind {
        Some(k) => {
            object_kind::parse(k).ok_or_else(|| CliError::usage(format!("unknown kind `{k}`")))?
        }
        None => object_kind::ANY,
    };
    let objects = ctx.client()?.list(kind_code)?;
    ctx.emit(
        || {
            json!(
                objects
                    .iter()
                    .filter_map(|o| wire_json::env_to_json(&pack(o)).ok())
                    .collect::<Vec<_>>()
            )
        },
        || {
            if objects.is_empty() {
                println!("scene is empty");
                return;
            }
            let mut t = Table::new(&["KIND", "ID", "X", "Y", "Z", "YAW", "PATH"]);
            for o in &objects {
                let p = o.props();
                t.row(vec![
                    object_kind::name(o.kind).into(),
                    p.get("id").unwrap_or_default(),
                    format!("{:.2}", o.x),
                    format!("{:.2}", o.y),
                    format!("{:.2}", o.z),
                    format!("{:.1}", o.yaw_deg),
                    p.get("path")
                        .or_else(|| p.get("machine_id"))
                        .unwrap_or_default(),
                ]);
            }
            t.print();
        },
    );
    Ok(())
}

fn pose(ctx: &Ctx, id: &str, watch: bool) -> Result<()> {
    let client = ctx.client()?;
    let machines = client.machines()?;
    if machines.iter().any(|m| m.machine_id() == id) {
        let mc = client.machine(id);
        let mut sub = mc.state()?;
        loop {
            let Some(s) = next_sample::<MachineState>(&mut sub, ctx.timeout)? else {
                return Err(CliError::timeout(format!("no state from machine `{id}`")));
            };
            let p = s.position();
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &json!({ "id": id, "x": p[0], "y": p[1], "z": p[2], "heading_rad": s.heading_rad })
                    )
                    .unwrap_or_default()
                );
            } else {
                println!(
                    "{id}: ({:.2}, {:.2}, {:.2}) heading {:.2} rad speed {:.2} m/s",
                    p[0],
                    p[1],
                    p[2],
                    s.heading_rad,
                    s.linear_speed()
                );
            }
            if !watch {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let mut last: Option<(f32, f32, f32)> = None;
    loop {
        let objects = client.list(object_kind::ANY)?;
        let obj = objects
            .iter()
            .find(|o| o.props().get("id").as_deref() == Some(id))
            .ok_or_else(|| CliError::new(1, format!("no object `{id}` in the scene")))?;
        let now = (obj.x, obj.y, obj.z);
        if last != Some(now) {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &json!({ "id": id, "x": obj.x, "y": obj.y, "z": obj.z, "yaw_deg": obj.yaw_deg })
                    )
                    .unwrap_or_default()
                );
            } else {
                println!(
                    "{id}: ({:.2}, {:.2}, {:.2}) yaw {:.1}°",
                    obj.x, obj.y, obj.z, obj.yaw_deg
                );
            }
            last = Some(now);
        }
        if !watch {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn events(ctx: &Ctx, kind: Option<String>, count: Option<usize>) -> Result<()> {
    let filter = match &kind {
        Some(k) => Some(
            (0..8u32)
                .find(|c| event_kind::name(*c) == k)
                .ok_or_else(|| CliError::usage(format!("unknown event kind `{k}`")))?,
        ),
        None => None,
    };
    let client = ctx.client()?;
    let mut sub = client.events()?;
    let mut seen = 0usize;
    let started = Instant::now();
    loop {
        let Some(ev) = next_sample::<SceneEvent>(&mut sub, Duration::from_secs(1))? else {
            if count.is_none()
                && ctx.timeout.as_secs_f64() > 0.0
                && started.elapsed() > Duration::from_secs(3600)
            {
                return Ok(());
            }
            continue;
        };
        if filter.is_some_and(|k| k != ev.kind) {
            continue;
        }
        if ctx.json {
            println!(
                "{}",
                serde_json::to_string(&wire_json::env_to_json(&pack(&ev)).unwrap_or(json!({})))
                    .unwrap_or_default()
            );
        } else {
            let p = ev.props();
            let extra: Vec<String> = p
                .iter()
                .into_iter()
                .filter(|(k, _)| k != "id")
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            println!(
                "{:14} {:20} ({:.2}, {:.2}, {:.2}) {}",
                event_kind::name(ev.kind),
                ev.id(),
                ev.x,
                ev.y,
                ev.z,
                extra.join(" ")
            );
        }
        seen += 1;
        if count.is_some_and(|n| seen >= n) {
            return Ok(());
        }
    }
}

fn reset(ctx: &Ctx) -> Result<()> {
    let client = ctx.client()?;
    check(client.clear(clear_scope::ALL)?, "clear")?;
    let mut reloaded = Vec::new();
    if let Some(world) = &ctx.context.world {
        let id = super::default_id(world);
        check(
            client.load(&UsdLoad::new(&id, world).category(category::WORLD))?,
            "reload world",
        )?;
        reloaded.push(world.clone());
    }
    for manifest in &ctx.context.manifests {
        super::spawn::apply_manifest(ctx, manifest)?;
        reloaded.push(manifest.clone());
    }
    ctx.done(
        "reset",
        &format!("scene reset; reloaded {}", reloaded.len()),
        || json!({ "reloaded": reloaded }),
    );
    Ok(())
}

/// `world` → every machine with its (composite) link tree, then props and
/// markers as single `base_link` leaves.
fn tree(ctx: &Ctx) -> Result<()> {
    let client = ctx.client()?;
    let objects = client.list(object_kind::ANY)?;
    let machines = client.machines()?;
    let mut attached: std::collections::HashSet<String> = Default::default();
    let mut machine_links: Vec<(String, Vec<gearbox_api::LinkRecord>)> = Vec::new();
    for m in &machines {
        let machine_id = m.machine_id();
        let mc = client.machine(&machine_id);
        if let Ok(info) = mc.info()
            && !info
                .props()
                .get("attached_to")
                .unwrap_or_default()
                .is_empty()
        {
            attached.insert(machine_id.clone());
            continue;
        }
        machine_links.push((machine_id, mc.links().unwrap_or_default()));
    }
    ctx.emit(
        || {
            json!({
                "machines": machine_links.iter().map(|(machine_id, links)| json!({
                    "machine_id": machine_id,
                    "links": links.iter().filter_map(|r| wire_json::env_to_json(&pack(r)).ok()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "objects": objects.iter().filter(|o| o.kind != object_kind::MACHINE)
                    .filter_map(|o| wire_json::env_to_json(&pack(o)).ok()).collect::<Vec<_>>(),
            })
        },
        || {
            println!("world");
            for (machine_id, links) in &machine_links {
                println!("  {machine_id}");
                let mut children: Vec<Vec<usize>> = vec![Vec::new(); links.len()];
                let mut roots = Vec::new();
                for (i, r) in links.iter().enumerate() {
                    if r.parent_index == gearbox_api::LinkRecord::NO_PARENT
                        || r.parent_index as usize >= links.len()
                    {
                        roots.push(i);
                    } else {
                        children[r.parent_index as usize].push(i);
                    }
                }
                fn walk(
                    i: usize,
                    depth: usize,
                    links: &[gearbox_api::LinkRecord],
                    children: &[Vec<usize>],
                ) {
                    println!("{}{}  [{}]", "  ".repeat(depth), links[i].name(), links[i].role());
                    for &c in &children[i] {
                        walk(c, depth + 1, links, children);
                    }
                }
                for r in roots {
                    walk(r, 2, links, &children);
                }
            }
            for o in objects.iter().filter(|o| o.kind != object_kind::MACHINE) {
                let p = o.props();
                println!(
                    "  {}  [{}]",
                    p.get("id").unwrap_or_default(),
                    object_kind::name(o.kind)
                );
                println!(
                    "    {}  ({:+.2}, {:+.2}, {:+.2})",
                    p.get("link").unwrap_or_else(|| "base_link".into()),
                    o.x,
                    o.y,
                    o.z
                );
            }
        },
    );
    Ok(())
}
