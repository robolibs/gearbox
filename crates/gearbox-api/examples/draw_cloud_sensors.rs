use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use gearbox_api::peerbus::Subscriber;
use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    CAMERA_COLOR, CameraFrame, Client, DataPipelineConfig, DataUpload, DataUploadPage,
    DrawDataClient, Env, IdentitySource, clock_op, next_sample, registry,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/typed-sensor-isolation";
#[path = "draw_cloud_sensors/fixture.rs"]
mod fixture;
#[path = "draw_cloud_sensors/legacy.rs"]
mod legacy;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn call(data: &mut DrawDataClient, mut request: DataRequest) -> Result<DataReply> {
    request.host_epoch = data.capabilities().host_epoch;
    let reply = data.request_with_deadline(&request, deadline())?;
    if reply.code != data_code::OK {
        return Err(format!(
            "API {}: {}",
            reply.code,
            String::from_utf8_lossy(&reply.message)
        )
        .into());
    }
    Ok(reply)
}

fn clear(data: &mut DrawDataClient, lease: u64) -> Result<()> {
    call(
        data,
        DataRequest {
            lease,
            ..DataRequest::new(data_op::CLEAR)
        }
        .with_metadata(&NamespaceFilter {
            namespace: NS.into(),
        })?,
    )?;
    Ok(())
}

fn camera_image(sub: &mut Subscriber<Env>, after: u32) -> Result<(u32, Vec<u8>)> {
    let until = deadline();
    let mut sample = 0;
    let mut pixels = vec![0; 160 * 120 * 4];
    let mut rows = [false; 120];
    while Instant::now() < until {
        let Some(frame) = next_sample::<CameraFrame>(sub, Duration::from_secs(2))? else {
            continue;
        };
        if frame.channel != CAMERA_COLOR || frame.sample <= after {
            continue;
        }
        if (frame.width, frame.height) != (160, 120) {
            return Err("unexpected sensor dimensions".into());
        }
        if sample != frame.sample {
            sample = frame.sample;
            rows.fill(false);
        }
        let start = frame.row_offset as usize;
        let end = start + frame.rows as usize;
        if end > 120 || frame.data.len() != (end - start) * 640 {
            return Err("malformed camera rows".into());
        }
        pixels[start * 640..end * 640].copy_from_slice(&frame.data);
        rows[start..end].fill(true);
        if rows.iter().all(|row| *row) {
            return Ok((sample, pixels));
        }
    }
    Err("no complete camera image within 60 seconds".into())
}

fn screenshot(bin: &str, instance: &str, out: &Path, stage: &str) -> Result<()> {
    let result = Command::new(bin)
        .args(["--instance", instance, "instance", "screenshot"])
        .arg(out.join(format!("viewer-{stage}.png")))
        .args(["--viewport", "--timeout", "30"])
        .status()?;
    if !result.success() {
        return Err("viewer screenshot failed".into());
    }
    Ok(())
}

fn visible(
    data: &mut DrawDataClient,
    key: &DataKey,
    revision: DatasetRevision,
) -> Result<GenerationVisibility> {
    let until = deadline();
    loop {
        let value = data.wait_visible_with_deadline(key, revision, None, until)?.visibility();
        let full = matches!(value, GenerationVisibility::Visible {
            source_samples, selected_items, disposition: FrontierDisposition::Drawn, ..
        } if source_samples == u64::from(fixture::POINTS) && selected_items == source_samples);
        if full {
            return Ok(value);
        }
        if Instant::now() >= until {
            return Err(format!("sensor fixture did not complete a fully drawn viewer frame: {value:?}").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn verify(
    client: &Client,
    instance: &str,
    machine: &str,
    link: &str,
    out: &Path,
    bin: &str,
) -> Result<()> {
    let mut frames = client.subscribe_env(&format!("/machines/{machine}/sensors/{link}"))?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let original = client.clock(clock_op::GET)?;
    let result = (|| -> Result<()> {
        let cleared = client.draw_clear("vfx-gallery/sensor-isolation")?;
        if !cleared.is_ok() {
            return Err(cleared.message().into());
        }
        clear(&mut data, writer.lease())?;
        client.clock(clock_op::PLAY)?;
        let (sample, _) = camera_image(&mut frames, 0)?;
        let (mut sample, baseline) = camera_image(&mut frames, sample + 3)?;
        client.clock(clock_op::PAUSE)?;
        if baseline
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|c| *c < 5))
        {
            return Err("sensor baseline is black".into());
        }
        std::fs::write(out.join("sensor-before.rgba"), &baseline)?;
        screenshot(bin, instance, out, "before")?;
        let fixture = fixture::make(&format!("{machine}/{link}"))?;
        let pages = [DataUploadPage {
            descriptor: &fixture.page,
            bytes: &fixture.bytes,
        }];
        let reply = writer.upload(
            &DataUpload::replace(&fixture.descriptor, &fixture.scans, &pages),
            deadline(),
            None,
        )?;
        let mut revision = DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        };
        let mut stages = Vec::new();
        for stage in ["opaque", "translucent", "octree", "surfels", "mesh", "volume", "elevation", "clear"] {
            if stage == "translucent" {
                let reply = call(
                    &mut data,
                    DataRequest {
                        lease: writer.lease(),
                        incarnation: revision.incarnation,
                        base_generation: revision.generation,
                        ..DataRequest::new(data_op::PATCH_PRESENTATION)
                    }
                    .with_key(&fixture.descriptor.key)?
                    .with_metadata(&fixture::presentation(0.55))?,
                )?;
                revision = DatasetRevision {
                    incarnation: reply.incarnation,
                    generation: reply.stored_generation,
                };
            }
            let receipt = if stage == "clear" {
                clear(&mut data, writer.lease())?;
                None
            } else if stage == "octree" {
                let (key, octree) = octree(&mut writer, &format!("{machine}/{link}"))?;
                Some(data.wait_visible_with_deadline(&key, octree, None, deadline())?.visibility())
            } else if matches!(stage, "surfels" | "mesh" | "volume" | "elevation") {
                let (key, revision) = surface(&mut writer, &format!("{machine}/{link}"), stage)?;
                Some(data.wait_visible_with_deadline(&key, revision, None, deadline())?.visibility())
            } else {
                Some(visible(&mut data, &fixture.descriptor.key, revision)?)
            };
            client.clock(clock_op::PLAY)?;
            let previous_sample = sample;
            let (new_sample, pixels) = camera_image(&mut frames, sample + 3)?;
            sample = new_sample;
            client.clock(clock_op::PAUSE)?;
            std::fs::write(out.join(format!("sensor-{stage}.rgba")), &pixels)?;
            screenshot(bin, instance, out, stage)?;
            let delta: u64 = baseline
                .iter()
                .zip(&pixels)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            let mean = delta as f64 / baseline.len() as f64;
            let magenta = pixels
                .chunks_exact(4)
                .filter(|p| p[0] > 150 && p[2] > 150 && p[1] < 80)
                .count();
            let report = serde_json::json!({"stage":stage,"sample":sample,"after_sample":previous_sample,
                "mean_channel_delta":mean,"magenta_pixels":magenta,"visibility":receipt});
            println!("{report}");
            stages.push(report);
            std::fs::write(
                out.join("sensor-metrics.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "link":link,"backend":"typed-cloud","points":fixture::POINTS,"stages":stages
                }))?,
            )?;
            if mean > 1.0 || magenta > 10 {
                return Err(format!("typed cloud contaminated {link} at {stage}").into());
            }
        }
        Ok(())
    })();
    let cleanup = clear(&mut data, writer.lease());
    let closed = writer.close(deadline());
    let restored = client.clock(if original.paused != 0 {
        clock_op::PAUSE
    } else {
        clock_op::PLAY
    });
    result?;
    cleanup?;
    closed?;
    restored?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 5 || args.len() == 6 && args[5] == "legacy") || !args[0].starts_with("draw-vfx-sensors-") {
        return Err("provide owned SENSOR_INSTANCE MACHINE LINK OUTPUT BIN [legacy]".into());
    }
    let entry = registry::find(&args[0]).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "typed-cloud-sensors")?;
    if args.len() == 6 {
        return legacy::verify(&client, &args[0], &args[1], &args[2], Path::new(&args[3]), &args[4]);
    }
    verify(
        &client,
        &args[0],
        &args[1],
        &args[2],
        Path::new(&args[3]),
        &args[4],
    )
}

/// A magenta occupancy wall of 0.4 m cells where the cloud wall stands, on the live camera link.
fn octree(writer: &mut gearbox_api::DrawDataWriter<'_>, frame: &str) -> Result<(DataKey, DatasetRevision)> {
    let config = OctreeConfig { leaf_resolution_m: 0.05, max_depth: 7, free_max: 0.3, occupied_min: 0.7 };
    let leaves = (0..15i128)
        .flat_map(|y| (0..10i128).map(move |z| [0, y, z]))
        .map(|cell| {
            Ok(OccupancyLeaf {
                key: VoxelKey::from_grid_cell(cell.map(|c| c << 3), 4, &config)?,
                value: OccupancyValue { probability: Some(0.95), confidence: None, class_id: None },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes = encode_occupancy_page(&config, &leaves)?;
    let page = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: 0 },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: leaves.len() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "octree")?,
        shape: DataShape::Octree {
            config,
            frame: DataFrame { frame: frame.into(), origin_m: [2.0, -3.0, -2.0], ..Default::default() },
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Occupancy {
                occupied: [255, 0, 255, 255],
                free: [255, 0, 255, 60],
                uncertain: [255, 0, 255, 200],
                unknown: [255, 0, 255, 80],
                mixed: [255, 0, 255, 160],
            },
            ..Presentation::for_octree()
        },
    };
    let pages = [DataUploadPage { descriptor: &page, bytes: &bytes }];
    let reply = writer.upload(&DataUpload::replace(&descriptor, &[], &pages), deadline(), None)?;
    Ok((descriptor.key, DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation }))
}

/// A magenta surfel wall, mesh wall or volume slice where the cloud wall stands, on the live camera link.
fn surface(writer: &mut gearbox_api::DrawDataWriter<'_>, frame: &str, kind: &str) -> Result<(DataKey, DatasetRevision)> {
    use gearbox_api::wire::draw_data::mesh::{MeshBlock, encode_mesh_block};
    use gearbox_api::wire::draw_data::volume::{VolumeBlock, encode_volume_block};
    let magenta = ColorMode::Constant { rgba: [255, 0, 255, 255] };
    let framed = DataFrame { frame: frame.into(), origin_m: [2.0, -3.0, -2.0], ..Default::default() };
    let attribute = |name: &str, scalar, components, semantic, units: &str| Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization: Normalization::None,
        nullable: false,
    };
    let page = |encoding, page_id, items, bytes: &[u8]| PageDescriptor {
        address: PageAddress { scan_key: 0, page_id },
        encoding,
        items,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(bytes),
        origin_m: [0.0; 3],
    };
    let (shape, pages, presentation) = match kind {
        "surfels" => {
            let schema = CloudSchema {
                revision: SCHEMA_REVISION,
                attributes: vec![
                    attribute("position", ScalarType::F32, 3, AttributeSemantic::Position, "m"),
                    attribute("id", ScalarType::U64, 1, AttributeSemantic::SourceId, ""),
                    attribute("normal", ScalarType::F32, 3, AttributeSemantic::Normal, ""),
                    attribute("radius", ScalarType::F32, 1, AttributeSemantic::Radius, "m"),
                ],
            };
            let mut columns = vec![ColumnData { values: vec![], validity: vec![] }; 4];
            let mut count = 0u32;
            for y in 0..20 {
                for z in 0..14 {
                    columns[0].values.extend([0.0f32, y as f32 * 0.3, z as f32 * 0.3].iter().flat_map(|v| v.to_le_bytes()));
                    columns[1].values.extend(u64::from(count).to_le_bytes());
                    columns[2].values.extend([1.0f32, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()));
                    columns[3].values.extend(0.22f32.to_le_bytes());
                    count += 1;
                }
            }
            let bytes = CloudPage { count, columns }.encode(&schema)?;
            let page = page(PageEncoding::CloudColumnsV1, 0, count, &bytes);
            (DataShape::Surfels { schema, frame: framed }, vec![(page, bytes)], Presentation { color: magenta, ..Default::default() })
        }
        "mesh" => {
            let block = MeshBlock {
                positions: vec![[0.0, 0.0, 0.0], [0.0, 6.0, 0.0], [0.0, 6.0, 4.0], [0.0, 0.0, 4.0]],
                indices: vec![0, 1, 2, 0, 2, 3],
                ..MeshBlock::default()
            };
            let bytes = encode_mesh_block(&block)?;
            let page = page(PageEncoding::MeshBlockV1, 0, 2, &bytes);
            (DataShape::MeshBlocks { frame: framed }, vec![(page, bytes)], Presentation { color: magenta, ..Default::default() })
        }
        "elevation" => {
            use gearbox_api::wire::draw_data::elevation::encode_elevation_tile;
            let grid = ElevationGrid { cell_m: 0.25, tile_side: 16, layers: vec![] };
            let mut pages = Vec::new();
            for key in [[0, 0], [0, 1]] {
                let tile = ElevationTile { key, layer: 0, values: vec![0.0; 256], valid: vec![true; 256] };
                let bytes = encode_elevation_tile(&grid, &tile)?;
                pages.push((page(PageEncoding::ElevationTileV1, elevation_page_id(key, 0).ok_or("key")?, 256, &bytes), bytes));
            }
            // The map's ground plane turned upright: map x up the link's z, elevation along its x.
            let upright = DataFrame { pose: RigidPose { translation_m: [0.0; 3], rotation_wxyz: [0.5f64.sqrt(), 0.0, -(0.5f64.sqrt()), 0.0] }, ..framed };
            (DataShape::Elevation { grid, frame: upright }, pages, Presentation { color: magenta, ..Default::default() })
        }
        _ => {
            let field = VolumeField { kind: FieldKind::Esdf { signed: true }, voxel_m: 0.1, block_side: 8, colors: false };
            let mut pages = Vec::new();
            for x in -1..=0 {
                for y in 0..8 {
                    for z in 0..5 {
                        let block = VolumeBlock { key: [x, y, z], values: vec![0.5; 512], valid: vec![true; 512], weights: None, colors: None };
                        let bytes = encode_volume_block(&field, &block)?;
                        pages.push((page(PageEncoding::VolumeBlockV1, volume_page_id([x, y, z]).ok_or("key")?, 512, &bytes), bytes));
                    }
                }
            }
            let view = VolumeView {
                slices: vec![VolumeSlice::across(0, [0.0, 3.0, 2.0], [6.0, 4.0], 0.1)],
                interpolation: Interpolation::Trilinear,
                band: None,
                contours: vec![],
                contour_rgba: [255; 4],
                unknown_rgba: [255, 0, 255, 255],
                frame_rgba: None,
            };
            (DataShape::Volume { field, frame: framed }, pages, Presentation { color: magenta, volume: Some(view), ..Default::default() })
        }
    };
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, kind)?,
        shape,
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation,
    };
    let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    let reply = writer.upload(&DataUpload::replace(&descriptor, &[], &borrowed), deadline(), None)?;
    Ok((descriptor.key, DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation }))
}
