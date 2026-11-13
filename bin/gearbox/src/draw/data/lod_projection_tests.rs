use super::*;

fn view(matrix: DMat4) -> View {
    View {
        clip_from_world: matrix,
        viewport: [1920, 1080],
        error_px: 1.5,
        hysteresis: 0.15,
    }
}

#[test]
fn sampling_error_bounds_actual_pairs_under_affine_and_perspective_transforms() {
    let bounds = [[-0.2, -0.3, 0.2], [0.4, 0.5, 0.6]];
    let perspective = DMat4::perspective_rh(1.1, 16.0 / 9.0, 0.1, 100.0);
    let mut shear = DMat4::IDENTITY;
    shear.y_axis.x = 0.7;
    for (projection, world) in [
        (DMat4::IDENTITY, DMat4::IDENTITY),
        (
            DMat4::IDENTITY,
            DMat4::from_scale(DVec3::new(1.3, 0.7, 0.5)),
        ),
        (DMat4::IDENTITY, shear * DMat4::from_rotation_z(0.7)),
        (
            perspective,
            DMat4::from_translation(DVec3::new(0.0, 0.0, -4.0)),
        ),
        (
            perspective,
            DMat4::from_translation(DVec3::new(3.0, 1.0, -4.0))
                * DMat4::from_rotation_y(0.7)
                * shear,
        ),
    ] {
        let view = view(projection);
        let points: Vec<_> = (0..27)
            .map(|i| {
                DVec3::from_array(std::array::from_fn(|axis| {
                    let step = i / 3usize.pow(axis as u32) % 3;
                    bounds[0][axis] + (bounds[1][axis] - bounds[0][axis]) * step as f64 * 0.5
                }))
            })
            .collect();
        let screen = |point: DVec3| {
            let clip = projection * world * point.extend(1.0);
            [clip.x / clip.w * 960.0, clip.y / clip.w * 540.0]
        };
        for a in &points {
            for b in &points {
                let radius = (*a - *b).length().next_up();
                let bound = view.sampling_error(world, [0.0; 3], bounds, 0.0, radius);
                assert!(bound.is_finite());
                let pa = screen(*a);
                let pb = screen(*b);
                let actual = (pa[0] - pb[0]).hypot(pa[1] - pb[1]);
                assert!(actual <= bound + 1e-10, "{actual} > {bound}");
            }
        }
    }
}

#[test]
fn sampling_error_is_monotonic_finite_when_rebased_and_unbounded_at_near_plane() {
    let projection = DMat4::perspective_rh(1.1, 16.0 / 9.0, 0.1, 100.0);
    let view = view(projection);
    let bounds = [[-0.25, -0.5, -0.25], [0.25, 0.5, 0.25]];
    let near = DMat4::from_translation(DVec3::new(0.0, 0.0, -2.0));
    let far = DMat4::from_translation(DVec3::new(0.0, 0.0, -8.0));
    let error = view.sampling_error(near, [0.0; 3], bounds, 0.0, 0.2);
    assert!(error.is_finite());
    assert!(view.sampling_error(far, [0.0; 3], bounds, 0.0, 0.2) < error);
    assert!(view.sampling_error(near, [0.0; 3], bounds, 0.0, 0.1) < error);
    assert!(view.sampling_error(near, [0.0; 3], bounds, 0.01, 0.2) > error);
    let rebase = [1e12; 3];
    let shifted = bounds.map(|side| std::array::from_fn(|axis| side[axis] + rebase[axis]));
    assert_eq!(view.sampling_error(near, rebase, shifted, 0.0, 0.2), error);
    for depth in [0.0, -0.1, -0.3, 2.0] {
        let transform = DMat4::from_translation(DVec3::new(0.0, 0.0, depth));
        assert_eq!(
            view.sampling_error(transform, [0.0; 3], bounds, 0.0, 0.2),
            f64::INFINITY
        );
    }
}
