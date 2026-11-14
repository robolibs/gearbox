use super::*;
use gearbox_api::{DrawEntity, draw_style};

const NS: &str = "vfx-gallery/compact-legacy-sensor";

fn entity(frame: &str, translucent: bool) -> DrawEntity {
    let points: Vec<_> = (0..33)
        .flat_map(|z| (0..49).map(move |y| [2.0, y as f32 * 0.125 - 3.0, z as f32 * 0.125 - 2.0]))
        .collect();
    DrawEntity::points(NS, "sensor-plane", &points)
        .in_frame(frame)
        .with_style(if translucent {
            draw_style::PLAIN
        } else {
            draw_style::SPLAT
        })
        .with_color(if translucent { 0x8cff00ff } else { 0xffff00ff })
        .with_radii(&vec![0.14; points.len()])
        .with_glow(0.2)
        .with_blur(0.8)
}

fn send(client: &Client, entity: &DrawEntity) -> Result<()> {
    entity.validate()?;
    let status = client.draw(entity)?;
    if !status.is_ok() {
        return Err(status.message().into());
    }
    Ok(())
}

pub fn verify(
    client: &Client,
    instance: &str,
    machine: &str,
    link: &str,
    out: &Path,
    bin: &str,
) -> Result<()> {
    let mut frames = client.subscribe_env(&format!("/machines/{machine}/sensors/{link}"))?;
    let original = client.clock(clock_op::GET)?;
    let result = (|| -> Result<()> {
        send(client, &DrawEntity::clear(NS))?;
        client.clock(clock_op::PLAY)?;
        let (sample, _) = camera_image(&mut frames, 0)?;
        let (mut sample, baseline) = camera_image(&mut frames, sample + 3)?;
        client.clock(clock_op::PAUSE)?;
        if baseline
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|c| *c < 5))
        {
            return Err("legacy sensor baseline is black".into());
        }
        std::fs::write(out.join("sensor-before.rgba"), &baseline)?;
        screenshot(bin, instance, out, "before")?;
        let mut stages = Vec::new();
        for stage in ["opaque", "translucent", "clear"] {
            let style = if stage == "clear" {
                send(client, &DrawEntity::clear(NS))?;
                None
            } else {
                let value = entity(&format!("{machine}/{link}"), stage == "translucent");
                send(client, &value)?;
                Some(value.style)
            };
            std::thread::sleep(Duration::from_secs(2));
            client.clock(clock_op::PLAY)?;
            let previous_sample = sample;
            let (new_sample, pixels) = camera_image(&mut frames, sample + 3)?;
            sample = new_sample;
            client.clock(clock_op::PAUSE)?;
            std::fs::write(out.join(format!("sensor-{stage}.rgba")), &pixels)?;
            screenshot(bin, instance, out, stage)?;
            let mean = baseline
                .iter()
                .zip(&pixels)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum::<u64>() as f64
                / baseline.len() as f64;
            let magenta = pixels
                .chunks_exact(4)
                .filter(|p| p[0] > 150 && p[2] > 150 && p[1] < 80)
                .count();
            let report = serde_json::json!({"stage":stage,"sample":sample,"after_sample":previous_sample,
                "mean_channel_delta":mean,"magenta_pixels":magenta,"legacy_style":style});
            println!("{report}");
            stages.push(report);
            std::fs::write(
                out.join("sensor-metrics.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "link":link,"backend":"legacy-compact","points":fixture::POINTS,"stages":stages
                }))?,
            )?;
            if mean > 1.0 || magenta > 10 {
                return Err(format!("compact legacy cloud contaminated {link} at {stage}").into());
            }
        }
        if !client.draw_list(NS)?.is_empty() {
            return Err("legacy sensor clear retained entities".into());
        }
        Ok(())
    })();
    let cleanup = send(client, &DrawEntity::clear(NS));
    let restored = client.clock(if original.paused != 0 {
        clock_op::PAUSE
    } else {
        clock_op::PLAY
    });
    result?;
    cleanup?;
    restored?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_plane_matches_typed_sensor_positions_and_preserves_frame() {
        let typed = fixture::make("sensor_probe/camera_link").unwrap();
        let DataShape::Cloud { schema } = &typed.descriptor.shape else {
            panic!()
        };
        let cloud = CloudPage::decode(schema, &typed.bytes).unwrap();
        let value = entity("sensor_probe/camera_link", false);
        value.validate().unwrap();
        assert_eq!(value.point_count(), fixture::POINTS as usize);
        assert_eq!(
            value
                .points
                .iter()
                .copied()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            cloud.columns[0].values
        );
        assert_eq!(
            value.props().get("frame").as_deref(),
            Some("sensor_probe/camera_link")
        );
    }

    #[test]
    fn legacy_sensor_paths_are_undecorated_and_keep_color_radius_and_frost() {
        for translucent in [false, true] {
            let value = entity("sensor_probe/camera_link", translucent);
            value.validate().unwrap();
            assert!(value.labels.is_empty());
            assert_eq!(value.flags, 0);
            assert_eq!(
                value.style,
                if translucent {
                    draw_style::PLAIN
                } else {
                    draw_style::SPLAT
                }
            );
            assert_eq!(value.color & 0xffffff, 0xff00ff);
            assert_eq!(value.color >> 24, if translucent { 140 } else { 255 });
            assert!(value.radii.iter().all(|r| *r == 0.14));
            assert_eq!(value.props().get("blur").as_deref(), Some("0.8"));
        }
    }
}
