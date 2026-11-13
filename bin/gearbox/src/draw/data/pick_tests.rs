use bevy::prelude::*;
use gearbox_api::wire::draw_data::{ColumnView, Normalization, OctreeConfig, ScalarType, VoxelKey};

use super::*;

fn pick(id: u64) -> Pick {
    Pick {
        id,
        camera: Entity::PLACEHOLDER,
        eye: GlobalTransform::IDENTITY,
        fov: 1.0,
        aspect: 1.5,
        ndc: Vec2::ZERO,
        prefix: String::new(),
        occluder: f32::INFINITY,
    }
}

#[test]
fn a_ray_enters_boxes_ahead_of_it_and_misses_the_rest() {
    let (low, high) = (Vec3::splat(-1.0), Vec3::splat(1.0));
    assert_eq!(enter(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, low, high), Some(4.0));
    assert_eq!(enter(Vec3::ZERO, Vec3::Y, low, high), Some(0.0));
    assert_eq!(enter(Vec3::new(-5.0, 2.0, 0.0), Vec3::X, low, high), None);
    assert_eq!(enter(Vec3::new(5.0, 0.0, 0.0), Vec3::X, low, high), None);
    let slanted = enter(Vec3::new(-3.0, -3.0, 0.0), Vec3::new(1.0, 1.0, 0.0).normalize(), low, high).unwrap();
    assert!((slanted - 2.0 * 2f32.sqrt()).abs() < 1e-5);
}

#[test]
fn a_drawn_cell_gives_back_the_exact_leaf_key_at_every_depth() {
    let config = OctreeConfig { leaf_resolution_m: 0.1, max_depth: 8, free_max: 0.3, occupied_min: 0.7 };
    for (cell, depth) in [([0i128, 0, 0], 8u8), ([-1, 0, 3], 6), ([37, -12, 5], 5), ([-300, 41, -2], 3), ([0, 0, 0], 0)] {
        let finest = cell.map(|c| c << (config.max_depth - depth));
        let key = VoxelKey::from_grid_cell(finest, depth, &config).unwrap();
        let (center, _) = super::super::octree::geometry(&config, &key).unwrap();
        let rebase = [12.3, -4.5, 0.25];
        let local = std::array::from_fn(|axis| (center[axis] - rebase[axis]) as f32);
        assert_eq!(leaf_key(&config, rebase, local, depth), Some(key), "cell {cell:?} depth {depth}");
    }
}

#[test]
fn the_queue_stays_bounded_and_answers_what_it_drops() {
    let picks = Picks::default();
    for id in 1..=MAX_PICKS as u64 + 1 {
        picks.submit(pick(id));
    }
    assert_eq!(picks.pending(), MAX_PICKS);
    assert_eq!(picks.take(), vec![(1, Found::Stale("superseded by a newer pick"))]);
    picks.cancel(2);
    assert_eq!(picks.pending(), MAX_PICKS - 1);
    assert!(picks.take().is_empty());
}

#[test]
fn the_cursor_at_the_centre_looks_along_the_camera() {
    let mut centre = pick(1);
    centre.eye = GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 3.0).looking_at(Vec3::new(4.0, 0.0, -1.0), Vec3::Y));
    let forward = centre.eye.forward().as_vec3();
    assert!(centre.direction().distance(forward) < 1e-5);
    centre.ndc = Vec2::new(1.0, 0.0);
    let right = centre.direction();
    let expected = (0.5f32).tan() * 1.5;
    let along = right.dot(forward);
    assert!(((right.dot(centre.eye.right().as_vec3()) / along) - expected).abs() < 1e-4);
}

#[test]
fn attribute_values_decode_with_units_and_explicit_invalid() {
    let floats: Vec<u8> = [2.5f32, -1.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let column = ColumnView { values: &floats, validity: &[0b01] };
    assert_eq!(value(column, ScalarType::F32, 1, Normalization::None, 0), "2.5");
    assert_eq!(value(column, ScalarType::F32, 1, Normalization::None, 1), "invalid");
    let rgba = [255u8, 128, 0, 255];
    let color = ColumnView { values: &rgba, validity: &[] };
    assert_eq!(value(color, ScalarType::U8, 4, Normalization::Unorm, 0), "255 128 0 255 (unorm)");
    let rings: Vec<u8> = [7u16, 9].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(value(ColumnView { values: &rings, validity: &[] }, ScalarType::U16, 1, Normalization::None, 1), "9");
}

#[test]
fn a_hit_on_a_superseded_generation_is_answered_stale() {
    use gearbox_api::wire::draw_data::{DataKey, DatasetRevision};
    let key = DataKey::new("vfx-test/pick", "wall").unwrap();
    let seen = DatasetRevision { incarnation: 1, generation: 3 };
    let hit = Found::Cloud(CloudHit {
        key: key.clone(),
        revision: seen,
        scan: 1,
        page: 0,
        source_id: Some(7),
        coarse: false,
        at: Vec3::ZERO,
        source_m: None,
        attributes: Vec::new(),
    });
    assert_eq!(verdict(hit.clone(), |_| Some(seen)), hit);
    let newer = DatasetRevision { generation: 4, ..seen };
    assert_eq!(verdict(hit.clone(), |_| Some(newer)), Found::Stale("the dataset changed before the answer"));
    assert!(matches!(verdict(hit, |_| None), Found::Stale(_)));
    assert_eq!(verdict(Found::Nothing, |_| None), Found::Nothing);
}

#[test]
fn a_page_off_the_cursor_is_skipped_and_one_around_the_eye_never_is() {
    let size = Vec2::new(1000.0, 800.0);
    let clip_from_world = Mat4::perspective_infinite_reverse_rh(1.0, 1.25, 0.1)
        * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
    let bounds = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];
    assert!(covers(clip_from_world, bounds, [0.0; 3], Vec2::ZERO, size, |_| 1.0));
    assert!(!covers(clip_from_world, bounds, [0.0; 3], Vec2::new(400.0, 0.0), size, |_| 1.0));
    assert!(covers(clip_from_world, bounds, [0.0; 3], Vec2::new(400.0, 0.0), size, |_| 400.0), "a wide glyph reaches it");
    let around = [[-1.0, -1.0, 5.0], [1.0, 1.0, 15.0]];
    assert!(covers(clip_from_world, around, [0.0; 3], Vec2::new(499.0, 399.0), size, |_| 0.0));
    let rebased = [[9.0, -1.0, -1.0], [11.0, 1.0, 1.0]];
    assert!(covers(clip_from_world, rebased, [10.0, 0.0, 0.0], Vec2::ZERO, size, |_| 1.0));
}
