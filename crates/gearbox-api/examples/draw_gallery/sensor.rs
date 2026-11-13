use super::*;
use gearbox_api::{CAMERA_COLOR, CameraFrame, clock_op};

fn camera_image(sub: &mut Subscriber<Env>, after: u32) -> Result<(u32, Vec<u8>)> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut sample = 0;
    let mut pixels = vec![0; 160 * 120 * 4];
    let mut rows = [false; 120];
    while Instant::now() < deadline {
        let Some(frame) = next_sample::<CameraFrame>(sub, Duration::from_secs(2))? else { continue; };
        if frame.channel != CAMERA_COLOR || frame.sample <= after { continue; }
        if (frame.width, frame.height) != (160, 120) { return Err("unexpected sensor dimensions".into()); }
        if sample != frame.sample {
            sample = frame.sample;
            rows.fill(false);
        }
        let start = frame.row_offset as usize;
        let end = start + frame.rows as usize;
        if end > 120 || frame.data.len() != (end - start) * 160 * 4 { return Err("malformed camera rows".into()); }
        pixels[start * 640..end * 640].copy_from_slice(&frame.data);
        rows[start..end].fill(true);
        if rows.iter().all(|row| *row) { return Ok((sample, pixels)); }
    }
    Err("no complete camera image within 60 seconds".into())
}

pub fn verify(client: &Client, machine: &str, link: &str, out: &Path) -> Result<()> {
    if !client.machines()?.iter().any(|entry| entry.machine_id() == machine) { return Err("sensor rig is missing".into()); }
    let mut frames = client.subscribe_env(&format!("/machines/{machine}/sensors/{link}"))?;
    let ns = format!("{NS}/sensor-isolation");
    let original = client.clock(clock_op::GET)?;
    let result = (|| -> Result<()> {
        send_draw(client, E::clear(&ns))?;
        client.clock(clock_op::PLAY)?;
        let (sample, _) = camera_image(&mut frames, 0)?;
        let (sample, baseline) = camera_image(&mut frames, sample + 1)?;
        client.clock(clock_op::PAUSE)?;
        std::fs::write(out.join("sensor-before.rgba"), &baseline)?;
        if baseline.chunks_exact(4).all(|p| p[..3].iter().all(|c| *c < 5)) { return Err("sensor baseline is black".into()); }
        let frame = format!("{machine}/{link}");
        let q = [(std::f64::consts::FRAC_PI_4).cos(), 0.0, (std::f64::consts::FRAC_PI_4).sin(), 0.0];
        send_draw(client, E::shapes(&ns, "surface", shape::RECT, &[Instance::at([0.0; 3], [8.0, 8.0, 1.0])])
            .in_frame(&frame).with_pose([2.0, 0.0, 0.0], q).with_style(style::PLAIN).with_color(0xffff00ff).with_blur(1.0))?;
        send_draw(client, E::lines(&ns, "stroke", &[[1.5, -4.0, 0.0], [1.5, 4.0, 0.0]])
            .in_frame(&frame).with_width(2.0).with_color(0xff00ffff).with_style(style::TUBE_PULSE).with_blur(1.0))?;
        send_draw(client, E::grid(&ns, "heat", 8, 8, 0.5, GridCells::U8(vec![100; 64]))
            .in_frame(&frame).with_pose([1.0, -2.0, 2.0], q).with_colormap(colormap::TURBO, 0.0, 100.0).with_blur(1.0))?;
        send_draw(client, E::ghost(&ns, "ghost", machine, &[Instance::at([-4.0, 0.0, 0.0], [1.0; 3])])
            .in_frame(&frame).with_color(0xffff00ff).with_glow(0.5).with_blur(1.0))?;
        std::thread::sleep(Duration::from_secs(2));
        client.clock(clock_op::PLAY)?;
        let (sample, with_draws) = camera_image(&mut frames, sample + 3)?;
        client.clock(clock_op::PAUSE)?;
        std::fs::write(out.join("sensor-with-draws.rgba"), &with_draws)?;
        let delta: u64 = baseline.iter().zip(&with_draws).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        let mean = delta as f64 / baseline.len() as f64;
        let magenta = with_draws.chunks_exact(4).filter(|p| p[0] > 150 && p[2] > 150 && p[1] < 80).count();
        let report = serde_json::json!({"sample": sample, "link": link, "mean_channel_delta": mean, "magenta_pixels": magenta,
            "width": 160, "height": 120, "draw_backends": ["surface", "stroke", "heat", "ghost"]});
        std::fs::write(out.join("sensor-metrics.json"), serde_json::to_vec_pretty(&report)?)?;
        println!("{report}");
        if mean > 1.0 || magenta > 10 { return Err("sensor image changed with viewer-only drawings".into()); }
        Ok(())
    })();
    client.clock(if original.paused != 0 { clock_op::PAUSE } else { clock_op::PLAY })?;
    result
}
