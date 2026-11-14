use super::*;

#[path = "../draw_cloud_scale/fixture.rs"]
mod bulk;
#[path = "upload_recovery.rs"]
mod recovery;
#[path = "upload_streaming.rs"]
mod streaming;
#[cfg(test)]
const OBSERVER: &str = "draw-upload-fairness-observer";

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .expect("required upload scheduling counter")
}

fn group(report: &Report, typed: bool, id: Option<u64>) -> Option<&serde_json::Value> {
    report.scheduler["groups"].as_array()?.iter().find(|value| {
        value["typed"].as_bool() == Some(typed)
            && value["selection"].as_bool() != Some(true)
            && id.is_none_or(|id| value["id"].as_u64() == Some(id))
    })
}

#[test]
fn selection_view_ids_do_not_alias_dataset_upload_counters() {
    let report = Report {
        sequence: 0,
        views: [None; 2],
        samples: [0; 2],
        completed: [None; 2],
        delayed_pages: 0,
        secondary_alive: false,
        active: true,
        gpu_reserved: 0,
        cpu: serde_json::Value::Null,
        allocations: serde_json::Value::Null,
        uploads: serde_json::Value::Null,
        legacy: serde_json::Value::Null,
        backdrops: serde_json::Value::Null,
        lod: serde_json::Value::Null,
        motion: serde_json::Value::Null,
        scheduler: serde_json::json!({"groups":[
            {"typed":true,"selection":true,"id":7,"bytes":8},
            {"typed":true,"selection":false,"id":7,"bytes":1024},
            {"typed":false,"selection":false,"id":7,"bytes":2048}
        ]}),
    };
    assert_eq!(group(&report, true, Some(7)).unwrap()["bytes"], 1024);
    assert_eq!(group(&report, false, Some(7)).unwrap()["bytes"], 2048);
}

pub(super) fn run(
    control: &mut Control,
    client: &Client,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
) -> Result<()> {
    control.set("configure", true, 0)?;
    let baseline = control.wait("baseline", |r| {
        r.views.iter().all(Option::is_some) && number(&r.allocations, "gpu_bytes") == 0
    })?;
    let large_key = DataKey::new(NS, "large")?;
    let small_key = DataKey::new(NS, "small")?;
    let mut fixture = bulk::make(large_key.clone(), false)?;
    fixture.pages.truncate(12);
    fixture.scans[0].source_count = 12 * u64::from(bulk::PAGE);
    fixture.scans[0].frame.pose.translation_m = [100.0, 0.0, 0.0];
    let pages: Vec<_> = fixture
        .pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    control.gpu_headroom = Some(0);
    control.set("configure", true, 0)?;
    let reply = writer.upload(
        &DataUpload::replace(&fixture.descriptor, &fixture.scans, &pages),
        deadline(),
        None,
    )?;
    let large = DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    };
    drop(pages);
    let small = upload_at(writer, &small_key, None, [255, 24, 24, 255], -2.5)?;
    let legacy =
        gearbox_api::DrawEntity::points(NS, "legacy", &[[1.1, 25.0, 2.0], [3.9, 25.0, 2.0]])
            .with_style(gearbox_api::draw_style::SPLAT)
            .with_color(gearbox_api::wire::draw::rgba(24, 24, 255, 255))
            .with_width(1.6)
            .with_glow(0.0)
            .with_pulse(-1.0);
    require(client.draw(&legacy)?.is_ok(), "legacy upload rejected")?;
    let until = deadline();
    loop {
        let denied = status(data, &large_key, Some(large), None)?
            .failure
            .is_some()
            && status(data, &small_key, Some(small), None)?
                .failure
                .is_some();
        let legacy_denied = std::fs::read_to_string(control.root.join("viewer.log"))
            .is_ok_and(|log| log.contains("compact legacy point admission: GpuAllocation"));
        if denied && legacy_denied {
            break;
        }
        if Instant::now() >= until {
            return Err("all three workloads did not reach admission together".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    control.gpu_headroom = None;
    control.set("configure", true, 0)?;
    let small_receipts = recovery::wait(control, data, &small_key, small, baseline.views, 2)?;
    let large_receipts = recovery::wait(
        control,
        data,
        &large_key,
        large,
        baseline.views,
        fixture.scans[0].source_count,
    )?;
    std::fs::write(
        control.root.join("burst-recovery.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"small":small_receipts,"large":large_receipts}),
        )?,
    )?;
    let ready = control.wait("all-groups-progressed", |r| {
        group(r, true, Some(large.incarnation)).is_some()
            && group(r, true, Some(small.incarnation)).is_some()
            && group(r, false, None).is_some()
            && r.legacy["queued"][0].as_u64() == Some(1)
            && r.legacy["queued"][1].as_u64() == Some(1)
            && number(&r.allocations, "staging_gpu_bytes") == 0
            && number(&r.allocations, "retiring_gpu_bytes") == 0
    })?;
    let big = group(&ready, true, Some(large.incarnation)).unwrap();
    for peer in [
        group(&ready, true, Some(small.incarnation)).unwrap(),
        group(&ready, false, None).unwrap(),
    ] {
        require(
            number(peer, "last_frame") <= number(big, "first_frame") + 1,
            "small/legacy workload did not finish within the first two shared upload frames",
        )?;
        require(
            number(peer, "last_frame") < number(big, "last_frame"),
            "small/legacy workload waited for the large cloud to finish",
        )?;
    }
    require(
        number(&ready.scheduler, "overflow") == 0,
        "native group observation overflow",
    )?;
    require(
        number(&ready.allocations, "max_frame_upload")
            <= data.capabilities().limits.upload_bytes_per_frame,
        "shared upload cap exceeded",
    )?;
    std::fs::write(
        control.root.join("progress.json"),
        serde_json::to_vec_pretty(&ready)?,
    )?;
    control.capture("contended-ready", 0, ["red+blue", "red+blue"])?;
    let stream = streaming::run(
        control,
        client,
        data,
        writer,
        &mut fixture,
        large,
        &small_key,
        small,
        &legacy,
    )?;
    control.capture("streamed-ready", 0, ["red+blue", "red+blue"])?;
    clear(data, writer.lease())?;
    require(client.draw_clear(NS)?.is_ok(), "legacy clear rejected")?;
    let cleared = control.wait("cleared", |r| {
        number(&r.allocations, "gpu_bytes") == 0
            && number(&r.legacy, "points") == 0
            && number(&r.cpu, "allocated") == number(&baseline.cpu, "allocated")
    })?;
    control.capture("cleared", 0, ["none", "none"])?;
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass","large_points":fixture.scans[0].source_count,
            "large_reference_bounds":bulk::manifest(),"scheduler":ready.scheduler,
            "max_frame_upload":ready.allocations["max_frame_upload"],
            "streaming":stream,
            "baseline_cpu":baseline.cpu,"cleared_cpu":cleared.cpu,
            "scope":"simultaneous large typed, small typed and legacy uploads; exact views, early peer completion and cleanup",
        }))?,
    )?;
    Ok(())
}
