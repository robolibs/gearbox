use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn replace(
    writer: &mut DrawDataWriter,
    fixture: &bulk::Fixture,
    base: DatasetRevision,
) -> Result<DatasetRevision> {
    let pages: Vec<_> = fixture
        .pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    let mut upload = DataUpload::replace(&fixture.descriptor, &fixture.scans, &pages);
    upload.base = Some(base);
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn advance(fixture: &mut bulk::Fixture) -> Result<()> {
    let DataShape::Cloud { schema } = &fixture.descriptor.shape else {
        unreachable!()
    };
    for (descriptor, bytes) in &mut fixture.pages {
        let mut page = descriptor.decode_cloud(schema, bytes)?;
        for point in page.columns[0].values.chunks_exact_mut(12) {
            let z = f32::from_le_bytes(point[8..12].try_into().unwrap()) + 0.005;
            point[8..12].copy_from_slice(&z.to_le_bytes());
        }
        *bytes = page.encode(schema)?;
        descriptor.byte_len = bytes.len() as u64;
        descriptor.crc32 = page_crc32(bytes);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    control: &mut Control,
    client: &Client,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
    fixture: &mut bulk::Fixture,
    large: DatasetRevision,
    small_key: &DataKey,
    mut small: DatasetRevision,
    legacy: &gearbox_api::DrawEntity,
) -> Result<serde_json::Value> {
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut producer = pipeline.writer(2, deadline())?;
    let stop = AtomicBool::new(false);
    let commits = AtomicU64::new(0);
    let large_key = fixture.descriptor.key.clone();
    let views = control.read()?.views;
    let (peers, source) = std::thread::scope(|scope| {
        let source = scope.spawn(|| -> std::result::Result<DatasetRevision, String> {
            let run = (|| -> Result<DatasetRevision> {
                let mut revision = large;
                let until = Instant::now() + Duration::from_secs(120);
                while !stop.load(Ordering::Acquire) {
                    if Instant::now() >= until {
                        return Err("continuous producer exceeded its run deadline".into());
                    }
                    advance(fixture)?;
                    revision = replace(&mut producer, fixture, revision)?;
                    commits.fetch_add(1, Ordering::Release);
                }
                Ok(revision)
            })();
            let closed = producer.close(deadline());
            run.and_then(|revision| closed.map(|_| revision).map_err(Into::into))
                .map_err(|error| error.to_string())
        });
        let peers = (|| -> Result<Vec<serde_json::Value>> {
            let until = deadline();
            while commits.load(Ordering::Acquire) == 0 {
                if Instant::now() >= until {
                    return Err("continuous producer did not commit".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut updates = Vec::new();
            for round in 0..6 {
                let before = control.read()?;
                let large_before = number(
                    group(&before, true, Some(large.incarnation)).unwrap(),
                    "bytes",
                );
                let legacy_before = before.legacy["uploaded"][2].as_u64().unwrap();
                let started = Instant::now();
                let color = if round % 2 == 0 {
                    [255, 24, 255, 255]
                } else {
                    [255, 24, 24, 255]
                };
                small = upload_at(writer, small_key, Some(small), color, -2.5)?;
                let width = if round % 2 == 0 { 1.64 } else { 1.6 };
                require(
                    client.draw(&legacy.clone().with_width(width))?.is_ok(),
                    "continuous legacy update rejected",
                )?;
                for view in views {
                    data.wait_visible_with_deadline(small_key, small, view, deadline())?;
                }
                let current = control.wait("streamed-peers", |r| {
                    r.legacy["uploaded"][2].as_u64().unwrap() > legacy_before
                        && r.legacy["queued"][0].as_u64() == Some(1)
                        && r.legacy["queued"][1].as_u64() == Some(1)
                        && group(r, true, Some(large.incarnation))
                            .is_some_and(|g| number(g, "bytes") > large_before)
                })?;
                let latency_ms = started.elapsed().as_millis() as u64;
                std::fs::write(
                    control.root.join("streaming-last-peer.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({"round":round,"revision":small,
                        "latency_ms":latency_ms,"producer_commits":commits.load(Ordering::Acquire),
                        "report":current}))?,
                )?;
                require(
                    latency_ms <= 3000,
                    "peer update stalled during continuous large uploads",
                )?;
                require(
                    number(&current.allocations, "max_frame_upload")
                        <= data.capabilities().limits.upload_bytes_per_frame,
                    "continuous upload cap exceeded",
                )?;
                require(
                    number(&current.allocations, "gpu_bytes")
                        <= data.capabilities().limits.gpu_bytes,
                    "continuous GPU cap exceeded",
                )?;
                require(
                    number(&current.allocations, "staging_gpu_bytes")
                        <= 2 * data.capabilities().limits.upload_bytes_per_frame,
                    "continuous staging exceeded two upload batches",
                )?;
                require(
                    number(&current.cpu, "committed") <= data.capabilities().limits.cpu_bytes,
                    "continuous CPU cap exceeded",
                )?;
                updates.push(serde_json::json!({"round":round,"revision":small,"latency_ms":latency_ms,
                    "producer_commits":commits.load(Ordering::Acquire),"scheduler":current.scheduler,
                    "legacy_uploaded":current.legacy["uploaded"],"cpu":current.cpu,"allocations":current.allocations}));
                std::fs::write(
                    control.root.join("streaming-progress.json"),
                    serde_json::to_vec_pretty(&updates)?,
                )?;
            }
            Ok(updates)
        })();
        stop.store(true, Ordering::Release);
        (
            peers,
            source
                .join()
                .map_err(|_| "continuous producer panicked".to_string())
                .and_then(|result| result),
        )
    });
    let updates = peers?;
    let source = source?;
    require(
        commits.load(Ordering::Acquire) >= 2,
        "continuous producer did not replace multiple generations",
    )?;
    let recovery = recovery::wait(
        control,
        data,
        &large_key,
        source,
        views,
        fixture.scans[0].source_count,
    )?;
    let result = serde_json::json!({"result":"pass","large_revision":source,"source_commits":commits.load(Ordering::Acquire),
        "peer_updates":updates,"recovery":recovery,"functional_latency_bound_ms":3000,"performance_acceptance":false});
    std::fs::write(
        control.root.join("streaming.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
