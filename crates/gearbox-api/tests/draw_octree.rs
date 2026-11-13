use gearbox_api::wire::draw_data::*;

fn config(depth: u8) -> OctreeConfig {
    OctreeConfig {
        leaf_resolution_m: 0.125,
        max_depth: depth,
        free_max: 0.3,
        occupied_min: 0.7,
    }
}

fn leaf(key: VoxelKey, probability: Option<f32>) -> OccupancyLeaf {
    OccupancyLeaf {
        key,
        value: OccupancyValue {
            probability,
            confidence: None,
            class_id: None,
        },
    }
}

#[test]
fn every_depth_and_signed_tile_roundtrips_exact_integer_bounds() {
    for max_depth in 0..=MAX_OCTREE_DEPTH {
        let config = config(max_depth);
        for depth in 0..=max_depth {
            for tile in [[0, 0, 0], [-1, -7, 2], [i64::MIN, 0, i64::MAX]] {
                let mask = (1u64 << (depth as u32 * 3)) - 1;
                for morton in [0, mask, 0x1249249249249249 & mask] {
                    let key = VoxelKey {
                        tile,
                        depth,
                        morton,
                    };
                    let bounds = key.grid_bounds(&config).unwrap();
                    assert_eq!(
                        VoxelKey::from_grid_cell(bounds.min, depth, &config).unwrap(),
                        key
                    );
                    let last = bounds.min.map(|v| v + i128::from(bounds.size) - 1);
                    assert_eq!(VoxelKey::from_grid_cell(last, depth, &config).unwrap(), key);
                    if let Some(parent) = key.parent() {
                        assert!(parent.contains(&key));
                        assert_eq!(parent.child((morton & 7) as u8, &config).unwrap(), key);
                    }
                }
            }
        }
    }
}

#[test]
fn child_digits_are_xyz_bits_with_most_significant_level_first() {
    let c = config(3);
    let root = VoxelKey {
        tile: [0; 3],
        depth: 0,
        morton: 0,
    };
    for child in 0..8 {
        let key = root.child(child, &c).unwrap();
        let bounds = key.grid_bounds(&c).unwrap();
        assert_eq!(
            bounds.min,
            std::array::from_fn(|axis| i128::from((child >> axis) & 1) * 4)
        );
        assert_eq!(bounds.size, 4);
    }
    let key = root
        .child(1, &c)
        .unwrap()
        .child(2, &c)
        .unwrap()
        .child(4, &c)
        .unwrap();
    assert_eq!(key.morton, 0b001_010_100);
    assert_eq!(key.grid_bounds(&c).unwrap().min, [4, 2, 1]);
}

#[test]
fn negative_cells_use_floor_division_without_world_growth_rekeying() {
    let c = config(3);
    let a = VoxelKey::from_grid_cell([-1, -8, -9], 3, &c).unwrap();
    assert_eq!(a.tile, [-1, -1, -2]);
    assert_eq!(a.grid_bounds(&c).unwrap().min, [-1, -8, -9]);
    let distant = VoxelKey::from_grid_cell([1_000_000, -1_000_000, 0], 3, &c).unwrap();
    validate_leaf_set(&c, &[leaf(a, Some(1.0)), leaf(distant, None)]).unwrap();
    assert_eq!(VoxelKey::from_grid_cell([-1, -8, -9], 3, &c).unwrap(), a);
}

#[test]
fn invalid_depth_morton_tiles_and_configurations_fail_without_panics() {
    let c = config(21);
    for key in [
        VoxelKey {
            tile: [0; 3],
            depth: 255,
            morton: 0,
        },
        VoxelKey {
            tile: [0; 3],
            depth: 21,
            morton: 1 << 63,
        },
        VoxelKey {
            tile: [0; 3],
            depth: 0,
            morton: 1,
        },
    ] {
        assert!(key.validate(&c).is_err());
        assert!(key.grid_bounds(&c).is_err());
        assert!(key.child(0, &c).is_err());
    }
    assert!(VoxelKey::from_grid_cell([i128::MAX; 3], 21, &c).is_err());
    assert!(VoxelKey::from_grid_cell([0; 3], 22, &c).is_err());
    let root = VoxelKey {
        tile: [0; 3],
        depth: 0,
        morton: 0,
    };
    assert!(root.child(8, &c).is_err());
    assert!(root.child(0, &config(0)).is_err());
    for resolution in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(
            OctreeConfig {
                leaf_resolution_m: resolution,
                ..c
            }
            .validate()
            .is_err()
        );
    }
    assert!(config(22).validate().is_err());
    for (free, occupied) in [(-0.1, 0.7), (0.3, 1.1), (0.5, 0.5), (f32::NAN, 0.7)] {
        assert!(
            OctreeConfig {
                free_max: free,
                occupied_min: occupied,
                ..c
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn unknown_free_uncertain_occupied_and_threshold_edges_are_distinct() {
    let c = config(4);
    for (probability, state) in [
        (None, OccupancyState::Unknown),
        (Some(0.0), OccupancyState::Free),
        (Some(0.3), OccupancyState::Free),
        (Some(0.5), OccupancyState::Uncertain),
        (Some(0.7), OccupancyState::Occupied),
        (Some(1.0), OccupancyState::Occupied),
    ] {
        let value = OccupancyValue {
            probability,
            confidence: Some(1.0),
            class_id: Some(0),
        };
        assert_eq!(value.state(&c).unwrap(), state);
    }
    for bad in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(
            OccupancyValue {
                probability: Some(bad),
                confidence: None,
                class_id: None
            }
            .state(&c)
            .is_err()
        );
        assert!(
            OccupancyValue {
                probability: None,
                confidence: Some(bad),
                class_id: None
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn leaf_snapshots_reject_duplicates_and_ancestor_overlaps_in_any_order() {
    let c = config(3);
    let parent = VoxelKey {
        tile: [-1, 0, 0],
        depth: 0,
        morton: 0,
    };
    let child = parent.child(3, &c).unwrap();
    let grandchild = child.child(5, &c).unwrap();
    let a = leaf(parent, Some(0.9));
    let b = leaf(grandchild, None);
    assert!(validate_leaf_set(&c, &[a, a]).is_err());
    assert!(validate_leaf_set(&c, &[a, b]).is_err());
    assert!(validate_leaf_set(&c, &[b, a]).is_err());
    let sibling = parent.child(4, &c).unwrap();
    validate_leaf_set(&c, &[b, leaf(sibling, Some(0.0))]).unwrap();
    let other_root = VoxelKey {
        tile: [0; 3],
        ..parent
    };
    validate_leaf_set(&c, &[a, leaf(other_root, None)]).unwrap();
    assert!(!parent.contains(&other_root));
}

#[test]
fn split_and_merge_results_validate_without_inventing_missing_children() {
    let c = config(3);
    let parent = VoxelKey {
        tile: [0; 3],
        depth: 0,
        morton: 0,
    };
    let children = [
        leaf(parent.child(0, &c).unwrap(), Some(0.9)),
        leaf(parent.child(7, &c).unwrap(), Some(0.1)),
    ];
    validate_leaf_set(&c, &children).unwrap();
    assert_eq!(children.len(), 2);
    validate_leaf_set(&c, &[leaf(parent, None)]).unwrap();
    validate_leaf_set(&c, &[]).unwrap();
}
