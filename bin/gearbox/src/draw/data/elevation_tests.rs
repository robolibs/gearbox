use std::sync::Arc;

use gearbox_api::draw_data::{CpuBudget, PageUpload};
use gearbox_api::wire::draw_data::elevation::encode_elevation_tile;
use gearbox_api::wire::draw_data::*;

use super::super::tests::{generation, lifetime};
use super::*;

/// 8-sample tiles of 0.5 m with slope and uncertainty layers.
fn grid() -> ElevationGrid {
    ElevationGrid {
        cell_m: 0.5,
        tile_side: 8,
        layers: vec![
            ElevationLayer { name: "slope".into(), units: "deg".into() },
            ElevationLayer { name: "uncertainty".into(), units: "m".into() },
        ],
    }
}

/// Layer `layer` of tile `key`: value `f(cell)`, missing where `hole(cell)`.
fn tile(budget: &CpuBudget, key: [i32; 2], layer: u8, f: impl Fn([i64; 2]) -> f32, hole: impl Fn([i64; 2]) -> bool) -> Arc<StoredPage> {
    let grid = grid();
    let mut t = ElevationTile { key, layer, values: vec![0.0; 64], valid: vec![false; 64] };
    for j in 0..8 {
        for i in 0..8 {
            let cell = [i64::from(key[0]) * 8 + i, i64::from(key[1]) * 8 + j];
            if !hole(cell) {
                t.valid[(i + j * 8) as usize] = true;
                t.values[(i + j * 8) as usize] = f(cell);
            }
        }
    }
    let bytes = encode_elevation_tile(&grid, &t).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: elevation_page_id(key, layer).unwrap() },
        encoding: PageEncoding::ElevationTileV1,
        items: 64,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    upload.push(0, &bytes).unwrap();
    upload.finish_elevation(&grid).unwrap().into_page()
}

fn ramp(cell: [i64; 2]) -> f32 {
    cell[0] as f32 * 0.1
}

fn none(_: [i64; 2]) -> bool {
    false
}

fn descriptor(color: &str) -> DataDescriptor {
    DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("elevation-test", "map").unwrap(),
        shape: DataShape::Elevation { grid: grid(), frame: DataFrame::default() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Scalar { name: color.into(), colormap: gearbox_api::colormap::TURBO, range: [0.0, 1.0], missing: [10, 20, 30, 255] },
            ..Presentation::default()
        },
    }
}

fn prepared(budget: &CpuBudget, color: &str, pages: &[Arc<StoredPage>], previous: Option<&Prepared>, generation_number: u64) -> Arc<Prepared> {
    let descriptor = descriptor(color);
    let revision = DatasetRevision { incarnation: 1, generation: generation_number };
    prepare(budget, descriptor.key.clone(), revision, lifetime(), generation(budget, &descriptor, pages), previous).unwrap()
}

#[test]
fn a_missing_elevation_is_a_hole_not_zero_height() {
    let budget = CpuBudget::new(1 << 22);
    let pages = [tile(&budget, [0, 0], 0, ramp, |c| c == [3, 3])];
    let map = prepared(&budget, "elevation", &pages, None, 1);
    let records = &map.pages[0].cells.records.values;
    assert_eq!(records.len(), 6 * (7 * 7 - 4), "the four quads around the missing sample are absent");
    let rebase = DVec3::from_array(map.pages[0].rebase_m);
    for record in records.iter() {
        let at = rebase + DVec3::from_array(record.center.map(f64::from));
        let cell = [(at.x / 0.5 - 0.5).round() as i64, (at.y / 0.5 - 0.5).round() as i64];
        assert_ne!(cell, [3, 3], "no vertex stands on the hole");
        assert!((at.z as f32 - ramp(cell)).abs() < 1e-5, "every vertex keeps its own elevation");
    }
}

#[test]
fn tiles_meet_their_neighbours_without_a_seam_gap() {
    let budget = CpuBudget::new(1 << 22);
    let pages = [tile(&budget, [0, 0], 0, ramp, none), tile(&budget, [1, 0], 0, ramp, none)];
    let map = prepared(&budget, "elevation", &pages, None, 1);
    let quads: Vec<usize> = map.pages.iter().map(|page| page.cells.records.values.len() / 6).collect();
    assert_eq!(quads, vec![8 * 7, 7 * 7], "the west tile reaches into the east tile's first column");
}

#[test]
fn switching_the_colour_layer_keeps_the_geometry_buffer() {
    let budget = CpuBudget::new(1 << 22);
    let pages = [
        tile(&budget, [0, 0], 0, ramp, none),
        tile(&budget, [0, 0], 1, |c| c[1] as f32 * 0.1, none),
        tile(&budget, [0, 0], 2, |_| 0.5, |c| c[0] > 4),
    ];
    let slope = prepared(&budget, "slope", &pages, None, 1);
    let uncertainty = prepared(&budget, "uncertainty", &pages, Some(&slope), 2);
    assert!(Arc::ptr_eq(&slope.pages[0].cells, &uncertainty.pages[0].cells), "same geometry");
    let (a, b) = (slope.pages[0].colors.as_ref().unwrap(), uncertainty.pages[0].colors.as_ref().unwrap());
    assert!(!Arc::ptr_eq(a, b), "new colours");
    let missing = super::super::octree::rgba([10, 20, 30, 255], 1.0);
    assert!(b.values.values.contains(&missing), "a missing layer value colours present geometry, it does not cut a hole");
    assert_eq!(b.values.values.len(), uncertainty.pages[0].cells.records.values.len());
    assert_eq!(uncertainty.uniform.extra.y, 1.0);
}

#[test]
fn a_layer_only_update_keeps_geometry_and_an_elevation_edit_rebuilds_its_neighbour() {
    let budget = CpuBudget::new(1 << 22);
    let west = tile(&budget, [0, 0], 0, ramp, none);
    let east = tile(&budget, [1, 0], 0, ramp, none);
    let first = prepared(&budget, "slope", &[west.clone(), east.clone(), tile(&budget, [0, 0], 1, |_| 0.2, none)], None, 1);
    let layer_only = prepared(&budget, "slope", &[west.clone(), east, tile(&budget, [0, 0], 1, |_| 0.8, none)], Some(&first), 2);
    assert!(first.pages.iter().zip(&layer_only.pages).all(|(a, b)| Arc::ptr_eq(&a.cells, &b.cells)));
    assert!(!Arc::ptr_eq(first.pages[0].colors.as_ref().unwrap(), layer_only.pages[0].colors.as_ref().unwrap()));
    let raised = tile(&budget, [1, 0], 0, |c| ramp(c) + 1.0, none);
    let edited = prepared(&budget, "slope", &[west, raised, tile(&budget, [0, 0], 1, |_| 0.8, none)], Some(&layer_only), 3);
    assert!(!Arc::ptr_eq(&layer_only.pages[0].cells, &edited.pages[0].cells), "the west tile's seam quads read the raised east tile");
}
