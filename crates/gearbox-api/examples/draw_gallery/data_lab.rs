use super::*;
use gearbox_api::wire::{draw::rgba, draw_data::*};

const ROOT: &str = "vfx-gallery/data-lab";

pub(super) fn manifest() -> serde_json::Value {
    serde_json::json!({"overview": {"focus": [0.0, -1.4, 2.0], "distance": 30,
    "pitch": 42, "bearing": 180,
    "bounds": {"min": [-10.0, -12.5, 0.4], "max": [10.0, 7.5, 5.8]}},
    "sections": [
        {"id":"cloud", "focus":[-6.5,3.0,1.9], "distance":8, "pitch":30, "bearing":150,
            "bounds":{"min":[-9.1,0.1,0.5],"max":[-3.9,5.8,3.8]}},
        {"id":"octree", "focus":[-3.2,-7.0,2.4], "distance":8, "pitch":30, "bearing":150,
            "bounds":{"min":[-5.2,-9.8,0.5],"max":[-1.2,-4.4,4.7]}}
    ]})
}

struct Sample {
    xyz: [f32; 3],
    color: u32,
    intensity: f32,
}

fn samples() -> Vec<Sample> {
    let mut out = Vec::new();
    for y in 0..44 {
        for x in 0..54 {
            let px = x as f32 / 53.0 * 4.4 - 2.2;
            let py = y as f32 / 43.0 * 3.4 - 1.7;
            out.push(Sample {
                xyz: [px, py, 0.8 + 0.12 * (px * 1.5).sin()],
                color: rgba(74, 142, 199, 255),
                intensity: (px + 2.2) / 4.4,
            });
        }
    }
    for z in 0..30 {
        for x in 0..54 {
            let px = x as f32 / 53.0 * 4.4 - 2.2;
            out.push(Sample {
                xyz: [px, 1.4 + 0.2 * px.cos(), 0.8 + z as f32 / 29.0 * 2.3],
                color: rgba(237, 158, 70, 255),
                intensity: 0.3 + 0.65 * z as f32 / 29.0,
            });
        }
    }
    for ring in 1..30 {
        let phi = std::f32::consts::PI * ring as f32 / 30.0;
        for azimuth in 0..64 {
            let theta = std::f32::consts::TAU * azimuth as f32 / 64.0;
            out.push(Sample {
                xyz: [
                    0.85 * phi.sin() * theta.cos(),
                    -0.25 + 0.85 * phi.sin() * theta.sin(),
                    1.75 + 0.85 * phi.cos(),
                ],
                color: rgba(94, 225, 175, 255),
                intensity: 0.1 + 0.9 * theta.cos().abs(),
            });
        }
    }
    out
}

fn label(id: &str, at: [f32; 3], text: &str, tint: u32, pixels: f32) -> E {
    Place::default()
        .no_drape()
        .put(E::text(ROOT, id, &[at], text))
        .with_style(style::LABEL)
        .with_width(-pixels)
        .with_color(color::WHITE)
        .with_prop("accent", &tint.to_string())
        .with_prop(
            "label_category",
            if pixels >= 16.0 { "SPATIAL DATA" } else { "" },
        )
        .with_glow(0.18)
}

fn frame(id: &str, at: [f32; 3], size: [f32; 3], tint: u32) -> E {
    Place::default()
        .no_drape()
        .put(E::shapes(ROOT, id, shape::BOX, &[Instance::at(at, size)]))
        .with_style(style::HOLO)
        .with_fill(flag::FILL_WIRE)
        .with_width(-1.0)
        .with_color(tint)
        .with_glow(0.2)
        .with_blur(0.25)
        .with_beam(-1.0)
}

fn energy_trace(id: &str, points: &[[f32; 3]], tint: u32) -> E {
    Place::default()
        .no_drape()
        .put(E::lines(ROOT, id, points))
        .with_style(style::RIBBON_FLOW)
        .with_color(tint)
        .with_width(0.035)
        .with_flow(0.65, 0.9)
        .with_pulse(0.35)
        .with_glow(0.45)
        .with_blur(0.3)
}

fn cloud_accents(index: usize, x: f32, tint: u32) -> Vec<E> {
    let p = Place::default().no_drape();
    let rim = [
        [x - 2.4, 1.05, 0.63],
        [x + 2.4, 1.05, 0.63],
        [x + 2.4, 4.95, 0.63],
        [x - 2.4, 4.95, 0.63],
        [x - 2.4, 1.05, 0.63],
    ];
    let mut out = vec![
        p.put(E::lines(ROOT, &format!("cloud-{index}-glass-rim"), &rim))
            .with_style(style::LIGHT_WALL)
            .with_color(tint)
            .with_width(0.02)
            .with_beam(0.18)
            .with_glow(0.32)
            .with_blur(0.55)
            .with_flow(0.45, 0.7)
            .with_pulse(0.25),
    ];
    let scan: Vec<_> = (0..54)
        .map(|i| {
            let px = i as f32 / 53.0 * 4.4 - 2.2;
            [x + px, 4.39 + 0.2 * px.cos(), 0.8 + 14.0 / 29.0 * 2.3]
        })
        .collect();
    let core = rgba(
        (150.0 + (tint & 255) as f32 * 0.4) as u8,
        (150.0 + ((tint >> 8) & 255) as f32 * 0.4) as u8,
        (150.0 + ((tint >> 16) & 255) as f32 * 0.4) as u8,
        240,
    );
    out.push(energy_trace(
        &format!("cloud-{index}-scan-row"),
        &scan,
        core,
    ));
    for (axis, points) in [
        [[x - 2.4, 1.05, 0.65], [x + 2.4, 1.05, 0.65]],
        [[x - 2.4, 1.05, 0.65], [x - 2.4, 4.95, 0.65]],
        [[x + 2.4, 4.95, 0.65], [x + 2.4, 4.95, 3.3]],
    ]
    .into_iter()
    .enumerate()
    {
        out.push(
            p.put(E::lines(
                ROOT,
                &format!("cloud-{index}-metric-{axis}"),
                &points,
            ))
            .with_style(style::RULER)
            .with_color(tint)
            .with_width(-1.1)
            .with_spacing(0.5)
            .with_glow(0.22),
        );
    }
    for plane in 0..3 {
        let mut arcs = Vec::new();
        for segment in 0..2 {
            let arc: Vec<_> = (0..41)
                .map(|i| {
                    let t = std::f32::consts::PI * (segment as f32 + i as f32 / 48.0);
                    let (a, b) = (t.cos() * 1.02, t.sin() * 1.02);
                    let offset = match plane {
                        0 => [a, b, 0.0],
                        1 => [a, 0.0, b],
                        _ => [0.0, a, b],
                    };
                    [x + offset[0], 2.75 + offset[1], 1.75 + offset[2]]
                })
                .collect();
            arcs.push(arc);
        }
        out.push(
            p.put(E::strips(
                ROOT,
                &format!("cloud-{index}-selection-{plane}"),
                &arcs,
            ))
            .with_style(style::RIBBON_FLOW)
            .with_color(tint)
            .with_width(0.022)
            .with_flow(0.4, 0.8)
            .with_glow(0.28)
            .with_blur(0.22)
            .with_pulse(0.3),
        );
    }
    out
}

fn occupancy() -> (OctreeConfig, Vec<OccupancyLeaf>) {
    let config = OctreeConfig {
        leaf_resolution_m: 0.2,
        max_depth: 4,
        free_max: 0.3,
        occupied_min: 0.7,
    };
    let mut work = vec![VoxelKey {
        tile: [0, 0, 0],
        depth: 0,
        morton: 0,
    }];
    let mut leaves = Vec::new();
    while let Some(key) = work.pop() {
        let split = key.depth == 0
            || (key.depth == 1 && ![0, 3, 7].contains(&key.morton))
            || (key.depth == 2 && key.morton % 8 < 2);
        if split {
            work.extend((0..8).map(|child| key.child(child, &config).unwrap()));
        } else {
            let probability = match key.morton % 4 {
                0 => None,
                1 => Some(0.12),
                2 => Some(0.52),
                _ => Some(0.91),
            };
            leaves.push(OccupancyLeaf {
                key,
                value: OccupancyValue {
                    probability,
                    confidence: None,
                    class_id: None,
                },
            });
        }
    }
    validate_leaf_set(&config, &leaves).unwrap();
    (config, leaves)
}

pub(super) fn scene() -> Vec<E> {
    let p = Place::default().no_drape();
    let data = samples();
    let mut out = vec![label(
        "title",
        [-2.0, 7.0, 5.0],
        "PERCEPTION / SPATIAL DATA LAB",
        color::CYAN,
        22.0,
    )];
    for (index, (name, tint)) in [
        ("RGB / SURFACE CLASSES", color::GREEN),
        ("INTENSITY / VIRIDIS", color::MAGENTA),
        ("HEIGHT / TURBO", color::ORANGE),
    ]
    .into_iter()
    .enumerate()
    {
        let x = (index as f32 - 1.0) * 6.5;
        let xyz: Vec<_> = data
            .iter()
            .map(|s| [s.xyz[0] + x, s.xyz[1] + 3.0, s.xyz[2]])
            .collect();
        let mut cloud = p
            .put(E::points(ROOT, &format!("cloud-{index}"), &xyz))
            .with_style(style::SPLAT)
            .with_sub(point_style::DISC)
            .with_width(-3.4)
            .with_glow(0.18)
            .with_blur(0.18)
            .with_pulse(-1.0);
        cloud = match index {
            0 => cloud.with_colors(&data.iter().map(|s| s.color).collect::<Vec<_>>()),
            1 => cloud
                .with_values(&data.iter().map(|s| s.intensity).collect::<Vec<_>>())
                .with_colormap(colormap::VIRIDIS, 0.0, 1.0),
            _ => cloud
                .with_values(&data.iter().map(|s| s.xyz[2]).collect::<Vec<_>>())
                .with_colormap(colormap::TURBO, 0.6, 3.2),
        };
        out.push(cloud);
        out.push(frame(
            &format!("cloud-{index}-frame"),
            [x, 3.0, 1.94],
            [4.75, 3.85, 2.65],
            tint,
        ));
        out.extend(cloud_accents(index, x, tint));
        out.push(label(
            &format!("cloud-{index}-label"),
            [x, 5.45, 3.4],
            name,
            tint,
            16.0,
        ));
        out.push(label(
            &format!("cloud-{index}-count"),
            [x, 0.6, 0.8],
            &format!("{} SAMPLES / SAME GEOMETRY", data.len()),
            tint,
            12.0,
        ));
    }
    let (config, leaves) = occupancy();
    for (index, x) in [-3.2f32, 3.2].into_iter().enumerate() {
        let mut instances = Vec::new();
        let mut colors = Vec::new();
        for leaf in &leaves {
            let bounds = leaf.key.grid_bounds(&config).unwrap();
            let size = bounds.size as f32 * config.leaf_resolution_m as f32;
            let centre: [f32; 3] = std::array::from_fn(|axis| {
                bounds.min[axis] as f32 * config.leaf_resolution_m as f32 + size * 0.5
            });
            instances.push(Instance::at(
                [centre[0] + x - 1.6, centre[1] - 8.6, centre[2] + 0.8],
                [size; 3],
            ));
            colors.push(if index == 0 {
                match leaf.value.probability {
                    None => rgba(190, 208, 223, 75),
                    Some(p) if p <= config.free_max => rgba(73, 227, 164, 110),
                    Some(p) if p >= config.occupied_min => rgba(250, 105, 76, 155),
                    _ => rgba(245, 201, 74, 125),
                }
            } else {
                match leaf.key.depth {
                    1 => rgba(103, 171, 235, 100),
                    2 => rgba(193, 122, 243, 125),
                    _ => rgba(71, 230, 200, 145),
                }
            });
        }
        out.push(
            p.put(E::shapes(
                ROOT,
                &format!("octree-{index}"),
                shape::BOX,
                &instances,
            ))
            .with_colors(&colors)
            .with_style(style::HOLO)
            .with_fill(flag::FILL_BOTH)
            .with_width(-0.8)
            .with_glow(0.22)
            .with_blur(0.4)
            .with_beam(-1.0)
            .with_pulse(-1.0),
        );
        out.push(frame(
            &format!("octree-{index}-frame"),
            [x, -7.0, 2.4],
            [3.3; 3],
            color::CYAN,
        ));
        for (slice, z) in [1.6f32, 3.2].into_iter().enumerate() {
            let ring = [
                [x - 1.67, -8.67, z],
                [x + 1.67, -8.67, z],
                [x + 1.67, -5.33, z],
                [x - 1.67, -5.33, z],
                [x - 1.67, -8.67, z],
            ];
            out.push(energy_trace(
                &format!("octree-{index}-slice-{slice}"),
                &ring,
                if index == 0 {
                    color::ORANGE
                } else {
                    color::MAGENTA
                },
            ));
        }
        let rails = [
            [x - 1.67, -8.67, 0.8],
            [x - 1.67, -8.67, 4.0],
            [x + 1.67, -5.33, 0.8],
            [x + 1.67, -5.33, 4.0],
        ];
        out.push(
            p.put(E::lines(
                ROOT,
                &format!("octree-{index}-depth-ticks"),
                &rails,
            ))
            .with_sub(line_mode::LIST)
            .with_style(style::RULER)
            .with_color(color::CYAN)
            .with_width(-1.2)
            .with_spacing(0.4)
            .with_glow(0.25),
        );
        out.push(label(
            &format!("octree-{index}-label"),
            [x, -4.85, 4.35],
            if index == 0 {
                "OCCUPANCY STATES"
            } else {
                "OCTREE DEPTH"
            },
            color::CYAN,
            16.0,
        ));
        out.push(label(
            &format!("octree-{index}-legend"),
            [x, -9.3, 0.85],
            if index == 0 {
                "UNKNOWN / FREE / UNCERTAIN / OCCUPIED"
            } else {
                "1.6 m / 0.8 m / 0.4 m CELLS"
            },
            color::CYAN,
            12.0,
        ));
    }
    out.push(label(
        "scope",
        [-2.0, -11.0, 0.8],
        "SYNTHETIC DATA / EXISTING RENDERER / BACKEND IN PROGRESS",
        color::WHITE,
        12.0,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_preview_preserves_identical_cloud_geometry_and_real_attribute_values() {
        let data = samples();
        let scene = scene();
        let clouds: Vec<_> = scene
            .iter()
            .filter(|e| ["cloud-0", "cloud-1", "cloud-2"].contains(&e.id().as_str()))
            .collect();
        assert_eq!(clouds.len(), 3);
        assert_eq!(clouds[0].colors.len(), data.len());
        assert_eq!(clouds[1].values.len(), data.len());
        assert_eq!(clouds[2].values.len(), data.len());
        for (index, cloud) in clouds.into_iter().enumerate() {
            for (point, sample) in cloud.points.chunks_exact(3).zip(&data) {
                assert!((point[0] - (index as f32 - 1.0) * 6.5 - sample.xyz[0]).abs() < 1e-5);
                assert_eq!(point[2], sample.xyz[2]);
            }
        }
        for entity in scene {
            entity.validate().unwrap();
        }
    }

    #[test]
    fn spatial_preview_occupancy_is_a_nonoverlapping_tree_not_uniform_box_noise() {
        let (config, leaves) = occupancy();
        validate_leaf_set(&config, &leaves).unwrap();
        assert_eq!(leaves.len(), 113);
        for depth in 1..=3 {
            assert!(leaves.iter().any(|leaf| leaf.key.depth == depth));
        }
        assert!(leaves.iter().any(|leaf| leaf.value.probability.is_none()));
        assert!(
            leaves
                .iter()
                .any(|leaf| leaf.value.probability.is_some_and(|p| p <= config.free_max))
        );
        assert!(leaves.iter().any(|leaf| {
            leaf.value
                .probability
                .is_some_and(|p| p >= config.occupied_min)
        }));
    }

    #[test]
    fn energized_detail_is_local_and_does_not_replace_data_colors() {
        let items = scene();
        assert_eq!(items.iter().filter(|e| e.style == style::RULER).count(), 11);
        assert_eq!(
            items
                .iter()
                .filter(|e| e.style == style::LIGHT_WALL)
                .count(),
            3
        );
        assert_eq!(
            items
                .iter()
                .filter(|e| e.style == style::RIBBON_FLOW)
                .count(),
            16
        );
        assert!(items.iter().all(|e| e.glow <= 0.45));
        for aspect in [(2468.0, 1348.0), (1234.0, 1348.0)] {
            for target in ["overview", "cloud", "octree"] {
                super::super::focus::camera(&manifest(), target, aspect.0, aspect.1, 58.0).unwrap();
            }
        }
    }
}
