//! `gearbox machine`: inspect and drive machines through claim / cmd_vel /
//! release on each machine's own agent.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::wire::json as wire_json;
use gearbox_api::{
    ClaimResponse, ControllerCommand, MachineClient, MachineInfo, MachineState, Props, code,
    next_sample, pack,
};
use serde_json::json;

use super::{check, parse_duration};
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
    Info { ns: Option<String> },
    /// Stream the machine's state
    State {
        ns: Option<String>,
        /// Keep printing
        #[arg(long)]
        watch: bool,
        /// Prints per second while watching
        #[arg(long, default_value_t = 5.0)]
        rate: f64,
    },
    /// Drive with a constant twist for a while, then stop
    Move {
        ns: Option<String>,
        /// Forward speed in m/s (negative reverses)
        #[arg(long, default_value_t = 1.0)]
        forward: f64,
        /// Yaw rate in rad/s (positive turns left)
        #[arg(long, default_value_t = 0.0)]
        turn: f64,
        /// Duration such as 3s, 500ms, 1m
        #[arg(long = "for", default_value = "3s")]
        duration: String,
        /// Keep streaming until Ctrl-C
        #[arg(long)]
        hold: bool,
        /// Steal the session from its holder
        #[arg(long)]
        take: bool,
    },
    /// Send a zero twist and release
    Stop {
        ns: Option<String>,
        /// Steal the session from its holder
        #[arg(long)]
        take: bool,
    },
    /// Drive from the keyboard: WASD or arrows, space stops, q quits
    Drive {
        ns: Option<String>,
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
        /// Machine namespace (default: the selection)
        #[arg(long)]
        ns: Option<String>,
        /// Route to a controller of this attached slave
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        take: bool,
    },
    /// Controller table with types and interfaces
    Controllers { ns: Option<String> },
    /// The link tree: names, roles, parents, static offsets
    Links {
        ns: Option<String>,
        /// One row per link instead of an indented tree
        #[arg(long)]
        flat: bool,
    },
    /// Stream link poses: switches the machine's tf on, prints, switches it off
    Tf {
        ns: Option<String>,
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
        ns: Option<String>,
        #[arg(long)]
        take: bool,
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
    List { ns: Option<String> },
    /// Hitches and couplers of a machine, free or occupied
    Couplings { ns: Option<String> },
    /// Hang SLAVE on a hitch of the master
    Attach {
        /// The slave's namespace
        slave: String,
        /// Master namespace (default: the selection)
        #[arg(long)]
        ns: Option<String>,
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
        ns: Option<String>,
        #[arg(long)]
        take: bool,
    },
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::List => list(ctx),
        Cmd::Info { ns } => info(ctx, ns),
        Cmd::State { ns, watch, rate } => state(ctx, ns, watch, rate),
        Cmd::Move {
            ns,
            forward,
            turn,
            duration,
            hold,
            take,
        } => move_for(ctx, ns, forward, turn, &duration, hold, take),
        Cmd::Stop { ns, take } => stop(ctx, ns, take),
        Cmd::Drive {
            ns,
            speed,
            yaw,
            take,
        } => drive(ctx, ns, speed, yaw, take),
        Cmd::Cmd {
            ns,
            controller,
            pairs,
            tool,
            take,
        } => command(ctx, ns, &controller, &pairs, tool, take),
        Cmd::Controllers { ns } => controllers(ctx, ns),
        Cmd::Links { ns, flat } => links(ctx, ns, flat),
        Cmd::Tf {
            ns,
            link,
            rate,
            count,
        } => tf(ctx, ns, link, rate, count),
        Cmd::SetValue {
            link,
            name,
            value,
            ns,
            take,
        } => set_value(ctx, ns, &link, &name, value, take),
        Cmd::Tools { cmd } => tools(ctx, cmd),
    }
}

fn list(ctx: &Ctx) -> Result<()> {
    let client = ctx.client()?;
    let refs = client.machines()?;
    let mut rows = Vec::new();
    for r in &refs {
        let ns = r.namespace();
        let mc = client.machine(&ns);
        let info = mc.info().ok();
        let session = mc.session().ok();
        rows.push((r.clone(), info, session));
    }
    ctx.emit(
        || {
            json!(rows
                .iter()
                .map(|(r, info, session)| {
                    let mut v = json!({ "namespace": r.namespace(), "did": r.did() });
                    let p = r.props();
                    v["kind"] = json!(p.get("kind").unwrap_or_default());
                    v["machine_id"] = json!(p.get("machine_id").unwrap_or_default());
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
            let mut t = Table::new(&["NAMESPACE", "KIND", "CONTROLLERS", "HELD BY", "DID"]);
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
                    r.namespace(),
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

fn info(ctx: &Ctx, ns: Option<String>) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mc = client.machine(&ns);
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
                ("namespace", p.get("namespace").unwrap_or_default()),
                ("kind", p.get("kind").unwrap_or_default()),
                ("machine id", p.get("machine_id").unwrap_or_default()),
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

fn controllers(ctx: &Ctx, ns: Option<String>) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let info = ctx.client()?.machine(&ns).info()?;
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

fn state(ctx: &Ctx, ns: Option<String>, watch: bool, rate: f64) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mc = client.machine(&ns);
    let mut sub = mc.state()?;
    let period = Duration::from_secs_f64(1.0 / rate.max(0.1));
    let mut last_print = Instant::now() - period;
    loop {
        let Some(s) = next_sample::<MachineState>(&mut sub, ctx.timeout)? else {
            return Err(CliError::timeout(format!(
                "no state from machine `{ns}` within {:.1}s",
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
            println!("{}", state_line(&ns, &s));
        }
        if !watch {
            return Ok(());
        }
    }
}

fn state_line(ns: &str, s: &MachineState) -> String {
    let p = s.position();
    format!(
        "{ns}: pos ({:+.2}, {:+.2}, {:+.2}) heading {:+.2} rad speed {:.2} m/s yaw {:+.2} rad/s roll {:+.2} pitch {:+.2} session {}",
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
fn claim(ctx: &Ctx, mc: &MachineClient<'_>, ns: &str, take: bool) -> Result<ClaimResponse> {
    let info = mc.info()?;
    if info.controllers_with_command("cmd_vel").is_empty() {
        let rows = controller_rows(&info);
        let have: Vec<String> = rows
            .iter()
            .filter(|c| !c.2.is_empty())
            .map(|c| format!("{}={}", c.0, c.2))
            .collect();
        return Err(CliError::unsupported(format!(
            "machine `{ns}` has no cmd_vel controller; command interfaces: {}",
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
            "machine `{ns}` is held by {}; add --take to steal it",
            res.holder()
        )));
    }
    if res.code != code::OK {
        return Err(CliError::new(
            crate::error::exit_for_wire_code(res.code),
            format!(
                "claim on `{ns}` refused: {}",
                Props::from_bytes(&res.props)
                    .get("message")
                    .unwrap_or_else(|| format!("code {}", res.code))
            ),
        ));
    }
    Ok(res)
}

fn move_for(
    ctx: &Ctx,
    ns: Option<String>,
    forward: f64,
    turn: f64,
    duration: &str,
    hold: bool,
    take: bool,
) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mut mc = client.machine(&ns);
    let session = claim(ctx, &mc, &ns, take)?.session;
    let period = Duration::from_secs_f64(1.0 / STREAM_HZ);
    let dur = parse_duration(duration)?;
    let deadline = Instant::now() + dur;
    let mut sub = mc.state().ok();
    let mut last_state: Option<MachineState> = None;
    let mut last_print = Instant::now();
    let stop_flag = install_ctrlc();
    while (hold || Instant::now() < deadline)
        && !stop_flag.load(std::sync::atomic::Ordering::Relaxed)
    {
        check(mc.cmd_vel(session, forward, turn)?, "cmd_vel")?;
        if let Some(sub) = sub.as_mut()
            && let Ok(Some(s)) = next_sample::<MachineState>(sub, Duration::from_millis(5))
        {
            last_state = Some(s);
        }
        if !ctx.json
            && !ctx.quiet
            && last_print.elapsed() >= Duration::from_millis(500)
            && let Some(s) = &last_state
        {
            eprintln!("{}", state_line(&ns, s));
            last_print = Instant::now();
        }
        std::thread::sleep(period);
    }
    let _ = mc.cmd_vel(session, 0.0, 0.0);
    let _ = mc.release(session);
    let pos = last_state
        .as_ref()
        .map(|s| s.position())
        .unwrap_or([0.0; 3]);
    ctx.done(
        &ns,
        &format!(
            "drove `{ns}` forward {forward} m/s turn {turn} rad/s for {:.1}s; now at ({:.2}, {:.2}, {:.2}); released",
            dur.as_secs_f64(),
            pos[0],
            pos[1],
            pos[2]
        ),
        || json!({ "namespace": ns, "forward": forward, "turn": turn, "seconds": dur.as_secs_f64(), "x": pos[0], "y": pos[1], "z": pos[2] }),
    );
    Ok(())
}

fn stop(ctx: &Ctx, ns: Option<String>, take: bool) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mut mc = client.machine(&ns);
    let session = claim(ctx, &mc, &ns, take)?.session;
    check(mc.cmd_vel(session, 0.0, 0.0)?, "cmd_vel")?;
    check(mc.release(session)?, "release")?;
    ctx.done(
        &ns,
        &format!("stopped `{ns}` and released"),
        || json!({ "namespace": ns, "stopped": true }),
    );
    Ok(())
}

fn command(
    ctx: &Ctx,
    ns: Option<String>,
    controller: &str,
    pairs: &[String],
    tool: Option<String>,
    take: bool,
) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mc = client.machine(&ns);
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
            "machine `{ns}` is held by {}; add --take",
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
        &ns,
        &format!("sent {controller} command to `{ns}`"),
        || json!({ "namespace": ns, "controller": controller, "value": value, "element": element }),
    );
    Ok(())
}

fn drive(ctx: &Ctx, ns: Option<String>, speed: f64, yaw: f64, take: bool) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mut mc = client.machine(&ns);
    let session = claim(ctx, &mc, &ns, take)?.session;
    let mut sub = mc.state().ok();
    let raw = RawTerminal::enable()?;
    eprintln!("driving `{ns}`: W/S forward/back, A/D turn, space stop, Q quit\r");
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
    eprintln!("\nreleased `{ns}`");
    Ok(())
}

fn install_ctrlc() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    let flag = FLAG
        .get_or_init(|| {
            let flag = Arc::new(AtomicBool::new(false));
            extern "C" fn handler(_: libc::c_int) {
                if let Some(f) = FLAG.get() {
                    f.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            // SAFETY: installing a plain signal handler that only stores a flag.
            unsafe {
                libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
                libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
            }
            flag
        })
        .clone();
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    flag
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

fn links(ctx: &Ctx, ns: Option<String>, flat: bool) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mc = client.machine(&ns);
    let records = mc.links()?;
    if records.is_empty() {
        return Err(CliError::unsupported(format!(
            "machine `{ns}` reports no links"
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
                "namespace": ns,
                "derived": derived,
                "links": records
                    .iter()
                    .filter_map(|r| wire_json::env_to_json(&pack(r)).ok())
                    .collect::<Vec<_>>(),
            })
        },
        || {
            if derived {
                println!("{ns}: link tree derived from rigid bodies; author GearboxLinkAPI to make it explicit");
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
    ns: Option<String>,
    link: Option<String>,
    rate: f64,
    count: Option<usize>,
) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mc = client.machine(&ns);
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
                        "no link poses from `{ns}` within {:.1}s; is the clock running?",
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
        ToolsCmd::List { ns } => {
            let ns = ctx.machine_ns(ns)?;
            let records = ctx.client()?.machine(&ns).tools()?;
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
                        println!("nothing attached to `{ns}`");
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
        ToolsCmd::Couplings { ns } => {
            let ns = ctx.machine_ns(ns)?;
            let client = ctx.client()?;
            let mc = client.machine(&ns);
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
                        println!("`{ns}` has no couplings");
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
            ns,
            hitch,
            coupler,
            teleport,
            take,
        } => {
            let ns = ctx.machine_ns(ns)?;
            let client = ctx.client()?;
            let mc = client.machine(&ns);
            let session = session_for(ctx, &mc, &ns, take)?;
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
            check(status, &format!("attach {slave} to {ns}"))?;
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
                &format!("attached `{slave}` to `{ns}` via {h} / {c} ({k})"),
                || json!({ "master": ns, "slave": slave, "hitch": h, "coupler": c, "type": k }),
            );
            Ok(())
        }
        ToolsCmd::Detach { slave, ns, take } => {
            let ns = ctx.machine_ns(ns)?;
            let client = ctx.client()?;
            let mc = client.machine(&ns);
            let session = session_for(ctx, &mc, &ns, take)?;
            let status = mc.detach(session, &slave)?;
            if session != 0 {
                let _ = mc.release(session);
            }
            check(status, &format!("detach {slave} from {ns}"))?;
            ctx.done(
                &slave,
                &format!("detached `{slave}` from `{ns}`"),
                || json!({ "master": ns, "slave": slave }),
            );
            Ok(())
        }
    }
}

/// The session an attachment request must carry: none while the master is
/// free, ours after a claim (stolen with `--take`) while someone holds it.
fn session_for(ctx: &Ctx, mc: &MachineClient<'_>, ns: &str, take: bool) -> Result<u64> {
    let session = mc.session()?;
    if session.held == 0 {
        return Ok(0);
    }
    if !take {
        return Err(CliError::busy(format!(
            "machine `{ns}` is held by {}; add --take",
            session.holder()
        )));
    }
    let res = mc.claim(ctx.timeout.as_millis() as u32, true)?;
    if res.code != code::OK {
        return Err(CliError::new(
            crate::error::exit_for_wire_code(res.code),
            format!(
                "claim on `{ns}` refused: {}",
                Props::from_bytes(&res.props)
                    .get("message")
                    .unwrap_or_else(|| format!("code {}", res.code))
            ),
        ));
    }
    Ok(res.session)
}

fn set_value(
    ctx: &Ctx,
    ns: Option<String>,
    link: &str,
    name: &str,
    value: f64,
    take: bool,
) -> Result<()> {
    let ns = ctx.machine_ns(ns)?;
    let client = ctx.client()?;
    let mc = client.machine(&ns);
    if !mc.links()?.iter().any(|r| r.name() == link) {
        return Err(CliError::usage(format!(
            "machine `{ns}` has no link `{link}`; see `gearbox machine links {ns}`"
        )));
    }
    let res = mc.claim(DEFAULT_HOLD_MS, take)?;
    if res.code == code::BUSY {
        return Err(CliError::busy(format!(
            "machine `{ns}` is held by {}; add --take",
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
        &ns,
        &format!("`{ns}` {link}.{name} = {value}"),
        || json!({ "namespace": ns, "link": link, "name": name, "value": value }),
    );
    Ok(())
}
