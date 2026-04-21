//! gearbox editor — Bevy + egui + big_space front-end.
//!
//! big_space gives floating-origin rendering so f32 precision stays usable
//! even when the planet mesh sits 6 371 km from the camera.

mod editor;
mod viz;

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

fn main() {
    App::new()
        // Sky-blue horizon fade so the DistanceFog blends into the clear colour.
        .insert_resource(ClearColor(Color::srgb(0.55, 0.70, 0.86)))
        .add_plugins(
            DefaultPlugins
                .build()
                .disable::<TransformPlugin>() // big_space supplies its own
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "gearbox editor".into(),
                        resolution: [1280u32, 800u32].into(),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(BigSpaceDefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(GearboxVizPlugin)
        .add_plugins(editor::EditorPlugin)
        .add_systems(Startup, setup_scene)
        .run();
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
    // High-resolution UV sphere — 512 sectors × 256 stacks gives ~130k
    // triangles and a visibly rounder horizon than `ico(79)` at any
    // zoom level. The extra triangles are cheap; they're uniform small
    // patches with no material cost.
    let planet_mesh = meshes.add(Sphere::new(radius).mesh().uv(512, 256));
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
            MeshMaterial3d(planet_mat),
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
    commands
        .spawn((
            Name::new("Sun"),
            BigSpatialBundle {
                transform: Transform::from_xyz(10.0, 20.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            },
            DirectionalLight {
                illuminance: 10_000.0,
                shadows_enabled: true,
                ..default()
            },
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
        id,
        &spec,
        root_id,
    );
    commands.entity(chassis).insert(PlayerControlled);
}
