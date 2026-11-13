//! Layered elevation tiles: codec, keys, rolling windows against a fresh map,
//! and atomic layer bundles in the store.

use std::collections::BTreeMap;
use std::sync::Arc;

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::elevation::{decode_elevation_tile, elevation_layout, elevation_schema, encode_elevation_tile};
use gearbox_api::wire::draw_data::*;

const EPOCH: u64 = 31;

fn grid() -> ElevationGrid {
    ElevationGrid {
        cell_m: 0.25,
        tile_side: 8,
        layers: vec![
            ElevationLayer { name: "slope".into(), units: "deg".into() },
            ElevationLayer { name: "traversability".into(), units: "".into() },
        ],
    }
}

fn tile(key: [i32; 2], layer: u8, f: impl Fn([i64; 2]) -> Option<f32>) -> ElevationTile {
    let mut t = ElevationTile { key, layer, values: vec![0.0; 64], valid: vec![false; 64] };
    for j in 0..8 {
        for i in 0..8 {
            if let Some(v) = f([i64::from(key[0]) * 8 + i, i64::from(key[1]) * 8 + j]) {
                t.values[(i + j * 8) as usize] = v;
                t.valid[(i + j * 8) as usize] = true;
            }
        }
    }
    t
}

#[test]
fn tiles_roundtrip_and_refuse_malformed_samples() {
    let good = tile([-2, 3], 1, |c| (c[0] % 3 != 0).then_some(c[1] as f32 * 0.5));
    let bytes = encode_elevation_tile(&grid(), &good).unwrap();
    assert_eq!(decode_elevation_tile(&grid(), &bytes).unwrap(), good);
    assert_eq!(elevation_layout(&grid(), &bytes).unwrap().valid_count as usize, good.valid.iter().filter(|v| **v).count());
    let mut nan = good.clone();
    nan.values[nan.valid.iter().position(|v| *v).unwrap()] = f32::NAN;
    assert!(encode_elevation_tile(&grid(), &nan).is_err(), "a valid sample must be finite");
    assert!(encode_elevation_tile(&grid(), &ElevationTile { layer: 3, ..good.clone() }).is_err(), "layer beyond the map");
    assert!(encode_elevation_tile(&grid(), &ElevationTile { values: vec![0.0; 63], ..good.clone() }).is_err());
    let mut noisy = bytes.clone();
    let hole = good.valid.iter().position(|v| !*v).unwrap();
    noisy[20 + hole * 4] = 7;
    assert!(elevation_layout(&grid(), &noisy).is_err(), "a missing sample must hold canonical zeros");
    for bad in [
        ElevationGrid { tile_side: 4, ..grid() },
        ElevationGrid { cell_m: 0.0, ..grid() },
        ElevationGrid { layers: vec![ElevationLayer { name: ELEVATION.into(), units: "m".into() }], ..grid() },
        ElevationGrid { layers: vec![grid().layers[0].clone(), grid().layers[0].clone()], ..grid() },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn page_ids_pack_negative_tiles_and_layers() {
    for (key, layer) in [([0, 0], 0), ([-1, 5], 2), ([(1 << 23) - 1, -(1 << 23)], 15)] {
        let id = elevation_page_id(key, layer).unwrap();
        assert_eq!(id >> 52, 0);
        assert_eq!(elevation_tile_of(id), (key, layer));
    }
    assert!(elevation_page_id([1 << 23, 0], 0).is_none());
    assert!(elevation_page_id([0, 0], 16).is_none());
    assert_eq!(grid().locate([-1, 9]), ([-1, 1], 7 + 8));
}

/// The stored pages after applying `delta` to `store`.
fn apply(store: &mut BTreeMap<u64, ElevationTile>, delta: &ElevationDelta) {
    for id in &delta.removed {
        store.remove(id);
    }
    for t in &delta.tiles {
        store.insert(elevation_page_id(t.key, t.layer).unwrap(), t.clone());
    }
}

fn fresh(map: &RollingMap) -> BTreeMap<u64, ElevationTile> {
    map.tiles().into_iter().map(|t| (elevation_page_id(t.key, t.layer).unwrap(), t)).collect()
}

/// Fills every cell of the window the way a robot would measure it.
fn measure(map: &mut RollingMap, skip: impl Fn([i64; 2]) -> bool) {
    let window = map.window;
    for y in window.origin[1]..window.origin[1] + i64::from(window.size[1]) {
        for x in window.origin[0]..window.origin[0] + i64::from(window.size[0]) {
            if !skip([x, y]) && map.get(0, [x, y]).is_none() {
                map.set(0, [x, y], Some((x * 3 + y) as f32 * 0.01)).unwrap();
                map.set(2, [x, y], Some(((x + y).rem_euclid(5)) as f32 / 4.0)).unwrap();
            }
        }
    }
}

#[test]
fn rolling_shifts_match_a_fresh_map_and_expose_only_missing_bands() {
    let window = ElevationWindow { origin: [-5, -3], size: [12, 10] };
    let mut map = RollingMap::new(grid(), window).unwrap();
    measure(&mut map, |_| false);
    let mut store = BTreeMap::new();
    apply(&mut store, &map.delta().unwrap());
    assert_eq!(store, fresh(&map));
    let before = map.get(0, [0, 0]);
    for (step, origin) in [[-2, -3], [-9, 1], [-9, 1], [40, -60], [38, -61]].into_iter().enumerate() {
        map.shift(origin);
        if step == 0 {
            assert_eq!(map.get(0, [0, 0]), before, "the overlap keeps its measurement");
            assert_eq!(map.get(0, [8, 0]), None, "a newly exposed column is missing until measured");
            assert_eq!(map.get(0, [-4, 0]), None, "a column that left the window is dropped");
        }
        let delta = map.delta().unwrap();
        if step == 2 {
            assert_eq!(delta, ElevationDelta::default(), "a repeated origin changes nothing");
        }
        apply(&mut store, &delta);
        assert_eq!(store, fresh(&map), "after shift {step}");
        measure(&mut map, |c| (c[0] + c[1]) % 7 == 0);
        apply(&mut store, &map.delta().unwrap());
        assert_eq!(store, fresh(&map), "after measuring at shift {step}");
    }
    assert!(store.values().all(|t| t.valid.iter().any(|v| *v)), "no empty tile stays stored");
    assert!(map.set(0, [0, 0], Some(1.0)).is_err(), "cells outside the window are refused");
}

fn req(op: u32) -> DataRequest {
    DataRequest { host_epoch: EPOCH, ..DataRequest::new(op) }
}

fn ok(store: &mut StreamingStore, request: &DataRequest) -> DataReply {
    let reply = store.handle(request, 0);
    assert_eq!(reply.code, data_code::OK, "{}", String::from_utf8_lossy(&reply.message));
    reply
}

fn edit(begin: &DataRequest, op: u32) -> DataRequest {
    DataRequest {
        lease: begin.lease,
        transaction: begin.transaction,
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        key: begin.key.clone(),
        ..req(op)
    }
}

fn descriptor(color: ColorMode) -> DataDescriptor {
    DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("vfx-test/elevation", "map").unwrap(),
        shape: DataShape::Elevation { grid: grid(), frame: DataFrame::default() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation { color, ..Presentation::default() },
    }
}

fn layer_color(name: &str) -> ColorMode {
    ColorMode::Scalar { name: name.into(), colormap: gearbox_api::colormap::TURBO, range: [0.0, 1.0], missing: [0; 4] }
}

fn commit(store: &mut StreamingStore, lease: u64, id: u64, mode: CommitMode, base: Option<DatasetRevision>, tiles: &[ElevationTile], removed: &[u64]) -> DataReply {
    let descriptor = descriptor(layer_color("traversability"));
    let pages: Vec<_> = tiles
        .iter()
        .map(|t| {
            let bytes = encode_elevation_tile(&grid(), t).unwrap();
            (
                PageDescriptor {
                    address: PageAddress { scan_key: 0, page_id: elevation_page_id(t.key, t.layer).unwrap() },
                    encoding: PageEncoding::ElevationTileV1,
                    items: 64,
                    byte_len: bytes.len() as u64,
                    crc32: page_crc32(&bytes),
                    origin_m: [0.0; 3],
                },
                bytes,
            )
        })
        .collect();
    let begin = DataRequest {
        lease,
        transaction: id,
        incarnation: base.map_or(0, |b| b.incarnation),
        base_generation: base.map_or(0, |b| b.generation),
        total_bytes: pages.iter().map(|(d, _)| d.byte_len).sum(),
        ..req(data_op::BEGIN)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&BeginMetadata { descriptor, mode, scans: 0, pages: pages.len() as u32, removed_scans: 0, removed_pages: removed.len() as u32 })
    .unwrap();
    ok(store, &begin);
    for (page, bytes) in &pages {
        ok(store, &DataRequest { page_id: page.address.page_id, ..edit(&begin, data_op::DECLARE_PAGE) }.with_metadata(page).unwrap());
        let reply = store.handle(&DataRequest { page_id: page.address.page_id, payload: bytes.clone(), ..edit(&begin, data_op::CHUNK) }, 0);
        if reply.code != data_code::OK {
            return reply;
        }
    }
    for id in removed {
        ok(store, &DataRequest { page_id: *id, ..edit(&begin, data_op::REMOVE_PAGE) });
    }
    store.handle(&edit(&begin, data_op::COMMIT), 0)
}

fn pages(store: &StreamingStore) -> Vec<Arc<StoredPage>> {
    let registry = store.registry();
    let registry = registry.lock().unwrap();
    let entry = registry.get(&("vfx-test/elevation".to_string(), "map".to_string())).unwrap();
    assert_eq!(entry.kind, DatasetKind::Elevation);
    entry.payload.resident().unwrap().pages().to_vec()
}

#[test]
fn a_layer_bundle_commits_atomically_and_a_layer_update_keeps_the_elevation_pages() {
    let mut store = StreamingStore::new(EPOCH, DataLimits::reference()).unwrap();
    let lease = ok(&mut store, &req(data_op::OPEN_WRITER)).lease;
    let height = |c: [i64; 2]| Some(c[0] as f32 * 0.1);
    let bundle = [tile([0, 0], 0, height), tile([1, 0], 0, height), tile([0, 0], 2, |_| Some(0.5)), tile([1, 0], 2, |_| Some(0.5))];
    let first = commit(&mut store, lease, 1, CommitMode::Replace, None, &bundle, &[]);
    assert_eq!(first.code, data_code::OK, "{}", String::from_utf8_lossy(&first.message));
    let before = pages(&store);
    let base = DatasetRevision { incarnation: first.incarnation, generation: first.stored_generation };
    let layer = commit(&mut store, lease, 2, CommitMode::Delta, Some(base), &[tile([1, 0], 2, |_| Some(0.9))], &[]);
    assert_eq!(layer.code, data_code::OK, "{}", String::from_utf8_lossy(&layer.message));
    let after = pages(&store);
    let elevation = |pages: &[Arc<StoredPage>]| pages.iter().filter(|p| p.elevation_layout().unwrap().layer == 0).cloned().collect::<Vec<_>>();
    assert!(elevation(&before).iter().zip(elevation(&after)).all(|(a, b)| Arc::ptr_eq(a, &b)), "elevation pages are shared, not re-sent");
    let next = DatasetRevision { incarnation: layer.incarnation, generation: layer.stored_generation };
    let dropped = commit(&mut store, lease, 3, CommitMode::Delta, Some(next), &[], &[elevation_page_id([1, 0], 0).unwrap(), elevation_page_id([1, 0], 2).unwrap()]);
    assert_eq!(dropped.code, data_code::OK);
    assert_eq!(pages(&store).len(), 2, "a whole tile leaves with both its layers");
    let mut wrong = tile([3, 3], 0, height);
    wrong.layer = 1;
    let mut misfiled = encode_elevation_tile(&grid(), &wrong).unwrap();
    let revision = DatasetRevision { incarnation: dropped.incarnation, generation: dropped.stored_generation };
    let page = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: elevation_page_id([3, 3], 0).unwrap() },
        encoding: PageEncoding::ElevationTileV1,
        items: 64,
        byte_len: misfiled.len() as u64,
        crc32: page_crc32(&misfiled),
        origin_m: [0.0; 3],
    };
    let begin = DataRequest { lease, transaction: 4, incarnation: revision.incarnation, base_generation: revision.generation, total_bytes: page.byte_len, ..req(data_op::BEGIN) }
        .with_key(&DataKey::new("vfx-test/elevation", "map").unwrap())
        .unwrap()
        .with_metadata(&BeginMetadata { descriptor: descriptor(layer_color("slope")), mode: CommitMode::Delta, scans: 0, pages: 1, removed_scans: 0, removed_pages: 0 })
        .unwrap();
    ok(&mut store, &begin);
    ok(&mut store, &DataRequest { page_id: page.address.page_id, ..edit(&begin, data_op::DECLARE_PAGE) }.with_metadata(&page).unwrap());
    let refused = store.handle(&DataRequest { page_id: page.address.page_id, payload: std::mem::take(&mut misfiled), ..edit(&begin, data_op::CHUNK) }, 0);
    assert_ne!(refused.code, data_code::OK, "a layer filed under another layer's page id");
}

#[test]
fn colour_modes_name_real_layers() {
    descriptor(layer_color("slope")).validate().unwrap();
    descriptor(layer_color(ELEVATION)).validate().unwrap();
    descriptor(ColorMode::Constant { rgba: [1, 2, 3, 255] }).validate().unwrap();
    assert!(descriptor(layer_color("roughness")).validate().is_err(), "no such layer");
    assert_eq!(elevation_schema(&grid()).attributes.len(), 4);
}

#[test]
fn the_rust_and_python_encoders_write_the_same_tile() {
    let golden = include_str!("fixtures/draw_data/elevation_tile.hex").trim();
    let t = tile([-1, 2], 1, |c| ((c[0] + c[1]) % 4 != 0).then_some((c[0] * c[1]) as f32 * 0.125));
    let hex: String = encode_elevation_tile(&grid(), &t).unwrap().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, golden);
}

#[test]
fn a_rolling_map_survives_its_state_round_trip() {
    let mut map = RollingMap::new(grid(), ElevationWindow { origin: [-3, -3], size: [10, 9] }).unwrap();
    measure(&mut map, |c| c[0] == c[1]);
    map.delta().unwrap();
    let json = serde_json::to_string(&map.state()).unwrap();
    let mut back = RollingMap::from_state(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back.state(), map.state());
    assert_eq!(back.delta().unwrap(), ElevationDelta::default(), "nothing changed since the saved delta");
    back.shift([1, -3]);
    map.shift([1, -3]);
    assert_eq!(back.delta().unwrap(), map.delta().unwrap());
}
