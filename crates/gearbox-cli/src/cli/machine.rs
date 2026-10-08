//! `gearbox machine`: inspect and drive machines through claim / cmd_vel /
//! release on each machine's own agent.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::wire::json as wire_json;
use gearbox_api::{
    ClaimResponse, ControllerCommand, MachineClient, MachineInfo, MachineState, Props, TwistCmd,
    code, next_sample, pack,
};
use serde_json::json;

use super::{check, install_ctrlc, parse_duration};
use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::out::{self, Table};

const DEFAULT_HOLD_MS: u32 = 500;
const STREAM_HZ: f64 = 20.0;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Machines with their controllers and holders
    List,
    /// Everything a machine reports about itself
    Info { machine: Option<String> },
    /// Save a paused standalone machine configuration, including applied/target tyre pressures
    Save {
        file: String,
        #[arg(long)]
        machine: Option<String>,
    },
    /// Stream the machine's state
    State {
        machine: Option<String>,
        /// Keep printing
        #[arg(long)]
        watch: bool,
        /// Prints per second while watching
        #[arg(long, default_value_t = 5.0)]
        rate: f64,
    },
    /// Drive with a constant twist for a while, then stop
    Move {
        machine: Option<String>,
        /// Forward speed in m/s (negative reverses); 1 unless --left or --up is given
        #[arg(long, allow_hyphen_values = true)]
        forward: Option<f64>,
        /// Sideways speed in m/s, positive to the left (flying machines)
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        left: f64,
        /// Climb rate in m/s, negative descends (flying machines)
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        up: f64,
        /// Yaw rate in rad/s (positive turns left)
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        turn: f64,
        /// Duration such as 3s, 500ms, 1m
        #[arg(long = "for", default_value = "3s")]
        duration: String,
        /// Stop once this many metres from where the move began, over the
        /// ground, instead of after --for
        #[arg(long, conflicts_with = "hold")]
        distance: Option<f64>,
        /// Keep streaming until Ctrl-C
        #[arg(long)]
        hold: bool,
        /// Steal the session from its holder
        #[arg(long)]
        take: bool,
    },
    /// Drive a route in one go: legs one after another with no stop between
    /// them, each `forward=M/S turn=RAD/S` held until `distance=M` or
    /// `angle=DEG` (heading turned either way). A leg without a turn steers
    /// along a line: the one the route starts on, or `offset=M` to its right,
    /// in the course the turns so far have set. `do=CONTROLLER:KEY=VAL[,…]`
    /// sends a controller command as the leg begins. A line is printed as
    /// each leg ends, and the machine brakes to a stand after the last
    Route {
        machine: String,
        /// Legs such as `forward=3 distance=150 do=hitch_rear:position=0.1`,
        /// `forward=1.5 turn=-0.5 angle=180` or `forward=3 offset=6 distance=80`
        #[arg(required = true)]
        legs: Vec<String>,
        /// The line straight legs steer along, as another route printed it
        /// (`X,Y,HEADING`), instead of the one this route starts on
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
        line: Option<Vec<f64>>,
        /// Steal the session from its holder
        #[arg(long)]
        take: bool,
    },
    /// Send a zero twist and release
    Stop {
        machine: Option<String>,
        /// Steal the session from its holder
        #[arg(long)]
        take: bool,
    },
    /// Drive from the keyboard: WASD or arrows, space stops, q quits
    Drive {
        machine: Option<String>,
        #[arg(long, default_value_t = 2.0)]
        speed: f64,
        #[arg(long, default_value_t = 0.8)]
        yaw: f64,
        #[arg(long)]
        take: bool,
    },
    /// Generic controller command: KEY=VAL pairs, `value=` and `element=` are numeric
    Cmd {
        controller: String,
        #[arg(value_name = "KEY=VAL")]
        pairs: Vec<String>,
        /// Machine (default: the selection)
        #[arg(long)]
        machine: Option<String>,
        /// Route to a controller of this attached slave
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        take: bool,
    },
    /// Controller table with types and interfaces
    Controllers { machine: Option<String> },
    /// The link tree: names, roles, parents, static offsets
    Links {
        machine: Option<String>,
        /// One row per link instead of an indented tree
        #[arg(long)]
        flat: bool,
    },
    /// Stream link poses: switches the machine's tf on, prints, switches it off
    Tf {
        machine: Option<String>,
        /// Only this link
        #[arg(long)]
        link: Option<String>,
        /// Prints per second
        #[arg(long, default_value_t = 5.0)]
        rate: f64,
        /// Stop after this many poses
        #[arg(short = 'n', long)]
        count: Option<usize>,
    },
    /// Set a named value on a link: LINK NAME VALUE (e.g. boom position 0.8)
    SetValue {
        link: String,
        name: String,
        value: f64,
        #[arg(long)]
        machine: Option<String>,
        #[arg(long)]
        take: bool,
    },
    /// Set gauge-bar tyre pressure atomically and wait for accepted target readback
    TyrePressure {
        bar: f64,
        /// One-based axle id from machine state (inferred front to rear unless authored)
        #[arg(long, conflicts_with = "wheel", value_parser = clap::value_parser!(u16).range(1..))]
        axle: Option<u16>,
        /// Link name; omit both selectors to change every tyre
        #[arg(long)]
        wheel: Option<String>,
        #[arg(long)]
        machine: Option<String>,
        #[arg(long)]
        take: bool,
        /// Reuse this CLI identity's active drive session without claiming or releasing it
        #[arg(long, conflicts_with = "take")]
        reuse_session: bool,
    },
    /// Attachments: what hangs on a machine, attach and detach slaves
    Tools {
        #[command(subcommand)]
        cmd: ToolsCmd,
    },
}

#[derive(Subcommand, Debug)]
enum ToolsCmd {
    /// Attachments below a master, depth-first
    List { machine: Option<String> },
    /// Hitches and couplers of a machine, free or occupied
    Couplings { machine: Option<String> },
    /// Hang SLAVE on a hitch of the master
    Attach {
        /// The slave's machine id
        slave: String,
        /// Master machine (default: the selection)
        #[arg(long)]
        machine: Option<String>,
        /// Hitch name on the master (default: the only free one of a matching type)
        #[arg(long)]
        hitch: Option<String>,
        /// Coupler name on the slave (default: the only one of a matching type)
        #[arg(long)]
        coupler: Option<String>,
        /// Move the slave onto the hitch first
        #[arg(long)]
        teleport: bool,
        /// Steal the master's session if someone holds it
        #[arg(long)]
        take: bool,
    },
    /// Release SLAVE from the master
    Detach {
        slave: String,
        #[arg(long)]
        machine: Option<String>,
        #[arg(long)]
        take: bool,
    },
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::List => list(ctx),
        Cmd::Info { machine } => info(ctx, machine),
        Cmd::Save { file, machine } => super::spawn::save_machine(ctx, &file, machine),
        Cmd::State {
            machine,
            watch,
            rate,
        } => state(ctx, machine, watch, rate),
        Cmd::Move {
            machine,
            forward,
            left,
            up,
            turn,
            duration,
            distance,
            hold,
            take,
        } => {
            let forward = forward.unwrap_or(if left == 0.0 && up == 0.0 { 1.0 } else { 0.0 });
            let until = match distance {
                Some(metres) => Until::Distance(metres),
                None if hold => Until::Interrupted,
                None => Until::Elapsed(duration),
            };
            move_for(ctx, machine, [forward, left, up], turn, until, take)
        }
        Cmd::Route {
            machine,
            legs,
            line,
            take,
        } => route(ctx, machine, &legs, line.as_deref(), take),
        Cmd::Stop { machine, take } => stop(ctx, machine, take),
        Cmd::Drive {
            machine,
            speed,
            yaw,
            take,
        } => drive(ctx, machine, speed, yaw, take),
        Cmd::Cmd {
            machine,
            controller,
            pairs,
            tool,
            take,
        } => command(ctx, machine, &controller, &pairs, tool, take),
        Cmd::Controllers { machine } => controllers(ctx, machine),
        Cmd::Links { machine, flat } => links(ctx, machine, flat),
        Cmd::Tf {
            machine,
            link,
            rate,
            count,
        } => tf(ctx, machine, link, rate, count),
        Cmd::SetValue {
            link,
            name,
            value,
            machine,
            take,
        } => set_value(ctx, machine, &link, &name, value, take),
        Cmd::TyrePressure { bar, axle, wheel, machine, take, reuse_session } => {
            let scope = axle.map(|a| format!("axle:{a}"))
                .or_else(|| wheel.map(|w| format!("wheel:{w}"))).unwrap_or_else(|| "all".into());
            set_tyre_pressure(ctx, machine, &scope, bar, take, reuse_session)
        }
        Cmd::Tools { cmd } => tools(ctx, cmd),
    }
}

fn list(ctx: &Ctx) -> Result<()> {
    let client = ctx.client()?;
    let refs = client.machines()?;
    let mut rows = Vec::new();
    for r in &refs {
        let machine_id = r.machine_id();
        let mc = client.machine(&machine_id);
        let info = mc.info().ok();
        let session = mc.session().ok();
        rows.push((r.clone(), info, session));
    }
    ctx.emit(
        || {
            json!(rows
                .iter()
                .map(|(r, info, session)| {
                    let mut v = json!({ "machine_id": r.machine_id(), "did": r.did() });
                    let p = r.props();
                    v["kind"] = json!(p.get("kind").unwrap_or_default());
                    v["addr"] = json!(p.get("addr").unwrap_or_default());
                    if let Some(i) = info {
                        v["controllers"] = json!(controller_rows(i)
                            .iter()
                            .map(|c| json!({ "instance": c.0, "type": c.1, "command": c.2, "state": c.3 }))
                            .collect::<Vec<_>>());
                    }
                    if let Some(s) = session {
                        v["held"] = json!(s.held != 0);
                        v["holder"] = json!(s.holder());
                    }
                    v
                })
                .collect::<Vec<_>>())
        },
        || {
            if rows.is_empty() {
                println!("no machines");
                return;
            }
            let mut t = Table::new(&["MACHINE", "KIND", "CONTROLLERS", "HELD BY", "DID"]);
            for (r, info, session) in &rows {
                let p = r.props();
                let ctrls = info
                    .as_ref()
                    .map(|i| {
                        controller_rows(i)
                            .iter()
                            .map(|c| format!("{}:{}", c.0, c.2))
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_else(|| "?".into());
                let holder = session
                    .as_ref()
                    .filter(|s| s.held != 0)
                    .map(|s| s.holder())
                    .unwrap_or_default();
                t.row(vec![
                    r.machine_id(),
                    p.get("kind").unwrap_or_default(),
                    ctrls,
                    holder,
                    r.did(),
                ]);
            }
            t.print();
        },
    );
    Ok(())
}

/// (instance, type, command interface, state interfaces) per controller.
fn controller_rows(info: &MachineInfo) -> Vec<(String, String, String, String)> {
    let p = info.props();
    (0..info.controller_count as usize)
        .map(|n| {
            (
                p.get(&format!("controller.{n}.instance"))
                    .unwrap_or_default(),
                p.get(&format!("controller.{n}.type")).unwrap_or_default(),
                p.get(&format!("controller.{n}.command_interface"))
                    .unwrap_or_default(),
                p.get(&format!("controller.{n}.state_interfaces"))
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn info(ctx: &Ctx, machine: Option<String>) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    let info = mc.info()?;
    let session = mc.session()?;
    let p = info.props();
    ctx.emit(
        || {
            let mut v = wire_json::env_to_json(&pack(&info)).unwrap_or(json!({}));
            v["session"] = wire_json::env_to_json(&pack(&session)).unwrap_or(json!({}));
            v
        },
        || {
            let mut pairs = vec![
                ("machine id", p.get("machine_id").unwrap_or_default()),
                ("kind", p.get("kind").unwrap_or_default()),
                ("did", p.get("did").unwrap_or_default()),
                ("controllers", info.controller_count.to_string()),
                (
                    "links",
                    format!(
                        "{}{}",
                        p.get("link_count").unwrap_or_default(),
                        if p.get("links_derived").as_deref() == Some("true") {
                            " (derived)"
                        } else {
                            ""
                        }
                    ),
                ),
                (
                    "held by",
                    if session.held != 0 {
                        format!(
                            "{} (session {}, idle {})",
                            session.holder(),
                            session.session,
                            out::fmt_ms(session.idle_ms)
                        )
                    } else {
                        "nobody".into()
                    },
                ),
            ];
            let rows = controller_rows(&info);
            let lines: Vec<String> = rows
                .iter()
                .map(|c| format!("{} {} cmd={} state={}", c.0, c.1, c.2, c.3))
                .collect();
            let joined = lines.join("\n              ");
            pairs.push(("controller", joined));
            out::kv(&pairs);
        },
    );
    Ok(())
}

fn controllers(ctx: &Ctx, machine: Option<String>) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let info = ctx.client()?.machine(&machine_id).info()?;
    let rows = controller_rows(&info);
    ctx.emit(
        || {
            json!(
                rows.iter()
                    .map(|c| json!({ "instance": c.0, "type": c.1, "command": c.2, "state": c.3 }))
                    .collect::<Vec<_>>()
            )
        },
        || {
            let mut t = Table::new(&["INSTANCE", "TYPE", "COMMAND", "STATE"]);
            for c in &rows {
                t.row(vec![c.0.clone(), c.1.clone(), c.2.clone(), c.3.clone()]);
            }
            t.print();
        },
    );
    Ok(())
}

fn state(ctx: &Ctx, machine: Option<String>, watch: bool, rate: f64) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    let mut sub = mc.state()?;
    let period = Duration::from_secs_f64(1.0 / rate.max(0.1));
    let mut last_print = Instant::now() - period;
    loop {
        let Some(s) = next_sample::<MachineState>(&mut sub, ctx.timeout)? else {
            return Err(CliError::timeout(format!(
                "no state from machine `{machine_id}` within {:.1}s",
                ctx.timeout.as_secs_f64()
            )));
        };
        if last_print.elapsed() < period {
            continue;
        }
        last_print = Instant::now();
        if ctx.json {
            println!(
                "{}",
                serde_json::to_string(&wire_json::env_to_json(&pack(&s)).unwrap_or(json!({})))
                    .unwrap_or_default()
            );
        } else {
            println!("{}", state_line(&machine_id, &s));
        }
        if !watch {
            return Ok(());
        }
    }
}

fn state_line(machine_id: &str, s: &MachineState) -> String {
    let p = s.position();
    // Where on Earth, then the pose in the machine's own datum (east, north, up).
    let props = s.props();
    let earth = match (props.get("lat"), props.get("lon"), props.get("alt")) {
        (Some(lat), Some(lon), Some(alt)) => format!("lla ({lat}, {lon}, {alt}) "),
        _ => String::new(),
    };
    format!(
        "{machine_id}: {earth}pos ({:+.2}, {:+.2}, {:+.2}) heading {:+.2} rad speed {:.2} m/s yaw {:+.2} rad/s roll {:+.2} pitch {:+.2} session {}",
        p[0],
        p[1],
        p[2],
        s.heading_rad,
        s.linear_speed(),
        s.yaw_rate(),
        s.roll_rad,
        s.pitch_rad,
        s.session
    )
}

/// Claim the machine, refusing early when it cannot take a twist.
fn claim(ctx: &Ctx, mc: &MachineClient<'_>, machine_id: &str, take: bool) -> Result<ClaimResponse> {
    let info = mc.info()?;
    if info.controllers_with_command("cmd_vel").is_empty() {
        let rows = controller_rows(&info);
        let have: Vec<String> = rows
            .iter()
            .filter(|c| !c.2.is_empty())
            .map(|c| format!("{}={}", c.0, c.2))
            .collect();
        return Err(CliError::unsupported(format!(
            "machine `{machine_id}` has no cmd_vel controller; command interfaces: {}",
            if have.is_empty() {
                "none".to_string()
            } else {
                have.join(", ")
            }
        )));
    }
    let hold = (ctx.timeout.as_millis() as u32).clamp(DEFAULT_HOLD_MS, 5_000);
    let res = mc.claim(hold, take)?;
    if res.code == code::BUSY {
        return Err(CliError::busy(format!(
            "machine `{machine_id}` is held by {}; add --take to steal it",
            res.holder()
        )));
    }
    if res.code != code::OK {
        return Err(CliError::new(
            crate::error::exit_for_wire_code(res.code),
            format!(
                "claim on `{machine_id}` refused: {}",
                Props::from_bytes(&res.props)
                    .get("message")
                    .unwrap_or_else(|| format!("code {}", res.code))
            ),
        ));
    }
    Ok(res)
}

type StateFeed = Option<gearbox_api::peerbus::Subscriber<gearbox_api::Env>>;

/// The newest state on the feed, the backlog skipped, so what is judged on it
/// is where the machine is now.
fn newest_state(feed: &mut StateFeed, last: &mut Option<MachineState>) {
    if let Some(sub) = feed.as_mut() {
        let mut wait = Duration::from_millis(5);
        while let Ok(Some(s)) = next_sample::<MachineState>(sub, wait) {
            *last = Some(s);
            wait = Duration::ZERO;
        }
    }
}

/// Holds a zero twist until the machine stands: one released at speed rolls on.
fn brake_to_stand(
    mc: &mut MachineClient<'_>,
    session: u64,
    feed: &mut StateFeed,
    last: &mut Option<MachineState>,
    stop_flag: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    let braking = Instant::now();
    while braking.elapsed() < Duration::from_secs(30)
        && !stop_flag.load(std::sync::atomic::Ordering::Relaxed)
    {
        check(
            mc.cmd_twist(&TwistCmd::with_lateral(session, 0.0, 0.0, 0.0, 0.0))?,
            "cmd_vel",
        )?;
        newest_state(feed, last);
        if last.as_ref().is_some_and(|s| s.linear_speed().abs() < 0.05) {
            break;
        }
        std::thread::sleep(Duration::from_secs_f64(1.0 / STREAM_HZ));
    }
    Ok(())
}

/// One leg of a route: a twist held until the machine has gone so far over
/// the ground or turned so far either way, and the controller commands sent
/// as it begins. A leg that does not turn steers itself along a line.
#[derive(Debug, Clone, PartialEq)]
struct Leg {
    forward: f64,
    turn: f64,
    end: LegEnd,
    /// Metres to the right of the route's first line the line lies, kept
    /// for the legs after until another says otherwise.
    offset: Option<f64>,
    /// `(controller, [(key, value)])`, sent through the route's own session.
    commands: Vec<(String, Vec<(String, String)>)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LegEnd {
    Metres(f64),
    Radians(f64),
}

/// `forward=M/S turn=RAD/S`, one of `distance=M` or `angle=DEG`, and any of
/// `offset=M` and `do=CONTROLLER:KEY=VAL[,KEY=VAL]`.
fn parse_leg(text: &str) -> Result<Leg> {
    let mut leg = Leg {
        forward: 0.0,
        turn: 0.0,
        end: LegEnd::Metres(0.0),
        offset: None,
        commands: Vec::new(),
    };
    let mut ends = false;
    for word in text.split_whitespace() {
        let (key, value) = word
            .split_once('=')
            .ok_or_else(|| CliError::error(format!("leg `{text}`: `{word}` is not KEY=VALUE")))?;
        if key == "do" {
            let (controller, pairs) = value.split_once(':').ok_or_else(|| {
                CliError::error(format!(
                    "leg `{text}`: `do={value}` needs CONTROLLER:KEY=VAL"
                ))
            })?;
            let pairs = pairs
                .split(',')
                .map(|pair| {
                    pair.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .ok_or_else(|| {
                            CliError::error(format!("leg `{text}`: `{pair}` is not KEY=VAL"))
                        })
                })
                .collect::<Result<Vec<_>>>()?;
            leg.commands.push((controller.to_string(), pairs));
            continue;
        }
        let number: f64 = value
            .parse()
            .map_err(|_| CliError::error(format!("leg `{text}`: `{value}` is not a number")))?;
        match key {
            "forward" => leg.forward = number,
            "turn" => leg.turn = number,
            "offset" => leg.offset = Some(number),
            "distance" => (leg.end, ends) = (LegEnd::Metres(number.abs()), true),
            "angle" => (leg.end, ends) = (LegEnd::Radians(number.abs().to_radians()), true),
            _ => {
                return Err(CliError::error(format!(
                    "leg `{text}`: unknown key `{key}`"
                )));
            }
        }
    }
    if !ends {
        return Err(CliError::error(format!(
            "leg `{text}` needs distance= or angle="
        )));
    }
    Ok(leg)
}

/// How hard a straight leg turns back onto its line: toward the heading that
/// closes the gap, and onto that heading.
const LINE_GAIN: f64 = 0.6;
const HEADING_GAIN: f64 = 1.5;
const LINE_TURN_MAX: f64 = 0.4;

fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// The turn that brings a machine at `at` heading `heading` onto the line
/// `offset` metres right of the one through `origin` along `first`, driving
/// it along `course` (the first line's way, or back down it).
fn line_turn(
    at: [f64; 3],
    heading: f64,
    origin: [f64; 3],
    first: f64,
    course: f64,
    offset: f64,
    forward: f64,
) -> f64 {
    let right = (first.sin(), -first.cos());
    let gap = (at[0] - origin[0]) * right.0 + (at[1] - origin[1]) * right.1 - offset;
    let way = (course - first).cos().signum();
    let aim = course + way * (LINE_GAIN * gap).atan2(forward.abs().max(0.5));
    (HEADING_GAIN * wrap_angle(aim - heading)).clamp(-LINE_TURN_MAX, LINE_TURN_MAX)
}

fn route(
    ctx: &Ctx,
    machine: String,
    legs: &[String],
    line: Option<&[f64]>,
    take: bool,
) -> Result<()> {
    let legs = legs
        .iter()
        .map(|leg| parse_leg(leg))
        .collect::<Result<Vec<_>>>()?;
    let line = match line {
        Some(&[x, y, heading]) => Some(([x, y, 0.0], heading)),
        Some(_) => return Err(CliError::usage("--line takes X,Y,HEADING")),
        None => None,
    };
    let machine_id = ctx.machine_id(Some(machine))?;
    let client = ctx.client()?;
    let mut mc = client.machine(&machine_id);
    let session = claim(ctx, &mc, &machine_id, take)?.session;
    let period = Duration::from_secs_f64(1.0 / STREAM_HZ);
    let mut feed = mc.state().ok();
    let mut last: Option<MachineState> = None;
    let stop_flag = install_ctrlc();
    // The first line runs from where the route starts the way the machine
    // faces, unless `--line` names one, which is then driven whichever way
    // along it the machine faces; each turning leg turns the course by its
    // angle. The line is printed so a later route can drive on from it.
    while last.is_none() && !stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
        newest_state(&mut feed, &mut last);
    }
    let Some(start) = &last else {
        return Ok(());
    };
    let (line_origin, first) = line.unwrap_or((start.position(), start.heading_rad));
    let facing = wrap_angle(start.heading_rad - first).abs() > std::f64::consts::FRAC_PI_2;
    let mut course = if facing {
        wrap_angle(first + std::f64::consts::PI)
    } else {
        first
    };
    let mut offset = 0.0;
    println!("line {},{},{}", line_origin[0], line_origin[1], first);
    let _ = std::io::stdout().flush();
    // Each leg starts from where the last one ended; the twist never lapses
    // between them, so the machine drives the route in one go.
    'legs: for (index, leg) in legs.iter().enumerate() {
        for (controller, pairs) in &leg.commands {
            let mut props = Props::from_pairs(&[("controller", controller.as_str())]);
            for (key, value) in pairs {
                props.set(key, value);
            }
            let command = ControllerCommand {
                session,
                value: 0.0,
                element: 0,
                _pad: 0,
                props: props.into_bytes(),
            };
            check(mc.command(&command)?, "controller command")?;
        }
        offset = leg.offset.unwrap_or(offset);
        let mut origin: Option<[f64; 3]> = None;
        let mut heading: Option<f64> = None;
        let mut turned = 0.0_f64;
        loop {
            if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
                break 'legs;
            }
            newest_state(&mut feed, &mut last);
            let turn = match (&last, leg.turn) {
                (Some(s), turn) if turn == 0.0 => line_turn(
                    s.position(),
                    s.heading_rad,
                    line_origin,
                    first,
                    course,
                    offset,
                    leg.forward,
                ),
                (_, turn) => turn,
            };
            // A sim slow to answer one twist is sent the next; a route is
            // not given up for a reply that came late.
            match mc.cmd_twist(&TwistCmd::with_lateral(
                session,
                leg.forward,
                0.0,
                0.0,
                turn,
            )) {
                Ok(response) => {
                    check(response, "cmd_vel")?;
                }
                Err(error) => eprintln!("gearbox: leg {}: {error}; sending again", index + 1),
            }
            if let Some(s) = &last {
                let at = s.position();
                let from = *origin.get_or_insert(at);
                if let Some(before) = heading.replace(s.heading_rad) {
                    turned += wrap_angle(s.heading_rad - before);
                }
                let done = match leg.end {
                    LegEnd::Metres(metres) => (at[0] - from[0]).hypot(at[1] - from[1]) >= metres,
                    LegEnd::Radians(radians) => turned.abs() >= radians,
                };
                if done && let LegEnd::Radians(radians) = leg.end {
                    course = wrap_angle(course + radians.copysign(leg.turn));
                }
                if done {
                    let line = format!(
                        "leg {} done at ({:.2}, {:.2}) heading {:.3}",
                        index + 1,
                        at[0],
                        at[1],
                        s.heading_rad
                    );
                    println!("{line}");
                    let _ = std::io::stdout().flush();
                    break;
                }
            }
            std::thread::sleep(period);
        }
    }
    brake_to_stand(&mut mc, session, &mut feed, &mut last, &stop_flag)?;
    let _ = mc.cmd_vel(session, 0.0, 0.0);
    let _ = mc.release(session);
    ctx.done(
        &machine_id,
        &format!("drove `{machine_id}` along {} legs; released", legs.len()),
        || json!({ "machine_id": machine_id, "legs": legs.len() }),
    );
    Ok(())
}

/// When a move ends.
enum Until {
    Elapsed(String),
    Distance(f64),
    Interrupted,
}

fn move_for(
    ctx: &Ctx,
    machine: Option<String>,
    [forward, left, up]: [f64; 3],
    turn: f64,
    until: Until,
    take: bool,
) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mut mc = client.machine(&machine_id);
    let session = claim(ctx, &mc, &machine_id, take)?.session;
    let period = Duration::from_secs_f64(1.0 / STREAM_HZ);
    let started = Instant::now();
    let deadline = match &until {
        Until::Elapsed(duration) => Some(started + parse_duration(duration)?),
        _ => None,
    };
    let mut sub = mc.state().ok();
    let mut last_state: Option<MachineState> = None;
    let mut origin: Option<[f64; 3]> = None;
    let mut last_print = Instant::now();
    let stop_flag = install_ctrlc();
    while deadline.is_none_or(|deadline| Instant::now() < deadline)
        && !stop_flag.load(std::sync::atomic::Ordering::Relaxed)
    {
        let twist = TwistCmd::with_lateral(session, forward, left, up, turn);
        check(mc.cmd_twist(&twist)?, "cmd_vel")?;
        newest_state(&mut sub, &mut last_state);
        if let (Until::Distance(metres), Some(s)) = (&until, &last_state) {
            let at = s.position();
            let from = *origin.get_or_insert(at);
            if (at[0] - from[0]).hypot(at[1] - from[1]) >= *metres {
                break;
            }
        }
        if !ctx.json
            && !ctx.quiet
            && last_print.elapsed() >= Duration::from_millis(500)
            && let Some(s) = &last_state
        {
            eprintln!("{}", state_line(&machine_id, s));
            last_print = Instant::now();
        }
        std::thread::sleep(period);
    }
    // A move that ends at a distance stops there.
    if matches!(until, Until::Distance(_)) {
        brake_to_stand(&mut mc, session, &mut sub, &mut last_state, &stop_flag)?;
    }
    let _ = mc.cmd_vel(session, 0.0, 0.0);
    let _ = mc.release(session);
    let pos = last_state
        .as_ref()
        .map(|s| s.position())
        .unwrap_or([0.0; 3]);
    let seconds = started.elapsed().as_secs_f64();
    ctx.done(
        &machine_id,
        &format!(
            "drove `{machine_id}` forward {forward} left {left} up {up} m/s turn {turn} rad/s for {seconds:.1}s; now at ({:.2}, {:.2}, {:.2}); released",
            pos[0],
            pos[1],
            pos[2]
        ),
        || json!({ "machine_id": machine_id, "forward": forward, "left": left, "up": up, "turn": turn, "seconds": seconds, "x": pos[0], "y": pos[1], "z": pos[2] }),
    );
    Ok(())
}

fn stop(ctx: &Ctx, machine: Option<String>, take: bool) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mut mc = client.machine(&machine_id);
    let session = claim(ctx, &mc, &machine_id, take)?.session;
    check(mc.cmd_vel(session, 0.0, 0.0)?, "cmd_vel")?;
    check(mc.release(session)?, "release")?;
    ctx.done(
        &machine_id,
        &format!("stopped `{machine_id}` and released"),
        || json!({ "machine_id": machine_id, "stopped": true }),
    );
    Ok(())
}

fn command(
    ctx: &Ctx,
    machine: Option<String>,
    controller: &str,
    pairs: &[String],
    tool: Option<String>,
    take: bool,
) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    let mut props = Props::from_pairs(&[("controller", controller)]);
    if let Some(tool) = &tool {
        props.set("tool", tool);
    }
    let mut value = 0.0;
    let mut element = 0u32;
    for pair in pairs {
        let Some((k, v)) = pair.split_once('=') else {
            return Err(CliError::usage(format!("expected KEY=VAL, got `{pair}`")));
        };
        match k {
            "value" => {
                value = v
                    .parse()
                    .map_err(|_| CliError::usage(format!("bad value `{v}`")))?
            }
            "element" => {
                element = v
                    .parse()
                    .map_err(|_| CliError::usage(format!("bad element `{v}`")))?
            }
            _ => props.set(k, v),
        }
    }
    let res = mc.claim(DEFAULT_HOLD_MS, take)?;
    if res.code == code::BUSY {
        return Err(CliError::busy(format!(
            "machine `{machine_id}` is held by {}; add --take",
            res.holder()
        )));
    }
    let cmd = ControllerCommand {
        session: res.session,
        value,
        element,
        _pad: 0,
        props: props.into_bytes(),
    };
    let status = mc.command(&cmd)?;
    let _ = mc.release(res.session);
    check(status, "controller command")?;
    ctx.done(
        &machine_id,
        &format!("sent {controller} command to `{machine_id}`"),
        || json!({ "machine_id": machine_id, "controller": controller, "value": value, "element": element }),
    );
    Ok(())
}

fn drive(ctx: &Ctx, machine: Option<String>, speed: f64, yaw: f64, take: bool) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mut mc = client.machine(&machine_id);
    let session = claim(ctx, &mc, &machine_id, take)?.session;
    let mut sub = mc.state().ok();
    let raw = RawTerminal::enable()?;
    eprintln!("driving `{machine_id}`: W/S forward/back, A/D turn, space stop, Q quit\r");
    let (mut v, mut w) = (0.0f64, 0.0f64);
    let period = Duration::from_secs_f64(1.0 / STREAM_HZ);
    let stop_flag = install_ctrlc();
    let mut buf = [0u8; 8];
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let n = raw.read_nonblocking(&mut buf);
        let mut quit = false;
        let mut i = 0;
        while i < n {
            match buf[i] {
                b'w' | b'W' => v = speed,
                b's' | b'S' => v = -speed,
                b'a' | b'A' => w = yaw,
                b'd' | b'D' => w = -yaw,
                b' ' => {
                    v = 0.0;
                    w = 0.0;
                }
                b'q' | b'Q' | 3 => quit = true,
                0x1b if i + 2 < n && buf[i + 1] == b'[' => {
                    match buf[i + 2] {
                        b'A' => v = speed,
                        b'B' => v = -speed,
                        b'D' => w = yaw,
                        b'C' => w = -yaw,
                        _ => {}
                    }
                    i += 2;
                }
                _ => {}
            }
            i += 1;
        }
        if quit {
            break;
        }
        if n == 0 {
            // Keys are held by repeating; silence lets the turn decay.
            w *= 0.85;
            if w.abs() < 0.02 {
                w = 0.0;
            }
        }
        if mc
            .cmd_vel(session, v, w)
            .map(|s| !s.is_ok())
            .unwrap_or(true)
        {
            break;
        }
        if let Some(sub) = sub.as_mut()
            && let Ok(Some(s)) = next_sample::<MachineState>(sub, Duration::from_millis(5))
        {
            let p = s.position();
            eprint!(
                "\rcmd v {:+.2} w {:+.2} | pos ({:+.1}, {:+.1}, {:+.1}) heading {:+.2} speed {:.2}   ",
                v,
                w,
                p[0],
                p[1],
                p[2],
                s.heading_rad,
                s.linear_speed()
            );
        }
        std::thread::sleep(period);
    }
    let _ = mc.cmd_vel(session, 0.0, 0.0);
    let _ = mc.release(session);
    drop(raw);
    eprintln!("\nreleased `{machine_id}`");
    Ok(())
}

/// Raw, non-blocking stdin for the interactive drive; restored on drop.
struct RawTerminal {
    original: libc::termios,
}

impl RawTerminal {
    fn enable() -> Result<Self> {
        // SAFETY: termios calls on fd 0 with a properly sized struct.
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut original) != 0 {
                return Err(CliError::error(
                    "stdin is not a terminal; `machine drive` needs one",
                ));
            }
            let mut raw = original;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 0;
            raw.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(0, libc::TCSANOW, &raw);
            Ok(Self { original })
        }
    }

    fn read_nonblocking(&self, buf: &mut [u8]) -> usize {
        let mut fds = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll on one valid pollfd with a zero timeout.
        let ready = unsafe { libc::poll(&mut fds, 1, 0) };
        if ready <= 0 {
            return 0;
        }
        std::io::stdin().read(buf).unwrap_or(0)
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        // SAFETY: restoring the attributes captured in `enable`.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.original);
        }
        let _ = std::io::stderr().flush();
    }
}

fn links(ctx: &Ctx, machine: Option<String>, flat: bool) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    let records = mc.links()?;
    if records.is_empty() {
        return Err(CliError::unsupported(format!(
            "machine `{machine_id}` reports no links"
        )));
    }
    let info = mc.info().ok();
    let derived = info
        .as_ref()
        .map(|i| i.props().get("links_derived").unwrap_or_default() == "true")
        .unwrap_or(false);
    ctx.emit(
        || {
            json!({
                "machine_id": machine_id,
                "derived": derived,
                "links": records
                    .iter()
                    .filter_map(|r| wire_json::env_to_json(&pack(r)).ok())
                    .collect::<Vec<_>>(),
            })
        },
        || {
            if derived {
                println!("{machine_id}: link tree derived from rigid bodies; author GearboxLinkAPI to make it explicit");
            }
            let offset = |r: &gearbox_api::LinkRecord| {
                if r.x == 0.0 && r.y == 0.0 && r.z == 0.0 && r.qw == 1.0 {
                    String::new()
                } else {
                    format!("({:+.2}, {:+.2}, {:+.2})", r.x, r.y, r.z)
                }
            };
            let values = |r: &gearbox_api::LinkRecord| {
                r.values()
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            if flat {
                let mut t = Table::new(&["LINK", "ROLE", "ELEMENT", "PARENT", "OFFSET", "VALUES", "PRIM"]);
                for r in &records {
                    let p = r.props();
                    t.row(vec![
                        r.name(),
                        r.role(),
                        r.element().unwrap_or_default(),
                        r.parent().unwrap_or_default(),
                        offset(r),
                        values(r),
                        p.get("prim").unwrap_or_default(),
                    ]);
                }
                t.print();
                return;
            }
            let mut children: Vec<Vec<usize>> = vec![Vec::new(); records.len()];
            let mut roots = Vec::new();
            for (i, r) in records.iter().enumerate() {
                if r.parent_index == gearbox_api::LinkRecord::NO_PARENT
                    || r.parent_index as usize >= records.len()
                {
                    roots.push(i);
                } else {
                    children[r.parent_index as usize].push(i);
                }
            }
            fn walk(
                i: usize,
                depth: usize,
                records: &[gearbox_api::LinkRecord],
                children: &[Vec<usize>],
                offset: &dyn Fn(&gearbox_api::LinkRecord) -> String,
                values: &dyn Fn(&gearbox_api::LinkRecord) -> String,
            ) {
                let r = &records[i];
                let p = r.props();
                let element = r
                    .element()
                    .map(|e| format!(" {e}"))
                    .unwrap_or_default();
                let coupling = p
                    .get("coupling")
                    .map(|c| format!("  coupling {c}"))
                    .unwrap_or_default();
                let via = p
                    .get("joint")
                    .map(|j| format!("  via {}", j.rsplit('/').next().unwrap_or(&j)))
                    .unwrap_or_default();
                let vals = values(r);
                let vals = if vals.is_empty() {
                    vals
                } else {
                    format!("  {vals}")
                };
                println!(
                    "{}{}  [{}{}]  {}{}{}{}",
                    "  ".repeat(depth),
                    r.name(),
                    r.role(),
                    element,
                    offset(r),
                    via,
                    coupling,
                    vals
                );
                for &c in &children[i] {
                    walk(c, depth + 1, records, children, offset, values);
                }
            }
            for root in roots {
                walk(root, 0, &records, &children, &offset, &values);
            }
        },
    );
    Ok(())
}

fn tf(
    ctx: &Ctx,
    machine: Option<String>,
    link: Option<String>,
    rate: f64,
    count: Option<usize>,
) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    let mut sub = mc.tf()?;
    check(mc.set_tf(true)?, "tf on")?;
    let stop_flag = install_ctrlc();
    let period = Duration::from_secs_f64(1.0 / rate.max(0.1));
    let mut last_print: std::collections::HashMap<String, Instant> = Default::default();
    let mut seen = 0usize;
    let started = Instant::now();
    let result = loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break Ok(());
        }
        let pose = match next_sample::<gearbox_api::LinkPose>(&mut sub, Duration::from_millis(200))
        {
            Ok(Some(p)) => p,
            Ok(None) => {
                if seen == 0 && started.elapsed() > ctx.timeout {
                    break Err(CliError::timeout(format!(
                        "no link poses from `{machine_id}` within {:.1}s; is the clock running?",
                        ctx.timeout.as_secs_f64()
                    )));
                }
                continue;
            }
            Err(err) => break Err(err.into()),
        };
        let name = pose.name();
        if link.as_deref().is_some_and(|l| l != name) {
            continue;
        }
        let due = last_print
            .get(&name)
            .map(|t| t.elapsed() >= period)
            .unwrap_or(true);
        if !due {
            continue;
        }
        last_print.insert(name.clone(), Instant::now());
        if ctx.json {
            println!(
                "{}",
                serde_json::to_string(&wire_json::env_to_json(&pack(&pose)).unwrap_or(json!({})))
                    .unwrap_or_default()
            );
        } else {
            println!(
                "{:<24} ({:+8.3}, {:+8.3}, {:+8.3})  q({:+.3}, {:+.3}, {:+.3}, {:+.3})  t={}ms",
                name, pose.x, pose.y, pose.z, pose.qw, pose.qx, pose.qy, pose.qz, pose.stamp_ms
            );
        }
        seen += 1;
        if count.is_some_and(|n| seen >= n) {
            break Ok(());
        }
    };
    let _ = mc.set_tf(false);
    result
}

fn tools(ctx: &Ctx, cmd: ToolsCmd) -> Result<()> {
    match cmd {
        ToolsCmd::List { machine } => {
            let machine_id = ctx.machine_id(machine)?;
            let records = ctx.client()?.machine(&machine_id).tools()?;
            ctx.emit(
                || {
                    json!(
                        records
                            .iter()
                            .filter_map(|r| wire_json::env_to_json(&pack(r)).ok())
                            .collect::<Vec<_>>()
                    )
                },
                || {
                    if records.is_empty() {
                        println!("nothing attached to `{machine_id}`");
                        return;
                    }
                    let mut t =
                        Table::new(&["SLAVE", "HITCH", "COUPLER", "TYPE", "CONTROLLED", "DENIED"]);
                    for r in &records {
                        let p = r.props();
                        t.row(vec![
                            format!("{}{}", "  ".repeat(r.depth as usize), r.slave()),
                            p.get("hitch").unwrap_or_default(),
                            p.get("coupler").unwrap_or_default(),
                            p.get("type").unwrap_or_default(),
                            out::yes_no(r.controlled != 0).into(),
                            p.get("denied")
                                .filter(|d| !d.is_empty())
                                .unwrap_or_else(|| "-".into()),
                        ]);
                    }
                    t.print();
                },
            );
            Ok(())
        }
        ToolsCmd::Couplings { machine } => {
            let machine_id = ctx.machine_id(machine)?;
            let client = ctx.client()?;
            let mc = client.machine(&machine_id);
            let links = mc.links()?;
            let attached = mc.tools()?;
            let info = mc.info()?;
            let attached_to = info.props().get("attached_to").unwrap_or_default();
            let mut rows: Vec<(String, String, String, String)> = Vec::new();
            for l in &links {
                let Some(c) = l.props().get("coupling") else {
                    continue;
                };
                let mut parts = c.splitn(3, '|');
                let side = parts.next().unwrap_or_default().to_string();
                let kind = parts.next().unwrap_or_default().to_string();
                let name = parts.next().unwrap_or_default().to_string();
                let state = if side == "hitch" {
                    attached
                        .iter()
                        .find(|a| a.props().get("hitch").as_deref() == Some(name.as_str()))
                        .map(|a| format!("holds {}", a.slave()))
                        .unwrap_or_else(|| "free".into())
                } else if !attached_to.is_empty() {
                    format!("on {attached_to}")
                } else {
                    "free".into()
                };
                rows.push((name, side, kind, state));
            }
            ctx.emit(
                || {
                    json!(
                        rows.iter()
                            .map(|r| json!({ "name": r.0, "side": r.1, "type": r.2, "state": r.3 }))
                            .collect::<Vec<_>>()
                    )
                },
                || {
                    if rows.is_empty() {
                        println!("`{machine_id}` has no couplings");
                        return;
                    }
                    let mut t = Table::new(&["COUPLING", "SIDE", "TYPE", "STATE"]);
                    for r in &rows {
                        t.row(vec![r.0.clone(), r.1.clone(), r.2.clone(), r.3.clone()]);
                    }
                    t.print();
                },
            );
            Ok(())
        }
        ToolsCmd::Attach {
            slave,
            machine,
            hitch,
            coupler,
            teleport,
            take,
        } => {
            let machine_id = ctx.machine_id(machine)?;
            let client = ctx.client()?;
            let mc = client.machine(&machine_id);
            let session = session_for(ctx, &mc, &machine_id, take)?;
            let mut req = gearbox_api::AttachRequest::new(session, &slave);
            if let Some(h) = &hitch {
                req = req.with_hitch(h);
            }
            if let Some(c) = &coupler {
                req = req.with_coupler(c);
            }
            if teleport {
                req = req.teleporting();
            }
            let status = mc.attach(&req)?;
            if session != 0 {
                let _ = mc.release(session);
            }
            check(status, &format!("attach {slave} to {machine_id}"))?;
            let record = mc
                .tools()
                .ok()
                .and_then(|list| list.into_iter().find(|r| r.slave() == slave));
            let (h, c, k) = record
                .map(|r| {
                    let p = r.props();
                    (
                        p.get("hitch").unwrap_or_default(),
                        p.get("coupler").unwrap_or_default(),
                        p.get("type").unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            ctx.done(
                &slave,
                &format!("attached `{slave}` to `{machine_id}` via {h} / {c} ({k})"),
                || json!({ "master": machine_id, "slave": slave, "hitch": h, "coupler": c, "type": k }),
            );
            Ok(())
        }
        ToolsCmd::Detach {
            slave,
            machine,
            take,
        } => {
            let machine_id = ctx.machine_id(machine)?;
            let client = ctx.client()?;
            let mc = client.machine(&machine_id);
            let session = session_for(ctx, &mc, &machine_id, take)?;
            let status = mc.detach(session, &slave)?;
            if session != 0 {
                let _ = mc.release(session);
            }
            check(status, &format!("detach {slave} from {machine_id}"))?;
            ctx.done(
                &slave,
                &format!("detached `{slave}` from `{machine_id}`"),
                || json!({ "master": machine_id, "slave": slave }),
            );
            Ok(())
        }
    }
}

/// The session an attachment request must carry: none while the master is
/// free, ours after a claim (stolen with `--take`) while someone holds it.
fn session_for(ctx: &Ctx, mc: &MachineClient<'_>, machine_id: &str, take: bool) -> Result<u64> {
    let session = mc.session()?;
    if session.held == 0 {
        return Ok(0);
    }
    if !take {
        return Err(CliError::busy(format!(
            "machine `{machine_id}` is held by {}; add --take",
            session.holder()
        )));
    }
    let res = mc.claim(ctx.timeout.as_millis() as u32, true)?;
    if res.code != code::OK {
        return Err(CliError::new(
            crate::error::exit_for_wire_code(res.code),
            format!(
                "claim on `{machine_id}` refused: {}",
                Props::from_bytes(&res.props)
                    .get("message")
                    .unwrap_or_else(|| format!("code {}", res.code))
            ),
        ));
    }
    Ok(res.session)
}

enum PressureSession {
    Claimed(u64),
    Borrowed(u64),
}

#[cfg(test)]
mod pressure_session_tests {
    use super::*;
    use clap::Parser;
    use std::cell::RefCell;

    fn held(holder: &str) -> gearbox_api::SessionInfo {
        gearbox_api::SessionInfo {
            session: 17, held: 1,
            props: Props::from_pairs(&[("holder", holder)]).into_bytes(),
            ..Default::default()
        }
    }

    #[test]
    fn pressure_session_reuse_flag_cannot_steal() {
        assert!(crate::Cli::try_parse_from(["gearbox", "machine", "tyre-pressure", "1.8", "--reuse-session"]).is_ok());
        assert!(crate::Cli::try_parse_from(["gearbox", "machine", "tyre-pressure", "1.8", "--reuse-session", "--take"]).is_err());
    }

    #[test]
    fn borrowed_pressure_session_requires_active_matching_identity() {
        assert!(PressureSession::borrowed(held("owner"), "owner").is_ok());
        assert!(PressureSession::borrowed(held("owner"), "other").is_err());
        assert!(PressureSession::borrowed(held(""), "").is_err());
        let mut idle = held("owner");
        idle.held = 0;
        assert!(PressureSession::borrowed(idle, "owner").is_err());
        let mut zero = held("owner");
        zero.session = 0;
        assert!(PressureSession::borrowed(zero, "owner").is_err());
    }

    #[test]
    fn pressure_claim_rejections_do_not_produce_a_session() {
        for code in [code::BUSY, code::REFUSED] {
            assert!(PressureSession::claimed(ClaimResponse { code, session: 99, ..Default::default() }).is_err());
        }
        assert!(PressureSession::claimed(ClaimResponse::granted(0)).is_err());
    }

    #[test]
    fn pressure_command_releases_only_claimed_sessions_even_on_error() {
        for borrowed in [false, true] {
            for outcome in 0..3 {
                let events = RefCell::new(Vec::new());
                let lease = if borrowed {
                    PressureSession::borrowed(held("owner"), "owner").unwrap()
                } else {
                    PressureSession::claimed(ClaimResponse::granted(17)).unwrap()
                };
                let result = lease.command(|id| {
                    events.borrow_mut().push(("send", id));
                    match outcome {
                        0 => Ok(gearbox_api::Status::ok()),
                        1 => Ok(gearbox_api::Status::err(code::REFUSED, "expired session")),
                        _ => Err(CliError::error("transport failure")),
                    }
                }, |id| events.borrow_mut().push(("release", id)));
                let expected = if borrowed { vec![("send", 17)] } else { vec![("send", 17), ("release", 17)] };
                assert_eq!(*events.borrow(), expected);
                match outcome {
                    0 => assert!(result.unwrap().is_ok()),
                    1 => assert!(!result.unwrap().is_ok()),
                    _ => assert!(result.is_err()),
                }
            }
        }
    }
}

impl PressureSession {
    fn claimed(claim: ClaimResponse) -> Result<Self> {
        if claim.code != code::OK {
            return Err(CliError::new(
                crate::error::exit_for_wire_code(claim.code),
                Props::from_bytes(&claim.props).get("message")
                    .unwrap_or_else(|| format!("pressure session claim refused: code {}", claim.code)),
            ));
        }
        if claim.session == 0 {
            return Err(CliError::error("pressure claim returned an invalid zero session"));
        }
        Ok(Self::Claimed(claim.session))
    }

    fn borrowed(info: gearbox_api::SessionInfo, requester: &str) -> Result<Self> {
        if info.held == 0 || info.session == 0 {
            return Err(CliError::busy("no active driving session to reuse"));
        }
        if requester.is_empty() || info.holder() != requester {
            return Err(CliError::busy("active session belongs to another CLI identity; cannot reuse it"));
        }
        Ok(Self::Borrowed(info.session))
    }

    fn command(
        self,
        send: impl FnOnce(u64) -> Result<gearbox_api::Status>,
        release: impl FnOnce(u64),
    ) -> Result<gearbox_api::Status> {
        let session = match self { Self::Claimed(id) | Self::Borrowed(id) => id };
        let result = send(session);
        if matches!(self, Self::Claimed(_)) {
            release(session);
        }
        result
    }
}

fn set_tyre_pressure(ctx: &Ctx, machine: Option<String>, scope: &str, bar: f64, take: bool, reuse_session: bool) -> Result<()> {
    use gearbox_api::tyres::{from_props, targets};
    if !bar.is_finite() || bar <= 0.0 {
        return Err(CliError::usage("pressure must be finite positive gauge bar"));
    }
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    let mut sub = mc.state()?;
    let state = next_sample::<MachineState>(&mut sub, ctx.timeout)?
        .ok_or_else(|| CliError::timeout("no tyre telemetry before pressure edit"))?;
    let tyres = from_props(&state.props()).map_err(CliError::usage)?;
    let links = targets(&tyres, scope, bar).map_err(CliError::usage)?;
    let session = if reuse_session {
        PressureSession::borrowed(mc.session()?, &client.did())?
    } else {
        let claim = mc.claim(DEFAULT_HOLD_MS, take)?;
        if claim.code == code::BUSY {
            return Err(CliError::busy(format!("machine `{machine_id}` is held by {}; use --reuse-session for your active drive or --take to steal", claim.holder())));
        }
        PressureSession::claimed(claim)?
    };
    let response = session.command(|session| Ok(mc.command(&ControllerCommand {
        session, value: bar, element: 0, _pad: 0,
        props: Props::from_pairs(&[("tyre_pressure_scope", scope)]).into_bytes(),
    })?), |session| { let _ = mc.release(session); });
    check(response?, "set tyre pressure")?;
    let deadline = Instant::now() + ctx.timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() { return Err(CliError::timeout("pressure request queued but accepted targets were not confirmed")); }
        let Some(state) = next_sample::<MachineState>(&mut sub, remaining)? else {
            return Err(CliError::timeout("pressure request queued but accepted targets were not confirmed"));
        };
        let readings = from_props(&state.props()).map_err(CliError::error)?;
        if links.iter().all(|link| readings.iter().any(|t| &t.link == link && (t.target - bar).abs() < 1e-6)) {
            ctx.done(&machine_id, &format!("`{machine_id}` {scope}: accepted {bar} bar target on {} tyres (ramps while playing)", links.len()),
                || json!({"machine_id": machine_id, "scope": scope, "target_bar": bar, "links": links, "accepted": true}));
            return Ok(());
        }
    }
}

fn set_value(
    ctx: &Ctx,
    machine: Option<String>,
    link: &str,
    name: &str,
    value: f64,
    take: bool,
) -> Result<()> {
    let machine_id = ctx.machine_id(machine)?;
    let client = ctx.client()?;
    let mc = client.machine(&machine_id);
    if !mc.links()?.iter().any(|r| r.name() == link) {
        return Err(CliError::usage(format!(
            "machine `{machine_id}` has no link `{link}`; see `gearbox machine links {machine_id}`"
        )));
    }
    let res = mc.claim(DEFAULT_HOLD_MS, take)?;
    if res.code == code::BUSY {
        return Err(CliError::busy(format!(
            "machine `{machine_id}` is held by {}; add --take",
            res.holder()
        )));
    }
    let cmd = ControllerCommand {
        session: res.session,
        value,
        element: 0,
        _pad: 0,
        props: Props::from_pairs(&[("link", link), ("name", name)]).into_bytes(),
    };
    let status = mc.command(&cmd)?;
    let _ = mc.release(res.session);
    check(status, "set value")?;
    ctx.done(
        &machine_id,
        &format!("`{machine_id}` {link}.{name} = {value}"),
        || json!({ "machine_id": machine_id, "link": link, "name": name, "value": value }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{LegEnd, line_turn, parse_leg};
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn a_leg_reads_its_twist_its_end_its_line_and_its_commands() {
        let leg = parse_leg("forward=3 distance=12 offset=6 do=hitch_rear:position=0.1").unwrap();
        assert_eq!(
            (leg.forward, leg.turn, leg.end, leg.offset),
            (3.0, 0.0, LegEnd::Metres(12.0), Some(6.0))
        );
        assert_eq!(
            leg.commands,
            vec![(
                "hitch_rear".to_string(),
                vec![("position".to_string(), "0.1".to_string())]
            )]
        );
        assert!(parse_leg("forward=3").is_err(), "a leg needs an end");
    }

    #[test]
    fn a_straight_leg_steers_back_onto_its_line_either_way_along_it() {
        // Driving north, a metre right of the line: turn left.
        assert!(
            line_turn(
                [1.0, 10.0, 0.0],
                FRAC_PI_2,
                [0.0; 3],
                FRAC_PI_2,
                FRAC_PI_2,
                0.0,
                3.0
            ) > 0.0
        );
        // Driving back south toward the line six metres right, still west
        // of it: turn left, which heads east.
        assert!(
            line_turn(
                [1.0, 10.0, 0.0],
                -FRAC_PI_2,
                [0.0; 3],
                FRAC_PI_2,
                FRAC_PI_2 - PI,
                6.0,
                3.0
            ) > 0.0
        );
        // On the line and on course: no turn.
        assert!(
            line_turn(
                [6.0, 10.0, 0.0],
                -FRAC_PI_2,
                [0.0; 3],
                FRAC_PI_2,
                FRAC_PI_2 - PI,
                6.0,
                3.0
            )
            .abs()
                < 1.0e-9
        );
    }
}
