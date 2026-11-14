use super::*;

const ROBOT: &str = "legacy_frame_robot";
const IDS: [&str; 4] = ["world", "odom", "live", "snapshot"];
const STAGES: [&str; 5] = [
    "01-initial",
    "02-frame-moved",
    "03-snapshot-restyled",
    "04-snapshot-local-pose",
    "05-capture-replaced",
];

fn entity(index: usize) -> E {
    let mut points = Vec::new();
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                points.push([x as f32 * 0.15, y as f32 * 0.15, z as f32 * 0.15]);
            }
        }
    }
    let frame = match index {
        0 => "world".into(),
        1 => format!("{ROBOT}/odom"),
        _ => ROBOT.into(),
    };
    E::points(NS, IDS[index], &points)
        .in_frame(&frame)
        .with_pose(
            [
                [-3.0, -3.0, 3.0],
                [3.0, -3.0, 3.0],
                [3.0, 3.0, 3.0],
                [-3.0, 3.0, 3.0],
            ][index],
            [1.0, 0.0, 0.0, 0.0],
        )
        .with_style(draw_style::SPLAT)
        .with_color([0xff00ffff, 0xffffff00, 0xff00ff00, 0xffff00ff][index])
        .with_width(0.3)
        .with_pulse(-1.0)
        .with_glow(0.3)
        .with_flags(if index == 3 {
            gearbox_api::draw_flag::SNAPSHOT
        } else {
            0
        })
}

fn cli(capture: &Capture, args: &[&str]) -> Result<Vec<u8>> {
    let result = Command::new("timeout")
        .args([
            "90",
            capture.bin,
            "--instance",
            capture.instance,
            "--timeout",
            "30",
        ])
        .args(args)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "frame CLI {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(result.stdout)
}

fn control(capture: &Capture, sequence: u64, action: &str) -> Result<()> {
    let value =
        serde_json::json!({"sequence":sequence,"action":action,"active":true,"max_generation":0});
    let path = capture.out.join("control.tmp");
    std::fs::write(&path, serde_json::to_vec(&value)?)?;
    std::fs::rename(path, capture.out.join("control.json"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(bytes) = std::fs::read(capture.out.join("report.json"))
            && let Ok(report) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && report["sequence"].as_u64() == Some(sequence)
        {
            if action == "remove" || report["samples"] == serde_json::json!([4, 1]) {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err("secondary 1x MSAA control did not complete".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn stage(capture: &Capture, index: usize, after: u64) -> Result<Metrics> {
    let name = format!("frame-{}", STAGES[index]);
    let m = capture.stage(&name, 108, 4, after)?;
    let pose = cli(capture, &["--json", "scene", "pose", ROBOT])?;
    std::fs::write(capture.out.join(format!("{name}.machine.json")), pose)?;
    control(capture, index as u64 + 2, "capture")?;
    Ok(m)
}

pub fn verify(client: &Client, capture: &Capture) -> Result<()> {
    let asset = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/machines/usd/clearpath_husky.usdz");
    cli(
        capture,
        &[
            "spawn",
            "machine",
            asset.to_str().ok_or("asset path")?,
            "--machine",
            ROBOT,
            "--at",
            "0",
            "0",
            "0",
            "--wait-timeout",
            "90",
        ],
    )?;
    client.clock(clock_op::PLAY)?;
    std::thread::sleep(Duration::from_secs(3));
    client.clock(clock_op::PAUSE)?;
    cli(
        capture,
        &[
            "instance",
            "camera",
            "look",
            "--lat",
            "52.370216",
            "--lon",
            "4.895168",
            "--bearing",
            "160",
            "--pitch",
            "45",
            "--distance",
            "20",
            "--above",
            "2",
        ],
    )?;
    control(capture, 1, "configure")?;
    let baseline = metrics(capture.out, 0, 0, 0)?;
    for i in 0..4 {
        send(client, &entity(i))?;
    }
    let initial = stage(capture, 0, baseline.frame)?;
    for i in 0..2 {
        if initial.uploaded[i] - baseline.uploaded[i] != 108 * 16 {
            return Err("frame records uploaded repeatedly".into());
        }
    }
    client.clock(clock_op::PLAY)?;
    cli(
        capture,
        &[
            "machine",
            "move",
            ROBOT,
            "--forward",
            "1",
            "--distance",
            "3",
        ],
    )?;
    client.clock(clock_op::PAUSE)?;
    let moved = stage(capture, 1, initial.frame)?;
    unchanged(&initial, &moved, &[0, 1])?;
    changed(&initial, &moved, 2)?;
    send(
        client,
        &E::update(NS, "snapshot").with_glow(0.4).updating(&["glow"]),
    )?;
    let restyled = stage(capture, 2, moved.frame)?;
    unchanged(&initial, &restyled, &[0, 1])?;
    send(
        client,
        &E::update(NS, "snapshot")
            .with_pose([-2.0, 3.0, 3.0], [1.0, 0.0, 0.0, 0.0])
            .updating(&["pose"]),
    )?;
    let local = stage(capture, 3, restyled.frame)?;
    unchanged(&initial, &local, &[0, 1])?;
    send(client, &entity(3))?;
    let replaced = stage(capture, 4, local.frame)?;
    unchanged(&initial, &replaced, &[0, 1])?;
    send(client, &E::clear(NS))?;
    let cleared = capture.stage("frame-06-clear", 0, 0, replaced.frame)?;
    control(capture, 7, "remove")?;
    if cleared.charged_cpu_bytes != baseline.charged_cpu_bytes {
        return Err("frame clear did not restore charged CPU baseline".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_markers_preserve_distinct_colors_and_only_snapshot_is_frozen() {
        for i in 0..4 {
            let e = entity(i);
            e.validate().unwrap();
            assert_eq!(e.point_count(), 27);
            assert_eq!(e.flags & gearbox_api::draw_flag::SNAPSHOT != 0, i == 3);
            assert_eq!(e.id(), IDS[i]);
            assert!(e.labels.is_empty());
        }
    }
}
