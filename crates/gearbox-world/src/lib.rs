//! The gearbox **world layer**.
//!
//! What lives here is the "permanent stuff" any gearbox app wants
//! around the content it loads:
//!
//! - LOD ground grid (`bevy_glacial::GroundGrid`) — the visual floor.
//! - RGB axis triad (`bevy_glacial::AxisGizmo`) — world-origin reference.
//! - Sun + ambient light.
//! - Sky clear colour.
//! - Orbit / pan / zoom camera (`bevy_glacial::ChaseCamera`).
//! - A static ground collider in `usd_bevy::physics::PhysicsWorld`
//!   so dynamic USD bodies actually rest on the floor.
//!
//! Everything visual comes from `bevy_glacial`; this crate just
//! configures, wires, and adds the physics-side ground collider.
//!
//! Mental model:
//!
//! ```text
//! ┌──────────────────────────────────────────────────────┐
//! │  gearbox world                                        │
//! │    ├── camera (ChaseCamera)                           │
//! │    ├── sky, sun, ambient                              │
//! │    ├── LOD grid + axis triad                          │
//! │    └── /loaded/   ← USD assets mounted here           │
//! │           ├── franka.usd                              │
//! │           └── tractor.usd                             │
//! └──────────────────────────────────────────────────────┘
//! ```
//!
//! ```ignore
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     .add_plugins(usd_bevy::UsdPlugin)
//!     .add_plugins(usd_bevy::physics::RapierAdapterPlugin)
//!     .add_plugins(gearbox_world::WorldPlugin::default())
//!     .run();
//! ```

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy_glacial::{
    AxisGizmo, AxisGizmoPlugin, ChaseCamera, ChaseCameraPlugin, GroundGrid, GroundGridPlugin,
};
use rapier3d::prelude::ColliderBuilder;
use usd_bevy::physics::PhysicsWorld;

/// Configuration for [`WorldPlugin`].
#[derive(Resource, Clone, Debug)]
pub struct WorldConfig {
    /// Where the orbit camera looks initially.
    pub camera_focus: Vec3,
    pub camera_distance: f32,

    /// Sky background colour (`ClearColor`). Default is the
    /// dark-navy editor look used by usdview / bevy_frost demos —
    /// reads well with the cool-blue grid.
    pub sky_color: Color,
    /// Grid line colour (RGBA, alpha controls fade).
    pub grid_color: Color,

    /// Sun position (the light is aimed at the world origin) and
    /// illuminance in lux.
    pub sun_translation: Vec3,
    pub sun_illuminance: f32,
    pub sun_shadows: bool,

    /// `GlobalAmbientLight` brightness.
    pub ambient_brightness: f32,

    /// Half-extent of the static ground collider (metres). The
    /// visual is bevy_glacial's infinite-fade grid, so this just
    /// gates where dynamic bodies actually find a floor.
    pub ground_half_size: f32,
    pub ground_friction: f32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            camera_focus: Vec3::new(0.0, 0.5, 0.0),
            camera_distance: 4.0,
            sky_color: Color::srgb(0.06, 0.08, 0.12),
            grid_color: Color::srgba(0.30, 0.38, 0.50, 0.42),
            sun_translation: Vec3::new(5.0, 10.0, 5.0),
            sun_illuminance: 15_000.0,
            sun_shadows: true,
            ambient_brightness: 400.0,
            ground_half_size: 50.0,
            ground_friction: 1.0,
        }
    }
}

/// Wires `bevy_glacial`'s grid / axis-triad / chase-camera plugins
/// and spawns the lights + collider at startup.
#[derive(Default)]
pub struct WorldPlugin {
    pub config: WorldConfig,
}

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .insert_resource(ClearColor(self.config.sky_color))
            .insert_resource(GroundGrid {
                visible: true,
                color: self.config.grid_color,
            })
            .add_plugins(ChaseCameraPlugin)
            .add_plugins(GroundGridPlugin)
            .add_plugins(AxisGizmoPlugin)
            .add_systems(Startup, (spawn_visuals, spawn_physics_ground));
    }
}

fn spawn_visuals(mut commands: Commands, config: Res<WorldConfig>) {
    let chase = ChaseCamera {
        focus: config.camera_focus,
        distance: config.camera_distance,
        ..default()
    };
    // `GroundGridPlugin` queries `&ChaseCamera` to size grid LOD; the
    // axis triad rides on the same entity. `Camera::clear_color` is
    // not set here — `ClearColor` resource handles it globally.
    commands.spawn((
        Camera3d::default(),
        chase,
        AxisGizmo::default(),
        Transform::from_xyz(config.camera_distance, config.camera_distance * 0.5, 0.0)
            .looking_at(config.camera_focus, Vec3::Y),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: config.sun_illuminance,
            shadows_enabled: config.sun_shadows,
            ..default()
        },
        Transform::from_translation(config.sun_translation).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(bevy::light::GlobalAmbientLight {
        brightness: config.ambient_brightness,
        ..default()
    });
}

/// Static cuboid in `usd_bevy::physics::PhysicsWorld` matching where
/// the LOD grid renders the ground. Top surface flush with `y = 0`.
fn spawn_physics_ground(config: Res<WorldConfig>, mut world: ResMut<PhysicsWorld>) {
    let half = config.ground_half_size as f64;
    let ground = ColliderBuilder::cuboid(half, 0.5, half)
        .translation(DVec3::new(0.0, -0.5, 0.0))
        .friction(config.ground_friction as f64)
        .build();
    world.colliders.insert(ground);
}
