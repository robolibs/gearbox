//! `gearbox spawn`: USD loads, markers, manifests, moves.

use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::{UsdLoad, category, object_kind};
use serde::Deserialize;
use serde_json::json;

use super::{check, default_id, usd_path, xyz};
use crate::ctx::Ctx;
use crate::error::{CliError, Result};

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(ClapArgs, Debug, Clone, Default)]
struct Place {
    /// Position X Y Z in metres north, up and east of home; two values mean X Z
    #[arg(long, num_args = 2..=3, value_name = "M")]
    at: Option<Vec<f32>>,
    /// Place on Earth instead of `--at`: LAT LON [ALT], degrees and metres
    /// above the ellipsoid; without ALT it stands on the ground
    #[arg(long, num_args = 2..=3, value_name = "DEG", allow_negative_numbers = true, conflicts_with = "at")]
    lla: Option<Vec<f64>>,
    /// Heading in degrees
    #[arg(long, default_value_t = 0.0)]
    yaw: f32,
}

impl Place {
    // The host reads the place on Earth from the request's props.
    fn on_earth(&self, mut req: UsdLoad) -> UsdLoad {
        if let Some(lla) = &self.lla {
            req = req.with_prop("lat", &lla[0].to_string()).with_prop("lon", &lla[1].to_string());
            if let Some(alt) = lla.get(2) {
                req = req.with_prop("alt", &alt.to_string());
            }
        }
        req
    }
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Load a USD as a prop (or another category)
    Usd {
        path: String,
        /// Runtime id (default: file stem)
        #[arg(long)]
        id: Option<String>,
        #[command(flatten)]
        place: Place,
        /// static, machine, variant, world, terrain
        #[arg(long, default_value = "static")]
        category: String,
        /// Variant selection PRIM SET OPTION, repeatable
        #[arg(long, num_args = 3, value_names = ["PRIM", "SET", "OPTION"])]
        variant: Vec<String>,
    },
    /// Load a machine USD under an id and wait for its agent
    Machine {
        path: String,
        /// Machine id (default: file stem)
        #[arg(long)]
        machine: Option<String>,
        #[command(flatten)]
        place: Place,
        /// Return as soon as the load is accepted
        #[arg(long)]
        no_wait: bool,
        /// Seconds to wait for the machine agent
        #[arg(long, default_value_t = 30.0)]
        wait_timeout: f64,
        /// Variant selection PRIM SET OPTION, repeatable (e.g. /robot hitchRear linked)
        #[arg(long, num_args = 3, value_names = ["PRIM", "SET", "OPTION"])]
        variant: Vec<String>,
    },
    /// Load a world USD
    World { path: String },
    /// Load a terrain USD
    Terrain { path: String },
    /// Place a marker
    Marker {
        id: String,
        #[arg(long, num_args = 2..=3, value_name = "M", required = true)]
        at: Vec<f32>,
    },
    /// Apply a TOML manifest of spawns in order
    From { file: String },
    /// Move a loaded object by re-issuing its load
    Move {
        id: String,
        #[command(flatten)]
        place: Place,
    },
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Usd {
            path,
            id,
            place,
            category: cat,
            variant,
        } => {
            let cat_code = category::parse(&cat)
                .ok_or_else(|| CliError::usage(format!("unknown category `{cat}`")))?;
            let id = id.unwrap_or_else(|| default_id(&path));
            let (x, y, z) = xyz(&place.at);
            let mut req = UsdLoad::new(&id, &usd_path(&path))
                .at(x, y, z)
                .yaw(place.yaw)
                .category(cat_code);
            if cat_code == category::MACHINE {
                req = req.with_prop("machine_id", &id);
            }
            for (n, chunk) in variant.chunks(3).enumerate() {
                if let [prim, set, opt] = chunk {
                    req = req.with_prop(&format!("variant.{n}"), &format!("{prim}|{set}|{opt}"));
                }
            }
            check(ctx.client()?.load(&req)?, &format!("load {id}"))?;
            ctx.done(&id, &format!("loaded {id} ({cat}) from {path}"), || {
                json!({ "id": id, "path": path, "category": cat, "x": x, "y": y, "z": z, "yaw_deg": place.yaw })
            });
            Ok(())
        }
        Cmd::Machine {
            path,
            machine,
            place,
            no_wait,
            wait_timeout,
            variant,
        } => {
            let machine_id = machine.unwrap_or_else(|| default_id(&path));
            let (x, y, z) = xyz(&place.at);
            let mut req = UsdLoad::new(&machine_id, &usd_path(&path))
                .at(x, y, z)
                .yaw(place.yaw)
                .category(category::MACHINE)
                .with_prop("machine_id", &machine_id);
            req = place.on_earth(req);
            for (n, chunk) in variant.chunks(3).enumerate() {
                if let [prim, set, opt] = chunk {
                    req = req.with_prop(&format!("variant.{n}"), &format!("{prim}|{set}|{opt}"));
                }
            }
            let client = ctx.client()?;
            check(client.load(&req)?, &format!("load machine {machine_id}"))?;
            let mut did = String::new();
            if !no_wait {
                did = wait_for_machine(ctx, &machine_id, Duration::from_secs_f64(wait_timeout))?;
            }
            ctx.done(
                &machine_id,
                &format!("machine {machine_id} ready {did}"),
                || {
                    json!({ "machine_id": machine_id, "path": path, "did": did, "x": x, "y": y, "z": z, "yaw_deg": place.yaw })
                },
            );
            Ok(())
        }
        Cmd::World { path } => {
            let id = default_id(&path);
            let req = UsdLoad::new(&id, &usd_path(&path)).category(category::WORLD);
            check(ctx.client()?.load(&req)?, &format!("load world {id}"))?;
            let mut context = ctx.context.clone();
            context.world = Some(usd_path(&path));
            context.save()?;
            ctx.done(
                &id,
                &format!("loaded world {id} from {path}"),
                || json!({ "id": id, "path": path, "category": "world" }),
            );
            Ok(())
        }
        Cmd::Terrain { path } => {
            let id = default_id(&path);
            let req = UsdLoad::new(&id, &usd_path(&path)).category(category::TERRAIN);
            check(ctx.client()?.load(&req)?, &format!("load terrain {id}"))?;
            ctx.done(
                &id,
                &format!("loaded terrain {id} from {path}"),
                || json!({ "id": id, "path": path, "category": "terrain" }),
            );
            Ok(())
        }
        Cmd::Marker { id, at } => {
            let (x, y, z) = xyz(&Some(at));
            check(
                ctx.client()?.marker_set(&id, x, y, z)?,
                &format!("marker {id}"),
            )?;
            ctx.done(
                &id,
                &format!("marker {id} at ({x:.2}, {y:.2}, {z:.2})"),
                || json!({ "id": id, "x": x, "y": y, "z": z }),
            );
            Ok(())
        }
        Cmd::From { file } => apply_manifest(ctx, &file),
        Cmd::Move { id, place } => {
            let client = ctx.client()?;
            let objects = client.list(object_kind::ANY)?;
            let obj = objects
                .iter()
                .find(|o| o.props().get("id").as_deref() == Some(id.as_str()))
                .ok_or_else(|| CliError::new(1, format!("no object `{id}` in the scene")))?;
            let props = obj.props();
            let path = props
                .get("path")
                .ok_or_else(|| CliError::unsupported(format!("`{id}` has no path to reload")))?;
            let (x, y, z) = match &place.at {
                Some(_) => xyz(&place.at),
                None => (obj.x, obj.y, obj.z),
            };
            let cat = match obj.kind {
                object_kind::MACHINE => category::MACHINE,
                object_kind::TERRAIN => category::TERRAIN,
                _ => category::STATIC,
            };
            let mut req = UsdLoad::new(&id, &path)
                .at(x, y, z)
                .yaw(place.yaw)
                .category(cat);
            if let Some(machine_id) = props.get("machine_id") {
                req = req.with_prop("machine_id", &machine_id);
            }
            check(client.load(&req)?, &format!("move {id}"))?;
            ctx.done(
                &id,
                &format!("moved {id} to ({x:.2}, {y:.2}, {z:.2})"),
                || json!({ "id": id, "x": x, "y": y, "z": z, "yaw_deg": place.yaw }),
            );
            Ok(())
        }
    }
}

pub fn wait_for_machine(ctx: &Ctx, machine_id: &str, timeout: Duration) -> Result<String> {
    let client = ctx.client()?;
    let deadline = Instant::now() + timeout;
    loop {
        for m in client.machines()? {
            if m.machine_id() == machine_id
                || m.machine_id().starts_with(&format!("{machine_id}_"))
            {
                return Ok(m.did());
            }
        }
        if Instant::now() >= deadline {
            return Err(CliError::timeout(format!(
                "machine `{machine_id}` did not appear within {:.0}s; is the USD a machine (GearboxMachineAPI)?",
                timeout.as_secs_f64()
            )));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[derive(Deserialize, Debug)]
struct Manifest {
    #[serde(default)]
    spawn: Vec<Entry>,
    /// `[[attach]]` tables applied after every spawn: master, slave,
    /// optional hitch and coupler names, teleport.
    #[serde(default)]
    attach: Vec<AttachEntry>,
}

#[derive(Deserialize, Debug)]
struct AttachEntry {
    master: String,
    slave: String,
    hitch: Option<String>,
    coupler: Option<String>,
    #[serde(default = "default_true")]
    teleport: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Debug)]
struct Entry {
    kind: String,
    #[serde(default)]
    path: String,
    id: Option<String>,
    #[serde(alias = "ns")]
    machine: Option<String>,
    #[serde(default)]
    at: Vec<f32>,
    #[serde(default)]
    yaw: f32,
}

/// `[[spawn]]` tables applied in order: kind = usd | machine | world |
/// terrain | marker, with path, id/machine, at = [x, y, z], yaw.
pub fn apply_manifest(ctx: &Ctx, file: &str) -> Result<()> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::error(format!("cannot read manifest {file}: {e}")))?;
    let manifest: Manifest = toml::from_str(&text)?;
    let base = std::path::Path::new(file)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let client = ctx.client()?;
    let mut applied = Vec::new();
    for e in &manifest.spawn {
        let at = if e.at.is_empty() {
            None
        } else {
            Some(e.at.clone())
        };
        let (x, y, z) = xyz(&at);
        let rel = base.join(&e.path);
        let path = if rel.exists() {
            usd_path(&rel.to_string_lossy())
        } else {
            usd_path(&e.path)
        };
        let id =
            e.id.clone()
                .or_else(|| e.machine.clone())
                .unwrap_or_else(|| default_id(&e.path));
        match e.kind.as_str() {
            "usd" | "static" | "prop" => {
                let req = UsdLoad::new(&id, &path).at(x, y, z).yaw(e.yaw);
                check(client.load(&req)?, &format!("load {id}"))?;
            }
            "machine" => {
                let req = UsdLoad::new(&id, &path)
                    .at(x, y, z)
                    .yaw(e.yaw)
                    .category(category::MACHINE)
                    .with_prop("machine_id", &id);
                check(client.load(&req)?, &format!("load machine {id}"))?;
                wait_for_machine(ctx, &id, Duration::from_secs(30))?;
            }
            "world" => {
                let req = UsdLoad::new(&id, &path).category(category::WORLD);
                check(client.load(&req)?, &format!("load world {id}"))?;
            }
            "terrain" => {
                let req = UsdLoad::new(&id, &path).category(category::TERRAIN);
                check(client.load(&req)?, &format!("load terrain {id}"))?;
            }
            "marker" => {
                check(client.marker_set(&id, x, y, z)?, &format!("marker {id}"))?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "manifest entry `{id}` has unknown kind `{other}`"
                )));
            }
        }
        if !ctx.json && !ctx.quiet {
            println!("{:8} {id}", e.kind);
        }
        applied.push(json!({ "kind": e.kind, "id": id, "path": path }));
    }
    for a in &manifest.attach {
        wait_for_machine(ctx, &a.slave, Duration::from_secs(30))?;
        wait_for_machine(ctx, &a.master, Duration::from_secs(30))?;
        let mut req = gearbox_api::AttachRequest::new(0, &a.slave);
        if let Some(h) = &a.hitch {
            req = req.with_hitch(h);
        }
        if let Some(c) = &a.coupler {
            req = req.with_coupler(c);
        }
        if a.teleport {
            req = req.teleporting();
        }
        check(
            client.machine(&a.master).attach(&req)?,
            &format!("attach {} to {}", a.slave, a.master),
        )?;
        if !ctx.json && !ctx.quiet {
            println!("{:8} {} -> {}", "attach", a.slave, a.master);
        }
        applied.push(json!({ "kind": "attach", "master": a.master, "slave": a.slave }));
    }
    let abs = std::fs::canonicalize(file)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| file.to_string());
    let mut context = ctx.context.clone();
    if !context.manifests.contains(&abs) {
        context.manifests.push(abs.clone());
    }
    context.save()?;
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "manifest": abs, "applied": applied }))
                .unwrap_or_default()
        );
    }
    Ok(())
}
