//! Terrain-layers lab: one elevation map with slope, traversability and
//! uncertainty layers, shown four times side by side, each coloured by one
//! layer over the same surface. A pond has no elevation and stays a hole; a
//! strip the robot never re-observed keeps its ground but misses uncertainty,
//! drawn in the missing colour. A rolling window follows a robot along a loop,
//! sending only the tiles each shift changes, newly exposed bands missing until
//! measured. Slope and traversability are the writer's own derivations.
//!
//! Usage: `draw_terrain_layers INSTANCE [SECONDS]`; the robot drives for
//! SECONDS (default 20) and the final rolling map is checked against a fresh one.

use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::elevation::encode_elevation_tile;
use gearbox_api::wire::{draw::rgba, draw_data::*, draw_widgets::Place};
use gearbox_api::{Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataWriter, DrawEntity as E, IdentitySource, Instance, colormap, registry, shape_kind};

#[path = "draw_lab/mod.rs"]
mod lab;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/plan/terrain";
/// South-west corner of the lab, east of the distance-field lab.
const AT: [f64; 2] = [160.0, -30.0];
/// Shown cell edge; the landscape is drawn at half scale, slopes unchanged.
const CELL: f64 = 0.125;
/// Landscape metres per shown metre.
const SCALE: f64 = 2.0;
const SIZE: [i64; 2] = [96, 64];
/// Half the map in metres: centred coordinates plus this are the map's own.
const HALF: [f64; 2] = [6.0, 4.0];
const TINT: [u8; 4] = [120, 220, 160, 255];
const MISSING: [u8; 4] = [96, 40, 112, 255];
/// Rolling window edge in cells and the sensor reach in metres.
const WINDOW: u32 = 64;
const REACH: f64 = 3.0;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

/// Ground height in metres at map metres `p`: two hills and a ditch.
fn height(p: [f64; 2]) -> f64 {
    let hill = |c: [f64; 2], h: f64, r: f64| h * (-((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)) / (r * r)).exp();
    let ditch = -0.5 * (-(p[1] - 0.35 * p[0] - 3.0).powi(2) / 0.8).exp();
    0.6 + hill([6.0, 10.0], 1.6, 3.5) + hill([18.0, 5.0], 0.9, 2.5) + ditch + 0.05 * (p[0] * 1.3).sin() * (p[1] * 0.9).cos()
}

fn pond(p: [f64; 2]) -> bool {
    (p[0] - 14.0).powi(2) + (p[1] - 11.5).powi(2) < 2.2 * 2.2
}

/// Slope in degrees from the height's central difference, where both sides are ground.
fn slope(p: [f64; 2]) -> Option<f64> {
    let h = CELL;
    let near = [[h, 0.0], [-h, 0.0], [0.0, h], [0.0, -h]].map(|d| [p[0] + d[0], p[1] + d[1]]);
    if near.iter().any(|q| pond(*q)) {
        return None;
    }
    let gx = (height(near[0]) - height(near[1])) / (2.0 * h);
    let gy = (height(near[2]) - height(near[3])) / (2.0 * h);
    Some(gx.hypot(gy).atan().to_degrees())
}

fn traversability(p: [f64; 2]) -> Option<f64> {
    slope(p).map(|s| (1.0 - s / 30.0).clamp(0.0, 1.0))
}

/// Height uncertainty, growing away from the survey line; missing where never re-observed.
fn uncertainty(p: [f64; 2]) -> Option<f64> {
    (p[0] < 19.0).then(|| 0.02 + 0.012 * (p[1] - 8.0).abs())
}

fn grid(side: u16) -> ElevationGrid {
    let layer = |name: &str, units: &str| ElevationLayer { name: name.into(), units: units.into() };
    ElevationGrid { cell_m: CELL, tile_side: side, layers: vec![layer("slope", "deg"), layer("traversability", ""), layer("uncertainty", "m")] }
}

/// Map metres of grid cell `cell`.
fn at(cell: [i64; 2]) -> [f64; 2] {
    cell.map(|c| (c as f64 + 0.5) * CELL)
}

/// Map metres of a centred cell: maps are written centred on their panels.
fn map_at(cell: [i64; 2]) -> [f64; 2] {
    landscape(at(cell))
}

/// Landscape metres of shown, centred metres `p`.
fn landscape(p: [f64; 2]) -> [f64; 2] {
    [(p[0] + HALF[0]) * SCALE, (p[1] + HALF[1]) * SCALE]
}

/// Every layer of centred `cell`, `None` where a layer is missing; elevation first.
fn measure(cell: [i64; 2]) -> [Option<f32>; 4] {
    let p = map_at(cell);
    if pond(p) {
        return [None; 4];
    }
    [Some(height(p) / SCALE), slope(p), traversability(p), uncertainty(p)].map(|v| v.map(|v| v as f32))
}

fn pages(grid: &ElevationGrid, tiles: &[ElevationTile]) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    let mut out = Vec::with_capacity(tiles.len());
    for tile in tiles {
        let bytes = encode_elevation_tile(grid, tile)?;
        out.push((
            PageDescriptor {
                address: PageAddress { scan_key: 0, page_id: elevation_page_id(tile.key, tile.layer).ok_or("tile out of range")? },
                encoding: PageEncoding::ElevationTileV1,
                items: grid.samples() as u32,
                byte_len: bytes.len() as u64,
                crc32: page_crc32(&bytes),
                origin_m: [0.0; 3],
            },
            bytes,
        ));
    }
    Ok(out)
}

fn descriptor(id: &str, grid: &ElevationGrid, origin: [f64; 3], layer: &str, map: u32, range: [f64; 2]) -> Result<DataDescriptor> {
    Ok(DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, id)?,
        shape: DataShape::Elevation { grid: grid.clone(), frame: DataFrame { origin_m: origin, ground_radius_m: Some(FOOTPRINT), ..Default::default() } },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Scalar { name: layer.into(), colormap: map, range, missing: MISSING },
            glow: 0.5,
            ..Default::default()
        },
    })
}

fn send(writer: &mut DrawDataWriter<'_>, descriptor: &DataDescriptor, pages: &[(PageDescriptor, Vec<u8>)], removed: &[u64], base: Option<DatasetRevision>) -> Result<DatasetRevision> {
    let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    let removed: Vec<_> = removed.iter().map(|page_id| PageAddress { scan_key: 0, page_id: *page_id }).collect();
    let upload = DataUpload {
        descriptor,
        mode: if base.is_some() { CommitMode::Delta } else { CommitMode::Replace },
        base,
        scans: &[],
        pages: &borrowed,
        removed_pages: &removed,
        removed_scans: &[],
    };
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
}

/// Panel centres, east and north of `AT`: elevation, slope, traversability, uncertainty.
const PANELS: [[f64; 2]; 4] = [[7.0, 17.0], [23.0, 17.0], [7.0, 3.0], [23.0, 3.0]];
const ROLLING: [f64; 2] = [7.0, 31.0];
/// Half the map's diagonal: panels stand on the highest ground under them.
const FOOTPRINT: f64 = 7.3;

fn labels(client: &Client) -> Result<()> {
    let tint = rgba(TINT[0], TINT[1], TINT[2], TINT[3]);
    let ns = format!("{NS}/labels");
    let bay = |id, at: [f64; 2], number: u32, title: &str, detail| lab::Bay {
        id,
        at: [AT[0] + at[0], AT[1] + at[1]],
        size: [13.5, 9.5, 2.0],
        title: format!("{number:02} / {title}"),
        detail,
        category: "TYPED ELEVATION",
        tint,
    };
    lab::bays(
        client,
        &ns,
        &[
            bay("elevation", PANELS[0], 1, "ELEVATION", "THE POND IS A HOLE, NOT ZERO HEIGHT"),
            bay("slope", PANELS[1], 2, "SLOPE", "DERIVED BY THE WRITER, IN DEGREES"),
            bay("traversability", PANELS[2], 3, "TRAVERSABILITY", "RED BLOCKED / GREEN OPEN"),
            bay("uncertainty", PANELS[3], 4, "UNCERTAINTY", "PURPLE = LAYER MISSING ON PRESENT GROUND"),
            bay("rolling", ROLLING, 5, "ROLLING WINDOW", "8 M AROUND THE ROBOT / ONLY CHANGED TILES SENT"),
        ],
    )?;
    lab::label(client, &ns, "title", [AT[0] + 15.0, AT[1] + 38.0, 3.2], "TERRAIN LAYERS / ONE SURFACE, FOUR LAYERS", -17.0, tint, Some("TYPED ELEVATION"))
}

/// The robot's map position after `t` seconds on its loop.
fn robot(t: f64) -> [f64; 2] {
    let a = t * 0.25;
    [3.5 * a.cos(), 2.25 * (2.0 * a).sin()]
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let instance = args.next().ok_or("usage: draw_terrain_layers INSTANCE [SECONDS]")?;
    let seconds: f64 = args.next().map_or(Ok(20.0), |s| s.parse())?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "terrain-layers")?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let mut clear = DataRequest { lease: writer.lease(), ..DataRequest::new(data_op::CLEAR) }.with_metadata(&NamespaceFilter { namespace: NS.into() })?;
    clear.host_epoch = data.capabilities().host_epoch;
    data.request_with_deadline(&clear, deadline())?;
    let _ = client.draw_clear(&format!("{NS}/labels"));
    let _ = client.draw_clear(&format!("{NS}/robot"));
    labels(&client)?;

    let static_grid = grid(32);
    let low = [-SIZE[0] / 2, -SIZE[1] / 2];
    let mut map = RollingMap::new(static_grid.clone(), ElevationWindow { origin: low, size: [SIZE[0] as u32, SIZE[1] as u32] })?;
    for y in low[1]..low[1] + SIZE[1] {
        for x in low[0]..low[0] + SIZE[0] {
            for (layer, value) in measure([x, y]).into_iter().enumerate() {
                map.set(layer as u8, [x, y], value)?;
            }
        }
    }
    let tiles = map.tiles();
    let static_pages = pages(&static_grid, &tiles)?;
    let panels = [
        ("elevation", "elevation", colormap::TURBO, [0.0, 1.2], PANELS[0]),
        ("slope", "slope", colormap::VIRIDIS, [0.0, 35.0], PANELS[1]),
        ("traversability", "traversability", colormap::RDYLGN, [0.0, 1.0], PANELS[2]),
        ("uncertainty", "uncertainty", colormap::VIRIDIS, [0.0, 0.12], PANELS[3]),
    ];
    let mut revisions = serde_json::Map::new();
    for (id, layer, map_id, range, at) in panels {
        let d = descriptor(id, &static_grid, [AT[0] + at[0], AT[1] + at[1], 0.0], layer, map_id, range)?;
        let r = send(&mut writer, &d, &static_pages, &[], None)?;
        revisions.insert(id.into(), serde_json::json!({ "incarnation": r.incarnation, "generation": r.generation }));
    }

    let rolling_grid = grid(16);
    let origin = [AT[0] + ROLLING[0], AT[1] + ROLLING[1], 0.0];
    let rolling = descriptor("rolling", &rolling_grid, origin, "traversability", colormap::RDYLGN, [0.0, 1.0])?;
    let window_at = |p: [f64; 2]| p.map(|c| (c / CELL).floor() as i64 - i64::from(WINDOW) / 2);
    let mut window = RollingMap::new(rolling_grid.clone(), ElevationWindow { origin: window_at(robot(0.0)), size: [WINDOW, WINDOW] })?;
    let mut revision = None;
    let (mut shifts, mut sent_tiles, mut removed_tiles) = (0u64, 0usize, 0usize);
    let start = Instant::now();
    loop {
        let t = start.elapsed().as_secs_f64();
        let p = robot(t);
        window.shift(window_at(p));
        let reach = (REACH / CELL).ceil() as i64;
        let c = p.map(|v| (v / CELL).floor() as i64);
        for y in c[1] - reach..=c[1] + reach {
            for x in c[0] - reach..=c[0] + reach {
                let q = at([x, y]);
                let inside = (-SIZE[0] / 2..SIZE[0] / 2).contains(&x) && (-SIZE[1] / 2..SIZE[1] / 2).contains(&y);
                if (q[0] - p[0]).hypot(q[1] - p[1]) <= REACH && window.window.contains([x, y]) && inside {
                    for (layer, value) in measure([x, y]).into_iter().enumerate() {
                        window.set(layer as u8, [x, y], value)?;
                    }
                }
            }
        }
        let delta = window.delta()?;
        if !delta.tiles.is_empty() || !delta.removed.is_empty() || revision.is_none() {
            sent_tiles += delta.tiles.len();
            removed_tiles += delta.removed.len();
            revision = Some(send(&mut writer, &rolling, &pages(&rolling_grid, &delta.tiles)?, &delta.removed, revision)?);
            shifts += 1;
        }
        let marker = Place::default()
            .put(E::shapes(
                "vfx-gallery/plan/terrain/robot",
                "robot",
                shape_kind::CYLINDER,
                &[Instance::at([(origin[0] + p[0]) as f32, (origin[1] + p[1]) as f32, (height(landscape(p)) / SCALE + 0.5) as f32], [0.6, 0.6, 1.0])],
            ))
            .with_color(rgba(255, 210, 90, 255));
        let _ = client.draw(&marker);
        if t >= seconds {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let fresh = window.tiles();
    writer.close(deadline())?;
    let revision = revision.ok_or("rolling map never sent")?;
    println!(
        "{}",
        serde_json::json!({
            "scene": "terrain-layers",
            "namespace": NS,
            "static": { "tiles": static_pages.len(), "revisions": revisions },
            "rolling": {
                "commits": shifts,
                "sent_tiles": sent_tiles,
                "removed_tiles": removed_tiles,
                "fresh_pages": fresh.len(),
                "fresh_samples": fresh.len() * rolling_grid.samples(),
                "revision": { "incarnation": revision.incarnation, "generation": revision.generation },
                "window": window.window,
            },
        })
    );
    Ok(())
}
