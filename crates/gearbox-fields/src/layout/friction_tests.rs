use super::*;
use crate::HeightGrid;

fn layout() -> FieldLayout {
    serde_json::from_str(
        r#"{
        "default": "grassland", "default_friction": 0.8,
        "fields": [
            {"name":"left", "profile":"grassland", "min":[-2,-2], "max":[0,2], "friction":0},
            {"name":"right", "profile":"concrete", "min":[0,-2], "max":[2,2], "friction":1.2}
        ]
    }"#,
    )
    .unwrap()
}

#[test]
fn friction_samples_are_row_major_and_preserve_authored_zero() {
    let grid = HeightGrid::sample(4.0, 1.0, |_, _| 0.0);
    let mut layout = layout();
    let samples = layout.friction_samples(&grid, 1.4).unwrap().unwrap();
    assert_eq!(samples.len(), 25);
    assert_eq!(&samples[5..10], &[0.0, 0.0, 1.2, 1.2, 0.8]);
    assert_eq!(&samples[20..25], &[0.8; 5]);
    layout.fields.reverse();
    assert_eq!(
        layout.friction_samples(&grid, 1.4).unwrap().unwrap(),
        samples
    );
}

#[test]
fn absent_friction_uses_collider_without_allocating_a_map() {
    let grid = HeightGrid::sample(800.0, 1.0, |_, _| 0.0);
    assert!(
        FieldLayout::default()
            .friction_samples(&grid, 1.4)
            .unwrap()
            .is_none()
    );
    let grid = HeightGrid::sample(4.0, 1.0, |_, _| 0.0);
    let mut layout = layout();
    layout.default_friction = None;
    layout.fields[1].friction = None;
    assert_eq!(
        layout.friction_samples(&grid, 0.6).unwrap().unwrap()[7],
        0.6
    );
    layout.default_friction = Some(0.0);
    assert_eq!(
        layout.friction_samples(&grid, 0.6).unwrap().unwrap()[7],
        0.0
    );
}

#[test]
fn invalid_material_geometry_or_grid_is_rejected() {
    let grid = HeightGrid::sample(4.0, 1.0, |_, _| 0.0);
    for invalid in [-0.1, f64::NAN, f64::INFINITY] {
        let mut layout = layout();
        layout.fields[1].friction = Some(invalid);
        assert!(layout.friction_samples(&grid, 1.0).is_err());
        layout.fields[1].friction = None;
        layout.default_friction = Some(invalid);
        assert!(layout.friction_samples(&grid, 1.0).is_err());
        assert!(self::layout().friction_samples(&grid, invalid).is_err());
    }
    let mut layout = layout();
    layout.fields[1].min[0] = -0.1;
    assert!(layout.friction_samples(&grid, 1.0).is_err());
    layout.fields[1].min[0] = 0.0;
    layout.fields[1].max[0] = 3.0;
    assert!(layout.friction_samples(&grid, 1.0).is_err());
    let mut invalid_grid = HeightGrid::sample(4.0, 1.0, |_, _| 0.0);
    invalid_grid.cols = usize::MAX;
    assert!(self::layout().friction_samples(&invalid_grid, 1.0).is_err());
}
