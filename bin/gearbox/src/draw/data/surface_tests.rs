use std::sync::Arc;

use bevy::math::{DVec3, Vec3};
use gearbox_api::draw_data::{CpuBudget, PageUpload, StoredPage};
use gearbox_api::wire::draw_data::mesh::{MeshBlock, encode_mesh_block};
use gearbox_api::wire::draw_data::*;

use super::super::tests::{add, attribute, fixture, stored};
use super::*;

fn surfels(budget: &CpuBudget, radius: [f32; 3], radius_mask: u8, normal_mask: u8) -> (CloudSchema, Arc<StoredPage>) {
    let (mut schema, mut page) = fixture(&[[1.0, 2.0, 3.0], [1.5, 2.0, 3.0], [2.0, 2.0, 3.0]], vec![]);
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
    let mut values: Vec<u8> = Vec::new();
    for (i, n) in normals.iter().enumerate() {
        let n = if normal_mask & (1 << i) != 0 { *n } else { [0.0; 3] };
        values.extend(n.iter().flat_map(|v| v.to_le_bytes()));
    }
    add(&mut schema, &mut page, attribute("n", ScalarType::F32, 3, AttributeSemantic::Normal, "", true), values, vec![normal_mask]);
    add(&mut schema, &mut page, attribute("r", ScalarType::F32, 1, AttributeSemantic::Radius, "m", true), radius.iter().flat_map(|v| v.to_le_bytes()).collect(), vec![radius_mask]);
    let page = stored(budget, &schema, &page, [10.0, 0.0, 0.0]);
    (schema, page)
}

fn mesh(budget: &CpuBudget, block: &MeshBlock) -> Arc<StoredPage> {
    let bytes = encode_mesh_block(block).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: 7 },
        encoding: PageEncoding::MeshBlockV1,
        items: block.triangles() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0, 0.0, 5.0],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    upload.push(0, &bytes).unwrap();
    upload.finish_mesh().unwrap().into_page()
}

fn square(normals: bool) -> MeshBlock {
    MeshBlock {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: normals.then(|| vec![[0.0, 0.0, 1.0]; 4]),
        colors: None,
        confidence: Some(vec![0.2, 0.4, 0.6, 0.8]),
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn world(page: &PageCells, cell: &super::super::octree::Cell) -> DVec3 {
    DVec3::from_array(page.rebase_m) + DVec3::from_array(cell.center.map(f64::from))
}

#[test]
fn octahedral_normals_round_trip_closely_in_every_octant() {
    for x in [-1.0f32, -0.3, 0.0, 0.4, 1.0] {
        for y in [-1.0f32, -0.2, 0.0, 0.7, 1.0] {
            for z in [-1.0f32, -0.5, 0.0, 0.6, 1.0] {
                let n = Vec3::new(x, y, z);
                if n.length() < 0.1 {
                    continue;
                }
                let n = n.normalize();
                let back = oct_decode(oct_encode(n));
                assert!(back.angle_between(n) < 1e-3, "{n} -> {back}");
            }
        }
    }
}

#[test]
fn a_surfel_page_keeps_ids_and_normals_and_flags_a_missing_normal() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = surfels(&budget, [0.1, 0.2, 0.3], 0b111, 0b011);
    let cells = surfel_page(&budget, &Presentation::default(), &schema, &page).unwrap();
    assert_eq!(cells.shape, Shape::Discs);
    let records = &cells.cells.records.values;
    assert_eq!(records.len(), 3);
    assert_eq!(cells.ids.as_ref().unwrap().values, vec![u64::MAX, u64::MAX - 1, u64::MAX - 2]);
    assert!((world(&cells, &records[1]) - DVec3::new(11.5, 2.0, 3.0)).length() < 1e-6);
    assert_eq!(records[2].half, 0.3);
    assert!(oct_decode(records[0].spare).angle_between(Vec3::Z) < 1e-3);
    assert!(oct_decode(records[1].spare).angle_between(Vec3::X) < 1e-3);
    assert_eq!(records[0].code & NORMAL_MISSING, 0);
    assert_ne!(records[2].code & NORMAL_MISSING, 0);
}

#[test]
fn a_surfel_without_a_radius_is_not_drawn() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = surfels(&budget, [0.1, 0.0, 0.0], 0b001, 0b011);
    let cells = surfel_page(&budget, &Presentation::default(), &schema, &page).unwrap();
    assert_eq!((cells.cells.records.values.len(), cells.counts.leaves), (1, 3));
}

#[test]
fn a_mesh_block_draws_three_vertex_records_per_triangle() {
    let budget = CpuBudget::new(1 << 20);
    let page = mesh(&budget, &square(true));
    let cells = mesh_page(&budget, &Presentation::default(), &page).unwrap();
    assert_eq!(cells.shape, Shape::Triangles);
    let records = &cells.cells.records.values;
    assert_eq!(records.len(), 6);
    assert_eq!(cells.counts.leaves, 2);
    assert!((world(&cells, &records[4]) - DVec3::new(1.0, 1.0, 5.0)).length() < 1e-6);
    assert!(records.iter().all(|r| r.code & NORMAL_MISSING == 0));
    assert!(oct_decode(records[3].spare).angle_between(Vec3::Z) < 1e-3);
    assert_eq!(records[5].probability, 0.8);
    let flat = mesh_page(&budget, &Presentation::default(), &mesh(&budget, &square(false))).unwrap();
    assert!(flat.cells.records.values.iter().all(|r| r.code & NORMAL_MISSING != 0));
}

#[test]
fn a_clip_keeps_whole_triangles_by_their_centroid() {
    let budget = CpuBudget::new(1 << 20);
    let page = mesh(&budget, &square(true));
    let presentation = Presentation {
        clip: Some(ClipBounds { min_m: [0.5, -1.0, 4.0], max_m: [2.0, 0.6, 6.0], ..Default::default() }),
        ..Default::default()
    };
    let cells = mesh_page(&budget, &presentation, &page).unwrap();
    assert_eq!(cells.cells.records.values.len(), 3, "only the lower-right triangle's centroid is inside");
}

#[test]
fn normal_arrows_follow_only_supplied_normals() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = surfels(&budget, [0.1, 0.2, 0.3], 0b111, 0b011);
    let pages = vec![Arc::new(surfel_page(&budget, &Presentation::default(), &schema, &page).unwrap())];
    let every = NormalArrows { every: 1, length_m: 0.5, rgba: [255, 255, 255, 255] };
    let arrows = normal_arrows(&budget, &every, &pages).unwrap();
    assert_eq!(arrows.cells.records.values.len(), 2 * 2 * 6);
    assert!(arrows.cells.records.values.iter().all(|r| r.code & ARROW != 0));
    let sparse = normal_arrows(&budget, &NormalArrows { every: 2, ..every }, &pages).unwrap();
    assert_eq!(sparse.cells.records.values.len(), 2 * 6);
}

#[test]
fn rays_hit_discs_within_their_radius_and_triangles_within_their_edges() {
    use super::super::octree::Cell;
    use super::super::pick::{disc, triangle};
    let flat = Cell { center: [0.0, 0.0, 0.0], half: 0.5, rgba: 0, code: 0, probability: -1.0, spare: oct_encode(Vec3::Z) };
    assert!(disc(Vec3::new(0.2, 0.0, 5.0), -Vec3::Z, &flat).is_some_and(|t| (t - 5.0).abs() < 1e-4));
    assert_eq!(disc(Vec3::new(0.6, 0.0, 5.0), -Vec3::Z, &flat), None);
    let facing = Cell { code: NORMAL_MISSING, ..flat };
    assert!(disc(Vec3::new(5.0, 0.0, 0.0), -Vec3::X, &facing).is_some_and(|t| (t - 5.0).abs() < 1e-4));
    let corner = |p: [f32; 3]| Cell { center: p, ..flat };
    let tri = [corner([0.0, 0.0, 0.0]), corner([1.0, 0.0, 0.0]), corner([0.0, 1.0, 0.0])];
    assert_eq!(triangle(Vec3::new(0.2, 0.2, 3.0), -Vec3::Z, &tri), Some(3.0));
    assert_eq!(triangle(Vec3::new(0.8, 0.8, 3.0), -Vec3::Z, &tri), None);
    assert_eq!(triangle(Vec3::new(0.2, 0.2, -3.0), Vec3::Z, &tri), Some(3.0), "either side");
}

#[test]
fn a_dirty_mesh_block_rebuilds_only_its_own_records() {
    use super::super::tests::{generation, lifetime};
    let budget = CpuBudget::new(1 << 22);
    let block = |id: u64, lift: f32| {
        let mut b = square(true);
        b.positions.iter_mut().for_each(|p| p[2] += lift);
        let bytes = encode_mesh_block(&b).unwrap();
        let descriptor = PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: id },
            encoding: PageEncoding::MeshBlockV1,
            items: 2,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        };
        let mut upload = PageUpload::new(&budget, descriptor).unwrap();
        upload.push(0, &bytes).unwrap();
        upload.finish_mesh().unwrap().into_page()
    };
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("surface-test", "mesh").unwrap(),
        shape: DataShape::MeshBlocks { frame: DataFrame::default() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation::default(),
    };
    let (a, b) = (block(0, 0.0), block(1, 2.0));
    let revision = |generation| DatasetRevision { incarnation: 1, generation };
    let key = descriptor.key.clone();
    let first = prepare(&budget, key.clone(), revision(1), lifetime(), generation(&budget, &descriptor, &[a.clone(), b]), None).unwrap();
    let second = prepare(&budget, key, revision(2), lifetime(), generation(&budget, &descriptor, &[a, block(1, 3.0)]), Some(&first)).unwrap();
    assert!(Arc::ptr_eq(&first.pages[0].cells, &second.pages[0].cells), "the clean block keeps its uploaded records");
    assert!(!Arc::ptr_eq(&first.pages[1].cells, &second.pages[1].cells));
}
