//! A deterministic catalogue sent through the live retained drawing API.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use gearbox_api::{
    Client, DrawEntity as E, Env, GridCells, IdentitySource, Instance, SceneEvent, colormap,
    draw_flag as flag, draw_op, draw_style as style, event_kind, interact_kind, line_mode,
    next_sample, panel_kind,
    peerbus::Subscriber,
    point_style, registry, shape_kind as shape,
    wire::{
        draw::color,
        draw_widgets::{self as w, Detection3d, DetectionStyle, Glyph, Place, Pose},
    },
};

#[path = "draw_gallery/variants.rs"]
mod variants;
#[path = "draw_gallery/world.rs"]
mod world;
#[path = "draw_gallery/exhibition.rs"]
mod exhibition;
#[path = "draw_gallery/focus.rs"]
mod focus;
#[path = "draw_gallery/review.rs"]
mod review;
#[path = "draw_gallery/lifecycle.rs"]
mod lifecycle;
#[path = "draw_gallery/sensor.rs"]
mod sensor;
#[path = "draw_gallery/data_lab.rs"]
mod data_lab;
#[path = "draw_gallery/legacy_points.rs"]
mod legacy_points;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const NS: &str = "vfx-gallery";
const SCENES: &[&str] = &[
    "hero",
    "geometry",
    "corner-review",
    "corner-helpers",
    "structure-language",
    "blur-review",
    "region-language",
    "sensors",
    "uncertainty",
    "fields",
    "measurements",
    "panels",
    "table-layout",
    "panel-content",
    "robot",
    "maps",
    "frames",
    "plain",
    "pick-point",
    "pick-pose",
    "controls",
    "dense",
    "survey",
    "spatial",
    "world-controls",
    "world-dense",
    "labels",
    "label-contrast",
    "annotation-layout",
    "halo",
    "styles",
    "exhibition",
    "review",
    "energy-review",
    "review-colors",
    "review-colors-light",
    "review-finishes",
    "review-system",
    "energy-colors",
    "fill-contract",
    "data-lab",
];

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "focus-camera") {
        let path = args.get(1).ok_or("focus-camera needs MANIFEST TARGET WIDTH HEIGHT")?;
        let manifest = serde_json::from_slice(&std::fs::read(path)?)?;
        let target = args.get(2).ok_or("focus target required")?;
        let width = args.get(3).ok_or("focus width required")?.parse()?;
        let height = args.get(4).ok_or("focus height required")?.parse()?;
        let fov = std::env::var("GEARBOX_FOV_DEG").ok().and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite()).unwrap_or(58.0).clamp(20.0, 100.0);
        println!("{}", focus::camera(&manifest, target, width, height, fov)?);
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "review-manifest") {
        println!("{}", review::manifest());
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "data-lab-manifest") {
        println!("{}", data_lab::manifest());
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "exhibition-manifest") {
        println!("{}", exhibition::manifest(args.get(1).map(String::as_str).unwrap_or("vfx_rover")));
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "world-contract") {
        println!("{}", world::contract());
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "composite-contract") {
        println!("{}", world::composite_contract());
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "helper-contract") {
        println!("{}", world::helper_contract());
        return Ok(());
    }
    let instance = args
        .first()
        .ok_or("usage: draw_gallery INSTANCE [SCENE] [MACHINE]")?;
    let scene = args.get(1).map(String::as_str).unwrap_or("hero");
    if !["all", "verify-interactions", "verify-lifecycle", "verify-sensor", "grid-probe", "stroke-probe", "legacy-point-probe", "drape-probe", "normals-probe", "exhibition-action"].contains(&scene) && !SCENES.contains(&scene) {
        return Err(format!("unknown scene {scene}; choose {} or all", SCENES.join(", ")).into());
    }
    let machine = args.get(2).map(String::as_str).unwrap_or("vfx_rover");
    let entry = registry::find(instance).ok_or("the named viewer is not running")?;
    let client = Client::connect(
        &entry.did,
        IdentitySource::Name("gearbox-cli".into()),
        "draw-gallery",
    )?;
    client.wait_ready(Duration::from_secs(15))?;
    if scene == "legacy-point-probe" {
        let generic: bool = args.get(2).ok_or("legacy-point-probe needs GENERIC")?.parse()?;
        let case = args.get(3).map(|value| value.parse::<usize>()).transpose()?;
        if case.is_some_and(|case| case >= 21) { return Err("case must be 0..20".into()); }
        send_draw(&client, legacy_points::background())?;
        send_draw(&client, E::clear(&format!("{NS}/legacy-point-probe")))?;
        for (index, entity) in legacy_points::fixture(generic).into_iter().enumerate() {
            if case.is_none_or(|case| case == index) { send_draw(&client, entity)?; }
        }
        if case == Some(20) {
            for entity in legacy_points::overlap(generic) { send_draw(&client, entity)?; }
        }
        return Ok(());
    }
    if scene == "normals-probe" {
        let entity = normals_probe(args.get(2).ok_or("normals-probe needs MODE")?)?;
        send_draw(&client, E::clear(&format!("{NS}/mesh-normals")))?;
        return send_draw(&client, entity);
    }
    if scene == "drape-probe" {
        let kind = args.get(2).ok_or("drape-probe needs KIND EXPLICIT")?;
        let explicit = args.get(3).ok_or("drape-probe needs EXPLICIT")?.parse()?;
        let entity = drape_probe(kind, explicit)?;
        send_draw(&client, E::clear(&format!("{NS}/implicit-drape")))?;
        return send_draw(&client, entity);
    }
    if scene == "stroke-probe" {
        let segments: usize = args.get(2).ok_or("stroke-probe needs SEGMENTS REVERSE")?.parse()?;
        let reverse: bool = args.get(3).ok_or("stroke-probe needs REVERSE")?.parse()?;
        if !(1..=512).contains(&segments) { return Err("segments must be 1..512".into()); }
        for e in stroke_probe(segments, reverse) { send_draw(&client, e)?; }
        return Ok(());
    }
    if scene == "grid-probe" {
        let frame = args.get(2).ok_or("grid-probe needs FRAME ANCHORED POSE_HEIGHT")?;
        let anchored: bool = args.get(3).ok_or("grid-probe needs ANCHORED POSE_HEIGHT")?.parse()?;
        let pose_height: bool = args.get(4).ok_or("grid-probe needs POSE_HEIGHT")?.parse()?;
        return send_draw(&client, grid_probe(frame, anchored, pose_height));
    }
    if scene == "verify-sensor" {
        let machine = args.get(2).ok_or("verify-sensor needs MACHINE OUTPUT")?;
        let out = args.get(3).ok_or("verify-sensor needs MACHINE OUTPUT")?;
        let link = args.get(4).map(String::as_str).unwrap_or("camera_link");
        return sensor::verify(&client, machine, link, Path::new(out));
    }
    if scene == "verify-lifecycle" {
        let bin = args.get(2).ok_or("verify-lifecycle needs BIN")?;
        return lifecycle::verify(&client, instance, Path::new(bin));
    }
    if scene == "exhibition-action" {
        let action = args.get(2).ok_or("exhibition-action needs handle, pick-point, pick-pose or step")?;
        let machine = args.get(3).map(String::as_str).unwrap_or("vfx_rover");
        for e in exhibition::action(action, machine)? { send_draw(&client, e)?; }
        return Ok(());
    }
    if scene == "verify-interactions" {
        let bin = args
            .get(2)
            .ok_or("verify-interactions needs BIN WINDOW_GEOMETRY")?;
        let geometry = args
            .get(3)
            .ok_or("verify-interactions needs BIN WINDOW_GEOMETRY")?;
        return verify_interactions(&client, instance, Path::new(bin), Path::new(geometry));
    }
    let cleared = client.draw_clear(NS)?;
    if !cleared.is_ok() {
        return Err(cleared.message().into());
    }
    let mut entities = catalogue(scene, machine);
    if let Ok(value) = std::env::var("GEARBOX_DRAW_BLUR") {
        let blur: f32 = value.parse()?;
        for entity in &mut entities { *entity = entity.clone().with_blur(blur); }
    }
    for e in &entities {
        e.validate()
            .map_err(|err| format!("{}/{}: {err}", e.ns(), e.id()))?;
        let status = client.draw(e)?;
        if !status.is_ok() {
            return Err(format!("{}/{}: {}", e.ns(), e.id(), status.message()).into());
        }
    }
    if scene == "maps" {
        let patch = E::grid_patch(
            &format!("{NS}/maps"),
            "u16",
            3,
            3,
            2,
            2,
            GridCells::U16(vec![950; 4]),
        );
        let status = client.draw(&patch)?;
        if !status.is_ok() {
            return Err(status.message().into());
        }
    }
    if scene == "hero" {
        let mut tail = Place::default().put(E::lines(
            &format!("{NS}/hero"),
            "trace",
            &[[6.5, -2.4, 0.04], [7.0, -2.7, 0.04]],
        ));
        tail.op = draw_op::EXTEND;
        let status = client.draw(&tail)?;
        if !status.is_ok() {
            return Err(status.message().into());
        }
    }
    println!(
        "{}: sent {} entities to {}; clear only with draw clear {NS}",
        scene,
        entities.len(),
        instance
    );
    if scene == "robot" || scene == "halo" || scene == "styles" || scene == "exhibition" || scene == "energy-review" || scene == "energy-colors" || scene == "all" {
        println!("Robot views require a loaded machine named {machine}.");
    }
    Ok(())
}

fn catalogue(scene: &str, machine: &str) -> Vec<E> {
    if scene == "all" || scene == "exhibition" { return exhibition::scene(machine); }
    let mut out = Vec::new();
    for name in SCENES {
        if scene != *name {
            continue;
        }
        let items = match *name {
            "hero" => hero(),
            "geometry" => geometry(),
            "corner-review" => corner_review(),
            "corner-helpers" => corner_helpers(),
            "structure-language" => review::structure_language(),
            "blur-review" => review::blur_review(machine),
            "region-language" => review::region_language(),
            "sensors" => sensors(),
            "uncertainty" => uncertainty(),
            "fields" => fields(),
            "measurements" => measurements(),
            "panels" => panels(),
            "table-layout" => table_layout(),
            "panel-content" => panel_content(),
            "robot" => robot(machine),
            "maps" => maps(),
            "frames" => frames(),
            "plain" => plain(),
            "pick-point" | "pick-pose" => picking(name),
            "controls" => variants::controls(),
            "dense" => variants::dense(),
            "survey" => world::survey(),
            "spatial" => world::spatial(),
            "world-controls" => world::controls(),
            "world-dense" => world::dense(),
            "labels" => world::labels(),
            "label-contrast" => world::label_contrast(),
            "annotation-layout" => annotation_layout(),
            "halo" => world::halo(machine),
            "styles" => variants::styles(machine),
            "review" => review::panels(),
            "energy-review" => review::energy(machine),
            "review-colors" => review::panel_colors(),
            "review-colors-light" => review::panel_colors_light(),
            "review-finishes" => review::panel_finishes(),
            "review-system" => review::system(machine),
            "energy-colors" => review::energy_colors(machine),
            "fill-contract" => fill_contract(),
            "data-lab" => data_lab::scene(),
            _ => unreachable!(),
        };
        out.extend(items);
    }
    out
}

fn fill_contract() -> Vec<E> {
    let ns = format!("{NS}/fill-contract");
    let cases = [
        (shape::BOX, style::HOLO, [1.1, 1.1, 1.4], 0xd0e0a050),
        (shape::ELLIPSOID, style::GHOST, [1.2, 1.0, 1.4], 0xd0c080e0),
        (shape::CYLINDER, style::SHIELD, [1.1, 1.1, 1.4], 0xd080d050),
        (shape::CONE, style::ALERT, [1.1, 1.1, 1.4], 0xd05090e0),
        (shape::CAPSULE, style::HOLO, [0.8, 0.8, 1.6], 0xd0d0a040),
        (shape::ELLIPSE, style::SHIELD, [1.3, 1.1, 0.6], 0xd0b0d040),
        (shape::RECT, style::FIELD, [1.3, 1.3, 0.1], 0xd0d080b0),
        (shape::FRUSTUM, style::SWEEP, [0.8, 0.6, 1.5], 0xd040c0e0),
        (shape::SECTOR, style::SWEEP, [1.0, 1.7, 0.15], 0xd0d09050),
    ];
    let mut out = Vec::new();
    for (i, (shape, style, size, color)) in cases.into_iter().enumerate() {
        let (x, y) = ((i % 3) as f32 * 7.0 - 7.0, (i / 3) as f32 * 5.0 - 5.0);
        out.push(E::text(&ns, &format!("heading-{i}"), &[[x, y + 1.7, 1.8]],
            &format!("{} / {}", shape::name(shape), style::name(style)))
            .with_color(0xffd2cdc2).with_width(-14.0));
        for (j, (fill, label)) in [(flag::FILL_SOLID, "solid"), (flag::FILL_WIRE, "wire"), (flag::FILL_BOTH, "both")].into_iter().enumerate() {
            let at = [x + j as f32 * 2.0 - 2.0, y, 1.0];
            out.push(E::shapes(&ns, &format!("{i}-{label}"), shape, &[Instance::at(at, size)])
                .with_style(style).with_fill(fill).with_color(color).with_color2(0xffd8e8f4)
                .with_width(-1.4).with_beam(-1.0).with_glow(0.35).with_pulse(0.65)
                .with_labels(&[label]).with_flags(flag::LABELS));
        }
    }
    out
}

fn window_points(text: &str) -> Result<[f64; 2]> {
    let dimension = |key: &str| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| name.trim() == key)
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && (400.0..=16384.0).contains(value))
            .ok_or_else(|| format!("missing or invalid native window {key}"))
    };
    Ok([dimension("width")?, dimension("height")?])
}

fn send_draw(client: &Client, entity: E) -> Result<()> {
    entity.validate()?;
    let status = client.draw(&entity)?;
    if !status.is_ok() {
        return Err(format!("{}/{}: {}", entity.ns(), entity.id(), status.message()).into());
    }
    Ok(())
}

fn inject(bin: &Path, instance: &str, script: &str) -> Result<()> {
    let output = Command::new(bin)
        .args([
            "--instance",
            instance,
            "--timeout",
            "30",
            "instance",
            "ui",
            script,
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "instance ui failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    println!("{}", serde_json::json!({"input_queued": script}));
    Ok(())
}

fn scoped_event(event: &SceneEvent, ns: &str, id: &str) -> bool {
    event.kind == event_kind::DRAW
        && event.props().get("ns").as_deref() == Some(ns)
        && event.id() == id
}

fn feedback(
    sub: &mut Subscriber<Env>,
    ns: &str,
    id: &str,
    deadline: Instant,
) -> Result<SceneEvent> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("timed out waiting for DRAW feedback from {ns}/{id}").into());
        }
        let Some(event) = next_sample::<SceneEvent>(sub, remaining)? else {
            return Err(format!("no DRAW feedback from {ns}/{id} before deadline").into());
        };
        if !scoped_event(&event, ns, id) {
            continue;
        }
        if ![event.x, event.y, event.z].iter().all(|v| v.is_finite()) {
            return Err(format!("nonfinite coordinates from {ns}/{id}").into());
        }
        let props: std::collections::BTreeMap<_, _> = event.props().iter().into_iter().collect();
        println!(
            "{}",
            serde_json::json!({"observed": {"kind": event.kind, "x": event.x, "y": event.y, "z": event.z, "props": props}})
        );
        return Ok(event);
    }
}

fn expected_event(sub: &mut Subscriber<Env>, ns: &str, id: &str, what: &str) -> Result<SceneEvent> {
    let event = feedback(sub, ns, id, Instant::now() + Duration::from_secs(15))?;
    if event.props().get("what").as_deref() != Some(what) {
        return Err(format!(
            "expected {what} from {ns}/{id}, received {:?}",
            event.props().get("what")
        )
        .into());
    }
    Ok(event)
}

fn event_distance(a: &SceneEvent, b: &SceneEvent) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn no_feedback(sub: &mut Subscriber<Env>, ns: &str, id: &str, duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let Some(event) = next_sample::<SceneEvent>(sub, deadline.saturating_duration_since(Instant::now()))? else { break };
        if scoped_event(&event, ns, id) {
            return Err(format!("unexpected {:?} from idle {ns}/{id}", event.props().get("what")).into());
        }
    }
    Ok(())
}

fn verify_interactions(client: &Client, instance: &str, bin: &Path, geometry: &Path) -> Result<()> {
    let [width, height] = window_points(&std::fs::read_to_string(geometry)?)?;
    let a = [width * 0.5, height * 0.55];
    let b = [a[0] + 120.0, a[1] - 70.0];
    let ns = format!("{NS}/acceptance-{}", std::process::id());
    let mut events = client.events()?;
    println!(
        "{}",
        serde_json::json!({"subscribed": {"instance": instance, "namespace": ns}, "window_points": [width, height], "layout": "fresh full viewport"})
    );
    let status = client.draw_clear(NS)?;
    if !status.is_ok() {
        return Err(status.message().into());
    }
    std::thread::sleep(Duration::from_secs(4));
    let result = (|| -> Result<()> {
        let mut picks = Vec::new();
        for (id, at) in [("point", a), ("heading-end", b)] {
            send_draw(
                client,
                E::interact(&ns, id, interact_kind::PICK_POINT).in_frame("world"),
            )?;
            inject(
                bin,
                instance,
                &format!("wait 250; click {} {}; wait 150", at[0], at[1]),
            )?;
            picks.push(expected_event(&mut events, &ns, id, "point")?);
        }
        let origin = &picks[0];
        let end = &picks[1];
        if event_distance(origin, end) < 0.05 {
            return Err("native ground picks did not resolve distinct points".into());
        }
        send_draw(
            client,
            E::interact(&ns, "pose", interact_kind::PICK_POSE).in_frame("world"),
        )?;
        inject(
            bin,
            instance,
            &format!(
                "wait 250; move {} {}; wait 100; down {} {}; wait 150; move {} {}; wait 150; up {} {}; wait 150",
                a[0], a[1], a[0], a[1], b[0], b[1], b[0], b[1],
            ),
        )?;
        let pose = expected_event(&mut events, &ns, "pose", "pose")?;
        let yaw = pose
            .props()
            .get_f64("yaw")
            .filter(|v| v.is_finite())
            .ok_or("pose feedback has no finite yaw")?;
        let expected = f64::from((end.y - origin.y).atan2(end.x - origin.x));
        let delta = (yaw - expected).sin().atan2((yaw - expected).cos()).abs();
        if event_distance(origin, &pose) > 0.02 || delta > 0.03 {
            return Err(format!(
                "pose differs from native point projection: origin error {}, yaw error {delta}",
                event_distance(origin, &pose)
            )
            .into());
        }
        send_draw(
            client,
            E::interact(&ns, "handle", interact_kind::HANDLE)
                .in_frame("world")
                .at_yaw([origin.x as f64, origin.y as f64, origin.z as f64], 0.0)
                .with_prop("controls", "move"),
        )?;
        inject(bin, instance, &format!("wait 250; move {} {}; wait 150", a[0], a[1]))?;
        no_feedback(&mut events, &ns, "handle", Duration::from_secs(1))?;
        inject(
            bin,
            instance,
            &format!(
                "wait 250; move {} {}; wait 150; down {} {}; wait 150; move {} {}; wait 150; move {} {}; wait 150; up {} {}; wait 150; move {} {}",
                a[0],
                a[1],
                a[0],
                a[1],
                a[0] + 40.0,
                a[1] + 20.0,
                a[0] + 80.0,
                a[1] + 40.0,
                a[0] + 80.0,
                a[1] + 40.0,
                b[0],
                b[1],
            ),
        )?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let (mut down, mut moved) = (false, false);
        loop {
            let event = feedback(&mut events, &ns, "handle", deadline)?;
            match event.props().get("what").as_deref() {
                Some("mouse_down") => down = true,
                Some("pose_update") if down => {
                    let norm = ["qw", "qx", "qy", "qz"]
                        .into_iter()
                        .map(|key| {
                            event
                                .props()
                                .get_f64(key)
                                .filter(|v| v.is_finite())
                                .ok_or("handle quaternion missing or nonfinite")
                        })
                        .collect::<std::result::Result<Vec<_>, _>>()?
                        .iter()
                        .map(|v| v * v)
                        .sum::<f64>();
                    if (norm - 1.0).abs() > 0.02 {
                        return Err("handle quaternion is not normalized".into());
                    }
                    moved |= event_distance(origin, &event) > 0.001;
                }
                Some("mouse_up") if down && moved => break,
                Some("mouse_up") => {
                    return Err("handle released without a changed pose_update".into());
                }
                other => return Err(format!("unexpected handle feedback {other:?}").into()),
            }
        }
        send_draw(client, E::delete(&ns, "handle"))?;
        send_draw(
            client,
            E::interact(&ns, "step", interact_kind::STEP).with_prop("text", ""),
        )?;
        inject(
            bin,
            instance,
            &format!(
                "wait 300; click {} {}; wait 150",
                width * 0.5 + 18.0,
                height - 54.0
            ),
        )?;
        let step = expected_event(&mut events, &ns, "step", "step")?;
        if step.props().get("full").as_deref() != Some("0") {
            return Err("step feedback did not preserve full=0".into());
        }
        for (id, kind) in [("cancel-point", interact_kind::PICK_POINT), ("cancel-pose", interact_kind::PICK_POSE)] {
            send_draw(client, E::interact(&ns, id, kind).in_frame("world"))?;
            let gesture = if kind == interact_kind::PICK_POSE {
                format!("wait 250; down {} {}; wait 100; move {} {}; key Escape; up {} {}; wait 150", a[0], a[1], b[0], b[1], b[0], b[1])
            } else { "wait 250; key Escape; wait 150".into() };
            inject(bin, instance, &gesture)?;
            expected_event(&mut events, &ns, id, "cancel")?;
        }
        std::thread::sleep(Duration::from_millis(500));
        send_draw(client, E::interact(&ns, "menu", interact_kind::HANDLE).in_frame("world")
            .at_yaw([origin.x as f64, origin.y as f64, origin.z as f64], 0.0)
            .with_prop("controls", "move").with_prop("menu", "Inspect,Reset")
            .with_color(color::MAGENTA).with_prop("panel_finish", "white"))?;
        inject(bin, instance, &format!("wait 250; click {} {} secondary; wait 300", a[0], a[1]))?;
        std::thread::sleep(Duration::from_secs(1));
        let screenshot = geometry.with_file_name("draw-menu.acceptance.png");
        let captured = Command::new(bin).args(["--instance", instance, "instance", "screenshot"])
            .arg(&screenshot).args(["--timeout", "30"]).output()?;
        if !captured.status.success() { return Err("native menu screenshot failed".into()); }
        inject(bin, instance, &format!("click {} {}; wait 150", a[0] + 50.0, a[1] + 16.0))?;
        let menu = expected_event(&mut events, &ns, "menu", "menu_select")?;
        if menu.props().get("menu").as_deref() != Some("Inspect") || event_distance(origin, &menu) > 0.02 {
            return Err("menu selection lost its label or entity pose".into());
        }
        inject(bin, instance, &format!("click {} {} secondary; wait 200; key Escape; wait 150; click {} {}; wait 150",
            a[0], a[1], a[0] + 50.0, a[1] + 16.0))?;
        std::thread::sleep(Duration::from_secs(1));
        let deadline = Instant::now() + Duration::from_millis(600);
        while let Some(event) = next_sample::<SceneEvent>(&mut events, deadline.saturating_duration_since(Instant::now()))? {
            if scoped_event(&event, &ns, "menu") && event.props().get("what").as_deref() == Some("menu_select") {
                return Err("Escape left the handle menu active".into());
            }
            if Instant::now() >= deadline { break; }
        }
        send_draw(client, E::delete(&ns, "menu"))?;
        println!(
            "{}",
            serde_json::json!({"accepted": ["point", "pose", "handle:no-hover-events", "handle:mouse_down,pose_update,mouse_up", "step", "cancel-point", "cancel-pose", "menu-select", "menu-dismiss"], "namespace": ns})
        );
        Ok(())
    })();
    if result.is_err() {
        let _ = inject(bin, instance, &format!("up {} {}; key Escape", a[0], a[1]));
    }
    let cleared = client.draw_clear(&ns)?;
    result?;
    if !cleared.is_ok() {
        return Err(cleared.message().into());
    }
    Ok(())
}

fn title(ns: &str, text: &str) -> E {
    Place::default()
        .put(E::text(ns, "title", &[[-7.0, 7.0, 2.5]], text))
        .with_width(-18.0)
}

fn curve(y: f32, z: f32) -> Vec<[f32; 3]> {
    (0..81)
        .map(|i| {
            let x = i as f32 * 0.15 - 6.0;
            [x, y + (x * 0.55).sin() * 1.1, z]
        })
        .collect()
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Vec<[f32; 3]> {
    vec![
        [x, y, 0.04],
        [x + w, y, 0.04],
        [x + w, y + h, 0.04],
        [x, y + h, 0.04],
    ]
}

fn hero() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/hero");
    vec![
        title(&ns, "ENERGY / path, history, prediction"),
        p.put(E::lines(&ns, "path", &curve(2.0, 0.05)))
            .with_width(0.9)
            .with_color(color::PLAN)
            .with_flow(1.2, 1.4),
        p.put(E::lines(&ns, "trace", &curve(-2.0, 0.04)))
            .with_color(color::CYAN)
            .with_beam(1.4)
            .with_pulse(0.4)
            .with_glow(0.28),
        p.put(E::lines(&ns, "horizon", &curve(5.0, 0.5)))
            .with_width(0.12)
            .with_color(color::PREDICTION),
        p.put(E::points(&ns, "goal", &[[6.0, 1.83, 0.06]]))
            .with_width(1.5)
            .with_beam(4.0)
            .with_color(color::GOAL),
        p.put(E::points(&ns, "lookahead", &[[0.0, 2.0, 0.35]]))
            .with_width(0.3)
            .with_color(color::LOOKAHEAD),
        p.put(E::lines(&ns, "plain-reference", &curve(-5.0, 0.04)))
            .with_style(style::PLAIN)
            .with_width(0.05)
            .with_color(color::WHITE),
        p.put(E::lines(&ns, "default-trace", &curve(-4.0, 0.04))),
        p.put(E::arrows(
            &ns,
            "arrow",
            &[([-5.0, -7.0, 0.2], [-1.0, -7.0, 0.2])],
        ))
        .with_width(0.12)
        .with_color(color::SENSOR),
        w::ruler(&p, &ns, "ruler", [1.0, -7.0, 0.1], [6.0, -7.0, 0.1], 0.5),
    ]
}

fn geometry() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/geometry");
    let mut out = vec![title(&ns, "GEOMETRY / nine shapes, fill, outline")];
    for k in 0..=shape::SECTOR {
        let x = (k % 3) as f32 * 4.5 - 4.5;
        let y = (k / 3) as f32 * 4.5 - 4.5;
        let size = match k {
            shape::FRUSTUM => [1.0, 0.7, 2.4],
            shape::SECTOR => [1.8, 1.8, 0.25],
            _ => [1.8, 1.4, 2.0],
        };
        let look = match k {
            shape::BOX | shape::CYLINDER | shape::CONE => style::HOLO,
            shape::ELLIPSOID | shape::CAPSULE => style::SHIELD,
            shape::FRUSTUM | shape::SECTOR => style::SWEEP,
            _ => style::DECAL,
        };
        let tint = [color::CYAN, color::MAGENTA, color::GREEN, color::ORANGE, color::WHITE][k as usize % 5];
        out.push(
            p.put(E::shapes(
                &ns,
                shape::name(k),
                k,
                &[Instance::at([x, y, 1.1], size)],
            ))
            .with_style(look)
            .with_color(tint)
            .with_fill(flag::FILL_BOTH)
            .with_spacing(0.5),
        );
        out.push(
            p.put(E::text(
                &ns,
                &format!("name-{k}"),
                &[[x, y, 2.5]],
                shape::name(k),
            ))
            .with_width(-13.0),
        );
    }
    out.push(
        p.put(E::triangles(
            &ns,
            "triangle",
            &[[-7.0, -3.0, 0.1], [-6.0, -1.0, 0.1], [-8.0, -1.0, 0.1]],
        ))
        .with_colors(&[color::PLAN, color::GOAL, color::PREDICTION]),
    );
    out.push(
        p.put(E::polygon(
            &ns,
            "holes",
            &[rect(7.0, -4.0, 4.0, 4.0), rect(8.0, -3.0, 2.0, 2.0)],
        ))
        .with_fill(flag::FILL_BOTH)
        .with_color(color::PREDICTION),
    );
    out.push(
        p.put(E::shapes(
            &ns,
            "alert",
            shape::BOX,
            &[Instance::at([8.0, 3.0, 0.5], [2.5, 2.0, 1.0])],
        ))
        .with_style(style::ALERT)
        .with_color(color::DANGER)
        .with_urgency(0.7),
    );
    out.extend(w::frame(
        &p,
        &ns,
        "frame",
        Pose::at(-8.0, 4.0, 0.0),
        Some([-8.0, 1.0, 0.0]),
        1.0,
        Some("ENU"),
    ));
    let asset = format!(
        "{}/../../tests/fixtures/draw_vfx/faceted.obj",
        env!("CARGO_MANIFEST_DIR")
    );
    out.push(
        p.put(E::mesh(&ns, "asset", &asset))
            .at_yaw([-8.0, -6.0, 0.0], 0.3)
            .with_style(style::HOLO)
            .with_color(color::SENSOR),
    );
    out
}

fn normals_probe(mode: &str) -> Result<E> {
    let asset = match mode {
        "flat" => "faceted.obj",
        "authored" => "normals-authored.obj",
        "smooth" => "normals-smooth.obj",
        "reference" => "normals-reference.stl",
        "concave" => "concave.obj",
        "concave-authored" => "concave-authored.obj",
        "concave-reference" => "concave-reference.obj",
        _ => return Err("unknown normals probe mode".into()),
    };
    let asset = format!("{}/../../tests/fixtures/draw_vfx/{asset}", env!("CARGO_MANIFEST_DIR"));
    Ok(E::mesh(&format!("{NS}/mesh-normals"), "sample", &asset).with_scale([2.0, 1.4, 2.6])
        .with_style(style::PLAIN).with_flags(flag::DRAPE | flag::SHADED).at_yaw([0.0, 0.0, 3.0], 0.2)
        .with_prop("drape_pose_height", "true")
        .with_color(color::MAGENTA).with_glow(-1.0).with_pulse(-1.0).with_flow(-1.0, 0.0))
}

fn drape_probe(kind: &str, explicit: bool) -> Result<E> {
    let ns = format!("{NS}/implicit-drape");
    let entity = match kind {
        "axes" => E::axes(&ns, "sample", 2.0).with_style(style::PLAIN).with_width(-3.0),
        "shape" => E::shapes(&ns, "sample", shape::BOX, &[]).with_scale([2.0; 3]).with_style(style::PLAIN),
        "mesh" => E::mesh(&ns, "sample", concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/draw_vfx/faceted.obj"))
            .with_scale([1.8; 3]).with_style(style::PLAIN),
        _ => return Err("drape probe kind must be axes, shape or mesh".into()),
    }.with_flags(flag::DRAPE).at_yaw([0.0, 0.0, 3.0], 0.0)
        .with_color(color::MAGENTA).with_glow(-1.0).with_pulse(-1.0).with_flow(-1.0, 0.0);
    Ok(if explicit { entity.with_instances(&[Instance::default()]) } else { entity })
}

fn grid_probe(frame: &str, anchored: bool, pose_height: bool) -> E {
    E::grid(&format!("{NS}/grid-cache"), "surface", 8, 8, 0.75, GridCells::U8(vec![100; 64]))
        .with_values(&[0.6; 64]).with_flags(flag::DRAPE).in_frame(frame)
        .at_yaw([-3.0, -3.0, 2.0], 0.0).with_colormap(colormap::COSTMAP, 0.0, 100.0).with_glow(-1.0)
        .with_prop("drape_anchor", if anchored { "true" } else { "false" })
        .with_prop("drape_pose_height", if pose_height { "true" } else { "false" })
}

fn stroke_probe(segments: usize, reverse: bool) -> Vec<E> {
    (0..4).map(|lane| {
        let x = -1.8 + lane as f32 * 1.2;
        let points: Vec<_> = (0..=segments).map(|i| {
            let t = i as f32 / segments as f32;
            let t = if reverse { 1.0 - t } else { t };
            [x, -12.0 + 24.0 * t, 1.4]
        }).collect();
        let mut e = E::lines(&format!("{NS}/stroke-probe"), &format!("lane-{lane}"), &points)
            .with_style(if lane % 2 == 0 { style::PLAIN } else { style::RIBBON_FLOW })
            .with_color(0xffff00ff).with_width(if lane == 0 { -6.0 } else { 0.18 })
            .with_glow(-1.0).with_pulse(-1.0).with_flow(-1.0, 0.75).with_fade(-1.0);
        if lane % 2 == 0 { e = e.with_flags(flag::DASHED); }
        if lane == 3 { e = e.with_flags(flag::FLAT); }
        if lane == 2 {
            let radii: Vec<_> = points.iter().map(|p| 0.06 + 0.08 * (p[1] + 12.0) / 24.0).collect();
            let colors: Vec<_> = points.iter().map(|p| {
                let alpha = (90.0 + 165.0 * (p[1] + 12.0) / 24.0).round() as u32;
                (alpha << 24) | 0x00ff00ff
            }).collect();
            e = e.with_radii(&radii).with_colors(&colors);
        }
        e
    }).collect()
}

fn sensors() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/sensors");
    let mut out = vec![title(&ns, "PERCEPTION / scans, splats, fields of view")];
    let points: Vec<_> = (0..1200)
        .map(|i| {
            let a = i as f32 * 2.399963;
            let r = (i as f32 / 1200.0).sqrt() * 2.6;
            [
                -4.0 + r * a.cos(),
                -3.0 + r * a.sin(),
                0.2 + 0.8 * (a * 0.1).sin().abs(),
            ]
        })
        .collect();
    let values: Vec<_> = points.iter().map(|p| p[2]).collect();
    out.push(w::cloud(
        &p,
        &ns,
        "cloud",
        &points,
        &values,
        colormap::VIRIDIS,
        -2.0,
        0.01,
    ));
    let ranges: Vec<_> = (0..181)
        .map(|i| 2.5 + (i as f32 * 0.08).sin() * 0.4)
        .collect();
    out.extend(w::scan(
        &p,
        &ns,
        "scan",
        Pose::at(3.0, -4.0, 0.1),
        &ranges,
        -1.57,
        0.01745,
        3.5,
    ));
    out.extend(w::range_cone(
        &p,
        &ns,
        "range",
        Pose::at(-5.0, 2.0, 0.3),
        0.9,
        0.2,
        3.5,
        2.5,
    ));
    out.push(w::camera_frustum(
        &p,
        &ns,
        "frustum",
        Pose::at(0.0, 2.0, 1.2),
        0.9,
        0.65,
        0.1,
        3.2,
    ));
    out.extend(w::detections_3d(
        &p,
        &ns,
        "detections",
        &[Detection3d {
            pose: Pose::at(5.0, 3.0, 0.9),
            size: [1.1, 0.8, 1.8],
            class: "person".into(),
            score: 0.94,
            track: Some(7),
            velocity: Some([0.7, 0.0, 0.0]),
            trail: vec![[3.5, 3.0, 0.1], [4.2, 3.0, 0.1]],
        }],
        &DetectionStyle::default(),
    ));
    out.extend(w::detections_2d(
        &p,
        &ns,
        "camera-box",
        Pose::at(-1.0, 6.0, 1.2),
        1.0,
        0.7,
        [640.0, 480.0],
        &[[320.0, 240.0, 180.0, 260.0]],
        &["person".into()],
        &[0.94],
        2.0,
    ));
    out
}

fn uncertainty() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/uncertainty");
    let mut out = vec![title(
        &ns,
        "UNCERTAINTY / estimates and forces",
    )];
    out.extend(
        w::covariance(
            &p,
            &ns,
            "cov2",
            Pose::at(-4.0, 3.0, 0.04),
            &[0.5, 0.2, 0.2, 0.3],
            &[1.0, 2.0, 3.0],
            color::PLAN,
        )
        .unwrap(),
    );
    out.extend(
        w::covariance(
            &p,
            &ns,
            "cov3",
            Pose::at(3.5, 3.0, 1.5),
            &[0.7, 0.1, 0.0, 0.1, 0.4, 0.0, 0.0, 0.0, 0.3],
            &[1.0, 2.0],
            color::OBSTACLE,
        )
        .unwrap(),
    );
    let poses: Vec<_> = (0..50)
        .map(|i| {
            Pose::yawed(
                -4.0 + (i % 10) as f64 * 0.5,
                -3.0 + (i / 10) as f64 * 0.5,
                0.06,
                i as f64 * 0.15,
            )
        })
        .collect();
    let weights: Vec<_> = (0..50).map(|i| (i + 1) as f32 / 50.0).collect();
    out.push(w::particles(
        &p,
        &ns,
        "particles",
        &poses,
        &weights,
        0.15,
        0.4,
        Glyph::Chevron,
    ));
    let strips: Vec<_> = (0..24)
        .map(|k| {
            (0..40)
                .map(|i| {
                    [
                        1.0 + i as f32 * 0.12,
                        -4.0 + k as f32 * 0.008 * (i as f32 * 0.12).powi(2),
                        0.06,
                    ]
                })
                .collect()
        })
        .collect();
    let costs: Vec<_> = (0..24).map(|i| (i as f32 - 10.0).abs()).collect();
    out.extend(w::rollouts(
        &p,
        &ns,
        "rollouts",
        &strips,
        &costs,
        &[false; 24],
        1,
    ));
    out.push(w::band(&p, &ns, "band", &curve(-6.0, 0.4), &[0.08]));
    out.extend(w::wrench(
        &p,
        &ns,
        "wrench",
        Pose::at(-7.0, -1.0, 0.5),
        [8.0, 4.0, 2.0],
        [0.0, 0.0, 5.0],
        0.15,
    ));
    out.extend(w::contact(
        &p,
        &ns,
        "contact",
        [7.0, -3.0, 0.1],
        [0.0, 0.0, 1.0],
        8.0,
        0.15,
    ));
    out
}

fn fields() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/fields");
    let outer = rect(-7.0, -6.0, 14.0, 12.0);
    let inner = rect(-5.5, -4.5, 11.0, 9.0);
    let paths: Vec<_> = (0..4)
        .map(|i| {
            vec![
                [-5.0, i as f32 * 2.0 - 3.0, 0.07],
                [5.0, i as f32 * 2.0 - 3.0, 0.07],
            ]
        })
        .collect();
    let mut out = vec![title(&ns, "FIELD / boundaries, swaths, coverage")];
    out.extend(w::field(
        &p,
        &ns,
        "boundary",
        &outer,
        &[],
        color::PREDICTION,
    ));
    out.push(w::headland(&p, &ns, "headland", &outer, &inner));
    out.extend(w::swaths(&p, &ns, "swaths", &paths, 0.9, 1));
    out.push(w::turns(
        &p,
        &ns,
        "turn",
        &[vec![
            [5.0, -3.0, 0.08],
            [6.0, -2.5, 0.08],
            [6.0, -1.5, 0.08],
            [5.0, -1.0, 0.08],
        ]],
        0.7,
    ));
    out.extend(w::ab_line(
        &p,
        &ns,
        "ab",
        [-5.0, -5.0, 0.06],
        [5.0, -5.0, 0.06],
        0.6,
    ));
    out.push(
        w::implement(&p, &ns, "implement", 4.0, 0.8, &[true, true, false, true])
            .at_yaw([0.0, 5.0, 0.1], 0.0),
    );
    out.push(w::zone(
        &p,
        &ns,
        "stop-zone",
        w::Zone::Stop,
        &[rect(-9.0, -3.0, 1.5, 3.0)],
    ));
    let counts: Vec<_> = (0..256).map(|i| ((i % 16) / 4 + 1) as u8).collect();
    out.push(w::coverage(
        &p,
        &ns,
        "coverage",
        16,
        16,
        0.2,
        Pose::at(8.0, 0.0, 0.04),
        &counts,
    ));
    out
}

fn measurements() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/measurements");
    let mut out = vec![title(&ns, "MEASUREMENTS / spatial annotations")];
    out.push(w::ruler(&p, &ns, "distance", [-7.0, 3.0, 0.1], [-3.5, 3.0, 0.1], 0.5)
        .with_color(color::ORANGE).with_prop("label_icon", "measurement"));
    out.extend(w::angle_colored(&p, &ns, "angle", [-0.8, 1.8, 0.1], 0.1, 1.4, 2.0, color::MAGENTA));
    out.extend(w::compass_colored(&p, &ns, "compass", [5.0, 2.5, 0.1], 1.3, color::CYAN));
    out.push(w::scale_bar(&p, &ns, "scale", [-7.0, -3.5, 0.1], 4.0).with_color(color::WHITE));
    out.extend(w::callout(&p, &ns, "callout", [-0.5, -3.0, 0.1], [1.0, 0.6, 0.9], "Inspection point", color::GREEN));
    out.extend(w::frame(&p, &ns, "frame", Pose::yawed(5.2, -3.0, 0.1, 0.4), Some([3.8, -3.0, 0.1]), 1.1, Some("Tool frame")));
    out
}

fn corner_helpers() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/corner-helpers");
    let mut out = vec![title(&ns, "STRUCTURAL HELPERS / shared corners, supplied data")];
    out.extend(w::clearance_box(&p, &ns, "clearance", Pose::yawed(-4.5, 3.0, 1.3, 0.15), [2.0; 3], color::ORANGE, "Supplied clearance"));
    out.extend(w::field_volume(&p, &ns, "field", Pose::yawed(0.0, 3.0, 1.3, 0.15), [2.0; 3], 0.5, color::GREEN));
    for (id, x, y, edge, class) in [("solid", 4.5, 3.0, false, "car"), ("wire", 0.0, -3.0, true, "person")] {
        out.extend(w::detections_3d(&p, &ns, id, &[Detection3d {
            pose: Pose::yawed(x, y, 1.3, 0.15), size: [2.0; 3], class: class.into(), score: 0.82,
            track: Some(17), velocity: None, trail: Vec::new(),
        }], &DetectionStyle { only_edge: edge, line_width: -1.0, ..Default::default() }));
    }
    out.extend(w::detections_2d(&p, &ns, "image", Pose::yawed(-4.5, -4.5, 1.5, std::f64::consts::FRAC_PI_2),
        1.0, 0.8, [640.0, 480.0], &[[320.0, 240.0, 520.0, 390.0]], &["cyclist".into()], &[0.94], 2.0));
    out.extend(w::waypoints_colored(&p, &ns, "route", &[[3.4, -3.5, 0.15], [4.5, -2.7, 0.15], [5.6, -3.0, 0.15]], 1, color::MAGENTA));
    for (id, x, y, label) in [("clearance", -4.5, 3.0, "CLEARANCE / corners"), ("field", 0.0, 3.0, "FIELD / major grid"),
        ("solid", 4.5, 3.0, "3D DETECTION / glass"), ("image", -4.5, -3.0, "2D DETECTION / image plane"),
        ("wire", 0.0, -3.0, "3D DETECTION / wire"), ("route", 4.5, -3.0, "WAYPOINT / selected")] {
        out.push(p.put(E::text(&ns, &format!("caption-{id}"), &[[x, y + 1.3, 2.8]], label)).with_width(-12.0));
    }
    out
}

fn corner_review() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/corner-review");
    let mut out = vec![title(&ns, "STRUCTURE / bold corners, fine edges")];
    for (column, (look, tint)) in [(style::HOLO, color::CYAN), (style::GHOST, color::ORANGE), (style::SHIELD, color::MAGENTA)].into_iter().enumerate() {
        for (row, (kind, fill, label)) in [(shape::BOX, flag::FILL_BOTH, "BOX / outline + glass"),
            (shape::BOX, flag::FILL_WIRE, "BOX / wire"), (shape::RECT, flag::FILL_BOTH, "RECT / outline + glass")].into_iter().enumerate() {
            let x = (column as f32 - 1.0) * 3.4;
            let y = (row as f32 - 1.0) * 3.5;
            let id = format!("{column}-{row}");
            out.push(p.put(E::shapes(&ns, &id, kind, &[Instance::at([x, y, 1.4], [1.8, 1.8, 1.8])]))
                .with_style(look).with_fill(fill).with_color(tint).with_beam(-1.0));
            out.push(p.put(E::text(&ns, &format!("label-{id}"), &[[x, y - 1.3, 0.15]], &format!("{} / {label}", style::name(look))))
                .with_width(-12.0));
        }
    }
    out
}

fn annotation_layout() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/annotation-layout");
    let mut out = vec![
        title(&ns, "ANNOTATIONS / shared label and panel layout"),
        p.put(E::text(&ns, "fixed", &[[0.0, 0.0, 3.0]], "WORLD ORIGIN")).with_width(-16.0),
        w::gauge(&p, &ns, "range", [0.0, 0.0, 3.0], 0.65, 0.0, 1.0, "RANGE / fixture", "m")
            .with_color(color::MAGENTA).with_prop("panel_finish", "white"),
        w::chip(&p, &ns, "state", [0.1, 0.1, 3.0], "ROVER / fixture", color::ORANGE)
            .with_prop("panel_finish", "black"),
    ];
    for (i, (name, icon, tint)) in [("Rover alpha", "robot", color::ORANGE), ("Depth camera", "sensor", color::CYAN), ("Map frame", "frame", color::GREEN)].into_iter().enumerate() {
        out.push(p.put(E::points(&ns, &format!("label-{i}"), &[[i as f32 * 0.08, 0.0, 3.0]]))
            .with_style(style::POINT_GLOW).with_width(0.06).with_color(tint).labelled(&[name])
            .with_prop("label_icon", icon).with_prop("label_category", "REFERENCE")
            .with_prop("label_detail", "Supplied pose / shared anchor")
            .with_prop("label_selected", if i == 0 { "true" } else { "false" }));
    }
    out
}

fn table_layout() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/table-layout");
    let rows = [
        ("Source".into(), "Deterministic fixture".into()),
        ("Grid".into(), "0–100 / unknown 255".into()),
        ("Velocity along local X".into(), "−1234.50 mm/s".into()),
        ("Long identifier".into(), "sensor_0123456789_abcdefghijklmnopqrstuvwxyz".into()),
        ("Unicode".into(), "α β / supplied text".into()),
    ];
    let mut out = vec![title(&ns, "TABLE LAYOUT / complete supplied values")];
    for (i, (finish, tint)) in [("color", color::GREEN), ("white", color::MAGENTA), ("black", color::ORANGE)].into_iter().enumerate() {
        out.push(w::table(&p, &ns, finish, [-5.0 + i as f32 * 5.0, 1.5, 1.0], &rows, &format!("Inspection / {finish}"))
            .with_color(tint).with_prop("panel_finish", finish));
    }
    out.push(w::panel(&p, &ns, "numeric", [0.0, -4.5, 0.7], panel_kind::TABLE, &[-1234.5, 0.0, 0.65],
        &["Velocity along local X".into(), "Stopped".into(), "Tracking error".into()], "Numeric fixture")
        .with_color(color::CYAN).with_prop("unit", "mm/s").with_prop("panel_finish", "white"));
    out
}

fn panel_content() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/panel-content");
    let mut out = vec![title(&ns, "PANEL CONTENT / complete supplied titles, labels and units")];
    for (i, kind) in (0..=panel_kind::LEGEND).enumerate() {
        let (finish, tint) = [("white", color::MAGENTA), ("black", color::ORANGE), ("color", color::GREEN)][i % 3];
        let at = [-6.0 + (i % 3) as f32 * 6.0, 3.5 - (i / 3) as f32 * 7.0, 1.0];
        out.push(w::panel(&p, &ns, &format!("kind-{kind}"), at, kind, &[-1234.5, 0.65, 9876.0],
            &["Forward velocity".into(), "Tracking error".into(), "Reference velocity".into()],
            "Local motion / measured longitudinal velocity")
            .with_color(tint).with_prop("panel_finish", finish).with_prop("unit", "millimetres / second")
            .with_prop("text", "Waiting for a valid localization estimate / α β")
            .with_prop("rows", "Coordinate frame=machine / odometry")
            .with_prop("min", "-2000").with_prop("max", "10000")
            .with_prop("world_anchor", "true").with_colormap(colormap::VIRIDIS, -2000.0, 10000.0));
    }
    out
}

fn panels() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/panels");
    let samples: Vec<_> = (0..48)
        .map(|i| 0.5 + (i as f32 * 0.19).sin() * 0.3)
        .collect();
    let mut out = vec![
        title(&ns, "INSTRUMENTS / world-anchored scalar data"),
        w::panel(
            &p,
            &ns,
            "spark",
            [-4.0, 2.0, 2.0],
            panel_kind::SPARKLINE,
            &samples,
            &[],
            "Tracking error",
        ),
        w::panel(
            &p,
            &ns,
            "bars",
            [3.5, 2.0, 2.0],
            panel_kind::BARS,
            &[0.7, 0.4, 0.9],
            &["A".into(), "B".into(), "C".into()],
            "Channels",
        ),
        w::gauge(
            &p,
            &ns,
            "gauge",
            [-4.0, -3.0, 1.0],
            72.0,
            0.0,
            100.0,
            "Battery",
            "%",
        ),
        w::chip(
            &p,
            &ns,
            "chip",
            [0.0, -2.0, 1.5],
            "AUTONOMOUS",
            color::PREDICTION,
        ),
        w::table(
            &p,
            &ns,
            "table",
            [4.0, -3.0, 1.0],
            &[
                ("Speed".into(), "1.2 m/s".into()),
                ("Mode".into(), "Tracking".into()),
            ],
            "Vehicle",
        ),
        w::legend(&ns, "legend", colormap::VIRIDIS, 0.0, 1.0, "Confidence", ""),
        p.put(E::interact(&ns, "handle", interact_kind::HANDLE))
            .at_yaw([0.0, 4.0, 0.7], 0.0)
            .with_prop("controls", "move,rotate")
            .with_prop("menu", "Inspect,Reset"),
        E::interact(&ns, "step", interact_kind::STEP).with_prop("text", "Continue inspection"),
    ];
    let cells: Vec<_> = (0..576)
        .map(|i| {
            if i % 29 == 0 {
                255
            } else {
                (((i % 24) as f32 * 0.3).sin().abs() * 100.0) as u8
            }
        })
        .collect();
    out.push(
        p.put(E::grid(&ns, "heat", 24, 24, 0.22, GridCells::U8(cells)))
            .at_yaw([-2.5, -2.5, 0.04], 0.0)
            .with_colormap(colormap::VIRIDIS, 0.0, 100.0)
            .with_prop("unknown", "255")
            .with_spacing(20.0),
    );
    out.extend(w::callout(
        &p,
        &ns,
        "callout",
        [0.0, 0.0, 0.2],
        [0.0, 0.0, 2.2],
        "Scalar field",
        color::PLAN,
    ));
    out.extend(w::compass(&p, &ns, "compass", [7.0, 5.0, 0.05], 2.0));
    out.extend(w::angle(&p, &ns, "angle", [-7.0, -5.0, 0.1], 0.0, 1.0, 1.3));
    out
}

fn robot(machine: &str) -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/robot");
    let poses: Vec<_> = (0..5)
        .map(|i| Pose::yawed(i as f64 * 2.0 - 4.0, -3.0, 0.0, i as f64 * 0.15))
        .collect();
    let mut out = vec![title(&ns, "ROBOT / ghost trail and attached instruments")];
    out.push(w::ghost_trail(
        &p,
        &ns,
        "ghosts",
        machine,
        &poses,
        &[],
        &[],
        1,
        color::PLAN,
    ));
    out.push(w::badge(
        &ns,
        "badge",
        machine,
        "VFX TEST VEHICLE",
        color::PREDICTION,
        2.0,
    ));
    out.push(w::tf_tree(&ns, "tf", machine));
    out.push(
        E::lines(&ns, "footprint", &rect(-1.2, -0.7, 2.4, 1.4))
            .with_sub(line_mode::LOOP)
            .in_frame(&format!("{machine}/base_link"))
            .with_style(style::DECAL)
            .with_color(color::PLAN),
    );
    out.extend(w::waypoints(
        &p,
        &ns,
        "waypoints",
        &[[-4.0, 3.0, 0.1], [0.0, 3.0, 0.1], [4.0, 3.0, 0.1]],
        1,
    ));
    out.extend(w::graph(
        &p,
        &ns,
        "graph",
        &[[-5.0, 5.0, 0.1], [-2.0, 6.0, 0.1], [2.0, 6.0, 0.1]],
        &[(0, 1), (1, 2)],
        &[],
    ));
    out.push(w::voxels(
        &p,
        &ns,
        "voxels",
        &[[6.0, 3.0, 0.5], [6.7, 3.0, 0.5], [6.0, 3.7, 0.5]],
        0.6,
        &[0.1, 0.5, 0.9],
        colormap::VIRIDIS,
        true,
    ));
    out
}

fn maps() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/maps");
    let scalar: Vec<_> = (0..144)
        .map(|i| ((i % 12) as f32 / 11.0 + (i / 12) as f32 / 11.0) * 0.5)
        .collect();
    let mut out = vec![title(
        &ns,
        "MAPS / scalar and categorical data",
    )];
    for (id, x, cells) in [
        (
            "u8",
            -7.0,
            GridCells::U8(scalar.iter().map(|v| (v * 100.0) as u8).collect()),
        ),
        (
            "u16",
            -1.0,
            GridCells::U16(scalar.iter().map(|v| (v * 1000.0) as u16).collect()),
        ),
        ("f32", 5.0, GridCells::F32(scalar.clone())),
    ] {
        let max = if id == "u8" {
            100.0
        } else if id == "u16" {
            1000.0
        } else {
            1.0
        };
        let mut e = p
            .put(E::grid(&ns, id, 12, 12, 0.35, cells))
            .at_yaw([x, 0.0, 0.05], 0.0)
            .with_colormap(colormap::VIRIDIS, 0.0, max)
            .with_spacing(max * 0.2);
        if id == "f32" {
            e = e.with_values(&scalar.iter().map(|v| v * 1.5).collect::<Vec<_>>());
        }
        out.push(e);
        out.push(
            p.put(E::text(
                &ns,
                &format!("label-{id}"),
                &[[x as f32 + 2.0, 4.5, 2.0]],
                id,
            ))
            .with_width(-15.0),
        );
    }
    let cells: Vec<_> = (0..144)
        .map(|i| {
            if i % 13 == 0 {
                255
            } else if i % 12 < 4 {
                0
            } else {
                ((i % 12) as u8 * 10).min(100)
            }
        })
        .collect();
    out.push(
        p.put(E::grid(
            &ns,
            "costmap",
            12,
            12,
            0.35,
            GridCells::U8(cells.clone()),
        ))
        .at_yaw([-6.0, -6.0, 0.05], 0.0)
        .with_colormap(colormap::COSTMAP, 0.0, 100.0)
        .with_prop("unknown", "255")
        .with_spacing(20.0),
    );
    out.push(
        p.put(E::grid(
            &ns,
            "occupancy",
            12,
            12,
            0.35,
            GridCells::U8(cells),
        ))
        .at_yaw([1.0, -6.0, 0.05], 0.0)
        .with_colormap(colormap::MAP, 0.0, 100.0),
    );
    out
}

fn frames() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/frames");
    let mut out = vec![title(
        &ns,
        "FRAMES / poses, odometry and uncertainty",
    )];
    for (i, glyph) in [Glyph::Chevron, Glyph::Arrow, Glyph::Axes]
        .into_iter()
        .enumerate()
    {
        let poses: Vec<_> = (0..5)
            .map(|j| {
                Pose::yawed(
                    j as f64 * 2.0 - 4.0,
                    i as f64 * 2.0 - 5.0,
                    0.1,
                    j as f64 * 0.3,
                )
            })
            .collect();
        out.push(w::pose_array(
            &p,
            &ns,
            &format!("poses-{i}"),
            &poses,
            glyph,
            0.8,
        ));
    }
    let mut covariance = vec![0.0; 36];
    for (i, value) in [0.3, 0.5, 0.2, 0.02, 0.02, 0.12].into_iter().enumerate() {
        covariance[i * 6 + i] = value;
    }
    out.extend(
        w::covariance(
            &p,
            &ns,
            "cov6",
            Pose::at(-4.0, 3.0, 1.0),
            &covariance,
            &[1.0, 2.0],
            color::PLAN,
        )
        .unwrap(),
    );
    out.extend(w::odometry(
        &p,
        &ns,
        "odom",
        Pose::at(3.0, 3.0, 0.1),
        [1.0, 0.4, 0.0],
        &[0.3, 0.0, 0.0, 0.15],
    ));
    out
}

fn plain() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/plain");
    let mut out = vec![title(
        &ns,
        "REFERENCE / plain scientific marks",
    )];
    for (i, fill) in [flag::FILL_SOLID, flag::FILL_WIRE, flag::FILL_BOTH]
        .into_iter()
        .enumerate()
    {
        out.push(
            p.put(E::shapes(
                &ns,
                &format!("plain-{i}"),
                shape::BOX,
                &[Instance::at([i as f32 * 4.0 - 4.0, 2.0, 1.0], [2.0; 3])],
            ))
            .with_style(style::PLAIN)
            .with_color(color::with_alpha(color::PLAN, 0.55))
            .with_fill(fill),
        );
    }
    for (i, sub) in [point_style::SPHERE, point_style::DISC, w::POINT_SQUARE]
        .into_iter()
        .enumerate()
    {
        let points: Vec<_> = (0..20)
            .map(|j| {
                [
                    i as f32 * 4.0 - 5.0 + (j % 5) as f32 * 0.5,
                    -2.0 - (j / 5) as f32 * 0.5,
                    0.2,
                ]
            })
            .collect();
        out.push(
            p.put(E::points(&ns, &format!("points-{i}"), &points))
                .with_sub(sub)
                .with_style(style::SPLAT)
                .with_width(-7.0)
                .with_color(color::SENSOR),
        );
    }
    out.push(w::voxels(
        &p,
        &ns,
        "voxels-plain",
        &[[-3.0, -6.0, 0.4], [-2.0, -6.0, 0.4]],
        0.8,
        &[0.2, 0.9],
        colormap::VIRIDIS,
        false,
    ));
    out.push(w::zone(
        &p,
        &ns,
        "keepout",
        w::Zone::Keepout,
        &[rect(0.0, -6.0, 2.0, 1.5)],
    ));
    out.push(w::zone(
        &p,
        &ns,
        "slow",
        w::Zone::Slow,
        &[rect(3.0, -6.0, 2.0, 1.5)],
    ));
    out
}

fn picking(scene: &str) -> Vec<E> {
    let ns = format!("{NS}/{scene}");
    let kind = if scene == "pick-point" {
        interact_kind::PICK_POINT
    } else {
        interact_kind::PICK_POSE
    };
    vec![
        title(&ns, "INPUT / point or press-and-drag heading"),
        E::interact(&ns, "request", kind)
            .in_frame("world")
            .with_prop("text", "Pick the marked region"),
        Place::default()
            .put(E::polygon(&ns, "target", &[rect(-3.0, -3.0, 6.0, 6.0)]))
            .with_color(color::PLAN)
            .with_fill(flag::FILL_BOTH),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn structure_language_review_contains_curves_extents_and_glass_not_cubes() {
        let items = super::review::structure_language();
        for e in &items { e.validate().unwrap(); }
        for kind in [super::shape::ELLIPSOID, super::shape::ELLIPSE, super::shape::CYLINDER,
            super::shape::CONE, super::shape::CAPSULE, super::shape::FRUSTUM, super::shape::SECTOR] {
            assert!(items.iter().any(|e| e.kind == gearbox_api::draw_kind::SHAPES && e.sub == kind));
        }
        assert!(!items.iter().any(|e| e.kind == gearbox_api::draw_kind::SHAPES && e.sub == super::shape::BOX));
        assert!(items.iter().any(|e| e.kind == gearbox_api::draw_kind::POLYGON && e.polygon_rings()[0].len() == 2));
        assert!(items.iter().any(|e| e.kind == gearbox_api::draw_kind::MESH));
        for finish in ["white", "black", "color"] {
            assert!(items.iter().any(|e| e.props().get("panel_finish").as_deref() == Some(finish)));
        }
    }

    use super::*;

    #[test]
    fn panel_content_fixture_covers_every_family_and_three_finishes() {
        let items = panel_content();
        for e in &items { e.validate().unwrap(); }
        let panels: Vec<_> = items.iter().filter(|e| e.kind == gearbox_api::draw_kind::PANEL).collect();
        assert_eq!(panels.len(), 6);
        for kind in 0..=panel_kind::LEGEND { assert_eq!(panels.iter().filter(|e| e.sub == kind).count(), 1); }
        for finish in ["white", "black", "color"] {
            assert_eq!(panels.iter().filter(|e| e.props().get("panel_finish").as_deref() == Some(finish)).count(), 2);
        }
    }

    #[test]
    fn table_layout_fixture_covers_finishes_and_numeric_rows() {
        let items = table_layout();
        for e in &items { e.validate().unwrap(); }
        let tables: Vec<_> = items.iter().filter(|e| e.kind == gearbox_api::draw_kind::PANEL).collect();
        assert_eq!(tables.len(), 4);
        for finish in ["color", "white", "black"] {
            let table = tables.iter().find(|e| e.id() == finish).unwrap();
            assert_eq!(table.props().get("panel_finish").as_deref(), Some(finish));
            assert_eq!(table.sub, panel_kind::TABLE);
        }
        assert_eq!(tables.iter().find(|e| e.id() == "numeric").unwrap().props().get("unit").as_deref(), Some("mm/s"));
    }

    #[test]
    fn normals_probe_changes_only_the_imported_asset() {
        let reference = normals_probe("reference").unwrap();
        for mode in ["flat", "authored", "smooth", "concave", "concave-authored", "concave-reference"] {
            let mut candidate = normals_probe(mode).unwrap();
            assert!(std::path::Path::new(&candidate.props().get("asset").unwrap()).is_file());
            candidate = candidate.with_prop("asset", &reference.props().get("asset").unwrap());
            assert_eq!(gearbox_api::pack(&candidate).wire(), gearbox_api::pack(&reference).wire());
        }
        assert!(normals_probe("unknown").is_err());
    }

    #[test]
    fn drape_probe_requests_differ_only_by_the_explicit_origin_instance() {
        for kind in ["axes", "shape", "mesh"] {
            let implicit = drape_probe(kind, false).unwrap();
            let explicit = drape_probe(kind, true).unwrap();
            implicit.validate().unwrap();
            explicit.validate().unwrap();
            assert!(implicit.instances.is_empty());
            assert_eq!(explicit.instance_count(), 1);
            assert_eq!(gearbox_api::pack(&implicit.with_instances(&[Instance::default()])).wire(), gearbox_api::pack(&explicit).wire());
        }
        assert!(drape_probe("ghost", false).is_err());
    }

    #[test]
    fn stroke_probe_preserves_extent_and_effects_across_subdivision() {
        for reverse in [false, true] {
            let coarse = stroke_probe(1, reverse);
            let dense = stroke_probe(256, reverse);
            for (a, b) in coarse.iter().zip(&dense) {
                a.validate().unwrap();
                b.validate().unwrap();
                assert_eq!(a.point_at(0), b.point_at(0));
                assert_eq!(a.point_at(1), b.point_at(256));
                assert_eq!((a.flags, a.style, a.width, a.dash, a.color), (b.flags, b.style, b.width, b.dash, b.color));
                assert_eq!((a.glow, a.pulse_hz, a.flow_speed), (-1.0, -1.0, -1.0));
            }
        }
    }
    use gearbox_api::draw_kind as kind;

    #[test]
    fn interaction_geometry_uses_native_logical_points() {
        assert_eq!(
            window_points("width=1226\nheight=1348\nx=0\ny=0\n").unwrap(),
            [1226.0, 1348.0]
        );
        assert!(window_points("width=NaN\nheight=1000\n").is_err());
        assert!(window_points("width=1600\n").is_err());
    }

    #[test]
    fn interaction_feedback_requires_exact_draw_scope() {
        let event = SceneEvent::new(event_kind::DRAW, "point").with_prop("ns", "vfx-gallery/test");
        assert!(scoped_event(&event, "vfx-gallery/test", "point"));
        assert!(!scoped_event(&event, "vfx-gallery/other", "point"));
        assert!(!scoped_event(&event, "vfx-gallery/test", "pose"));
        let unrelated =
            SceneEvent::new(event_kind::LOADED, "point").with_prop("ns", "vfx-gallery/test");
        assert!(!scoped_event(&unrelated, "vfx-gallery/test", "point"));
    }

    #[test]
    fn every_gallery_entity_is_valid_and_namespaced() {
        for e in catalogue("all", "vfx_rover") {
            e.validate()
                .unwrap_or_else(|err| panic!("{}/{}: {err}", e.ns(), e.id()));
            assert!(e.ns().starts_with(NS));
        }
    }

    #[test]
    fn fill_contract_has_each_shape_in_three_distinct_modes() {
        let entities = fill_contract();
        for entity in &entities { entity.validate().unwrap(); }
        for shape in 0..=shape::SECTOR {
            let shapes: Vec<_> = entities.iter().filter(|e| e.kind == kind::SHAPES && e.sub == shape).collect();
            assert_eq!(shapes.len(), 3);
            assert_eq!(shapes.iter().map(|e| e.flags & flag::FILL_MASK).collect::<Vec<_>>(), [flag::FILL_SOLID, flag::FILL_WIRE, flag::FILL_BOTH]);
            assert!(shapes.iter().all(|e| e.color != e.color2 && e.glow > 0.0 && e.pulse_hz > 0.0));
        }
    }

    #[test]
    fn gallery_covers_every_wire_kind() {
        let entities = catalogue("all", "vfx_rover");
        for k in 1..=kind::LAST {
            assert!(
                entities.iter().any(|e| e.kind == k),
                "missing {}",
                kind::name(k)
            );
        }
        for s in 0..=shape::SECTOR {
            assert!(entities.iter().any(|e| e.kind == kind::SHAPES && e.sub == s), "missing shape {}", shape::name(s));
        }
        for s in style::PLAIN..=style::FIELD {
            assert!(entities.iter().any(|e| e.style == s), "missing style {}", style::name(s));
        }
    }
}
