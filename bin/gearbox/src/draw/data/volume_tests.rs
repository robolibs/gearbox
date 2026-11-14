use std::sync::Arc;

use gearbox_api::draw_data::{CpuBudget, PageUpload, StoredPage};
use gearbox_api::wire::draw_data::volume::{VolumeBlock, encode_volume_block};
use gearbox_api::wire::draw_data::*;

use super::*;

/// A metric field `z - 0.5` on 0.1 m voxels in 8-sample blocks.
fn field() -> VolumeField {
    VolumeField { kind: FieldKind::Esdf { signed: true }, voxel_m: 0.1, block_side: 8, colors: false }
}

/// Block `key` of `z - 0.5`, unknown where `unknown(i, j, k)` holds.
fn block(key: [i32; 3], unknown: impl Fn(usize, usize, usize) -> bool) -> VolumeBlock {
    let mut out = VolumeBlock { key, values: vec![0.0; 512], valid: vec![false; 512], weights: None, colors: None };
    for k in 0..8 {
        for j in 0..8 {
            for i in 0..8 {
                let n = i + 8 * (j + 8 * k);
                let z = (f64::from(key[2]) * 8.0 + k as f64 + 0.5) * 0.1;
                out.valid[n] = !unknown(i, j, k);
                out.values[n] = if out.valid[n] { (z - 0.5) as f32 } else { 0.0 };
            }
        }
    }
    out
}

fn stored(budget: &CpuBudget, field: &VolumeField, block: &VolumeBlock) -> Arc<StoredPage> {
    let bytes = encode_volume_block(field, block).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: volume_page_id(block.key).unwrap() },
        encoding: PageEncoding::VolumeBlockV1,
        items: 512,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    upload.push(0, &bytes).unwrap();
    upload.finish_volume(field).unwrap().into_page()
}

fn view(slices: Vec<VolumeSlice>) -> VolumeView {
    VolumeView {
        slices,
        interpolation: Interpolation::Trilinear,
        band: None,
        contours: vec![],
        contour_rgba: [255, 255, 255, 255],
        unknown_rgba: [90, 0, 90, 255],
        frame_rgba: None,
    }
}

fn presentation(view: VolumeView) -> Presentation {
    Presentation {
        color: ColorMode::Scalar { name: "value".into(), colormap: gearbox_api::colormap::TURBO, range: [-0.5, 0.5], missing: [0; 4] },
        volume: Some(view),
        ..Presentation::default()
    }
}

/// A vertical slice through x = 0.4 spanning both blocks of the column, 0.1 m cells.
fn vertical() -> VolumeSlice {
    VolumeSlice { center_m: [0.4, 0.4, 0.8], u: [0.0, 1.0, 0.0], v: [0.0, 0.0, 1.0], size_m: [0.6, 1.2], cell_m: 0.1 }
}

fn groups(budget: &CpuBudget, blocks: &[VolumeBlock], view: &VolumeView) -> Groups {
    let field = field();
    let pages: Vec<_> = blocks.iter().map(|b| stored(budget, &field, b)).collect();
    let volume = Volume::new(&field, pages.iter().map(|p| BlockView::trusted(p.bytes(), p.volume_layout().unwrap())));
    let presentation = presentation(view.clone());
    let samples = sample_slice(&volume, view, &view.slices[0], Channel::Value, false);
    slice_groups(&field, &presentation, view, &view.slices[0], &samples).unwrap()
}

fn cells(groups: &Groups) -> Vec<Cell> {
    groups.iter().flat_map(|(_, cells, ..)| cells.iter().step_by(6).copied()).collect()
}

#[test]
fn slice_cells_sample_the_field_and_group_by_the_block_holding_their_centre() {
    let budget = CpuBudget::new(1 << 24);
    let all = |_: usize, _: usize, _: usize| false;
    let groups = groups(&budget, &[block([0, 0, 0], all), block([0, 0, 1], all)], &view(vec![vertical()]));
    assert_eq!(groups.len(), 2, "one group per block the slice crosses");
    let drawn: u64 = groups.iter().map(|(_, _, count, _)| count).sum();
    assert_eq!(drawn, 6 * 12);
    let values: Vec<f32> = cells(&groups).iter().filter(|c| c.code & UNKNOWN == 0).map(|c| c.probability).collect();
    assert!(values.iter().all(|v| v.is_finite()));
    assert_eq!(values.len(), 72, "every cell centre has eight valid corners in the two-block column");
    let lowest = values.iter().copied().fold(f32::MAX, f32::min);
    assert!((lowest + 0.25).abs() < 1e-5, "the bottom row samples z - 0.5 at z = 0.25");
}

#[test]
fn unknown_and_unallocated_support_draw_as_a_checker_never_as_a_value() {
    let budget = CpuBudget::new(1 << 24);
    let hole = |_: usize, _: usize, k: usize| (3..5).contains(&k);
    let groups = groups(&budget, &[block([0, 0, 0], hole)], &view(vec![vertical()]));
    let cells = cells(&groups);
    let unknown: Vec<_> = cells.iter().filter(|c| c.code & UNKNOWN != 0).collect();
    assert!(!unknown.is_empty());
    assert!(unknown.iter().all(|c| c.probability.is_nan()), "unknown cells carry no value");
    let tones: std::collections::HashSet<u32> = unknown.iter().map(|c| c.rgba).collect();
    assert_eq!(tones.len(), 2, "a two-tone checker");
    let palette = colormap_rgba(gearbox_api::colormap::TURBO, 0.0, -0.5, 0.5);
    assert!(unknown.iter().all(|c| c.rgba != super::super::octree::rgba(palette, 1.0)));
}

#[test]
fn a_band_leaves_out_cells_outside_it_but_keeps_unknown_cells() {
    let budget = CpuBudget::new(1 << 24);
    let all = |_: usize, _: usize, _: usize| false;
    let mut banded = view(vec![vertical()]);
    banded.band = Some([-0.1, 0.0]);
    let groups = groups(&budget, &[block([0, 0, 0], all)], &banded);
    let cells = cells(&groups);
    let known: Vec<_> = cells.iter().filter(|c| c.code & UNKNOWN == 0).collect();
    assert_eq!(known.len(), 6, "only the row just below the zero crossing");
    assert!(known.iter().all(|c| (c.probability + 0.05).abs() < 1e-5));
    assert!(cells.iter().any(|c| c.code & UNKNOWN != 0));
}

#[test]
fn contours_mark_cells_whose_neighbour_straddles_a_level() {
    let budget = CpuBudget::new(1 << 24);
    let all = |_: usize, _: usize, _: usize| false;
    let mut contoured = view(vec![vertical()]);
    contoured.contours = vec![0.0];
    contoured.contour_rgba = [255, 255, 0, 255];
    let groups = groups(&budget, &[block([0, 0, 0], all), block([0, 0, 1], all)], &contoured);
    let marked: Vec<_> = cells(&groups).into_iter().filter(|c| c.rgba == super::super::octree::rgba([255, 255, 0, 255], 1.0)).collect();
    assert_eq!(marked.len(), 6, "one row along the zero level");
    assert!(marked.iter().all(|c| c.probability.abs() <= 0.1));
}

#[test]
fn an_unchanged_group_keeps_its_buffer_and_a_changed_one_does_not() {
    let budget = CpuBudget::new(1 << 24);
    let all = |_: usize, _: usize, _: usize| false;
    let before = groups(&budget, &[block([0, 0, 0], all), block([0, 0, 1], all)], &view(vec![vertical()]));
    let address = |page| PageAddress { scan_key: SLICE_SCAN, page_id: page };
    let pages: Vec<_> = before
        .into_iter()
        .map(|(id, cells, count, rebase)| page(&budget, &Default::default(), address(id), cells, rebase, count, true).unwrap())
        .collect();
    let previous: HashMap<PageAddress, &Arc<PageCells>> = pages.iter().map(|p| (p.address, p)).collect();
    let hole = |_: usize, _: usize, k: usize| k == 6;
    let after = groups(&budget, &[block([0, 0, 0], all), block([0, 0, 1], hole)], &view(vec![vertical()]));
    let rebuilt: Vec<_> = after
        .into_iter()
        .map(|(id, cells, count, rebase)| page(&budget, &previous, address(id), cells, rebase, count, true).unwrap())
        .collect();
    let lower = volume_page_id([0, 0, 0]).unwrap();
    let kept = |p: &Arc<PageCells>| previous.get(&p.address).is_some_and(|old| Arc::ptr_eq(&old.cells, &p.cells));
    assert!(rebuilt.iter().filter(|p| p.address.page_id == lower).all(kept), "the untouched lower block keeps its buffer");
    assert!(!rebuilt.iter().filter(|p| p.address.page_id != lower).all(kept), "the edited block uploads again");
}

#[test]
fn a_slice_border_is_four_unlit_unpickable_quads() {
    let cells = border(&vertical(), [255, 255, 255, 255]).unwrap();
    assert_eq!(cells.len(), 24);
    assert!(cells.iter().all(|c| c.code & ARROW != 0 && c.code & FRAME != 0 && c.code & SLICE == 0));
}

#[test]
fn a_block_delta_reuploads_only_the_slice_groups_it_moved() {
    use super::super::tests::{generation, lifetime};
    let budget = CpuBudget::new(1 << 24);
    let all = |_: usize, _: usize, _: usize| false;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("volume-test", "field").unwrap(),
        shape: DataShape::Volume { field: field(), frame: DataFrame::default() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: presentation(VolumeView { frame_rgba: Some([255; 4]), ..view(vec![vertical()]) }),
    };
    let lower = stored(&budget, &field(), &block([0, 0, 0], all));
    let upper = stored(&budget, &field(), &block([0, 0, 1], all));
    let edited = stored(&budget, &field(), &block([0, 0, 1], |_, _, k| k == 6));
    let revision = |generation| DatasetRevision { incarnation: 1, generation };
    let key = descriptor.key.clone();
    let first = prepare(&budget, key.clone(), revision(1), lifetime(), generation(&budget, &descriptor, &[lower.clone(), upper]), None).unwrap();
    let second = prepare(&budget, key, revision(2), lifetime(), generation(&budget, &descriptor, &[lower, edited]), Some(&first)).unwrap();
    assert_eq!(first.pages.len(), 3, "two cell groups and the border");
    let same = |page: &Arc<PageCells>| first.pages.iter().any(|old| Arc::ptr_eq(&old.cells, &page.cells));
    let moved: Vec<_> = second.pages.iter().filter(|page| !same(page)).map(|page| page.address.page_id).collect();
    assert_eq!(moved, vec![volume_page_id([0, 0, 1]).unwrap()], "only the edited block's group re-uploads");
}
