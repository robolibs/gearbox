//! `gearbox greenhouse`: the crops and rooms of a greenhouse world, and what a
//! crew does to them.

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::{PlantRecord, Props, RoomRecord, greenhouse_op as op};
use serde_json::{Value, json};

use super::check;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::out;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Every plant with its stage, health and place
    Plants {
        /// Only plants ready to harvest
        #[arg(long)]
        ripe: bool,
        /// Only this tray's plants
        #[arg(long)]
        tray: Option<String>,
        /// Only this crop (lettuce, tomato, potato, wheat, ...)
        #[arg(long)]
        species: Option<String>,
    },
    /// Each room's air, nutrient tank and the power bus
    Rooms,
    /// Sow a crop in a tray's empty places, or in one place
    Sow {
        tray: String,
        species: String,
        #[arg(long)]
        slot: Option<usize>,
        /// Only where this machine reaches (within 0.3 m)
        #[arg(long)]
        by: Option<String>,
    },
    /// Take in one plant (TRAY/SLOT) or every ripe plant of a tray
    Harvest {
        target: String,
        /// Only what this machine reaches (within 0.3 m)
        #[arg(long)]
        by: Option<String>,
    },
    /// Lift whatever stands in one place, dead or alive
    Clear {
        plant: String,
        /// Only if this machine reaches it (within 0.3 m)
        #[arg(long)]
        by: Option<String>,
    },
    /// Wash the perchlorate out of a regolith tray
    Wash { tray: String },
    /// Work organic matter into a regolith tray
    Amend {
        tray: String,
        /// Share of the bed, 0 to 0.5
        #[arg(long, default_value_t = 0.15)]
        share: f64,
    },
    /// Set the spectrum or output of a tray's lamps (`all` for every tray)
    Lamps {
        tray: String,
        /// growth (red-blue) or inspection (white)
        #[arg(long)]
        spectrum: Option<String>,
        /// Output, 0 to 1
        #[arg(long)]
        dimming: Option<f64>,
    },
    /// Flush and refill a room's nutrient tank
    CleanTank { room: String },
    /// Brush the dust off the solar panels
    CleanPanels,
    /// Brush the dust off a room's glazing, by a machine at the room if `--by`
    CleanGlazing {
        room: String,
        #[arg(long)]
        by: Option<String>,
    },
    /// Break something: power, reactor, fans:ROOM, hvac:ROOM, co2:ROOM,
    /// leak:ROOM, pump:ROOM, water_leak:ROOM or lamp:TRAY
    Fault { fault: String },
    /// Mend a fault, by a machine standing at its service spot if `--by`
    Repair {
        fault: String,
        #[arg(long)]
        by: Option<String>,
    },
    /// Change a room's climate targets: temperature, humidity, co2, pressure
    Setpoint {
        room: String,
        /// KEY=VALUE pairs
        #[arg(required = true)]
        set: Vec<String>,
    },
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Plants { ripe, tray, species } => {
            let mut filter = Props::new();
            if let Some(tray) = tray {
                filter.set("tray", &tray);
            }
            if let Some(species) = species {
                filter.set("species", &species);
            }
            plants(ctx, &ctx.client()?.plants(ripe, &filter)?);
            Ok(())
        }
        Cmd::Rooms => {
            rooms(ctx, &ctx.client()?.greenhouse_rooms()?);
            Ok(())
        }
        Cmd::Sow { tray, species, slot, by } => {
            let slot = slot.map(|s| s.to_string());
            let mut keys = vec![("tray", tray.as_str()), ("species", species.as_str())];
            if let Some(slot) = &slot {
                keys.push(("slot", slot));
            }
            act(ctx, op::SOW, &with_by(keys, &by), &tray)
        }
        Cmd::Harvest { target, by } => {
            let key = if target.contains('/') { "plant" } else { "tray" };
            act(ctx, op::HARVEST, &with_by(vec![(key, &target)], &by), &target)
        }
        Cmd::Clear { plant, by } => act(ctx, op::CLEAR, &with_by(vec![("plant", &plant)], &by), &plant),
        Cmd::Wash { tray } => act(ctx, op::WASH, &[("tray", &tray)], &tray),
        Cmd::Amend { tray, share } => act(ctx, op::AMEND, &[("tray", &tray), ("share", &share.to_string())], &tray),
        Cmd::Lamps { tray, spectrum, dimming } => {
            if spectrum.is_none() && dimming.is_none() {
                return Err(CliError::usage("give --spectrum, --dimming or both"));
            }
            let dimming = dimming.map(|d| d.to_string());
            let mut keys = vec![("tray", tray.as_str())];
            if let Some(spectrum) = &spectrum {
                keys.push(("spectrum", spectrum));
            }
            if let Some(dimming) = &dimming {
                keys.push(("dimming", dimming));
            }
            act(ctx, op::LAMPS, &keys, &tray)
        }
        Cmd::CleanTank { room } => act(ctx, op::CLEAN_TANK, &[("room", &room)], &room),
        Cmd::CleanPanels => act(ctx, op::CLEAN_PANELS, &[], ""),
        Cmd::CleanGlazing { room, by } => act(ctx, op::CLEAN_GLAZING, &with_by(vec![("room", &room)], &by), &room),
        Cmd::Fault { fault } => act(ctx, op::FAULT, &[("fault", &fault)], &fault),
        Cmd::Repair { fault, by } => {
            let mut keys = vec![("fault", fault.as_str())];
            if let Some(by) = &by {
                keys.push(("by", by));
            }
            act(ctx, op::REPAIR, &keys, &fault)
        }
        Cmd::Setpoint { room, set } => {
            let mut pairs = Vec::new();
            for pair in &set {
                let Some((key, value)) = pair.split_once('=') else {
                    return Err(CliError::usage(format!("`{pair}` is not KEY=VALUE")));
                };
                if value.trim().parse::<f64>().map_or(true, |v| !v.is_finite()) {
                    return Err(CliError::usage(format!("`{key}` needs a number, not `{value}`")));
                }
                pairs.push((key.trim(), value.trim()));
            }
            let mut keys = vec![("room", room.as_str())];
            keys.extend(pairs);
            act(ctx, op::SETPOINT, &keys, &room)
        }
    }
}

fn with_by<'a>(mut keys: Vec<(&'a str, &'a str)>, by: &'a Option<String>) -> Vec<(&'a str, &'a str)> {
    if let Some(by) = by {
        keys.push(("by", by));
    }
    keys
}

fn act(ctx: &Ctx, which: u32, keys: &[(&str, &str)], id: &str) -> Result<()> {
    let name = op::name(which);
    let status = check(ctx.client()?.greenhouse(which, &Props::from_pairs(keys))?, &format!("greenhouse {name}"))?;
    let detail = status.props().iter();
    let said: Vec<String> = detail.iter().map(|(k, v)| format!("{k} {v}")).collect();
    let done = if id.is_empty() { name.to_string() } else { format!("{name} {id}") };
    let message = if said.is_empty() { done } else { format!("{done}: {}", said.join(", ")) };
    ctx.done(id, &message, || {
        let mut answer = serde_json::Map::new();
        answer.insert("op".into(), json!(name));
        answer.insert("target".into(), json!(id));
        for (k, v) in detail {
            answer.insert(k, v.parse::<f64>().map_or(json!(v), |n| json!(n)));
        }
        Value::Object(answer)
    });
    Ok(())
}

fn as_json(props: &Props) -> serde_json::Map<String, Value> {
    props.iter().into_iter().map(|(k, v)| (k, v.parse::<f64>().map_or_else(|_| json!(v), |n| json!(n)))).collect()
}

fn plants(ctx: &Ctx, list: &[PlantRecord]) {
    ctx.emit(
        || {
            let rows = list.iter().map(|p| {
                let mut row = as_json(&p.props());
                row.insert("at".into(), json!([p.x, p.y, p.z]));
                row.insert("maturity".into(), json!(p.maturity));
                row.insert("health".into(), json!(p.health));
                row.insert("height_m".into(), json!(p.height_m));
                row.insert("alive".into(), json!(p.alive != 0));
                row.insert("ripe".into(), json!(p.ripe != 0));
                Value::Object(row)
            });
            Value::Array(rows.collect())
        },
        || {
            let mut table = out::Table::new(&["PLANT", "CROP", "STAGE", "GROWN", "HEALTH", "HEIGHT", "EDIBLE", "MOULD", "AT"]);
            for p in list {
                let props = p.props();
                let get = |k: &str| props.get(k).unwrap_or_default();
                let num = |k: &str| props.get_f64(k).unwrap_or(0.0);
                table.row(vec![
                    get("plant"),
                    get("species"),
                    get("stage"),
                    format!("{:.0}%", p.maturity * 100.0),
                    format!("{:.2}", p.health),
                    format!("{:.2} m", p.height_m),
                    format!("{:.3} kg", num("edible_kg")),
                    format!("{:.2}", num("mould")),
                    format!("{:.1} {:.1} {:.1}", p.x, p.y, p.z),
                ]);
            }
            if table.is_empty() {
                println!("no plants");
            } else {
                table.print();
            }
        },
    );
}

fn rooms(ctx: &Ctx, list: &[RoomRecord]) {
    ctx.emit(
        || {
            let rows = list.iter().map(|r| {
                let mut row = as_json(&r.props());
                row.insert("temperature_c".into(), json!(r.temperature_c));
                row.insert("humidity".into(), json!(r.humidity));
                row.insert("co2_ppm".into(), json!(r.co2_ppm));
                row.insert("o2_kpa".into(), json!(r.o2_kpa));
                row.insert("pressure_kpa".into(), json!(r.pressure_kpa));
                row.insert("lux".into(), json!(r.lux));
                Value::Object(row)
            });
            Value::Array(rows.collect())
        },
        || {
            if list.is_empty() {
                println!("no greenhouse in the scene");
            }
            for (i, r) in list.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                let props = r.props();
                let get = |k: &str| props.get(k).unwrap_or_default();
                let num = |k: &str| props.get_f64(k).unwrap_or(0.0);
                let faults = get("faults");
                out::kv(&[
                    ("room", get("room")),
                    ("air", format!("{:.1} °C, {:.0}% rh, {:.0} ppm CO2, {:.1} kPa O2, {:.1} kPa", r.temperature_c, r.humidity * 100.0, r.co2_ppm, r.o2_kpa, r.pressure_kpa)),
                    ("light", format!("{:.0} lux, glazing dust {:.2}", r.lux, num("glazing_dust"))),
                    ("tank", format!("{:.0} L, EC {:.2}, biofilm {:.2}, pump {}", num("tank_l"), num("ec"), num("biofilm"), get("pump"))),
                    ("equipment", format!("fans {}, hvac {}, co2 feed {}, leak {:.1e}/s", get("fans"), get("hvac"), get("co2_feed"), num("leak_per_s"))),
                    ("power", format!("{:.0} W in, {:.0} W out, battery {:.0} Wh, panel dust {:.2}, lamps at {:.0}%", num("supply_w"), num("demand_w"), num("battery_wh"), num("panel_dust"), num("lamp_power") * 100.0)),
                    ("faults", if faults.is_empty() { "none".into() } else { faults }),
                ]);
            }
        },
    );
}
