use super::*;
use gearbox_api::wire::draw::{colormap, point_style, rgba};

fn entity() -> DrawEntity {
    DrawEntity::points(
        "legacy-test",
        "samples",
        &[[1.0, 2.0, 3.0], [-2.0, 0.0, 5.0]],
    )
    .with_style(draw_style::SPLAT)
    .with_color(rgba(80, 160, 220, 255))
}

#[test]
fn compact_layout_has_two_sixteen_byte_records_and_no_indices() {
    assert_eq!(size_of::<Position>(), 16);
    assert_eq!(size_of::<Sample>(), 16);
    assert_eq!(std::mem::offset_of!(Sample, radius), 4);
    assert_eq!(std::mem::offset_of!(Sample, phase), 8);
    let budget = CpuBudget::new(1 << 20);
    let points = Points::prepare(&budget, &entity(), "legacy-test", "samples", None)
        .unwrap()
        .unwrap();
    assert_eq!(points.positions.values.len(), 2);
    assert_eq!(uniform_bytes(&points.uniform).len(), 128);
    drop(points);
    assert_eq!(budget.usage().committed(), 0);
}

use super::super::style::uniform_bytes;

#[test]
fn color_radius_and_effect_changes_reuse_unchanged_positions() {
    let budget = CpuBudget::new(1 << 20);
    let mut e = entity();
    let first = Points::prepare(&budget, &e, "legacy-test", "samples", None)
        .unwrap()
        .unwrap();
    e.color = rgba(255, 20, 80, 128);
    let recolored = Points::prepare(&budget, &e, "legacy-test", "samples", Some(&first))
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&first.positions, &recolored.positions));
    assert_eq!(first.center, recolored.center);
    assert!(!Arc::ptr_eq(&first.samples, &recolored.samples));
    e = e.with_blur(0.8);
    e.glow = 0.5;
    e.px = 100.0;
    let restyled = Points::prepare(&budget, &e, "legacy-test", "samples", Some(&recolored))
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&recolored.positions, &restyled.positions));
    assert!(Arc::ptr_eq(&recolored.samples, &restyled.samples));
    assert_eq!(recolored.center, restyled.center);
    e.points[0] = 2.0;
    let moved = Points::prepare(&budget, &e, "legacy-test", "samples", Some(&restyled))
        .unwrap()
        .unwrap();
    assert!(!Arc::ptr_eq(&restyled.positions, &moved.positions));
    assert_ne!(restyled.center, moved.center);
    drop((first, recolored, restyled, moved));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn transparent_sort_center_matches_transformed_generic_mesh_bounds() {

    let budget = CpuBudget::new(1 << 20);
    let e = entity().with_style(draw_style::PLAIN);
    let points = Points::prepare(&budget, &e, "legacy-test", "samples", None)
        .unwrap()
        .unwrap();
    let mesh = super::super::super::mesh::build(&e, "legacy-test", "samples")
        .strokes
        .into_mesh();
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("legacy points mesh has float3 positions");
    };
    let (min, max) = positions
        .iter()
        .map(|p| Vec3::from_array(*p))
        .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let center = (min + max) / 2.0;
    assert_eq!(points.center, center);
    for world in [
        Mat4::IDENTITY,
        Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 0.5, 3.0),
            Quat::from_euler(EulerRot::ZYX, 0.7, -0.3, 0.2),
            Vec3::new(30.0, -40.0, 7.0),
        ),
    ] {
        assert_eq!(points.sort_center(world), world.transform_point3(center));
        assert!(
            points
                .sort_center(world)
                .distance(world.transform_point3(Vec3::ZERO))
                > 1.0
        );
    }
}

#[test]
fn unsupported_decorations_labels_and_drape_remain_explicit_fallbacks() {
    let budget = CpuBudget::new(1 << 20);
    for style in [
        draw_style::BEACON,
        draw_style::HOLO,
        draw_style::SHIELD,
        draw_style::ALERT,
        draw_style::DECAL,
    ] {
        assert!(
            Points::prepare(
                &budget,
                &entity().with_style(style),
                "legacy-test",
                "samples",
                None
            )
            .unwrap()
            .is_none()
        );
    }
    for flag in [draw_flag::DRAPE, 1 << 30] {
        assert!(
            Points::prepare(
                &budget,
                &entity().with_flags(flag),
                "legacy-test",
                "samples",
                None
            )
            .unwrap()
            .is_none()
        );
    }
    let e = entity()
        .with_labels(&["one", "two"])
        .with_flags(draw_flag::LABELS);
    assert!(
        Points::prepare(&budget, &e, "legacy-test", "samples", None)
            .unwrap()
            .is_none()
    );
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn palettes_color_precedence_and_signed_radii_match_existing_look() {
    let budget = CpuBudget::new(1 << 20);
    let mut e = entity();
    e.colormap = colormap::VIRIDIS;
    e.vmin = 0.0;
    e.vmax = 1.0;
    e.values = vec![0.0, 1.0];
    e.colors = vec![rgba(255, 0, 0, 200), 0];
    e.radii = vec![-3.0, 0.2];
    for style in [
        draw_style::PLAIN,
        draw_style::HAIRLINE,
        draw_style::ORB,
        draw_style::POINT_GLOW,
        draw_style::SPLAT,
    ] {
        for shape in [
            point_style::SPHERE,
            point_style::DISC,
            gearbox_api::wire::draw_widgets::POINT_SQUARE,
        ] {
            e.style = style;
            e.sub = shape;
            let look = super::super::super::mesh::compact_point_look(&e, "legacy-test", "samples")
                .unwrap();
            let points = Points::prepare(&budget, &e, "legacy-test", "samples", None)
                .unwrap()
                .unwrap();
            for (index, sample) in points.samples.values.iter().enumerate() {
                let (color, radius, phase) = look.sample(index);
                assert_eq!(
                    (sample.rgba.to_le_bytes(), sample.radius, sample.phase),
                    (color, radius, phase)
                );
            }
            assert_eq!(
                points.samples.values[0].rgba.to_le_bytes()[..3],
                [255, 0, 0]
            );
            assert_eq!(points.samples.values[0].radius, -3.0);
            assert_eq!(points.samples.values[1].radius, 0.2);
        }
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn allocation_failure_releases_partial_records() {
    let budget = CpuBudget::new(63);
    assert!(Points::prepare(&budget, &entity(), "legacy-test", "samples", None).is_err());
    assert_eq!(budget.usage().committed(), 0);
}
