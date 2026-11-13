use super::*;

pub fn background() -> E {
    let mut vertices = Vec::new();
    let mut colors = Vec::new();
    for row in 0..16 {
        for column in 0..16 {
            let x = column as f32 * 6.0 - 48.0;
            let y = row as f32 * 6.0 - 48.0;
            vertices.extend([
                [x, y, 1.0],
                [x + 6.0, y, 1.0],
                [x, y + 6.0, 1.0],
                [x + 6.0, y, 1.0],
                [x + 6.0, y + 6.0, 1.0],
                [x, y + 6.0, 1.0],
            ]);
            let shade = if (row + column) % 2 == 0 { 45 } else { 75 };
            colors.extend([gearbox_api::wire::draw::rgba(shade, shade, shade, 255); 6]);
        }
    }
    let mut entity = E::triangles(
        &format!("{NS}/legacy-point-background"),
        "checker",
        &vertices,
    )
    .with_style(style::PLAIN)
    .with_colors(&colors);
    entity.glow = -1.0;
    entity.pulse_hz = -1.0;
    entity
}

pub fn fixture(generic: bool) -> Vec<E> {
    let mut out = Vec::new();
    let points: Vec<_> = (0..16)
        .map(|i| {
            [
                (i % 4) as f32 * 0.35 - 0.525,
                (i / 4) as f32 * 0.35 - 0.525,
                0.0,
            ]
        })
        .collect();
    for (row, style) in [
        style::PLAIN,
        style::HAIRLINE,
        style::ORB,
        style::POINT_GLOW,
        style::SPLAT,
    ]
    .into_iter()
    .enumerate()
    {
        for column in 0..4 {
            let mut entity = E::points(
                &format!("{NS}/legacy-point-probe"),
                &format!("case-{row}-{column}"),
                &points,
            )
            .with_style(style)
            .with_color(color::MAGENTA)
            .with_pose(
                [column as f64 * 3.8 - 5.7, row as f64 * 2.8 - 5.6, 2.0],
                [1.0, 0.0, 0.0, 0.0],
            );
            entity.radii = (0..16)
                .map(|i| {
                    if column == 0 {
                        -(3.0 + (i % 4) as f32)
                    } else {
                        0.055 + (i % 4) as f32 * 0.025
                    }
                })
                .collect();
            entity.colors = (0..16)
                .map(|i| {
                    gearbox_api::wire::draw::rgba(
                        230,
                        35 + (i % 3) as u8 * 15,
                        170,
                        if column == 3 { 128 } else { 255 },
                    )
                })
                .collect();
            entity.glow = 0.4;
            entity.pulse_hz = -1.0;
            entity.flow_speed = -1.0;
            if column == 1 {
                entity.sub = w::POINT_SQUARE;
            }
            if column == 2 {
                entity.flags |= flag::FLAT;
            }
            if column == 3 {
                entity.flags |= flag::ON_TOP;
                entity = entity.with_blur(0.7);
            }
            if generic {
                entity = entity.labelled(&vec![""; points.len()]);
            }
            out.push(entity);
        }
    }
    out
}

pub fn overlap(generic: bool) -> Vec<E> {
    let pitch = 55.0_f32.to_radians();
    [
        ("far", -12.0, 0.0, [240, 30, 210]),
        ("near", 12.0, 2.0, [30, 210, 240]),
    ]
    .into_iter()
    .map(|(id, origin_y, depth, color)| {
        let points = [-1.4, 1.4].map(|x| [x, -depth * pitch.cos() - origin_y, depth * pitch.sin()]);
        let mut entity = E::points(&format!("{NS}/legacy-point-probe"), id, &points)
            .with_style(style::PLAIN)
            .with_color(gearbox_api::wire::draw::rgba(
                color[0], color[1], color[2], 160,
            ))
            .with_pose([0.0, origin_y as f64, 2.0], [1.0, 0.0, 0.0, 0.0]);
        entity.radii = vec![0.9; points.len()];
        entity.glow = -1.0;
        entity.pulse_hz = -1.0;
        entity.flow_speed = -1.0;
        if generic {
            entity = entity.labelled(&["", ""]);
        }
        entity
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_is_static_opaque_and_below_points() {
        let entity = background();
        entity.validate().unwrap();
        assert_eq!(entity.point_count(), 1536);
        assert_eq!(entity.colors.len(), entity.point_count());
        assert!(entity.points.chunks_exact(3).all(|p| p[2] == 1.0));
        assert!(entity.colors.iter().all(|c| c >> 24 == 255));
        assert_eq!(entity.style, style::PLAIN);
        assert!(entity.glow < 0.0 && entity.pulse_hz < 0.0);
    }

    #[test]
    fn generic_fixture_changes_only_invisible_labels() {
        let compact = fixture(false);
        assert_eq!(compact.len(), 20);
        for (a, mut b) in compact.into_iter().zip(fixture(true)) {
            a.validate().unwrap();
            b.validate().unwrap();
            assert_eq!(a.point_count(), 16);
            assert!(b.label_list().iter().all(String::is_empty));
            b.labels.clear();
            b.flags &= !flag::LABELS;
            assert_eq!(gearbox_api::pack(&a).wire(), gearbox_api::pack(&b).wire());
        }
    }

    #[test]
    fn overlapping_transparency_reverses_entity_origin_order_only() {
        let compact = overlap(false);
        let pitch = 55.0_f32.to_radians();
        let eye_depth = |point: [f32; 3]| -point[1] * pitch.cos() + point[2] * pitch.sin();
        let centers: Vec<_> = compact
            .iter()
            .map(|e| [0.0, e.points[1] + e.py as f32, e.points[2] + e.pz as f32])
            .collect();
        assert!(eye_depth(centers[1]) > eye_depth(centers[0]));
        assert!(compact[1].py > compact[0].py);
        for (a, mut b) in compact.into_iter().zip(overlap(true)) {
            a.validate().unwrap();
            b.validate().unwrap();
            assert_eq!(a.point_count(), 2);
            assert_ne!(b.labels, a.labels);
            assert!(b.label_list().iter().all(String::is_empty));
            b.labels.clear();
            b.flags &= !flag::LABELS;
            assert_eq!(gearbox_api::pack(&a).wire(), gearbox_api::pack(&b).wire());
        }
    }
}
