use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use gearbox_api::{
    Client, DrawEntity as E, IdentitySource, clock_op, draw_kind, draw_op, draw_style, registry,
};
use serde::{Deserialize, Serialize};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/legacy-lifecycle";
#[path = "draw_legacy_lifecycle/frames.rs"]
mod frames;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Metrics {
    frame: u64,
    points: u64,
    pages: u64,
    ready: u64,
    positions: u64,
    attributes: u64,
    uniforms: u64,
    gpu_bytes: u64,
    auxiliary_gpu_bytes: u64,
    staging_gpu_bytes: u64,
    retiring_gpu_bytes: u64,
    other_gpu_bytes: u64,
    charged_cpu_bytes: u64,
    uploaded: [u64; 3],
    max_frame_upload: u64,
}

#[derive(Deserialize)]
struct Report {
    legacy: Metrics,
}

fn fixture(ns: &str, id: &str) -> E {
    let points: Vec<_> = (0..16)
        .map(|i| {
            [
                (i % 4) as f32 * 0.5 - 0.75,
                (i / 4) as f32 * 0.5 - 0.75,
                2.0,
            ]
        })
        .collect();
    E::points(ns, id, &points)
        .with_style(draw_style::SPLAT)
        .with_color(0xffff00ff)
        .with_width(0.3)
        .with_glow(0.3)
        .with_pulse(-1.0)
}

fn send(client: &Client, entity: &E) -> Result<()> {
    entity.validate()?;
    let status = client.draw(entity)?;
    if !status.is_ok() {
        return Err(status.message().into());
    }
    Ok(())
}

fn layout(m: &Metrics, points: u64, pages: u64) -> bool {
    m.points == points
        && m.pages == pages
        && m.ready == pages
        && m.positions == points * 16
        && m.attributes == points * 16
        && m.uniforms == pages * 128
        && m.gpu_bytes == m.positions + m.attributes + m.uniforms + m.auxiliary_gpu_bytes
        && m.staging_gpu_bytes == 0 && m.retiring_gpu_bytes == 0 && m.other_gpu_bytes == 0
        && m.max_frame_upload <= 4 * 1024 * 1024
}

fn metrics(out: &Path, points: u64, pages: u64, after: u64) -> Result<Metrics> {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut previous = Metrics::default();
    let mut stable = 0;
    loop {
        if let Ok(bytes) = std::fs::read(out.join("report.json"))
            && let Ok(Report { legacy: value }) = serde_json::from_slice(&bytes)
        {
            if value.frame > after
                && value.frame > previous.frame
                && layout(&value, points, pages)
                && previous.uploaded == value.uploaded
                && previous.charged_cpu_bytes == value.charged_cpu_bytes
            {
                stable += 1;
            } else {
                stable = 0;
            }
            previous = value;
            if stable >= 3 {
                return Ok(previous);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "legacy metrics did not settle for {points}/{pages}: {previous:?}"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

struct Capture<'a> {
    instance: &'a str,
    bin: &'a str,
    out: &'a Path,
}

impl Capture<'_> {
    fn stage(&self, name: &str, points: u64, pages: u64, after: u64) -> Result<Metrics> {
        let m = metrics(self.out, points, pages, after)?;
        let status = Command::new(self.bin)
            .args([
                "--instance",
                self.instance,
                "--timeout",
                "30",
                "instance",
                "screenshot",
            ])
            .arg(self.out.join(format!("{name}.png")))
            .args(["--viewport", "--timeout", "30"])
            .status()?;
        if !status.success() {
            return Err("legacy lifecycle screenshot failed".into());
        }
        std::fs::write(
            self.out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&m)?,
        )?;
        println!("{}", serde_json::json!({"stage":name,"metrics":m}));
        Ok(m)
    }
}

fn unchanged(before: &Metrics, after: &Metrics, indices: &[usize]) -> Result<()> {
    if indices
        .iter()
        .any(|i| before.uploaded[*i] != after.uploaded[*i])
    {
        return Err(format!(
            "unexpected record upload: {:?} -> {:?}",
            before.uploaded, after.uploaded
        )
        .into());
    }
    Ok(())
}

fn changed(before: &Metrics, after: &Metrics, index: usize) -> Result<()> {
    if after.uploaded[index] <= before.uploaded[index] {
        return Err(format!("expected upload did not occur at buffer {index}").into());
    }
    Ok(())
}

fn listed(client: &Client, ns: &str, id: &str, count: usize) -> Result<()> {
    let entities = client.draw_list(ns)?;
    let entity = entities
        .iter()
        .find(|e| e.ns() == ns && e.id() == id)
        .ok_or("legacy listing lost entity")?;
    if entity.kind != draw_kind::POINTS
        || entity.props().get("points").as_deref() != Some(&count.to_string())
    {
        return Err("legacy listing changed identity or point count".into());
    }
    Ok(())
}

fn run(client: &Client, capture: &Capture) -> Result<()> {
    client.clock(clock_op::PAUSE)?;
    send(client, &E::clear(NS))?;
    let baseline = capture.stage("00-empty", 0, 0, 0)?;
    let mut e = fixture(NS, "cloud");
    send(client, &e)?;
    listed(client, NS, "cloud", 16)?;
    let initial = capture.stage("01-initial", 16, 1, baseline.frame)?;
    if initial.uploaded != [256, 256, 128] {
        return Err("initial legacy buffers were uploaded repeatedly".into());
    }

    e.px = 2.0;
    send(
        client,
        &E::update(NS, "cloud")
            .with_pose([2.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0])
            .updating(&["pose"]),
    )?;
    let posed = capture.stage("02-pose", 16, 1, initial.frame)?;
    unchanged(&initial, &posed, &[0, 1])?;
    changed(&initial, &posed, 2)?;

    e.color = 0xffffff00;
    send(
        client,
        &E::update(NS, "cloud")
            .with_color(e.color)
            .updating(&["color"]),
    )?;
    let recolored = capture.stage("03-color", 16, 1, posed.frame)?;
    unchanged(&posed, &recolored, &[0])?;
    changed(&posed, &recolored, 1)?;

    send(
        client,
        &E::update(NS, "cloud")
            .with_style(draw_style::PLAIN)
            .with_color(0x8cff00ff)
            .with_blur(0.8)
            .updating(&["style", "color", "props"]),
    )?;
    let glass = capture.stage("04-glass", 16, 1, recolored.frame)?;
    unchanged(&recolored, &glass, &[0])?;
    send(
        client,
        &E::update(NS, "cloud")
            .with_blur(0.25)
            .with_glow(0.5)
            .updating(&["props", "glow"]),
    )?;
    let effect = capture.stage("05-effect-only", 16, 1, glass.frame)?;
    unchanged(&glass, &effect, &[0, 1])?;
    changed(&glass, &effect, 2)?;

    e = fixture(NS, "cloud");
    send(client, &e.clone().labelled(&vec![""; 16]))?;
    let generic = capture.stage("06-label-fallback", 0, 0, effect.frame)?;
    listed(client, NS, "cloud", 16)?;
    send(client, &e.clone().with_style(draw_style::HOLO))?;
    let decorated = capture.stage("07-decorated-fallback", 0, 0, generic.frame)?;
    send(client, &e)?;
    let restored = capture.stage("08-compact-restored", 16, 1, decorated.frame)?;

    let changed_point = [-1.5, -0.75, 2.0];
    send(
        client,
        &E::patch(draw_kind::POINTS, NS, "cloud", 0).with_points(&[changed_point]),
    )?;
    let patched = capture.stage("09-patch", 16, 1, restored.frame)?;
    unchanged(&restored, &patched, &[1])?;
    changed(&restored, &patched, 0)?;
    let invalid = E::patch(draw_kind::POINTS, NS, "cloud", 0).with_points(&[[f32::NAN, 0.0, 2.0]]);
    if client.draw_raw(&invalid)?.is_ok() {
        return Err("non-finite patch was accepted".into());
    }
    let rejected = capture.stage("10-rejected-patch", 16, 1, patched.frame)?;
    unchanged(&patched, &rejected, &[0, 1, 2])?;

    let mut extend = E::points(
        NS,
        "cloud",
        &[
            [-1.5, 0.0, 2.0],
            [-1.5, 0.5, 2.0],
            [-1.5, 1.0, 2.0],
            [-1.5, 1.5, 2.0],
        ],
    )
    .with_prop("max_points", "18");
    extend.op = draw_op::EXTEND;
    send(client, &extend)?;
    listed(client, NS, "cloud", 18)?;
    let extended = capture.stage("11-extend", 18, 1, rejected.frame)?;
    send(client, &E::delete(NS, "cloud"))?;
    let deleted = capture.stage("12-delete", 0, 0, extended.frame)?;
    if !client.draw_list(NS)?.is_empty() {
        return Err("delete kept a legacy entity".into());
    }
    if deleted.charged_cpu_bytes != baseline.charged_cpu_bytes {
        return Err("delete did not restore charged CPU baseline".into());
    }

    let child = format!("{NS}/child");
    let sibling = format!("{NS}/children");
    send(client, &fixture(&child, "remove"))?;
    send(
        client,
        &fixture(&sibling, "keep").with_pose([2.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
    )?;
    let both = capture.stage("13-namespaces", 32, 2, deleted.frame)?;
    send(client, &E::clear(&child))?;
    listed(client, &sibling, "keep", 16)?;
    if !client.draw_list(&child)?.iter().all(|e| e.ns() != child) {
        return Err("scoped clear left child".into());
    }
    let scoped = capture.stage("14-scoped-clear", 16, 1, both.frame)?;
    send(client, &E::clear(NS))?;
    let cleared = capture.stage("15-clear", 0, 0, scoped.frame)?;

    send(client, &fixture(NS, "expires").with_lifetime(0.2))?;
    std::thread::sleep(Duration::from_millis(350));
    listed(client, NS, "expires", 16)?;
    let paused = capture.stage("16-ttl-paused", 16, 1, cleared.frame)?;
    client.clock(clock_op::PLAY)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !client.draw_list(NS)?.is_empty() {
        if Instant::now() >= deadline {
            return Err("legacy TTL did not expire".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    client.clock(clock_op::PAUSE)?;
    let expired = capture.stage("17-ttl-expired", 0, 0, paused.frame)?;
    if expired.charged_cpu_bytes != baseline.charged_cpu_bytes {
        return Err("TTL did not restore charged CPU baseline".into());
    }
    println!(
        "{}",
        serde_json::json!({"checkpoint":"retained-lifecycle","result":"pass","baseline":baseline,"final":expired})
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 || !args[0].starts_with("draw-vfx-legacy-life-") {
        return Err("provide owned INSTANCE OUTPUT BIN".into());
    }
    let entry = registry::find(&args[0]).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "legacy-lifecycle")?;
    let original = client.clock(clock_op::GET)?;
    let capture = Capture {
        instance: &args[0],
        out: Path::new(&args[1]),
        bin: &args[2],
    };
    let result = run(&client, &capture).and_then(|_| frames::verify(&client, &capture));
    let cleanup = send(&client, &E::clear(NS));
    let restore = client.clock(if original.paused != 0 {
        clock_op::PAUSE
    } else {
        clock_op::PLAY
    });
    result?;
    cleanup?;
    restore?;
    println!("{}", serde_json::json!({"result":"pass","scope":"legacy-lifecycle-and-frames"}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_fixture_and_generic_switch_validate() {
        let e = fixture(NS, "cloud");
        e.validate().unwrap();
        assert_eq!(e.point_count(), 16);
        assert!(e.labels.is_empty());
        let fallback = e.labelled(&vec![""; 16]);
        fallback.validate().unwrap();
        assert_ne!(fallback.flags & gearbox_api::draw_flag::LABELS, 0);
        assert!(!fallback.labels.is_empty());
    }

    #[test]
    fn accounting_oracle_rejects_unready_leaked_or_oversized_buffers() {
        let m = Metrics {
            points: 16,
            pages: 1,
            ready: 1,
            positions: 256,
            attributes: 256,
            uniforms: 128,
            gpu_bytes: 640,
            ..Default::default()
        };
        assert!(layout(&m, 16, 1));
        for invalid in [
            Metrics {
                ready: 0,
                ..m.clone()
            },
            Metrics {
                gpu_bytes: 641,
                ..m.clone()
            },
            Metrics {
                positions: 456 * 16,
                ..m.clone()
            },
            Metrics {
                max_frame_upload: 4 * 1024 * 1024 + 1,
                ..m.clone()
            },
        ] {
            assert!(!layout(&invalid, 16, 1));
        }
    }
}
