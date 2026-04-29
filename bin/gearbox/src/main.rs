//! gearbox — the one-and-only binary.
//!
//! Hosts three layers in-process:
//!
//!   * **Simulator** — `gearbox_physics::Sim` wrapped by `gearbox_viz`'s
//!     `GearboxSim` resource. Rapier-f64 physics world, stepped at 60 Hz.
//!   * **Renderer** — Bevy + egui, also from `gearbox_viz` + `gearbox_editor`.
//!     Draws the simulator state and presents the editor UI.
//!   * **Tool API** — `gearbox_api`'s `GearboxApiPlugin`. Opens a zenoh
//!     session so external tools (robots, CLIs, scripting agents) can
//!     observe / command the simulator across the network.
//!
//! The **simulator ↔ renderer** split is in-process only (shared Bevy
//! resource). The **tool API** is the only *network* boundary this
//! binary exposes. A future headless mode — simulator + tool API, no
//! renderer — can be added as a `--headless` flag on this same binary
//! (one-binary policy).

use bevy::asset::RenderAssetUsages;
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy_egui::EguiPlugin;

use gearbox_core::presets;
use datapod::{Point, Pose, Quaternion};

use gearbox_api::GearboxApiPlugin;
use gearbox_editor::EditorPlugin;
use gearbox_viz::grid::{spawn_circle_meshes, GroundGrid};
use gearbox_viz::window_settings;
use gearbox_viz::{
    spawn_height_for, spawn_vehicle_visuals, ChaseCamera, GearboxSim,
    GearboxVizPlugin, PlayerControlled, UsdAssetRoot,
};

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
                .set(WindowPlugin {
                    primary_window: Some(window_settings::geometry_to_window(window_geometry)),
                    ..default()
                })
                // Asset path: anchor at the package root via the
                // compile-time `CARGO_MANIFEST_DIR`. cargo's launched-
                // binary cwd is unstable across `make run` / direct
                // `cargo run` / IDE launches; an absolute path is the
                // only setup that resolves the same regardless of
                // who invoked us. Resolves to
                // `<repo>/bin/gearbox/assets/`.
                .set(bevy::asset::AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").to_string(),
                    ..default()
                }),
        )
        // Tell `gearbox-viz` where the asset root lives so it can
        // forward sibling-reference search paths to bevy_openusd's
        // loader. Must be the same dir as `AssetPlugin.file_path`
        // above.
        .insert_resource(UsdAssetRoot(
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")),
        ))
        .add_plugins(EguiPlugin::default())
        // OpenUSD asset pipeline — registers `UsdAsset` + the
        // `.usda` / `.usdc` / `.usdz` loader so any code in the
        // app can `asset_server.load("…/foo.usda")` and get a
        // composed Bevy `Scene`.
        .add_plugins(bevy_openusd::UsdPlugin)
        .add_plugins(GearboxVizPlugin)
        .add_plugins(EditorPlugin)
        // Robot / sim API — opens a zenoh session and bridges
        // `GearboxSim` to the network. Plugin no-ops if zenoh fails
        // to bring up (e.g. restricted ports), so the editor still
        // runs offline. Add after the sim plugin so the resource is
        // already there for our publisher system.
        .add_plugins(GearboxApiPlugin)
        // Pluggable per-vehicle topics (cmd_vel / odom / fix). Drop
        // this line + `crates/gearbox-api/src/vehicle_api.rs` to
        // remove cleanly.
        .add_plugins(gearbox_api::VehicleApiPlugin)
        // Pluggable "go to point" — uses the `ondrive` crate to
        // drive vehicles to a target pose. Drop this line +
        // `crates/gearbox-api/src/goto_api.rs` + the `ondrive` dep
        // in `crates/gearbox-api/Cargo.toml` to remove.
        .add_plugins(gearbox_api::GotoApiPlugin)
        // Pluggable world markers (cones / boxes / spheres) over
        // `gearbox/markers/<id>`. Drop this line +
        // `crates/gearbox-api/src/markers_api.rs` to remove.
        .add_plugins(gearbox_api::MarkersApiPlugin)
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
    asset_server: Res<bevy::asset::AssetServer>,
) {
    // Physics: flat ground plane. (A ball collider at Earth radius hits
    // f32-precision limits in rapier's distance checks — wheels go
    // jittery at ~0.5 m noise. The sphere lives purely as visuals.)
    sim.0.add_ground_plane(2_000.0);

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
    // Rotation that aligns the sphere's mesh-local +Y ("Amsterdam") with
    // the geographic datum point.
    let planet_rot = rotation_from_latlon_to_top(
        sim.0.planet.datum.latitude,
        sim.0.planet.datum.longitude,
    );
    commands.spawn((
        Name::new("Planet"),
        Transform {
            translation: Vec3::new(0.0, -radius, 0.0),
            rotation: planet_rot,
            scale: Vec3::ONE,
        },
        Mesh3d(planet_mesh),
        MeshMaterial3d(planet_mat.clone()),
        NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));

    // Camera-following, finely-tessellated spherical cap.
    let shadow_patch_mesh = meshes.add(spherical_cap_mesh(radius, 300.0, 200));
    commands.spawn((
        Name::new("ShadowPatch"),
        ShadowPatch,
        Transform::default(),
        Mesh3d(shadow_patch_mesh),
        MeshMaterial3d(planet_mat),
        NotShadowCaster,
    ));

    // Two line-meshes that track the machine — one for its latitude
    // circle, one for its meridian. Mesh data is rebuilt every frame
    // in `viz::grid::update_circle_meshes`.
    // Subtler than `GroundGrid::default()` (which is tuned for the
    // bevy_glacial demo's plain ground): on top of the planet sphere
    // a high-alpha grid feels like ink stains. Drop the alpha so the
    // grid is a soft hint and the world reads first. Inserting the
    // resource overrides whatever `GlacialPlugin` initialised, and
    // the per-frame `build_grid_meshes` system reads from this
    // resource so the alpha sticks across the whole session.
    let grid_cfg = GroundGrid {
        color: Color::srgba(80.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0, 0.26),
        ..GroundGrid::default()
    };
    commands.insert_resource(grid_cfg);
    spawn_circle_meshes(&mut commands, &mut meshes, &mut materials, &grid_cfg);

    // Cloud shell — translucent sphere at ~planet_radius + 4 km.
    gearbox_viz::clouds::spawn_cloud_shell(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        radius_f64,
        gearbox_viz::clouds::DEFAULT_CLOUD_ALTITUDE_M,
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
    commands.spawn((
        Name::new("Sun"),
        Transform::from_xyz(5.0, 50.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: true,
            ..default()
        },
        sun_shadow,
    ));

    // --- Camera ---
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

    commands.spawn((
        Name::new("Camera"),
        Transform::from_xyz(0.0, 8.0, -15.0).looking_at(Vec3::ZERO, Vec3::Y),
        Camera3d::default(),
        projection,
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
        bevy_glacial::GizmoCamera,
    ));

    // --- Starter tractor ---
    let spec = presets::tractor_articulated();
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
        &asset_server,
        id,
        &spec,
    );
    commands.entity(chassis).insert(PlayerControlled);
}

/// Rotate a unit-Y up vector so the Earth's surface direction at
/// `(lat, lon)` ends up pointing along world +Y. Lets the planet
/// sphere mesh sit "right side up" relative to the configured
/// datum point. Lives here (not in the reusable scene crate) since
/// it's a planet/world-specific concept.
fn rotation_from_latlon_to_top(lat_deg: f64, lon_deg: f64) -> Quat {
    let lat = (lat_deg as f32).to_radians();
    let lon = (lon_deg as f32).to_radians();
    let dir = Vec3::new(
        lat.cos() * lon.cos(),
        lat.sin(),
        lat.cos() * lon.sin(),
    )
    .normalize();
    Quat::from_rotation_arc(dir, Vec3::Y)
}
