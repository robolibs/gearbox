use super::*;
use gearbox_api::{DrawEntity, draw_style};

const TARGET: &str = "legacy-admission/target";
const NEIGHBOR: &str = "legacy-admission/neighbor";

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .expect("required legacy admission counter")
}

fn fixture(ns: &str, id: &str, offset: f32, rgba: [u8; 4]) -> DrawEntity {
    DrawEntity::points(
        ns,
        id,
        &[[-1.4 + offset, 25.0, 2.0], [1.4 + offset, 25.0, 2.0]],
    )
    .with_style(draw_style::SPLAT)
    .with_color(gearbox_api::wire::draw::rgba(
        rgba[0], rgba[1], rgba[2], rgba[3],
    ))
    .with_width(1.6)
    .with_glow(0.0)
    .with_pulse(-1.0)
}

fn send(client: &Client, entity: DrawEntity) -> Result<()> {
    let reply = client.draw(&entity)?;
    require(reply.is_ok(), &reply.message())
}

fn settled(report: &Report, points: u64) -> bool {
    number(&report.legacy, "points") == points
        && number(&report.legacy, "pages") == points / 2
        && number(&report.legacy, "ready") == points / 2
        && report.legacy["queued"][0].as_u64() == Some(points / 2)
        && report.legacy["queued"][1].as_u64() == Some(points / 2)
        && number(&report.legacy, "positions") == points * 16
        && number(&report.legacy, "attributes") == points * 16
        && number(&report.legacy, "staging_gpu_bytes") == 0
        && number(&report.legacy, "retiring_gpu_bytes") == 0
}

fn record(control: &Control, name: &str, report: &Report) -> Result<()> {
    require(
        number(&report.cpu, "committed") <= number(&report.cpu, "limit"),
        "CPU cap exceeded",
    )?;
    require(
        number(&report.legacy, "max_frame_upload") <= 4 * 1024 * 1024,
        "upload cap exceeded",
    )?;
    std::fs::write(
        control.root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}

pub(super) fn run(control: &mut Control, client: &Client) -> Result<()> {
    control.set("configure", true, 0)?;
    let baseline = control.wait("baseline", |r| {
        r.views.iter().all(Option::is_some) && settled(r, 0)
    })?;
    record(control, "baseline", &baseline)?;
    send(client, fixture(TARGET, "cloud", -2.5, [255, 24, 24, 255]))?;
    send(client, fixture(NEIGHBOR, "cloud", 2.5, [24, 24, 255, 255]))?;
    let initial = control.wait("initial", |r| settled(r, 4))?;
    record(control, "initial", &initial)?;
    control.capture("initial", 0, ["red+blue", "red+blue"])?;

    control.cpu_headroom = Some(0);
    control.set("configure", true, 0)?;
    let pressure = control.wait("pressure", |r| {
        number(&r.cpu, "held") > 0 && number(&r.cpu, "committed") == number(&r.cpu, "limit")
    })?;
    record(control, "pressure", &pressure)?;
    send(client, fixture(TARGET, "cloud", -2.5, [255, 24, 255, 255]))?;
    send(client, fixture(TARGET, "new", 0.0, [255, 24, 255, 255]))?;
    let root = control.root.clone();
    let denied = control.wait("denied", |r| {
        settled(r, 4)
            && number(&r.legacy, "frame") > number(&pressure.legacy, "frame") + 3
            && r.legacy["uploaded"] == pressure.legacy["uploaded"]
            && std::fs::read_to_string(root.join("viewer.log")).is_ok_and(|log| {
                ["cloud", "new"]
                    .iter()
                    .all(|id| log.contains(&format!("compact legacy {TARGET}/{id} deferred:")))
            })
    })?;
    record(control, "denied", &denied)?;
    control.capture("denied", 0, ["red+blue", "red+blue"])?;
    send(client, DrawEntity::delete(TARGET, "new"))?;
    control.cpu_headroom = None;
    control.set("configure", true, 0)?;
    let recovered = control.wait("recovered", |r| {
        settled(r, 4)
            && number(&r.cpu, "held") == 0
            && r.legacy["uploaded"][1].as_u64() > denied.legacy["uploaded"][1].as_u64()
    })?;
    require(
        recovered.legacy["uploaded"][0] == denied.legacy["uploaded"][0],
        "color retry rebuilt positions",
    )?;
    record(control, "recovered", &recovered)?;
    control.capture("recovered", 0, ["magenta+blue", "magenta+blue"])?;

    control.cpu_headroom = Some(0);
    control.set("configure", true, 0)?;
    control.wait("pressure-again", |r| {
        number(&r.cpu, "held") > 0 && number(&r.cpu, "committed") == number(&r.cpu, "limit")
    })?;
    send(client, fixture(TARGET, "cloud", -2.5, [255, 24, 24, 255]))?;
    send(client, DrawEntity::clear(TARGET))?;
    let partial = control.wait("namespace-cleared", |r| {
        settled(r, 2) && number(&r.cpu, "held") > 0
    })?;
    record(control, "namespace-cleared", &partial)?;
    control.capture("namespace-cleared", 0, ["blue", "blue"])?;
    send(client, DrawEntity::clear(NEIGHBOR))?;
    let deleted = control.wait("deleted", |r| {
        settled(r, 0)
            && number(&r.cpu, "held") > 0
            && number(&r.legacy, "gpu_bytes") == 0
            && number(&r.cpu, "allocated") == number(&baseline.cpu, "allocated")
    })?;
    record(control, "deleted", &deleted)?;
    control.capture("deleted", 0, ["none", "none"])?;
    control.cpu_headroom = None;
    control.set("configure", true, 0)?;
    let released = control.wait("released", |r| {
        settled(r, 0)
            && number(&r.cpu, "held") == 0
            && number(&r.cpu, "allocated") == number(&baseline.cpu, "allocated")
    })?;
    record(control, "released", &released)?;
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass", "scope":"compact legacy denied create/replace, preserved neighbors, automatic retry and zero-headroom deletion in two native views",
            "baseline_cpu":baseline.cpu, "deleted_cpu":deleted.cpu, "released_cpu":released.cpu,
        }))?,
    )?;
    Ok(())
}
