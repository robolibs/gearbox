//! `gearbox spawn`: USD loads, markers, manifests, moves.

use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::{UsdLoad, category, object_kind};
use serde::{Deserialize, Serialize};
use gearbox_api::tyres::{SavedTyrePressure, SavedTyres, validate_saved};
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
    From {
        file: String,
        /// Seconds to wait for each machine to finish loading
        #[arg(long, default_value_t = 90.0)]
        wait_timeout: f64,
    },
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
        Cmd::From { file, wait_timeout } => {
            if !wait_timeout.is_finite() || wait_timeout <= 0.0 || wait_timeout > 86400.0 {
                return Err(CliError::usage("wait timeout must be finite and within 0..=86400 seconds"));
            }
            apply_manifest_with_timeout(ctx, &file, Duration::from_secs_f64(wait_timeout))
        },
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

#[derive(Deserialize, Serialize, Debug)]
struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configuration_version: Option<u32>,
    #[serde(default)]
    spawn: Vec<Entry>,
    /// `[[attach]]` tables applied after every spawn: master, slave,
    /// optional hitch and coupler names, teleport.
    #[serde(default)]
    attach: Vec<AttachEntry>,
}

#[derive(Deserialize, Serialize, Debug)]
struct AttachEntry {
    master: String,
    slave: String,
    hitch: Option<String>,
    coupler: Option<String>,
    #[serde(default = "default_true")]
    teleport: bool,
}

pub fn save_machine(ctx: &Ctx, file: &str, machine: Option<String>) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    if client.clock(gearbox_api::clock_op::GET)?.paused == 0 {
        return Err(CliError::refused("pause the scene before saving a machine configuration"));
    }
    let mc = client.machine(&machine_id);
    let mut sub = mc.state()?;
    let state = gearbox_api::next_sample::<gearbox_api::MachineState>(&mut sub, ctx.timeout)?
        .ok_or_else(|| CliError::timeout("no pressure state to save"))?;
    let readings = gearbox_api::tyres::from_props(&state.props()).map_err(CliError::error)?;
    if readings.is_empty() { return Err(CliError::unsupported("machine has no registered pressure tyres")); }
    let tyres: SavedTyres = readings.into_iter().map(|t| (t.link, SavedTyrePressure {
        applied_bar: t.applied, target_bar: t.target,
    })).collect();
    validate_saved(&tyres).map_err(CliError::error)?;
    let objects = client.list(object_kind::MACHINE)?;
    let object = objects.iter().find(|o| o.props().get("machine_id").as_deref() == Some(&machine_id))
        .ok_or_else(|| CliError::unsupported("machine has no source scene entry"))?;
    let props = object.props();
    if props.get("configuration_snapshot").as_deref() != Some("1") {
        return Err(CliError::unsupported("configuration save requires a standalone single-machine Molla asset without attachment or attribute overrides"));
    }
    let path = props.get("path").ok_or_else(|| CliError::unsupported("machine source path unavailable"))?;
    if !std::path::Path::new(&path).is_absolute() {
        return Err(CliError::unsupported("machine source path must be absolute"));
    }
    let variants = serde_json::from_str(&props.get("snapshot_variants").ok_or_else(|| CliError::unsupported("variant snapshot unavailable"))?)?;
    let manifest = Manifest {
        configuration_version: Some(1), attach: vec![],
        spawn: vec![Entry {
            kind: "machine".into(), path, id: Some(machine_id.clone()), machine: None,
            at: vec![object.x, object.y, object.z], yaw: object.yaw_deg, variants, tyres,
        }],
    };
    if client.clock(gearbox_api::clock_op::GET)?.paused == 0 {
        return Err(CliError::refused("scene resumed while saving; no file written"));
    }
    let text = toml::to_string_pretty(&manifest).map_err(|e| CliError::error(e.to_string()))?;
    write_new_configuration(std::path::Path::new(file), text.as_bytes())
        .map_err(|e| CliError::error(format!("cannot save configuration {file}: {e}")))?;
    ctx.done(&machine_id, &format!("saved `{machine_id}` configuration to {file}; restore with `spawn from` (paused)"),
        || json!({"machine_id": machine_id, "file": file, "configuration_version": 1}));
    Ok(())
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn manifest_round_trips_distinct_applied_and_target_pressures() {
        let manifest = Manifest {
            configuration_version: Some(1), attach: vec![],
            spawn: vec![Entry {
                kind: "machine".into(), path: "/assets/tractor.usdz".into(), id: Some("tractor".into()), machine: None,
                at: vec![1.0, 0.0, 2.0], yaw: 30.0,
                variants: vec![("/robot".into(), "frontLoader".into(), "none".into())],
                tyres: SavedTyres::from([("wheel.front.left".into(), SavedTyrePressure { applied_bar: 1.3, target_bar: 2.2 })]),
            }],
        };
        let text = toml::to_string_pretty(&manifest).unwrap();
        let restored: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(restored.configuration_version, Some(1));
        assert_eq!(restored.spawn[0].tyres, manifest.spawn[0].tyres);
        assert_eq!(restored.spawn[0].variants, manifest.spawn[0].variants);
        assert_eq!(restored.spawn[0].at, manifest.spawn[0].at);
        let old: Manifest = toml::from_str("[[spawn]]\nkind='machine'\npath='tractor.usd'").unwrap();
        assert!(old.spawn[0].tyres.is_empty());
        assert!(old.configuration_version.is_none());
    }

    #[test]
    fn configuration_publication_never_overwrites_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("gearbox-snapshot-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("machine.toml");
        write_new_configuration(&path, b"original").unwrap();
        assert!(write_new_configuration(&path, b"replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn write_new_configuration(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().ok_or_else(|| std::io::Error::other("file name required"))?;
    let temp = path.with_file_name(format!(".{}.{}.{}.tmp", name.to_string_lossy(), std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
    let result = (|| {
        output.write_all(data)?;
        output.sync_all()?;
        std::fs::hard_link(&temp, path)?;
        std::fs::File::open(path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new(".")))?.sync_all()
    })();
    drop(output);
    let _ = std::fs::remove_file(temp);
    result
}

fn verify_restored_tyres(ctx: &Ctx, machine: &str, expected: &SavedTyres) -> Result<()> {
    let client = ctx.client()?;
    let mut sub = client.machine(machine).state()?;
    let deadline = Instant::now() + ctx.timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() { return Err(CliError::timeout("loaded machine did not confirm saved tyre state")); }
        let Some(state) = gearbox_api::next_sample::<gearbox_api::MachineState>(&mut sub, remaining)? else {
            return Err(CliError::timeout("loaded machine did not confirm saved tyre state"));
        };
        let Ok(tyres) = gearbox_api::tyres::from_props(&state.props()) else { continue };
        if expected.iter().all(|(link, saved)| tyres.iter().any(|t| &t.link == link
            && (t.applied - saved.applied_bar).abs() < 1e-6 && (t.target - saved.target_bar).abs() < 1e-6)) {
            if client.clock(gearbox_api::clock_op::GET)?.paused == 0 {
                return Err(CliError::refused("restored pressure configuration unexpectedly resumed physics"));
            }
            return Ok(());
        }
    }
}

fn load_saved_machine(ctx: &Ctx, id: &str, request: &UsdLoad, expected: &SavedTyres, timeout: Duration) -> Result<()> {
    let client = ctx.client()?;
    let mut events = client.events()?;
    check(client.load(request)?, &format!("load machine {id}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CliError::timeout(format!("saved machine `{id}` did not load")));
        }
        if let Some(event) = gearbox_api::next_sample::<gearbox_api::SceneEvent>(&mut events, remaining.min(Duration::from_millis(200)))?
            && event.kind == gearbox_api::event_kind::MACHINE_REJECTED
            && event.props().get("id").as_deref() == Some(id)
        {
            return Err(CliError::refused(event.props().get("reason").unwrap_or_else(|| "saved machine rejected".into())));
        }
        if client.machines()?.iter().any(|machine| machine.machine_id() == id) {
            return verify_restored_tyres(ctx, id, expected);
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Serialize, Debug)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    variants: Vec<(String, String, String)>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    tyres: SavedTyres,
}

/// `[[spawn]]` tables applied in order: kind = usd | machine | world |
/// terrain | marker, with path, id/machine, at = [x, y, z], yaw.
pub fn apply_manifest(ctx: &Ctx, file: &str) -> Result<()> {
    apply_manifest_with_timeout(ctx, file, Duration::from_secs(90))
}

fn apply_manifest_with_timeout(ctx: &Ctx, file: &str, wait_timeout: Duration) -> Result<()> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::error(format!("cannot read manifest {file}: {e}")))?;
    let manifest: Manifest = toml::from_str(&text)?;
    if manifest.configuration_version.is_some_and(|version| version != 1) {
        return Err(CliError::unsupported("unknown machine configuration version"));
    }
    for entry in &manifest.spawn {
        validate_saved(&entry.tyres).map_err(CliError::usage)?;
        if !entry.tyres.is_empty() && entry.kind != "machine" {
            return Err(CliError::usage("saved tyres require a machine entry"));
        }
    }
    let base = std::path::Path::new(file)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let client = ctx.client()?;
    let restore_paused = manifest.configuration_version == Some(1) || manifest.spawn.iter().any(|e| !e.tyres.is_empty());
    if restore_paused {
        let existing = client.list(object_kind::MACHINE)?;
        let mut ids = std::collections::HashSet::new();
        for entry in manifest.spawn.iter().filter(|e| e.kind == "machine") {
            let id = entry.id.as_ref().or(entry.machine.as_ref()).cloned().unwrap_or_else(|| default_id(&entry.path));
            if !ids.insert(id) { return Err(CliError::usage("duplicate machine id in saved configuration")); }
        }
        for entry in manifest.spawn.iter().filter(|e| e.kind == "machine") {
            let id = entry.id.as_ref().or(entry.machine.as_ref()).ok_or_else(|| CliError::usage("saved machine requires an explicit id"))?;
            if existing.iter().any(|o| o.props().get("machine_id").as_ref() == Some(id)) {
                return Err(CliError::refused(format!("machine {id} already exists; remove it or edit the saved id before restoring")));
            }
        }
    }
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
                let mut req = UsdLoad::new(&id, &path)
                    .at(x, y, z)
                    .yaw(e.yaw)
                    .category(category::MACHINE)
                    .with_prop("machine_id", &id);
                for (i, (prim, set, choice)) in e.variants.iter().enumerate() {
                    req = req.with_prop(&format!("variant.{i}"), &format!("{prim}|{set}|{choice}"));
                }
                if restore_paused { req = req.with_prop("start_paused", "true"); }
                if !e.tyres.is_empty() {
                    req = req.with_prop("tyre_snapshot", &serde_json::to_string(&e.tyres)?);
                }
                if !e.tyres.is_empty() {
                    load_saved_machine(ctx, &id, &req, &e.tyres, wait_timeout)?;
                } else {
                    check(client.load(&req)?, &format!("load machine {id}"))?;
                    wait_for_machine(ctx, &id, wait_timeout)?;
                }
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
