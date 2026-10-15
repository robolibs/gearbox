const GLOBE: &str = include_str!("../shaders/terrain.wgsl");
const LOCAL: &str = include_str!("../../gearbox-fields/src/bare/shaders/material.wgsl");
const WATER: &str = include_str!("../shaders/water.wgsl");

#[test]
fn barren_globes_resolve_local_height_cells_without_changing_earth() {
    use gearbox_planet::config::{leaf_size, PlanetConfig, GRID_QUADS};
    for (radius, kind) in [(3_389_500.0, 1), (1_737_400.0, 2)] {
        let config = PlanetConfig::barren(radius, kind);
        assert!(leaf_size(radius, config.max_depth) / GRID_QUADS as f32 <= 1.0);
        assert_eq!(config.lod0_range, leaf_size(radius, config.max_depth) * 8.0);
    }
    assert_eq!(PlanetConfig::earth(6_371_000.0).max_depth, 12);
    assert!(LOCAL.contains("land = planetary_normal(place, pixel_m)"));
}

#[test]
fn globe_surfaces_filter_heights_and_morph_in_world_space() {
    for shader in [GLOBE, WATER] {
        assert!(shader.contains("textureLoad(heightmaps, lo, i32(layer), 0)"));
        assert!(shader.contains("return mix(mix(a, b, f.x), mix(c, d, f.x), f.y)"));
        assert!(!shader.contains("textureSampleLevel(heightmaps"));
        assert!(!shader.contains("heightmap_sampler"));
        assert!(shader.contains("distance(view.world_position, p_world.xyz)"));
        assert!(!shader.contains("distance(view.world_position, p_f)"));
    }
    assert!(!GLOBE.contains("cross(p_x - p_c, p_y - p_c)"));
}

#[test]
fn planetary_join_uses_shared_geology_without_altitude_discard() {
    for shader in [GLOBE, LOCAL] {
        assert!(shader.contains("#import gearbox_globe::surface::planet_albedo"));
        assert!(!shader.contains("fn geology_noise("));
        assert!(!shader.contains("planet_local_fraction"));
        assert!(!shader.contains("planet_screen_noise"));
        assert!(shader.contains("pbr_input.material.reflectance = vec3<f32>(0.03)"));
    }
    assert!(LOCAL.contains("ground.world_to_planet * in.world_position"));
    assert!(LOCAL.contains("smoothstep(0.15, 0.9, pixel_m)"));
    assert!(GLOBE.contains("let covered = in.world_position.xyz - tg.cover_origin.xyz"));
    assert!(!GLOBE.contains("let covered = p_c -"));
}

#[test]
fn rendered_coverage_tracks_rotated_floating_origin_frames() {
    use bevy::prelude::*;
    use gearbox_planet::planet::material::TerrainCoverage;
    let centre = Vec2::new(-123.0, 456.0);
    for offset in [Vec3::ZERO, Vec3::new(9841.0, -1823.0, 7654.0)] {
        let frame = GlobalTransform::from(Transform {
            translation: offset,
            rotation: Quat::from_euler(EulerRot::XYZ, 0.4, -1.1, 0.7),
            ..default()
        });
        let coverage = TerrainCoverage::from_frame(&frame, centre, 600.0);
        for x in [-600.05_f32, -599.95, 0.0, 599.95, 600.05] {
            for z in [-600.05_f32, -599.95, 0.0, 599.95, 600.05] {
                for y in [-500.0, 0.0, 500.0] {
                    let world = frame.transform_point(Vec3::new(centre.x + x, y, centre.y + z));
                    let relative = world - coverage.origin.truncate();
                    let n = relative.dot(coverage.north.truncate());
                    let e = relative.dot(coverage.east.truncate());
                    assert!((n - x).abs() < 0.003 && (e - z).abs() < 0.003);
                    assert_eq!(n.abs().max(e.abs()) < coverage.origin.w, x.abs().max(z.abs()) < 600.0);
                }
            }
        }
    }
}
