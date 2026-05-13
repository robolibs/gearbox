//! Heading indicator — a single `>` chevron sitting just past the
//! selection-ring halo, pointing along the vehicle's current
//! horizontal velocity. Alpha is the only thing that animates:
//! fades in quickly on acceleration and lingers for seconds on
//! deceleration via an asymmetric smoothing curve.

use bevy::light::NotShadowCaster;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use gearbox_physics::DriveMode;
use gearbox_viz::{GearboxSim, SimClock};

use super::selection::Selection;
use super::style::AccentColor;

pub type HeadingArrowsMaterial = ExtendedMaterial<StandardMaterial, HeadingArrowsExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct HeadingArrowsExtension {
    #[uniform(100)]
    pub color_r: f32,
    #[uniform(100)]
    pub color_g: f32,
    #[uniform(100)]
    pub color_b: f32,
    #[uniform(100)]
    pub speed_fade: f32,
    #[uniform(100)]
    pub dir_x: f32,
    #[uniform(100)]
    pub dir_z: f32,
    #[uniform(100)]
    pub apex_u: f32,
    #[uniform(100)]
    pub _pad0: f32,
    #[uniform(100)]
    pub center_x: f32,
    #[uniform(100)]
    pub center_z: f32,
    #[uniform(100)]
    pub _pad1: f32,
    #[uniform(100)]
    pub _pad2: f32,
}

impl Default for HeadingArrowsExtension {
    fn default() -> Self {
        Self {
            color_r: 0.5,
            color_g: 0.5,
            color_b: 0.5,
            speed_fade: 0.0,
            dir_x: 1.0,
            dir_z: 0.0,
            apex_u: 2.0,
            center_x: 0.0,
            center_z: 0.0,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        }
    }
}

impl MaterialExtension for HeadingArrowsExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_editor/shaders/heading_arrows.wgsl".into()
    }
}

pub struct HeadingArrowsPlugin;

impl Plugin for HeadingArrowsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/heading_arrows.wgsl");
        app.add_plugins(MaterialPlugin::<HeadingArrowsMaterial>::default())
            .init_resource::<HeadingArrowsSettings>();
    }
}

/// Tweakable knobs for the heading indicator.
#[derive(Resource, Copy, Clone, Debug)]
pub struct HeadingArrowsSettings {
    /// Gap in metres between the halo's outer edge and the chevron
    /// apex. Tuned so the `>` reads as "belonging to" the vehicle
    /// without overlapping the halo.
    pub apex_gap: f32,
}

impl Default for HeadingArrowsSettings {
    fn default() -> Self {
        // 1.3 m gap from the halo to the chevron apex — the arms
        // trail back ~0.55 m, so the trailing tip still sits ~0.75 m
        // clear of the selection ring.
        Self { apex_gap: 1.3 }
    }
}

#[derive(Component)]
pub struct HeadingArrows {
    pub material: Handle<HeadingArrowsMaterial>,
    pub mesh: Handle<Mesh>,
    pub built_for: f32,
    /// Smoothed heading in radians (XZ plane, world frame). `None`
    /// while hidden — the next reveal snaps to the fresh velocity
    /// direction rather than lerping from a stale angle.
    pub displayed_angle: Option<f32>,
    /// Smoothed 0..1 intensity driving chevron alpha. Asymmetric
    /// rise / fall rates give an inertial-trail feel: rising is
    /// snappy, falling is sluggish.
    pub displayed_speed_fade: f32,
}

/// Exponential smoothing rate (Hz) for the direction pointer.
const ANGLE_SMOOTH_HZ: f32 = 6.0;

/// Asymmetric fade rates. Rising is snappy, falling is deliberately
/// sluggish so releasing throttle leaves a long inertial trail.
const SPEED_FADE_RISE_HZ: f32 = 6.0;
const SPEED_FADE_FALL_HZ: f32 = 0.25;
/// Below this displayed intensity the chevron is effectively
/// invisible — hide the entity.
const HIDE_EPSILON: f32 = 0.004;

/// Tiny lift above y = 0 so the disc doesn't z-fight the ground.
const ARROWS_GROUND_OFFSET: f32 = 0.03;
const DISC_RESOLUTION: u32 = 48;

/// Below this XZ speed the direction latch is frozen — prevents
/// atan2 noise / snap-jumps while the vehicle is effectively parked.
const MIN_SHOW_SPEED: f32 = 0.15;

/// Mirror of the selection-ring sizing so the chevron sits flush
/// outside the halo. Kept in sync with `selection_ring.rs`.
const RING_PADDING_GROUND: f32 = 0.65;
const RING_PADDING_DRONE: f32 = 0.3;
const MIN_RING_OUTER_GROUND: f32 = 1.3;
const MIN_RING_OUTER_DRONE: f32 = 0.9;

fn selection_ring_outer(state: &gearbox_physics::VehicleState) -> f32 {
    let mut max_reach = {
        let hx = state.spec.chassis.size.x * 0.5;
        let hz = state.spec.chassis.size.z * 0.5;
        hx.max(hz) as f32
    };
    for p in &state.spec.parts {
        let hx = p.size.x * 0.5;
        let hz = p.size.z * 0.5;
        let reach_x = (p.position.x.abs() + hx) as f32;
        let reach_z = (p.position.z.abs() + hz) as f32;
        max_reach = max_reach.max(reach_x).max(reach_z);
    }
    let (padding, min_outer) = match state.spec.drive_mode {
        DriveMode::Drone => (RING_PADDING_DRONE, MIN_RING_OUTER_DRONE),
        _ => (RING_PADDING_GROUND, MIN_RING_OUTER_GROUND),
    };
    (max_reach + padding).max(min_outer)
}

fn make_disc_mesh(radius: f32) -> Mesh {
    Circle::new(radius)
        .mesh()
        .resolution(DISC_RESOLUTION)
        .build()
}

pub fn setup_heading_arrows(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HeadingArrowsMaterial>>,
) {
    let initial_radius = 2.0_f32;
    let mesh_handle = meshes.add(make_disc_mesh(initial_radius));
    let mat_handle = materials.add(HeadingArrowsMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: HeadingArrowsExtension::default(),
    });

    commands.spawn((
        Name::new("HeadingArrow"),
        HeadingArrows {
            material: mat_handle.clone(),
            mesh: mesh_handle.clone(),
            built_for: initial_radius,
            displayed_angle: None,
            displayed_speed_fade: 0.0,
        },
        Transform {
            translation: Vec3::new(0.0, ARROWS_GROUND_OFFSET, 0.0),
            rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        },
        Mesh3d(mesh_handle),
        MeshMaterial3d(mat_handle),
        Visibility::Hidden,
        NotShadowCaster,
    ));
}

pub fn update_heading_arrows(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    clock: Res<SimClock>,
    accent: Res<AccentColor>,
    settings: Res<HeadingArrowsSettings>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HeadingArrowsMaterial>>,
    mut q: Query<(&mut HeadingArrows, &mut Transform, &mut Visibility)>,
) {
    let Ok((mut arrows, mut tr, mut vis)) = q.single_mut() else {
        return;
    };

    let hard_reset = |arrows: &mut HeadingArrows, vis: &mut Visibility| {
        *vis = Visibility::Hidden;
        arrows.displayed_angle = None;
        arrows.displayed_speed_fade = 0.0;
    };

    if clock.paused {
        hard_reset(&mut arrows, &mut vis);
        return;
    }

    let Some(id) = selection.vehicle else {
        hard_reset(&mut arrows, &mut vis);
        return;
    };
    let Some(state) = sim.0.vehicle(id) else {
        hard_reset(&mut arrows, &mut vis);
        return;
    };

    let pose = sim.0.vehicle_pose(id);
    let linvel = sim.0.vehicle_linvel(id);
    let vx = linvel.vx as f32;
    let vz = linvel.vz as f32;
    let speed_xz = (vx * vx + vz * vz).sqrt();

    let dt = time.delta_secs().max(0.0);
    let moving = speed_xz >= MIN_SHOW_SPEED;

    // Latch the heading only while the vehicle is actually moving —
    // below the threshold we freeze the last direction so the
    // coasting trail points the way the vehicle was going.
    if moving {
        let target_angle = vz.atan2(vx);
        let angle_alpha = 1.0 - (-dt * ANGLE_SMOOTH_HZ).exp();
        let angle = match arrows.displayed_angle {
            Some(prev) => {
                let two_pi = std::f32::consts::TAU;
                let diff = ((target_angle - prev) + std::f32::consts::PI).rem_euclid(two_pi)
                    - std::f32::consts::PI;
                prev + diff * angle_alpha
            }
            None => target_angle,
        };
        arrows.displayed_angle = Some(angle);
    }

    // Normalise against this vehicle's top speed so a slow husky
    // reads as "flat-out" at 1 m/s the same way a drone does at 15 m/s.
    let vehicle_max_speed = (state.spec.max_speed as f32).max(0.5);
    let target_speed_fade = if moving {
        (speed_xz / vehicle_max_speed).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let rising = target_speed_fade > arrows.displayed_speed_fade;
    let rate = if rising {
        SPEED_FADE_RISE_HZ
    } else {
        SPEED_FADE_FALL_HZ
    };
    let fade_alpha = 1.0 - (-dt * rate).exp();
    arrows.displayed_speed_fade += (target_speed_fade - arrows.displayed_speed_fade) * fade_alpha;

    if arrows.displayed_speed_fade < HIDE_EPSILON {
        hard_reset(&mut arrows, &mut vis);
        return;
    }

    let Some(angle) = arrows.displayed_angle else {
        hard_reset(&mut arrows, &mut vis);
        return;
    };
    let dir_x = angle.cos();
    let dir_z = angle.sin();

    // Disc just needs to be a hair bigger than the chevron apex plus
    // its arm reach — single `>`, no margin for scroll.
    let apex_u = selection_ring_outer(state) + settings.apex_gap.max(0.0);
    let radius = apex_u + 1.5;
    if (arrows.built_for - radius).abs() > 1e-3 {
        if let Some(m) = meshes.get_mut(&arrows.mesh) {
            *m = make_disc_mesh(radius);
        }
        arrows.built_for = radius;
    }

    let arrows_y = match state.spec.drive_mode {
        DriveMode::Drone => pose.point.y as f32,
        _ => ARROWS_GROUND_OFFSET,
    };

    *vis = Visibility::Visible;
    tr.translation = Vec3::new(pose.point.x as f32, arrows_y, pose.point.z as f32);
    tr.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    tr.scale = Vec3::ONE;

    if let Some(mat) = materials.get_mut(&arrows.material) {
        let [r, g, b, _] = egui_to_linear(accent.0);
        mat.extension.color_r = r;
        mat.extension.color_g = g;
        mat.extension.color_b = b;
        mat.extension.speed_fade = arrows.displayed_speed_fade.clamp(0.0, 1.0);
        mat.extension.dir_x = dir_x;
        mat.extension.dir_z = dir_z;
        mat.extension.apex_u = apex_u;
        mat.extension.center_x = tr.translation.x;
        mat.extension.center_z = tr.translation.z;
    }
}

fn egui_to_linear(c: bevy_egui::egui::Color32) -> [f32; 4] {
    let to_f = |v: u8| (v as f32) / 255.0;
    [to_f(c.r()), to_f(c.g()), to_f(c.b()), to_f(c.a())]
}
