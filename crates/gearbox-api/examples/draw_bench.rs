//! Scenario writer for `bench-draw-data`. Each command prints JSON events with
//! wall-clock `unix_s`, which the analyzer lines up with the host's per-frame log.
//!
//! Usage: `draw_bench INSTANCE COMMAND [ARGS]`:
//! - `cloud N`: a static N-point cloud; commit and retained-to-visible times.
//! - `live N HZ SECONDS`: an N-point latest cloud replaced at HZ; per-update latency.
//! - `octree N`: N finest occupancy leaves in 4096-leaf pages, then a one-page delta.
//! - `sparse` / `restyle`: a one-page delta / a presentation patch of `cloud`.
//! - `cycles N POINTS`: N load/clear cycles of a POINTS cloud.
//! - `reject SMALL BIG`: keep SMALL visible, try BIG; the old generation must survive.
//! - `status ID [NS]`: the dataset's status, including paging.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataClient, DrawDataPipeline,
    IdentitySource, colormap, registry,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-bench";
const PAGE: usize = 65_536;
const SIDE: usize = 256;
const SPACING: f64 = 0.1;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(120)
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn event(name: &str, fields: serde_json::Value) {
    let mut value = serde_json::json!({ "event": name, "unix_s": now() });
    if let (Some(out), serde_json::Value::Object(more)) = (value.as_object_mut(), fields) {
        out.extend(more);
    }
    println!("{value}");
}

fn schema() -> CloudSchema {
    let field = |name: &str, scalar, components, semantic, units: &str| Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization: Normalization::None,
        nullable: false,
    };
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            field("position", ScalarType::F32, 3, AttributeSemantic::Position, "m"),
            field("id", ScalarType::U64, 1, AttributeSemantic::SourceId, ""),
            field("intensity", ScalarType::F32, 1, AttributeSemantic::Intensity, ""),
        ],
    }
}

/// Page `page` of an N-point field of 25.6 m tiles, four tiles a row.
fn page(page: usize, count: usize, phase: f32) -> Result<(PageDescriptor, Vec<u8>)> {
    let mut columns = vec![ColumnData { values: Vec::new(), validity: Vec::new() }; 3];
    for j in 0..count {
        let (x, y) = ((j % SIDE) as f32 * SPACING as f32, (j / SIDE) as f32 * SPACING as f32);
        let z = 1.0 + 0.4 * (x * 0.31 + phase).sin() * (y * 0.23).cos();
        columns[0].values.extend([x, y, z].iter().flat_map(|v| v.to_le_bytes()));
        columns[1].values.extend(((page * PAGE + j) as u64).to_le_bytes());
        columns[2].values.extend((z - 0.6).to_le_bytes());
    }
    let bytes = CloudPage { count: count as u32, columns }.encode(&schema())?;
    let tile = SIDE as f64 * SPACING;
    Ok((
        PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: page as u64 },
            encoding: PageEncoding::CloudColumnsV1,
            items: count as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [(page % 4) as f64 * tile - 2.0 * tile, (page / 4) as f64 * tile, 0.0],
        },
        bytes,
    ))
}

fn pages(points: usize, phase: f32) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    (0..points.div_ceil(PAGE)).map(|p| page(p, PAGE.min(points - p * PAGE), phase)).collect()
}

fn descriptor(id: &str, stream: StreamMode) -> Result<DataDescriptor> {
    Ok(DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, id)?,
        shape: DataShape::Cloud { schema: schema() },
        stream,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Scalar { name: "intensity".into(), colormap: colormap::TURBO, range: [0.0, 1.0], missing: [0; 4] },
            size: PointSize::Pixels { radius: 2.0 },
            ..Default::default()
        },
    })
}

fn scan(key: u64, count: u64, capture: Option<f64>) -> ScanDescriptor {
    let stamp = capture.map(|t| CaptureStamp {
        simulation_time_s: t,
        source_time_ns: None,
        source_clock: None,
        sample: key,
        scene_epoch: 1,
    });
    ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: key,
        source: Some("bench".into()),
        frame: DataFrame { capture: stamp, ..Default::default() },
        source_count: count,
        organization: None,
    }
}

struct Bench {
    data: DrawDataClient,
    pipeline: DrawDataPipeline,
}

impl Bench {
    fn upload(
        &mut self,
        descriptor: &DataDescriptor,
        scans: &[ScanDescriptor],
        pages: &[(PageDescriptor, Vec<u8>)],
        mode: CommitMode,
        base: Option<DatasetRevision>,
    ) -> Result<DatasetRevision> {
        let mut writer = self.pipeline.writer(4, deadline())?;
        let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
        let mut upload = DataUpload::replace(descriptor, scans, &borrowed);
        upload.base = base;
        if base.is_some() {
            upload.mode = mode;
        }
        let reply = writer.upload(&upload, deadline(), None);
        let _ = writer.close(deadline());
        let reply = reply?;
        Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
    }

    fn visible(&mut self, key: &DataKey, revision: DatasetRevision, until: Instant) -> Option<GenerationVisibility> {
        let completion = self.data.wait_visible_with_deadline(key, revision, None, until).ok()?;
        Some(completion.visibility())
    }

    fn current(&mut self, key: &DataKey) -> Result<Option<DatasetRevision>> {
        let status = self.status(key)?;
        Ok(status.and_then(|s| s.get("retained").and_then(|r| serde_json::from_value(r.clone()).ok())))
    }

    fn status(&mut self, key: &DataKey) -> Result<Option<serde_json::Value>> {
        let mut request = DataRequest::new(data_op::STATUS)
            .with_key(key)?
            .with_metadata(&DataStatusQuery { generation: None, view: None })?;
        request.host_epoch = self.data.capabilities().host_epoch;
        let reply = self.data.request_with_deadline(&request, deadline())?;
        if reply.code != data_code::OK {
            return Ok(None);
        }
        Ok(serde_json::from_slice(&reply.metadata).ok())
    }

    fn clear(&mut self) -> Result<()> {
        let writer = self.pipeline.writer(4, deadline())?;
        let mut request = DataRequest { lease: writer.lease(), ..DataRequest::new(data_op::CLEAR) }
            .with_metadata(&NamespaceFilter { namespace: NS.into() })?;
        request.host_epoch = self.data.capabilities().host_epoch;
        self.data.request_with_deadline(&request, deadline())?;
        Ok(())
    }

    fn static_cloud(&mut self, id: &str, points: usize, base: Option<DatasetRevision>) -> Result<(DataDescriptor, DatasetRevision)> {
        let descriptor = descriptor(id, StreamMode::RetainedMap)?;
        let pages = pages(points, 0.0)?;
        let started = Instant::now();
        // A host whose in-flight cap cannot hold the whole cloud receives it in deltas.
        let cap = (self.data.capabilities().limits.writer_incomplete_bytes / 2).max(1) as usize;
        let per = (cap / pages[0].1.len().max(1)).clamp(1, pages.len());
        let (mut revision, mut sent) = (base, 0usize);
        for (step, group) in pages.chunks(per).enumerate() {
            sent += group.iter().map(|(d, _)| d.items as usize).sum::<usize>();
            let mode = if step == 0 { CommitMode::Replace } else { CommitMode::Delta };
            revision = Some(self.upload(&descriptor, &[scan(0, sent as u64, None)], group, mode, revision)?);
        }
        let revision = revision.ok_or("empty cloud")?;
        let committed = Instant::now();
        event("committed", serde_json::json!({ "id": id, "points": points, "pages": pages.len(), "transactions": pages.len().div_ceil(per), "upload_ms": started.elapsed().as_secs_f64() * 1e3, "revision": revision }));
        let seen = self.visible(&descriptor.key, revision, deadline());
        event("visible", serde_json::json!({ "id": id, "retained_to_visible_ms": committed.elapsed().as_secs_f64() * 1e3, "visibility": seen }));
        Ok((descriptor, revision))
    }
}

fn octree_page(config: &OctreeConfig, id: usize, leaves: &[OccupancyLeaf]) -> Result<(PageDescriptor, Vec<u8>)> {
    let bytes = encode_occupancy_page(config, leaves)?;
    Ok((
        PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: id as u64 },
            encoding: PageEncoding::OccupancyLeavesV1,
            items: leaves.len() as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        },
        bytes,
    ))
}

fn octree(bench: &mut Bench, count: usize) -> Result<()> {
    let config = OctreeConfig { leaf_resolution_m: 0.1, max_depth: 8, free_max: 0.3, occupied_min: 0.7 };
    let side = 128i128;
    let leaves: Vec<OccupancyLeaf> = (0..count as i128)
        .map(|i| {
            let cell = [i % side, (i / side) % side, i / (side * side)];
            let p = if (cell[0] / 8 + cell[1] / 8 + cell[2]) % 3 == 0 { 0.9 } else { 0.1 };
            Ok(OccupancyLeaf {
                key: VoxelKey::from_grid_cell(cell, config.max_depth, &config)?,
                value: OccupancyValue { probability: Some(p), confidence: None, class_id: None },
            })
        })
        .collect::<Result<_>>()?;
    let pages: Vec<_> = leaves.chunks(4096).enumerate().map(|(id, c)| octree_page(&config, id, c)).collect::<Result<_>>()?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "octree")?,
        shape: DataShape::Octree { config, frame: DataFrame { origin_m: [-30.0, 60.0, 2.0], ..Default::default() } },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation::for_octree(),
    };
    let started = Instant::now();
    let revision = bench.upload(&descriptor, &[], &pages, CommitMode::Replace, None)?;
    let committed = Instant::now();
    event("committed", serde_json::json!({ "id": "octree", "leaves": count, "pages": pages.len(), "upload_ms": started.elapsed().as_secs_f64() * 1e3 }));
    let seen = bench.visible(&descriptor.key, revision, deadline());
    event("visible", serde_json::json!({ "id": "octree", "retained_to_visible_ms": committed.elapsed().as_secs_f64() * 1e3, "visibility": seen }));
    std::thread::sleep(Duration::from_secs(5));
    let mut flipped = decode_occupancy_page(&config, &pages[0].1)?;
    for leaf in &mut flipped {
        leaf.value.probability = leaf.value.probability.map(|p| 1.0 - p);
    }
    let delta = [octree_page(&config, 0, &flipped)?];
    event("delta_begin", serde_json::json!({ "id": "octree", "pages": 1, "total_pages": pages.len(), "page_bytes": delta[0].1.len() }));
    let started = Instant::now();
    let changed = bench.upload(&descriptor, &[], &delta, CommitMode::Delta, Some(revision))?;
    let seen = bench.visible(&descriptor.key, changed, deadline());
    event("delta", serde_json::json!({ "id": "octree", "commit_to_visible_ms": started.elapsed().as_secs_f64() * 1e3, "visibility": seen }));
    Ok(())
}

fn live(bench: &mut Bench, points: usize, hz: f64, seconds: f64) -> Result<()> {
    let descriptor = descriptor("live", StreamMode::Latest)?;
    let period = Duration::from_secs_f64(1.0 / hz);
    let started = Instant::now();
    let mut base = None;
    let (mut updates, mut missed) = (0u64, 0u64);
    while started.elapsed().as_secs_f64() < seconds {
        let tick = Instant::now();
        let k = updates + 1;
        let mut page = pages(points, k as f32 * 0.1)?;
        for (descriptor, _) in &mut page {
            descriptor.address.scan_key = k;
        }
        let revision = bench.upload(&descriptor, &[scan(k, points as u64, Some(k as f64 / hz))], &page, CommitMode::Replace, base)?;
        base = Some(revision);
        let committed = Instant::now();
        let seen = bench.visible(&descriptor.key, revision, tick + period * 3);
        let latency = seen.as_ref().map(|_| committed.elapsed().as_secs_f64() * 1e3);
        missed += u64::from(latency.is_none());
        event("update", serde_json::json!({ "generation": revision.generation, "commit_ms": committed.duration_since(tick).as_secs_f64() * 1e3, "stored_to_visible_ms": latency }));
        updates += 1;
        std::thread::sleep(period.saturating_sub(tick.elapsed()));
    }
    event("live_done", serde_json::json!({ "updates": updates, "not_visible_within_three_periods": missed }));
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [instance, command, rest @ ..] = args.as_slice() else {
        return Err("usage: draw_bench INSTANCE COMMAND [ARGS]".into());
    };
    let number = |i: usize| -> Result<usize> { Ok(rest.get(i).ok_or("missing argument")?.parse()?) };
    let entry = registry::find(instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "draw-bench")?;
    let data = client.draw_data()?;
    let queue = data.capabilities().limits.client_queue_bytes;
    let small = queue < 32 << 20;
    let config = DataPipelineConfig {
        max_bytes: (queue as usize).min(32 << 20),
        parallel_requests: if small { 2 } else { 4 },
        max_operations: if small { 16 } else { 64 },
        ..DataPipelineConfig::default()
    };
    let mut bench = Bench { data, pipeline: client.draw_data_pipeline(config, deadline())? };
    match command.as_str() {
        "cloud" => {
            bench.static_cloud("cloud", number(0)?, None)?;
        }
        "live" => live(&mut bench, number(0)?, rest.get(1).ok_or("hz")?.parse()?, rest.get(2).ok_or("seconds")?.parse()?)?,
        "octree" => octree(&mut bench, number(0)?)?,
        "sparse" => {
            let key = DataKey::new(NS, "cloud")?;
            let revision = bench.current(&key)?.ok_or("no cloud to edit")?;
            let total = number(0)?;
            let edit = [page(0, PAGE.min(total), 1.7)?];
            event("delta_begin", serde_json::json!({ "id": "cloud", "pages": 1, "total_pages": total.div_ceil(PAGE), "page_bytes": edit[0].1.len() }));
            let started = Instant::now();
            let changed = bench.upload(&descriptor("cloud", StreamMode::RetainedMap)?, &[], &edit, CommitMode::Delta, Some(revision))?;
            let seen = bench.visible(&key, changed, deadline());
            event("delta", serde_json::json!({ "id": "cloud", "commit_to_visible_ms": started.elapsed().as_secs_f64() * 1e3, "visibility": seen }));
        }
        "restyle" => {
            let key = DataKey::new(NS, "cloud")?;
            let revision = bench.current(&key)?.ok_or("no cloud to restyle")?;
            let writer = bench.pipeline.writer(4, deadline())?;
            let presentation = Presentation {
                color: ColorMode::Scalar { name: "intensity".into(), colormap: colormap::VIRIDIS, range: [0.0, 1.0], missing: [0; 4] },
                glow: 0.3,
                ..descriptor("cloud", StreamMode::RetainedMap)?.presentation
            };
            let mut request = DataRequest {
                lease: writer.lease(),
                incarnation: revision.incarnation,
                base_generation: revision.generation,
                ..DataRequest::new(data_op::PATCH_PRESENTATION)
            }
            .with_key(&key)?
            .with_metadata(&presentation)?;
            request.host_epoch = bench.data.capabilities().host_epoch;
            event("restyle_begin", serde_json::json!({ "id": "cloud" }));
            let reply = bench.data.request_with_deadline(&request, deadline())?;
            event("restyled", serde_json::json!({ "code": reply.code, "generation": reply.stored_generation }));
        }
        "cycles" => {
            let (cycles, points) = (number(0)?, number(1)?);
            for cycle in 0..cycles {
                bench.static_cloud("cycle", points, None)?;
                event("loaded", serde_json::json!({ "cycle": cycle }));
                bench.clear()?;
                std::thread::sleep(Duration::from_millis(1500));
                event("cleared", serde_json::json!({ "cycle": cycle }));
            }
        }
        "reject" => {
            let (small, big) = (number(0)?, number(1)?);
            let (_, kept) = bench.static_cloud("guarded", small, None)?;
            let descriptor = descriptor("guarded", StreamMode::RetainedMap)?;
            let attempt = bench.upload(&descriptor, &[scan(0, big as u64, None)], &pages(big, 0.0)?, CommitMode::Replace, Some(kept));
            let retained = bench.current(&descriptor.key)?;
            let still = bench.visible(&descriptor.key, kept, Instant::now() + Duration::from_secs(10));
            event("reject", serde_json::json!({
                "big_points": big,
                "big_accepted": attempt.is_ok(),
                "big_error": attempt.err().map(|e| e.to_string()),
                "retained": retained,
                "previous_generation_retained": retained == Some(kept),
                "previous_generation_visible": still.is_some(),
            }));
        }
        "status" => {
            let ns = rest.get(1).map_or(NS, String::as_str);
            let key = DataKey::new(ns, rest.first().ok_or("id")?)?;
            event("status", serde_json::json!({ "status": bench.status(&key)? }));
        }
        "clear" => bench.clear()?,
        other => return Err(format!("unknown command {other}").into()),
    }
    Ok(())
}
