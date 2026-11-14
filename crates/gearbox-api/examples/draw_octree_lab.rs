//! Typed occupancy octree lab: occupied wall, observed-free corridor, unknown gap,
//! mixed resolutions, a live split, two caller palettes and depth-limited aggregates.

use std::time::{Duration, Instant};

use gearbox_api::wire::{draw::rgba, draw_data::*, draw_widgets::Place};
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawEntity as E, IdentitySource,
    Instance, draw_flag, draw_style, registry, shape_kind,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/plan/octree";

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn require(value: bool, reason: &str) -> Result<()> {
    if value { Ok(()) } else { Err(reason.into()) }
}

pub fn config() -> OctreeConfig {
    OctreeConfig {
        leaf_resolution_m: 0.1,
        max_depth: 8,
        free_max: 0.3,
        occupied_min: 0.7,
    }
}

/// Key of the depth-`depth` cell whose integer cell coordinate is `cell`.
pub fn key(config: &OctreeConfig, depth: u8, cell: [i64; 3]) -> VoxelKey {
    let per_tile = 1i64 << depth;
    let tile = cell.map(|c| c.div_euclid(per_tile));
    let local = cell.map(|c| c.rem_euclid(per_tile) as u64);
    let mut morton = 0u64;
    for level in (0..u32::from(depth)).rev() {
        let child =
            (local[0] >> level & 1) | (local[1] >> level & 1) << 1 | (local[2] >> level & 1) << 2;
        morton = morton << 3 | child;
    }
    let _ = config;
    VoxelKey {
        tile,
        depth,
        morton,
    }
}

fn leaf(
    config: &OctreeConfig,
    depth: u8,
    cell: [i64; 3],
    probability: Option<f32>,
) -> OccupancyLeaf {
    OccupancyLeaf {
        key: key(config, depth, cell),
        value: OccupancyValue {
            probability,
            confidence: None,
            class_id: None,
        },
    }
}

/// Wall cells covered by the split page; the rest of the map sits in page 0.
pub const SPLIT: [i64; 3] = [6, 6, 1];

pub fn base_leaves(config: &OctreeConfig) -> Vec<OccupancyLeaf> {
    let mut leaves = Vec::new();
    for x in 0..8 {
        for z in 0..3 {
            let door = (3..5).contains(&x) && z < 2;
            if !door && [x, 6, z] != SPLIT {
                leaves.push(leaf(config, 5, [x, 6, z], Some(0.92)));
            }
        }
    }
    for x in 0..4 {
        for y in 1..3 {
            leaves.push(leaf(config, 4, [x, y, 0], Some(0.08)));
        }
    }
    for z in 0..10 {
        leaves.push(leaf(config, 7, [2, 30, z], Some(0.97)));
    }
    for x in [0, 1, 7] {
        leaves.push(leaf(config, 6, [x * 2, 14, 0], Some(0.5)));
    }
    leaves.push(leaf(config, 5, [7, 8, 0], None));
    leaves
}

pub fn split_parent(config: &OctreeConfig) -> Vec<OccupancyLeaf> {
    vec![leaf(config, 5, SPLIT, Some(0.9))]
}

pub fn split_children(config: &OctreeConfig) -> Vec<OccupancyLeaf> {
    (0..8)
        .map(|child: i64| {
            let cell = [
                SPLIT[0] * 2 + (child & 1),
                SPLIT[1] * 2 + (child >> 1 & 1),
                SPLIT[2] * 2 + (child >> 2 & 1),
            ];
            leaf(
                config,
                6,
                cell,
                Some(if child % 3 == 0 { 0.12 } else { 0.95 }),
            )
        })
        .collect()
}

fn page(
    config: &OctreeConfig,
    id: u64,
    leaves: &[OccupancyLeaf],
) -> Result<(PageDescriptor, Vec<u8>)> {
    let bytes = encode_occupancy_page(config, leaves)?;
    Ok((
        PageDescriptor {
            address: PageAddress {
                scan_key: 0,
                page_id: id,
            },
            encoding: PageEncoding::OccupancyLeavesV1,
            items: leaves.len() as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        },
        bytes,
    ))
}

struct Bay {
    id: &'static str,
    origin: [f64; 3],
    title: &'static str,
    detail: &'static str,
    presentation: Presentation,
}

fn bays() -> Vec<Bay> {
    let roi = ClipBounds {
        min_m: [2.4, 4.8, 0.0],
        max_m: [4.0, 5.6, 1.6],
        ..Default::default()
    };
    let display = OctreeDisplay {
        show_free: true,
        show_unknown: true,
        roi: Some(roi),
        unknown_cell_budget: 2_000,
        ..OctreeDisplay::default()
    };
    vec![
        Bay {
            id: "occupancy",
            origin: [52.0, -12.0, 0.0],
            title: "01 / OCCUPANCY STATES",
            detail: "WALL / FREE CORRIDOR / UNKNOWN GAP / LIVE SPLIT",
            presentation: Presentation {
                octree: Some(display.clone()),
                glow: 0.6,
                opacity: 0.65,
                ..Presentation::for_octree()
            },
        },
        Bay {
            id: "palette",
            origin: [52.0, 4.0, 0.0],
            title: "02 / CALLER PALETTE",
            detail: "SURFACE STYLE / SAME LEAVES",
            presentation: Presentation {
                color: ColorMode::Occupancy {
                    occupied: [70, 150, 235, 255],
                    free: [235, 235, 245, 70],
                    uncertain: [240, 110, 160, 200],
                    unknown: [120, 120, 140, 90],
                    mixed: [150, 120, 230, 170],
                },
                octree: Some(OctreeDisplay {
                    style: VoxelStyle::Surface,
                    ..display.clone()
                }),
                glow: 0.6,
                opacity: 0.65,
                ..Presentation::for_octree()
            },
        },
        Bay {
            id: "aggregate",
            origin: [70.0, -4.0, 0.0],
            title: "03 / DEPTH-3 AGGREGATES",
            detail: "OCCUPIED IF ANY / FREE ONLY IF FULLY OBSERVED",
            presentation: Presentation {
                octree: Some(OctreeDisplay {
                    depth_limit: Some(3),
                    style: VoxelStyle::Wire,
                    show_free: true,
                    ..OctreeDisplay::default()
                }),
                glow: 0.6,
                opacity: 0.65,
                ..Presentation::for_octree()
            },
        },
    ]
}

fn descriptor(config: &OctreeConfig, bay: &Bay) -> Result<DataDescriptor> {
    Ok(DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, bay.id)?,
        shape: DataShape::Octree {
            config: *config,
            frame: DataFrame {
                origin_m: bay.origin,
                ground_radius_m: Some(9.7),
                ..Default::default()
            },
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: bay.presentation.clone(),
    })
}

fn annotations(client: &Client, bay: &Bay) -> Result<()> {
    let place = Place::default();
    let tint = rgba(230, 150, 60, 255);
    let [x, y, _] = bay.origin.map(|v| v as f32);
    let ns = format!("{NS}/{}-labels", bay.id);
    for object in [
        place
            .put(E::text(&ns, "title", &[[x + 3.2, y + 7.0, 4.2]], bay.title))
            .with_style(draw_style::LABEL)
            .with_width(-17.0)
            .with_color(rgba(225, 235, 242, 255))
            .with_prop("accent", &tint.to_string())
            .with_prop("label_category", "TYPED OCTREE"),
        place
            .put(E::text(
                &ns,
                "detail",
                &[[x + 3.2, y - 0.6, 0.3]],
                bay.detail,
            ))
            .with_style(draw_style::LABEL)
            .with_width(-12.0)
            .with_color(rgba(225, 235, 242, 255))
            .with_prop("accent", &tint.to_string()),
        place
            .put(E::shapes(
                &ns,
                "bounds",
                shape_kind::BOX,
                &[Instance::at([x + 3.2, y + 3.6, 1.4], [8.0, 9.2, 2.8])],
            ))
            .with_style(draw_style::HOLO)
            .with_fill(draw_flag::FILL_WIRE)
            .with_width(-0.8)
            .with_color(tint)
            .with_glow(0.15),
    ] {
        let result = client.draw(&object)?;
        require(result.is_ok(), &result.message())?;
    }
    Ok(())
}

/// Stress map of `count` finest cells in 4096-leaf pages; returns pages and one replacement.
fn stress_pages(config: &OctreeConfig, count: usize) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    let side = 128i64;
    let leaves: Vec<_> = (0..count as i64)
        .map(|i| {
            let cell = [i % side, (i / side) % side, i / (side * side)];
            let p = if (cell[0] / 8 + cell[1] / 8 + cell[2]) % 3 == 0 {
                0.9
            } else {
                0.1
            };
            leaf(config, config.max_depth, cell, Some(p))
        })
        .collect();
    leaves
        .chunks(4096)
        .enumerate()
        .map(|(id, chunk)| page(config, id as u64, chunk))
        .collect()
}

fn stress(
    writer: &mut gearbox_api::DrawDataWriter<'_>,
    config: &OctreeConfig,
    count: usize,
) -> Result<serde_json::Value> {
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "stress")?,
        shape: DataShape::Octree {
            config: *config,
            frame: DataFrame {
                origin_m: [52.0, 22.0, 2.0],
                ..Default::default()
            },
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            opacity: std::env::var("GEARBOX_OCTREE_OPACITY").ok().and_then(|v| v.parse().ok()).unwrap_or(0.65),
            glow: 0.6,
            ..Presentation::for_octree()
        },
    };
    let pages = stress_pages(config, count)?;
    let upload: Vec<_> = pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    let started = Instant::now();
    let reply = writer.upload(
        &DataUpload::replace(&descriptor, &[], &upload),
        deadline(),
        None,
    )?;
    let replace_ms = started.elapsed().as_secs_f64() * 1e3;
    let base = DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    };
    let (first, bytes) = &pages[0];
    let mut flipped = decode_occupancy_page(config, bytes)?;
    for leaf in &mut flipped {
        leaf.value.probability = leaf.value.probability.map(|p| 1.0 - p);
    }
    let replacement = page(config, first.address.page_id, &flipped)?;
    let started = Instant::now();
    writer.upload(
        &DataUpload {
            descriptor: &descriptor,
            mode: CommitMode::Delta,
            base: Some(base),
            scans: &[],
            pages: &[DataUploadPage {
                descriptor: &replacement.0,
                bytes: &replacement.1,
            }],
            removed_pages: &[],
            removed_scans: &[],
        },
        deadline(),
        None,
    )?;
    let delta_ms = started.elapsed().as_secs_f64() * 1e3;
    Ok(serde_json::json!({
        "leaves": count,
        "pages": pages.len(),
        "replace_ms": replace_ms,
        "delta_leaves": flipped.len(),
        "delta_ms": delta_ms,
    }))
}

fn main() -> Result<()> {
    let instance = std::env::args().nth(1).ok_or("provide INSTANCE")?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "typed-octree-lab")?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let config = config();
    let base = page(&config, 0, &base_leaves(&config))?;
    let parent = page(&config, 1, &split_parent(&config))?;
    let children = page(&config, 2, &split_children(&config))?;
    let cleared = client.draw_clear(NS)?;
    require(cleared.is_ok(), &cleared.message())?;
    let mut revisions = Vec::new();
    for bay in bays() {
        let descriptor = descriptor(&config, &bay)?;
        let pages = [
            DataUploadPage {
                descriptor: &base.0,
                bytes: &base.1,
            },
            DataUploadPage {
                descriptor: &parent.0,
                bytes: &parent.1,
            },
        ];
        let reply = writer.upload(
            &DataUpload::replace(&descriptor, &[], &pages),
            deadline(),
            None,
        )?;
        let revision = DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        };
        annotations(&client, &bay)?;
        revisions.push((descriptor, revision));
    }
    let split_after = std::env::var("GEARBOX_OCTREE_SPLIT_S")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(3.0);
    std::thread::sleep(Duration::from_secs_f64(split_after));
    let (descriptor, revision) = &revisions[0];
    let removed = [parent.0.address];
    let pages = [DataUploadPage {
        descriptor: &children.0,
        bytes: &children.1,
    }];
    let reply = writer.upload(
        &DataUpload {
            descriptor,
            mode: CommitMode::Delta,
            base: Some(*revision),
            scans: &[],
            pages: &pages,
            removed_pages: &removed,
            removed_scans: &[],
        },
        deadline(),
        None,
    )?;
    let stress = match std::env::var("GEARBOX_OCTREE_STRESS") {
        Ok(count) => Some(stress(&mut writer, &config, count.parse()?)?),
        Err(_) => None,
    };
    writer.close(deadline())?;
    println!(
        "{}",
        serde_json::json!({"scene":"octree-lab","datasets":revisions.len(),"leaves":base_leaves(&config).len() + 1,"split_generation":reply.stored_generation,"stress":stress})
    );
    Ok(())
}
