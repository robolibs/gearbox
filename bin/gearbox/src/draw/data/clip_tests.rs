use super::super::tests::{fixture, stored};
use super::*;
use gearbox_api::draw_data::CpuBudget;

fn prepare(
    points: &[[f64; 3]],
    frame: DataFrame,
    origin: [f64; 3],
    bounds: Option<ClipBounds>,
) -> Result<Clip, StorageError> {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = fixture(points, vec![]);
    let geometry = Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, origin))?;
    let result = Clip::prepare(&geometry, &frame, bounds);
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
    result
}

#[test]
fn clip_omission_and_inclusive_boundaries_preserve_sample_centers() {
    let points = [[-1.0, 0.0, 1.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
    let all = prepare(&points, DataFrame::default(), [0.0; 3], None).unwrap();
    assert_eq!((all.selected, all.min.w), (3, 0.0));
    let bounds = ClipBounds {
        min_m: [0.0, -1.0, 0.0],
        max_m: [1.0, 1.0, 2.0],
        ..Default::default()
    };
    let partial = prepare(&points, DataFrame::default(), [0.0; 3], Some(bounds)).unwrap();
    assert_eq!((partial.selected, partial.min.w), (2, 1.0));
}

#[test]
fn clip_rotates_after_both_origins_and_rebases_large_source_coordinates() {
    let q = DQuat::from_rotation_z(0.4);
    let mut frame = DataFrame::default();
    frame.origin_m = [1e12, -1e12, 20.0];
    frame.pose.rotation_wxyz = [q.w, q.x, q.y, q.z];
    frame.pose.translation_m = [4.0, 5.0, 6.0];
    let origin = [100.0, 200.0, 300.0];
    let point = DVec3::new(0.125, 0.25, 0.5);
    let source = q * (DVec3::from_array(frame.origin_m) + DVec3::from_array(origin) + point)
        + DVec3::from_array(frame.pose.translation_m);
    let bounds = ClipBounds {
        min_m: (source - DVec3::splat(0.05)).to_array(),
        max_m: (source + DVec3::splat(0.05)).to_array(),
        ..Default::default()
    };
    let clip = prepare(
        &[point.to_array(), [2.0, 2.0, 2.0]],
        frame,
        origin,
        Some(bounds),
    )
    .unwrap();
    assert_eq!(clip.selected, 1);
    assert!(clip.min.truncate().abs().max_element() < 0.1);
    assert!(clip.max.truncate().abs().max_element() < 0.1);
}

#[test]
fn distant_and_enclosing_f64_bounds_never_upload_infinities() {
    let points = [[0.0; 3], [1.0; 3]];
    let outside = prepare(
        &points,
        DataFrame::default(),
        [0.0; 3],
        Some(ClipBounds {
            min_m: [1e300; 3],
            max_m: [2e300; 3],
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!((outside.selected, outside.min.w), (0, 2.0));
    let inside = prepare(
        &points,
        DataFrame::default(),
        [0.0; 3],
        Some(ClipBounds {
            min_m: [-1e300; 3],
            max_m: [1e300; 3],
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(inside.selected, 2);
    assert!(inside.min.is_finite() && inside.max.is_finite());
    let empty = prepare(&[], DataFrame::default(), [0.0; 3], inside.bounds).unwrap();
    assert_eq!(empty.selected, 0);
}

#[test]
fn intersecting_clip_boundaries_must_preserve_millimetre_precision() {
    let points = [[0.0; 3], [2.0f64.powi(30), 0.0, 0.0]];
    assert!(
        prepare(
            &points,
            DataFrame::default(),
            [0.0; 3],
            Some(ClipBounds {
                min_m: [2.0f64.powi(29) + 0.125, -1.0, -1.0],
                max_m: [2.0f64.powi(30), 1.0, 1.0],
                ..Default::default()
            })
        )
        .is_err()
    );
}

fn line() -> Vec<[f64; 3]> {
    (0..21).map(|i| { let t = i as f64 * 0.25 - 2.5; [t, t, 0.5] }).chain((0..21).map(|i| [i as f64 * 0.25 - 2.5, 0.0, 0.5])).collect()
}

#[test]
fn a_turned_thin_box_is_a_cross_section_along_its_own_axes() {
    let q = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4);
    let section = ClipBounds {
        min_m: [-10.0, -0.05, -10.0],
        max_m: [10.0, 0.05, 10.0],
        rotation_wxyz: Some([q.w, q.x, q.y, q.z]),
        invert: false,
    };
    let clip = prepare(&line(), DataFrame::default(), [0.0; 3], Some(section)).unwrap();
    assert_eq!((clip.selected, clip.min.w), (22, 1.0), "the diagonal plus the shared origin sample");
    let level = prepare(&line(), DataFrame::default(), [0.0; 3], Some(ClipBounds { rotation_wxyz: None, ..section })).unwrap();
    assert_eq!(level.selected, 22, "unturned, the slab keeps the x-axis row and the origin sample");
}

#[test]
fn an_inverted_box_keeps_the_outside_and_settles_the_trivial_cases() {
    let points = [[-1.0, 0.0, 1.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
    let middle = ClipBounds { min_m: [-0.5, -1.0, 0.0], max_m: [0.5, 1.0, 2.0], rotation_wxyz: None, invert: true };
    let outside = prepare(&points, DataFrame::default(), [0.0; 3], Some(middle)).unwrap();
    assert_eq!((outside.selected, outside.min.w), (2, INVERTED));
    let away = ClipBounds { min_m: [10.0; 3], max_m: [11.0; 3], ..middle };
    let all = prepare(&points, DataFrame::default(), [0.0; 3], Some(away)).unwrap();
    assert_eq!((all.selected, all.min.w), (3, 0.0));
    let around = ClipBounds { min_m: [-5.0; 3], max_m: [5.0; 3], ..middle };
    let none = prepare(&points, DataFrame::default(), [0.0; 3], Some(around)).unwrap();
    assert_eq!((none.selected, none.min.w), (0, 2.0));
}

#[test]
fn a_box_with_one_far_bound_is_a_half_space_plane() {
    let q = DQuat::from_rotation_y(0.3);
    let plane = ClipBounds {
        min_m: [-1e6, -1e6, -1e6],
        max_m: [0.0, 1e6, 1e6],
        rotation_wxyz: Some([q.w, q.x, q.y, q.z]),
        invert: false,
    };
    let clip = prepare(&line(), DataFrame::default(), [0.0; 3], Some(plane)).unwrap();
    let expected = line().iter().filter(|p| (q.inverse() * DVec3::from_array(**p)).x <= 0.0).count() as u64;
    assert_eq!(clip.selected, expected);
    assert!(expected > 0 && expected < line().len() as u64);
}

#[test]
fn a_box_turned_by_a_non_unit_quaternion_is_refused() {
    let bad = ClipBounds { rotation_wxyz: Some([2.0, 0.0, 0.0, 0.0]), ..Default::default() };
    assert!(prepare(&line(), DataFrame::default(), [0.0; 3], Some(bad)).is_err());
}
