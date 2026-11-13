use super::*;
use gearbox_api::draw_data::{DatasetMetadata, Generation, PageUpload};
use gearbox_api::wire::draw_data::{
    DataFrame, OccupancyValue, PageAddress, PageDescriptor, PageEncoding, SCHEMA_REVISION,
    StreamMode, encode_occupancy_page, page_crc32,
};

fn config() -> OctreeConfig {
    OctreeConfig {
        leaf_resolution_m: 0.1,
        max_depth: 6,
        free_max: 0.3,
        occupied_min: 0.7,
    }
}

fn leaf(depth: u8, cell: [i128; 3], probability: Option<f32>) -> OccupancyLeaf {
    let size = 1i128 << (config().max_depth - depth);
    OccupancyLeaf {
        key: key_at(&config(), depth, cell.map(|c| c * size)),
        value: OccupancyValue {
            probability,
            confidence: None,
            class_id: None,
        },
    }
}

fn dataset(
    budget: &CpuBudget,
    leaves: &[OccupancyLeaf],
    display: OctreeDisplay,
) -> Result<Arc<Prepared>, StorageError> {
    let bytes = encode_occupancy_page(&config(), leaves).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: leaves.len() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    upload.push(0, &bytes).unwrap();
    let page = upload.finish_occupancy(&config()).unwrap().into_page();
    let key = DataKey::new("octree-test", "map").unwrap();
    let descriptor = gearbox_api::wire::draw_data::DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key.clone(),
        shape: DataShape::Octree {
            config: config(),
            frame: DataFrame::default(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            octree: Some(display),
            ..Presentation::for_octree()
        },
    };
    let generation = Generation::build(
        budget,
        DatasetMetadata::new(budget, &descriptor).unwrap(),
        &[],
        &[page],
    )
    .unwrap();
    let storage = DatasetStorage::from_resident(budget, generation).unwrap();
    let revision = DatasetRevision {
        incarnation: 1,
        generation: 1,
    };
    prepare(budget, key, revision, lifetime(), storage, None)
}

fn all() -> OctreeDisplay {
    OctreeDisplay {
        show_occupied: true,
        show_free: true,
        show_uncertain: true,
        show_unknown: false,
        ..OctreeDisplay::default()
    }
}

#[test]
fn cell_records_stay_within_the_compact_budget() {
    assert_eq!(size_of::<Cell>(), 32);
    assert!(size_of::<Cell>() <= 64);
}

#[test]
fn negative_cells_round_trip_through_keys_and_geometry() {
    let config = config();
    for depth in 0..=config.max_depth {
        let size = 1i128 << (config.max_depth - depth);
        for cell in [[-1i128, 0, 0], [-65, 3, -2], [64, -64, 1], [0, 0, 0]] {
            let key = key_at(&config, depth, cell.map(|c| c * size));
            key.validate(&config).unwrap();
            let bounds = key.grid_bounds(&config).unwrap();
            assert_eq!(bounds.min, cell.map(|c| c * size));
            let (center, half) = geometry(&config, &key).unwrap();
            let expected = cell.map(|c| (c as f64 + 0.5) * size as f64 * config.leaf_resolution_m);
            assert!((center - DVec3::from_array(expected)).length() < 1e-9);
            assert!((half - size as f64 * config.leaf_resolution_m * 0.5).abs() < 1e-12);
        }
    }
}

#[test]
fn states_follow_thresholds_and_display_flags() {
    let budget = CpuBudget::new(8 << 20);
    let leaves = [
        leaf(4, [0, 0, 0], Some(0.9)),
        leaf(4, [1, 0, 0], Some(0.1)),
        leaf(4, [2, 0, 0], Some(0.5)),
        leaf(4, [3, 0, 0], None),
    ];
    let shown = dataset(&budget, &leaves, all()).unwrap();
    assert_eq!(
        (
            shown.counts.occupied,
            shown.counts.free,
            shown.counts.uncertain
        ),
        (1, 1, 1)
    );
    assert_eq!(shown.counts.cells, 3);
    let hidden = dataset(
        &budget,
        &leaves,
        OctreeDisplay {
            show_free: false,
            show_uncertain: false,
            ..all()
        },
    )
    .unwrap();
    assert_eq!(hidden.counts.cells, 1);
    let parts: Vec<_> = shown.pages.iter().map(|page| (page.opaque, page.cells.records.values.len())).collect();
    assert_eq!(parts, [(true, 1), (false, 2)], "the occupied cell keeps depth beside translucent ones");
}

/// The hidden-face bits of the drawn cell standing where `key` stands.
fn hidden_bits(prepared: &Prepared, key: &VoxelKey) -> u32 {
    let (center, _) = geometry(&config(), key).unwrap();
    prepared
        .pages
        .iter()
        .flat_map(|page| page.cells.records.values.iter().map(move |cell| (page.rebase_m, cell)))
        .find(|(rebase, cell)| (DVec3::from_array(*rebase) + DVec3::from_array(cell.center.map(f64::from)) - center).length() < 1e-4)
        .map(|(_, cell)| cell.code >> HIDDEN_SHIFT & 0x3f)
        .unwrap()
}

#[test]
fn faces_covered_by_a_same_state_neighbour_are_hidden_except_in_wire() {
    let budget = CpuBudget::new(8 << 20);
    let near = leaf(6, [0, 0, 0], Some(0.9));
    let next = leaf(6, [1, 0, 0], Some(0.9));
    let coarse = leaf(5, [1, 0, 0], Some(0.9));
    let free = leaf(6, [0, 1, 0], Some(0.1));
    let leaves = [near, next, coarse, free];
    let solid = dataset(&budget, &leaves, all()).unwrap();
    assert_eq!(hidden_bits(&solid, &near.key), 0b01, "+x touches an occupied twin; +y is free");
    assert_eq!(hidden_bits(&solid, &next.key), 0b11, "the twin over -x, the coarser cell over +x");
    assert_eq!(hidden_bits(&solid, &coarse.key), 0, "a finer neighbour covers only part of a face");
    assert_eq!(hidden_bits(&solid, &free.key), 0, "another state never hides a face");
    let wire = dataset(&budget, &leaves, OctreeDisplay { style: VoxelStyle::Wire, ..all() }).unwrap();
    assert!(leaves.iter().all(|leaf| hidden_bits(&wire, &leaf.key) == 0));
}

#[test]
fn opaque_cells_draw_apart_from_translucent_ones_on_the_same_page() {
    let budget = CpuBudget::new(8 << 20);
    let leaves = [leaf(4, [0, 0, 0], Some(0.9)), leaf(4, [1, 0, 0], Some(0.1)), leaf(4, [2, 0, 0], Some(0.5))];
    let mixed = dataset(&budget, &leaves, all()).unwrap();
    let parts: Vec<_> = mixed
        .pages
        .iter()
        .map(|page| (page.address.scan_key & TRANSLUCENT_PART != 0, page.opaque, page.cells.records.values.len()))
        .collect();
    assert_eq!(parts, [(false, true, 1), (true, false, 2)]);
    assert!(mixed.pages.iter().all(|page| page.source.is_some()));
    assert_eq!((mixed.counts.cells, mixed.counts.occupied, mixed.counts.free), (3, 1, 1));
    let occupied = dataset(&budget, &leaves[..1], all()).unwrap();
    assert_eq!(occupied.pages.len(), 1);
    assert!(occupied.pages[0].opaque);
}

#[test]
fn aggregates_are_occupied_if_any_and_free_only_when_fully_observed() {
    let budget = CpuBudget::new(8 << 20);
    let mut leaves = Vec::new();
    for child in 0..8i128 {
        let base = [child & 1, child >> 1 & 1, child >> 2 & 1];
        leaves.push(leaf(3, base, Some(0.05)));
        leaves.push(leaf(
            3,
            [base[0] + 2, base[1], base[2]],
            Some(if child == 5 { 0.95 } else { 0.05 }),
        ));
        if child != 2 {
            leaves.push(leaf(3, [base[0] + 4, base[1], base[2]], Some(0.05)));
        }
    }
    let limited = OctreeDisplay {
        depth_limit: Some(2),
        ..all()
    };
    let prepared = dataset(&budget, &leaves, limited).unwrap();
    assert_eq!(prepared.counts.aggregated, leaves.len() as u64);
    assert_eq!(
        (
            prepared.counts.free,
            prepared.counts.occupied,
            prepared.counts.mixed
        ),
        (1, 1, 1),
        "complete free parent, one occupied child, one unobserved child"
    );
    assert!(prepared.cells().all(|cell| cell.code & AGGREGATE != 0));
}

#[test]
fn bounded_roi_shows_only_uncovered_cells_as_unknown() {
    let budget = CpuBudget::new(8 << 20);
    let leaves = [leaf(4, [0, 0, 0], Some(0.9)), leaf(4, [1, 0, 0], Some(0.1))];
    let cell = 0.1 * 4.0;
    let display = OctreeDisplay {
        show_unknown: true,
        roi: Some(ClipBounds {
            min_m: [0.0, 0.0, 0.0],
            max_m: [4.0 * cell, cell, cell],
            ..Default::default()
        }),
        unknown_cell_budget: 100,
        depth_limit: Some(4),
        ..all()
    };
    let prepared = dataset(&budget, &leaves, display.clone()).unwrap();
    assert_eq!(prepared.counts.unknown, 2);
    assert!(
        prepared
            .cells()
            .filter(|cell| cell.code & IMPLICIT != 0)
            .count()
            == 2
    );
    let coarse = dataset(
        &budget,
        &leaves,
        OctreeDisplay {
            unknown_cell_budget: 1,
            ..display
        },
    )
    .unwrap();
    assert!(coarse.counts.unknown <= 1);
}

fn lifetime() -> DatasetLifetime {
    let mut registry = gearbox_api::draw_data::DrawingStore::new(1, 1).unwrap();
    let key = DataKey::new("octree-test", "lifetime").unwrap();
    let permit = registry
        .begin_dataset(&key, DatasetKind::Octree, None)
        .unwrap();
    registry.commit_dataset(permit, Arc::new(())).unwrap();
    registry
        .get(&(key.namespace, key.id))
        .unwrap()
        .lifetime
        .clone()
}

fn stored(
    budget: &CpuBudget,
    id: u64,
    leaves: &[OccupancyLeaf],
) -> Arc<gearbox_api::draw_data::StoredPage> {
    let bytes = encode_occupancy_page(&config(), leaves).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: id,
        },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: leaves.len() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    upload.push(0, &bytes).unwrap();
    upload.finish_occupancy(&config()).unwrap().into_page()
}

fn storage(
    budget: &CpuBudget,
    pages: &[Arc<gearbox_api::draw_data::StoredPage>],
    presentation: Presentation,
) -> Arc<DatasetStorage> {
    let descriptor = gearbox_api::wire::draw_data::DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("octree-test", "map").unwrap(),
        shape: DataShape::Octree {
            config: config(),
            frame: DataFrame::default(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation,
    };
    let generation = Generation::build(
        budget,
        DatasetMetadata::new(budget, &descriptor).unwrap(),
        &[],
        pages,
    )
    .unwrap();
    DatasetStorage::from_resident(budget, generation).unwrap()
}

#[test]
fn a_delta_rebuilds_only_the_pages_it_replaced() {
    let budget = CpuBudget::new(8 << 20);
    let presentation = Presentation {
        octree: Some(all()),
        ..Presentation::for_octree()
    };
    let a = stored(&budget, 0, &[leaf(4, [0, 0, 0], Some(0.9))]);
    let b = stored(&budget, 1, &[leaf(4, [3, 0, 0], Some(0.9))]);
    let b2 = stored(&budget, 1, &[leaf(4, [3, 0, 0], Some(0.1))]);
    let key = DataKey::new("octree-test", "map").unwrap();
    let revision = |generation| DatasetRevision {
        incarnation: 1,
        generation,
    };
    let first = prepare(
        &budget,
        key.clone(),
        revision(1),
        lifetime(),
        storage(&budget, &[a.clone(), b], presentation.clone()),
        None,
    )
    .unwrap();
    let second = prepare(
        &budget,
        key.clone(),
        revision(2),
        lifetime(),
        storage(&budget, &[a, b2], presentation.clone()),
        Some(&first),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&first.pages[0].cells, &second.pages[0].cells));
    assert!(!Arc::ptr_eq(&first.pages[1].cells, &second.pages[1].cells));
    assert_eq!((second.counts.occupied, second.counts.free), (1, 1));
    let restyled = Presentation {
        opacity: 0.5,
        ..presentation
    };
    let third = prepare(
        &budget,
        key,
        revision(3),
        lifetime(),
        storage(
            &budget,
            &[second.pages[0].source.clone().unwrap()],
            restyled,
        ),
        Some(&second),
    )
    .unwrap();
    assert!(!Arc::ptr_eq(&second.pages[0].cells, &third.pages[0].cells));
}

#[test]
fn clip_bounds_slice_cells_without_changing_the_map() {
    let budget = CpuBudget::new(8 << 20);
    let leaves: Vec<_> = (0..4).map(|z| leaf(4, [0, 0, z], Some(0.9))).collect();
    let cell = 0.4;
    let sliced = Presentation {
        octree: Some(all()),
        clip: Some(ClipBounds {
            min_m: [0.0, 0.0, 1.5 * cell],
            max_m: [cell, cell, 2.5 * cell],
            ..Default::default()
        }),
        ..Presentation::for_octree()
    };
    let key = DataKey::new("octree-test", "map").unwrap();
    let revision = DatasetRevision {
        incarnation: 1,
        generation: 1,
    };
    let page = stored(&budget, 0, &leaves);
    let prepared = prepare(
        &budget,
        key,
        revision,
        lifetime(),
        storage(&budget, &[page], sliced),
        None,
    )
    .unwrap();
    assert_eq!(prepared.counts.leaves, 4);
    assert_eq!(prepared.counts.cells, 2);
}

#[test]
fn a_turned_or_inverted_clip_keeps_cells_by_their_whole_extent() {
    use bevy::math::{DQuat, DVec3};
    use gearbox_api::wire::draw_data::ClipBounds;
    let q = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4);
    let slab = ClipBounds {
        min_m: [-10.0, -0.1, -10.0],
        max_m: [10.0, 0.1, 10.0],
        rotation_wxyz: Some([q.w, q.x, q.y, q.z]),
        invert: false,
    };
    assert!(super::kept(DVec3::new(2.0, 2.0, 0.0), 0.2, &slab));
    assert!(!super::kept(DVec3::new(2.0, -2.0, 0.0), 0.2, &slab));
    assert!(super::kept(DVec3::new(0.4, 0.0, 0.0), 0.2, &slab), "a corner reaches into the slab");
    let hole = ClipBounds { min_m: [-1.0; 3], max_m: [1.0; 3], rotation_wxyz: None, invert: true };
    assert!(!super::kept(DVec3::ZERO, 0.5, &hole), "wholly inside the inverted box");
    assert!(super::kept(DVec3::new(0.8, 0.0, 0.0), 0.5, &hole), "partly outside it");
    assert!(super::kept(DVec3::new(5.0, 0.0, 0.0), 0.5, &hole));
}
