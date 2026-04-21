//! gearbox editor — Bevy + egui + big_space front-end.
//!
//! big_space gives floating-origin rendering so f32 precision stays usable
//! even when the planet mesh sits 6 371 km from the camera.

mod editor;
mod viz;
mod window_settings;

use bevy::asset::RenderAssetUsages;
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use big_space::prelude::*;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    presets,
};

use viz::{
    ChaseCamera, GearboxSim, GearboxVizPlugin, PlayerControlled, spawn_height_for,
    spawn_vehicle_visuals,
};
use viz::grid::{rotation_from_latlon_to_top, spawn_circle_meshes, GroundGrid};

/// Handle to the BigSpace root so UI-initiated vehicle spawns can add
/// themselves into the same floating-origin hierarchy.
#[derive(Resource, Copy, Clone)]
pub struct BigSpaceRoot(pub Entity);

/// Tag for the fine-tessellated spherical cap that follows the
/// camera. Same material as the planet; curved to match the sphere
/// exactly. Provides enough local triangle density for vehicle
/// shadows to land cleanly on it (cheap to render, no visible seam
/// because it's co-planar with the planet surface everywhere).
#[derive(Component)]
pub struct ShadowPatch;

/// Build a spherical-cap mesh: a square grid `(n+1)²` vertices wide
/// projected onto the surface of a sphere of radius `r` centred at
/// `(0, -r, 0)`. The mesh's local origin sits on the sphere's tangent
/// point at the "top" (y = 0); vertices curve downward by the exact
/// amount the sphere does, so the patch is geometrically identical
/// to the underlying planet where they overlap.
fn spherical_cap_mesh(radius: f32, half_size: f32, n: u32) -> Mesh {
    let n = n.max(1) as i32;
    let step = (2.0 * half_size) / n as f32;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    let mut normals:   Vec<[f32; 3]> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    let mut uvs:       Vec<[f32; 2]> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    let mut indices:   Vec<u32>      = Vec::with_capacity((n * n * 6) as usize);

    for i in 0..=n {
        for j in 0..=n {
            let x = -half_size + i as f32 * step;
            let z = -half_size + j as f32 * step;
            // Sphere centred at (0, -r, 0). Surface above XZ plane:
            //   x² + (y + r)² + z² = r²   →   y = √(r² − x² − z²) − r.
            let dist2 = x * x + z * z;
            let y = (radius * radius - dist2).sqrt() - radius;
            positions.push([x, y, z]);
            // Outward sphere normal = (x, y + r, z) / r.
            normals.push([x / radius, (y + radius) / radius, z / radius]);
            uvs.push([
                (i as f32) / (n as f32),
                (j as f32) / (n as f32),
            ]);
        }
    }
    let row = (n + 1) as u32;
    for i in 0..n as u32 {
        for j in 0..n as u32 {
            let a = i * row + j;
            let b = a + 1;
            let c = a + row;
            let d = c + 1;
            // CCW winding viewed from above (+Y looking down), so
            // Bevy's default back-face culling doesn't hide the
            // patch. This is the bug that stopped shadows: if the
            // patch is culled, there's no surface to receive them.
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}


fn main() {
    // Restore the last-saved window geometry (size + position) so we
    // don't boot into a default tiny pane at the top-left every run.
    let window_geometry = window_settings::load_window_geometry();

    App::new()
        // Sky-blue horizon fade so the DistanceFog blends into the clear colour.
        .insert_resource(ClearColor(Color::srgb(0.55, 0.70, 0.86)))
        // 8 k shadow map (4× Bevy's 2048 default). Directional-light
        // CSM packs all cascades into a single texture, so more texels
        // = finer shadows per cascade — essential when the receiver
        // is a 6 371 km sphere tangent to the vehicles.
        .insert_resource(bevy::light::DirectionalLightShadowMap { size: 8192 })
        .add_plugins(
            DefaultPlugins
                .build()
                .disable::<TransformPlugin>() // big_space supplies its own
                .set(WindowPlugin {
                    primary_window: Some(window_settings::geometry_to_window(window_geometry)),
                    ..default()
                }),
        )
        .add_plugins(BigSpaceDefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(GearboxVizPlugin)
        .add_plugins(editor::EditorPlugin)
        // Persists the primary window's size + position to
        // ~/.config/gearbox/window.txt on every resize / move.
        .add_plugins(window_settings::WindowSettingsPlugin)
        .add_systems(Startup, setup_scene)
        .add_systems(Update, follow_camera_shadow_patch)
        .run();
}

/// Re-centre the `ShadowPatch` under the chase-camera's focus every
/// frame so vehicle shadows always land on it, no matter where in
/// the world the user drives.
fn follow_camera_shadow_patch(
    cameras: Query<&ChaseCamera>,
    mut patches: Query<&mut Transform, With<ShadowPatch>>,
) {
    let Ok(cam) = cameras.single() else { return };
    for mut tr in patches.iter_mut() {
        tr.translation.x = cam.focus.x;
        tr.translation.y = 0.0;
        tr.translation.z = cam.focus.z;
    }
}

fn setup_scene(
    mut commands: Commands,
    mut sim: ResMut<GearboxSim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<bevy::image::Image>>,
) {
    // Physics: flat ground plane. (A ball collider at Earth radius hits
    // f32-precision limits in rapier's distance checks — wheels go
    // jittery at ~0.5 m noise. The sphere lives purely as visuals.)
    sim.0.add_ground_plane(2_000.0);

    // BigSpace root — every renderable entity becomes a child of it so
    // big_space's transform-propagation handles precision.
    let root_id = commands
        .spawn((Name::new("BigSpace"), BigSpaceRootBundle::default()))
        .id();
    commands.insert_resource(BigSpaceRoot(root_id));

    let radius = sim.0.planet.radius as f32;
    let radius_f64 = sim.0.planet.radius;

    // --- Planet ground colour ---
    // (The sphere mesh itself is the ground — a 4 km flat tangent patch
    // was removed because it sat at y=0 and hid the grid circles, which
    // drop ~3 cm below y=0 just ~1 km out from the machine. With the
    // sphere alone the circles sit cleanly above the sphere's triangle
    // faces everywhere.)
    // Warm, light brown ground — sandy/tan rather than grass.
    let planet_green = Color::srgb(0.62, 0.48, 0.33);

    // --- Planet sphere ---
    //
    // Moderate UV resolution (1024 × 512 → ~500 k triangles) — this
    // covers the whole 6 371 km ball, renders instantly, and is
    // plenty for the distant horizon. Tangent-plane triangles are
    // ~40 km wide at this density, too coarse for precise shadow
    // reception near the vehicle; the `ShadowPatch` spawned below is
    // a small, camera-following spherical cap that matches the
    // sphere curvature exactly and provides the fine geometry the
    // shadow needs.
    let planet_mesh = meshes.add(Sphere::new(radius).mesh().uv(1024, 512));
    let planet_mat = materials.add(StandardMaterial {
        base_color: planet_green,
        perceptual_roughness: 0.95,
        ..default()
    });
    // Planet centre in world coords.
    let (planet_cell, planet_offset) =
        Grid::default().translation_to_grid(DVec3::new(0.0, -radius_f64, 0.0));

    // Rotation that aligns the sphere's mesh-local +Y ("Amsterdam") with
    // the geographic datum point. This puts the tractor (at world origin
    // = sphere top) on the planet at the datum's lat/lon, while leaving
    // the geographic north pole OFF to one side.
    let planet_rot = rotation_from_latlon_to_top(
        sim.0.planet.datum.latitude,
        sim.0.planet.datum.longitude,
    );
    commands
        .spawn((
            Name::new("Planet"),
            BigSpatialBundle {
                transform: Transform {
                    translation: planet_offset,
                    rotation: planet_rot,
                    scale: Vec3::ONE,
                },
                cell: planet_cell,
                ..default()
            },
            Mesh3d(planet_mesh),
            MeshMaterial3d(planet_mat.clone()),
            // Planet doesn't cast (sun is outside a 6 371 km ball).
            // It also doesn't *receive* shadows here — its triangles
            // are far too coarse for CSM to hit them precisely. The
            // `ShadowPatch` below catches shadows instead.
            NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ))
        .insert(ChildOf(root_id));

    // Camera-following, finely-tessellated spherical cap. Sits on
    // the planet surface, curved exactly to match the sphere, so it
    // is visually indistinguishable from the planet (same material,
    // zero gap at the edge). Tangent triangles are ~3 m wide → chord
    // sag is ~μm, far below the shadow-bias threshold. The `follow`
    // system re-positions it under the camera each frame.
    let shadow_patch_mesh = meshes.add(spherical_cap_mesh(radius, 300.0, 200));
    commands
        .spawn((
            Name::new("ShadowPatch"),
            ShadowPatch,
            BigSpatialBundle::default(),
            Mesh3d(shadow_patch_mesh),
            MeshMaterial3d(planet_mat),
            NotShadowCaster,
        ))
        .insert(ChildOf(root_id));

    // Two line-meshes that track the machine — one for its latitude
    // circle, one for its meridian. Mesh data is rebuilt every frame in
    // `viz::grid::update_circle_meshes`, but the entities are spawned
    // once here and parented to the BigSpace root.
    let grid_cfg = GroundGrid::default();
    spawn_circle_meshes(&mut commands, &mut meshes, &mut materials, root_id, &grid_cfg);
    let _ = (planet_rot, planet_cell, planet_offset);

    // Cloud shell — translucent sphere at ~planet_radius + 4 km.
    viz::clouds::spawn_cloud_shell(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        root_id,
        radius_f64,
    );

    // --- Sun ---
    //
    // Single cascade, 100 m max — all shadow-map texels land inside
    // the ~100 m vehicle-neighbourhood. Sun angle steepened so the
    // shadow has a clear direction near the horizon.
    let sun_shadow = CascadeShadowConfigBuilder {
        num_cascades: 1,
        minimum_distance: 0.1,
        maximum_distance: 100.0,
        first_cascade_far_bound: 100.0,
        overlap_proportion: 0.0,
    }
    .build();
    commands
        .spawn((
            Name::new("Sun"),
            BigSpatialBundle {
                transform: Transform::from_xyz(5.0, 50.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            },
            DirectionalLight {
                illuminance: 10_000.0,
                shadows_enabled: true,
                ..default()
            },
            sun_shadow,
        ))
        .insert(ChildOf(root_id));

    // --- Camera (FloatingOrigin) ---
    let projection = Projection::Perspective(PerspectiveProjection {
        near: 0.1,
        far: radius * 2.5,
        ..default()
    });
    // DistanceFog with atmospheric falloff: Rayleigh-ish blue scatters
    // into the distance, with the extinction/inscattering tuned for
    // kilometre-scale views. Blue channel extinguishes/inscatters more
    // than red/green so the horizon gently shifts toward sky-blue —
    // the same visual cue you see looking out over flat terrain IRL.
    let fog = DistanceFog {
        color: Color::srgb(0.55, 0.70, 0.86),
        falloff: FogFalloff::Atmospheric {
            extinction:   Vec3::new(0.00008, 0.00012, 0.00020),
            inscattering: Vec3::new(0.00010, 0.00015, 0.00025),
        },
        ..default()
    };

    commands
        .spawn((
            Name::new("Camera"),
            BigSpatialBundle {
                transform: Transform::from_xyz(0.0, 8.0, -15.0).looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            },
            Camera3d::default(),
            projection,
            FloatingOrigin,
            fog,
            AmbientLight {
                color: Color::WHITE,
                brightness: 120.0,
                ..default()
            },
            ChaseCamera {
                focus: Vec3::new(0.0, 0.5, 0.0),
                distance: 14.0,
                elevation: 25f32.to_radians(),
                max_distance: radius * 3.0,
                ..default()
            },
        ))
        .insert(ChildOf(root_id));

    // --- Starter tractor ---
    let spec = presets::tractor();
    let pose = Pose {
        point: Point::new(0.0, spawn_height_for(&spec), 0.0),
        rotation: Quaternion::identity(),
    };
    let id = sim.0.spawn_vehicle(spec.clone(), pose);
    let chassis = spawn_vehicle_visuals(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        id,
        &spec,
        root_id,
    );
    commands.entity(chassis).insert(PlayerControlled);
}
