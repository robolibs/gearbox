//! Exercise a running host from the shell:
//!
//! ```text
//! cargo run -p gearbox-api --no-default-features --example probe -- [instance|did] [cmd...]
//!   info                         host info
//!   list                         scene objects
//!   machines                     machine refs
//!   load <id> <path> [x y z]     load a USD (category static)
//!   machine <id> <path> <ns> [x y z]   load a machine USD
//!   drive <ns> <v> <w> <secs>    claim, stream cmd_vel, release
//!   clear                        clear the scene
//!   pause | play | shutdown      clock
//! ```

use std::time::Duration;

use gearbox_api::{
    Client, IdentitySource, UsdLoad, category, clock_op, next_sample, object_kind, registry,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (target, rest) = match args.split_first() {
        Some((t, rest))
            if !matches!(
                t.as_str(),
                "info"
                    | "list"
                    | "machines"
                    | "load"
                    | "machine"
                    | "drive"
                    | "clear"
                    | "pause"
                    | "play"
                    | "shutdown"
            ) =>
        {
            (t.clone(), rest.to_vec())
        }
        _ => (String::new(), args.clone()),
    };
    let did = if target.starts_with("did:key:") {
        target
    } else {
        let entries = registry::list();
        let entry = if target.is_empty() {
            entries.first().cloned()
        } else {
            entries.into_iter().find(|e| e.name == target)
        };
        match entry {
            Some(e) => e.did,
            None => {
                eprintln!(
                    "no running gearbox instance found in {}",
                    registry::registry_dir().display()
                );
                std::process::exit(3);
            }
        }
    };
    let client = Client::connect(
        &did,
        IdentitySource::Name("gearbox-probe".into()),
        "gearbox-probe",
    )
    .expect("client agent");
    let info = client
        .wait_ready(Duration::from_secs(10))
        .expect("host did not answer");
    let cmd = rest.first().map(String::as_str).unwrap_or("info");
    match cmd {
        "info" => {
            let p = info.props();
            println!(
                "instance {} version {} did {}\n  uptime {} ms  paused {}  machines {}  objects {}",
                p.get("name").unwrap_or_default(),
                p.get("version").unwrap_or_default(),
                p.get("did").unwrap_or_default(),
                info.uptime_ms,
                info.paused,
                info.machine_count,
                info.object_count
            );
        }
        "list" => {
            for o in client.list(object_kind::ANY).expect("list") {
                let p = o.props();
                println!(
                    "{:8} {:20} ({:.2}, {:.2}, {:.2}) yaw {:.1}  {}",
                    object_kind::name(o.kind),
                    p.get("id").unwrap_or_default(),
                    o.x,
                    o.y,
                    o.z,
                    o.yaw_deg,
                    p.get("path").unwrap_or_default()
                );
            }
        }
        "machines" => {
            for m in client.machines().expect("machines") {
                let p = m.props();
                println!(
                    "{:12} kind {:10} {}",
                    m.machine_id(),
                    p.get("kind").unwrap_or_default(),
                    m.did()
                );
            }
        }
        "load" | "machine" => {
            let id = rest.get(1).expect("id");
            let path = rest.get(2).expect("path");
            let mut req = UsdLoad::new(id, path);
            let mut pos = 3;
            if cmd == "machine" {
                let ns = rest.get(3).expect("namespace");
                req = req.category(category::MACHINE).with_prop("namespace", ns);
                pos = 4;
            }
            if let (Some(x), Some(y), Some(z)) =
                (rest.get(pos), rest.get(pos + 1), rest.get(pos + 2))
            {
                req = req.at(x.parse().unwrap(), y.parse().unwrap(), z.parse().unwrap());
            }
            let status = client.load(&req).expect("load");
            println!("load {id}: code {} {}", status.code, status.message());
        }
        "drive" => {
            let ns = rest.get(1).expect("namespace");
            let v: f64 = rest.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let w: f64 = rest.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let secs: f64 = rest.get(4).and_then(|s| s.parse().ok()).unwrap_or(3.0);
            let mut m = client.machine(ns);
            let claim = m.claim(500, true).expect("claim");
            if claim.code != 0 {
                eprintln!(
                    "claim refused: code {} holder {}",
                    claim.code,
                    claim.holder()
                );
                std::process::exit(6);
            }
            let mut state = m.state().expect("state");
            let deadline = std::time::Instant::now() + Duration::from_secs_f64(secs);
            while std::time::Instant::now() < deadline {
                m.cmd_vel(claim.session, v, w).expect("cmd_vel");
                if let Ok(Some(s)) =
                    next_sample::<gearbox_api::MachineState>(&mut state, Duration::from_millis(50))
                {
                    let p = s.position();
                    println!(
                        "pos ({:.2}, {:.2}, {:.2}) heading {:.2} speed {:.2}",
                        p[0],
                        p[1],
                        p[2],
                        s.heading_rad,
                        s.linear_speed()
                    );
                }
            }
            m.cmd_vel(claim.session, 0.0, 0.0).expect("stop");
            m.release(claim.session).expect("release");
            println!("released");
        }
        "clear" => {
            let s = client.clear(0).expect("clear");
            println!("clear: code {}", s.code);
        }
        "pause" => println!(
            "paused {}",
            client.clock(clock_op::PAUSE).expect("clock").paused
        ),
        "play" => println!(
            "paused {}",
            client.clock(clock_op::PLAY).expect("clock").paused
        ),
        "shutdown" => {
            let _ = client.clock(clock_op::SHUTDOWN);
            println!("shutdown requested");
        }
        other => eprintln!("unknown command {other}"),
    }
}
