use super::*;
use gearbox_api::{DRAW_CHUNK_BYTES, clock_op, draw_kind, pack};

fn header(client: &Client, ns: &str, id: &str) -> Result<E> {
    client.draw_list(ns)?.into_iter().find(|e| e.ns() == ns && e.id() == id)
        .ok_or_else(|| format!("missing retained draw {ns}/{id}").into())
}

fn exists(client: &Client, ns: &str, id: &str) -> Result<bool> {
    Ok(client.draw_list(ns)?.iter().any(|e| e.ns() == ns && e.id() == id))
}

fn same_header(client: &Client, expected: &E, stage: &str) -> Result<()> {
    let actual = header(client, &expected.ns(), &expected.id())?;
    if pack(&actual).wire() != pack(expected).wire() {
        return Err(format!("{stage}: unexpected retained header change\nexpected {expected:?}\nactual {actual:?}").into());
    }
    println!("{}", serde_json::json!({"passed": stage}));
    Ok(())
}

fn cli(bin: &Path, instance: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(bin).args(["--instance", instance, "--timeout", "30"])
        .args(args).output()?;
    if !output.status.success() {
        return Err(format!("CLI {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

fn odometry_updates(client: &Client, ns: &str) -> Result<()> {
    let root = format!("{ns}/odom");
    let cov_ns = format!("{root}/cov");
    let sentinels = [cov_ns.clone(), format!("{cov_ns}/nested"), format!("{ns}/odometry/cov")];
    for sibling in &sentinels { send_draw(client, E::points(sibling, "custom", &[[0.0; 3]]))?; }
    let mut six = vec![0.0; 36];
    (six[0], six[7], six[14], six[35]) = (0.25, 1.0, 0.5, 0.09);
    for (cov, expected) in [(&[0.25, 0.0, 0.0, 1.0][..], vec!["custom", "mean", "rings"]),
        (&six, vec!["custom", "ellipsoid", "yaw"]),
        (&[0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5], vec!["custom", "ellipsoid"])] {
        for values in [cov, &[]] {
            for entity in w::odometry(&Place::default(), ns, "odom", Pose::default(), [1.0, 0.0, 0.0], values) {
                send_draw(client, entity)?;
            }
            let mut ids: Vec<_> = client.draw_list(&cov_ns)?.iter().filter(|e| e.ns() == cov_ns).map(|e| e.id()).collect();
            ids.sort();
            let wanted = if values.is_empty() { vec!["custom"] } else { expected.clone() };
            if ids != wanted { return Err(format!("odometry covariance retained {ids:?}, expected {wanted:?}").into()); }
            header(client, &root, "pose")?;
            header(client, &root, "twist")?;
            for sibling in &sentinels { header(client, sibling, "custom")?; }
        }
    }
    println!("{}", serde_json::json!({"passed": "Rust odometry omission clears all covariance dimensions without touching neighbors"}));
    Ok(())
}

pub fn verify(client: &Client, instance: &str, bin: &Path) -> Result<()> {
    let ns = format!("{NS}/lifecycle-{}", std::process::id());
    let clock = client.clock(clock_op::GET)?;
    let result = (|| -> Result<()> {
        let points = [[-2.0, 0.0, 0.4], [0.0, 1.0, 0.6], [2.0, 0.0, 0.8]];
        send_draw(client, E::lines(&ns, "live", &points).with_color(0xb4dc803c).with_color2(0xc81450dc)
            .with_width(0.12).with_style(style::LIGHT_WALL).with_glow(0.6).with_pulse(0.8)
            .with_flow(1.2, 2.0).with_beam(3.0).with_fade(0.2).with_urgency(0.4)
            .with_colormap(colormap::TURBO, -2.0, 10.0).at_yaw([0.5, 0.2, 0.3], 0.4)
            .with_scale([1.0, 1.1, 1.2]).with_flags(flag::FLAT).with_prop("panel_finish", "black"))?;
        let mut expected = header(client, &ns, "live")?;
        cli(bin, instance, &["draw", "update", &ns, "live", "--glow", "0.22"])?;
        expected.glow = 0.22;
        same_header(client, &expected, "CLI glow preserves motion, geometry, tint and palette")?;
        cli(bin, instance, &["draw", "update", &ns, "live", "--panel-finish", "white"])?;
        expected = expected.with_prop("panel_finish", "white");
        same_header(client, &expected, "CLI metadata preserves all header fields")?;
        cli(bin, instance, &["draw", "update", &ns, "live", "--color", "#30cc90a0"])?;
        expected.color = 0xa090cc30;
        same_header(client, &expected, "CLI tint preserves secondary color")?;
        cli(bin, instance, &["draw", "update", &ns, "live", "--vmin", "0"])?;
        expected.vmin = 0.0;
        same_header(client, &expected, "CLI lower bound preserves palette and upper bound")?;
        send_draw(client, E::update(&ns, "live").with_pulse(-1.0).updating(&["pulse_hz"]))?;
        expected.pulse_hz = -1.0;
        same_header(client, &expected, "Rust scalar pulse update preserves flow and glow")?;
        send_draw(client, E::update(&ns, "live").with_prop("label_detail", "unchanged geometry").updating(&["props"]))?;
        expected = expected.with_prop("label_detail", "unchanged geometry");
        same_header(client, &expected, "Rust metadata-only update preserves all headers")?;
        odometry_updates(client, &ns)?;

        let mut invalid_pose = E::update(&ns, "live").updating(&["pose"]);
        invalid_pose.px = f64::NAN;
        let mut invalid_scale = E::update(&ns, "live").updating(&["scale"]);
        invalid_scale.sy = f32::INFINITY;
        let mut invalid_lifetime = E::update(&ns, "live").updating(&["lifetime"]);
        invalid_lifetime.lifetime_s = f64::NAN;
        let invalid_radius = E::lines(&ns, "live", &points).with_radii(&[f32::NEG_INFINITY]);
        for invalid in [invalid_pose, invalid_scale, invalid_lifetime, invalid_radius] {
            let response = client.draw_raw(&invalid)?;
            if response.is_ok() || !response.message().contains("non-finite") {
                return Err(format!("invalid draw was not rejected by the host: {}", response.message()).into());
            }
            same_header(client, &expected, "host rejects non-finite input without mutating retained headers")?;
        }

        let dense: Vec<_> = (0..6000).map(|i| [(i % 100) as f32 * 0.04, (i / 100) as f32 * 0.04, 1.0]).collect();
        let chunks = E::points(&ns, "live", &dense).with_style(style::SPLAT).with_width(-2.0).chunks(DRAW_CHUNK_BYTES);
        if chunks.len() < 2 { return Err("fixture did not chunk".into()); }
        for (i, chunk) in chunks.iter().enumerate() {
            let status = client.draw_raw(chunk)?;
            if !status.is_ok() { return Err(status.message().into()); }
            if i + 1 < chunks.len() { same_header(client, &expected, "partial chunk leaves previous entity visible")?; }
        }
        let complete = header(client, &ns, "live")?;
        if complete.kind != draw_kind::POINTS || complete.props().get("points").as_deref() != Some("6000") {
            return Err("chunk completion did not atomically replace the live entity".into());
        }
        println!("{}", serde_json::json!({"passed": "chunked replacement", "chunks": chunks.len(), "points": 6000}));
        let status = client.draw_raw(&chunks[0])?;
        if !status.is_ok() { return Err(status.message().into()); }
        send_draw(client, E::delete(&ns, "live"))?;
        if client.draw_raw(&chunks[1])?.is_ok() || exists(client, &ns, "live")? {
            return Err("delete failed to cancel pending assembly".into());
        }
        println!("{}", serde_json::json!({"passed": "delete cancels pending chunks without resurrection"}));

        send_draw(client, E::lines(&ns, "tail", &points))?;
        let mut extend = E::lines(&ns, "tail", &[[3.0, 0.0, 0.8], [4.0, 1.0, 0.9]]).with_prop("max_points", "4");
        extend.op = draw_op::EXTEND;
        send_draw(client, extend)?;
        if header(client, &ns, "tail")?.props().get("points").as_deref() != Some("4") {
            return Err("extend did not enforce retained point limit".into());
        }
        send_draw(client, E::patch(draw_kind::LINES, &ns, "tail", 3).with_points(&[[5.0, 2.0, 1.0], [6.0, 2.0, 1.0]]))?;
        if header(client, &ns, "tail")?.props().get("points").as_deref() != Some("5") {
            return Err("partial patch did not grow retained geometry".into());
        }
        println!("{}", serde_json::json!({"passed": "bounded extend and growing partial patch"}));

        client.clock(clock_op::PAUSE)?;
        std::thread::sleep(Duration::from_millis(250));
        send_draw(client, E::points(&ns, "expires", &[[0.0, 0.0, 1.0]]).with_lifetime(0.15))?;
        std::thread::sleep(Duration::from_millis(300));
        header(client, &ns, "expires")?;
        client.clock(clock_op::PLAY)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while exists(client, &ns, "expires")? {
            if Instant::now() > deadline { return Err("lifetime failed to expire while simulation advanced".into()); }
            std::thread::sleep(Duration::from_millis(100));
        }
        println!("{}", serde_json::json!({"passed": "lifetime pauses and expires with simulated time"}));

        send_draw(client, E::points(&format!("{ns}/child"), "remove", &[[0.0; 3]]))?;
        send_draw(client, E::points(&format!("{ns}/children"), "keep", &[[0.0; 3]]))?;
        send_draw(client, E::clear(&format!("{ns}/child")))?;
        if exists(client, &format!("{ns}/child"), "remove")? {
            return Err("nested clear left a child entity".into());
        }
        header(client, &format!("{ns}/children"), "keep")?;
        println!("{}", serde_json::json!({"passed": "namespace clear respects path-component boundaries"}));
        Ok(())
    })();
    let cleared = client.draw_clear(&ns);
    let restored = client.clock(if clock.paused != 0 { clock_op::PAUSE } else { clock_op::PLAY });
    result?;
    let status = cleared?;
    if !status.is_ok() { return Err(status.message().into()); }
    restored?;
    if !client.draw_list(&ns)?.is_empty() { return Err("lifecycle cleanup left retained entities".into()); }
    println!("{}", serde_json::json!({"passed": "lifecycle cleanup", "namespace": ns}));
    Ok(())
}
