//! Heading / inertia arrows — flat chevron disc under the selected
//! vehicle, flowing outward in the direction of horizontal motion.
//! Arrow extent + opacity scale with speed; a stationary vehicle
//! shows nothing. Mirrors the `selection_ring` plugin structure.

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
    pub time: f32,
    #[uniform(100)]
    pub dir_x: f32,
    #[uniform(100)]
    pub dir_z: f32,
    #[uniform(100)]
    pub speed: f32,
    #[uniform(100)]
    pub speed_fade: f32,
    #[uniform(100)]
    pub radius: f32,
    #[uniform(100)]
    pub inner_radius: f32,
    #[uniform(100)]
    pub center_x: f32,
    #[uniform(100)]
    pub center_z: f32,
    #[uniform(100)]
    pub curvature: f32,
    #[uniform(100)]
    pub _pad0: f32,
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
            time: 0.0,
            dir_x: 1.0,
            dir_z: 0.0,
            speed: 0.0,
            speed_fade: 0.0,
            radius: 5.0,
            inner_radius: 1.0,
            center_x: 0.0,
            center_z: 0.0,
            curvature: 0.0,
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

/// Tweakable knobs for the heading arrows.
#[derive(Resource, Copy, Clone, Debug)]
pub struct HeadingArrowsSettings {
    /// Width of the chevron band past the halo ring, in world units.
    /// The disc's outer radius is `inner_radius + band_width`, so a
    /// harvester with a 6 m silhouette gets exactly the same visible
    /// band depth as a 50 cm robot — small machines don't drown in
    /// chevrons and big machines still get a full-depth train.
    pub band_width: f32,
}

impl Default for HeadingArrowsSettings {
    fn default() -> Self {
        Self { band_width: 7.0 }
    }
}

#[derive(Component)]
pub struct HeadingArrows {
    pub material: Handle<HeadingArrowsMaterial>,
    pub mesh: Handle<Mesh>,
    pub built_for: f32,
    /// Currently displayed heading in radians (XZ plane, world frame).
    /// Exponentially eased toward the live velocity angle so a rapid
    /// steer produces a smooth rotation around the halo centre rather
    /// than a jump. `None` while the arrows are hidden — the next
    /// reveal snaps straight to the new velocity direction instead of
    /// lerping from a stale heading.
    pub displayed_angle: Option<f32>,
    /// Smoothed signed path curvature (1/m). Drives the parabolic
    /// bend applied to chevrons in the shader. Eases into zero so
    /// rolling off the steering produces a gentle un-bend rather
    /// than a snap to straight.
    pub displayed_curvature: f32,
    /// Smoothed 0..1 intensity driving chevron alpha / reach. Uses
    /// asymmetric rates: accelerations ramp this up quickly,
    /// decelerations drain it over many seconds so the chevron
    /// train lingers well after the vehicle has stopped pushing
    /// — reads as inertial trail rather than a binary on/off.
    pub displayed_speed_fade: f32,
}

/// Exponential smoothing rate (Hz) for the direction pointer.
/// Higher = snappier, lower = more sluggish. ~6 Hz reads as
/// "intentional rotation" without feeling laggy.
const ANGLE_SMOOTH_HZ: f32 = 6.0;
/// Exponential smoothing rate (Hz) for the curvature term.
/// Separate from the angle rate so we can keep rotation crisp
/// while the bend settles a bit more gently.
const CURVATURE_SMOOTH_HZ: f32 = 5.0;
/// Hard cap on |curvature| (1/m). Tightest represented turn radius
/// is `1/CURVATURE_MAX`; keeps slow-spinning vehicles from producing
/// absurd bends (since `k = yaw / speed` blows up at low speed).
const CURVATURE_MAX: f32 = 0.35;

/// Asymmetric fade rates for the intensity term. Rising is snappy so
/// chevrons appear as soon as the vehicle starts moving; falling is
/// deliberately sluggish so releasing throttle produces a long
/// inertial trail rather than an abrupt cut-off.
const SPEED_FADE_RISE_HZ: f32 = 6.0;
const SPEED_FADE_FALL_HZ: f32 = 0.25;
/// Below this displayed intensity the effect is invisible anyway —
/// hide the entity so it stops eating fragment shader cycles on
/// distant zero-alpha fragments.
const HIDE_EPSILON: f32 = 0.004;

/// Tiny lift above y = 0 so the disc doesn't z-fight the ground.
/// Kept distinct from the selection-ring offset so the two can
/// stack without fighting each other.
const ARROWS_GROUND_OFFSET: f32 = 0.03;
const DISC_RESOLUTION: u32 = 96;

/// Below this XZ speed the arrows hide entirely — prevents
/// noise-flicker while a vehicle is effectively parked.
const MIN_SHOW_SPEED: f32 = 0.15;

/// Mirror of the selection-ring sizing so chevrons sit exactly outside
/// the halo. Kept in sync with `selection_ring.rs` — if ring padding
/// changes there, update these too.
const RING_PADDING_GROUND: f32 = 0.65;
const RING_PADDING_DRONE: f32 = 0.3;
const MIN_RING_OUTER_GROUND: f32 = 1.3;
const MIN_RING_OUTER_DRONE: f32 = 0.9;
/// Extra breathing room past the halo's outer edge before chevrons begin.
const INNER_MARGIN: f32 = 0.25;

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
    Circle::new(radius).mesh().resolution(DISC_RESOLUTION).build()
}

pub fn setup_heading_arrows(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HeadingArrowsMaterial>>,
    settings: Res<HeadingArrowsSettings>,
) {
    // Start with the settings `band_width` as the disc radius; the
    // per-frame update system will grow the mesh once it knows the
    // selected vehicle's silhouette.
    let initial_radius = settings.band_width.max(1.0);
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
        extension: HeadingArrowsExtension {
            radius: initial_radius,
            ..default()
        },
    });

    commands.spawn((
        Name::new("HeadingArrows"),
        HeadingArrows {
            material: mat_handle.clone(),
            mesh: mesh_handle.clone(),
            built_for: initial_radius,
            displayed_angle: None,
            displayed_curvature: 0.0,
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
    let Ok((mut arrows, mut tr, mut vis)) = q.single_mut() else { return };

    let t = time.elapsed_secs();

    // Hard-reset paths: hide immediately and clear every smoothed
    // field. Used when the underlying vehicle goes away, the sim is
    // paused, or there's nothing selected — no reason to keep the
    // trail coasting when the context has changed.
    let hard_reset = |arrows: &mut HeadingArrows, vis: &mut Visibility| {
        *vis = Visibility::Hidden;
        arrows.displayed_angle = None;
        arrows.displayed_curvature = 0.0;
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

    // Only update the direction and curvature targets while the
    // vehicle is actually moving fast enough for them to be
    // meaningful. When it slows below the threshold we freeze both
    // at their last values so the fading trail keeps flowing in the
    // direction the vehicle came from instead of snapping or
    // dividing-by-zero.
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

        let yaw_rate = sim.0.vehicle_angvel(id).vy as f32;
        let target_curvature = (-yaw_rate / speed_xz).clamp(-CURVATURE_MAX, CURVATURE_MAX);
        let curv_alpha = 1.0 - (-dt * CURVATURE_SMOOTH_HZ).exp();
        arrows.displayed_curvature += (target_curvature - arrows.displayed_curvature) * curv_alpha;
    }

    // Drive the intensity fade toward the target, asymmetrically:
    // snap up when accelerating, coast down slowly when decelerating.
    // A stationary vehicle targets 0 so the train fades out over
    // several seconds instead of popping off.
    // Normalise against *this vehicle's* top speed — a husky at 1 m/s
    // is flat-out and should read as "full intensity", the same way
    // a drone at 15 m/s does. The spec carries the tuned value; fall
    // back to a safe 1 m/s if it's zero to avoid a divide-by-nothing.
    let vehicle_max_speed = (state.spec.max_speed as f32).max(0.5);
    let target_speed_fade = if moving {
        (speed_xz / vehicle_max_speed).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let rising = target_speed_fade > arrows.displayed_speed_fade;
    let rate = if rising { SPEED_FADE_RISE_HZ } else { SPEED_FADE_FALL_HZ };
    let fade_alpha = 1.0 - (-dt * rate).exp();
    arrows.displayed_speed_fade += (target_speed_fade - arrows.displayed_speed_fade) * fade_alpha;

    // Once the trail has effectively faded, hide and drop the
    // direction latch — next time the vehicle starts, we want a
    // fresh snap to the new velocity direction.
    if arrows.displayed_speed_fade < HIDE_EPSILON {
        hard_reset(&mut arrows, &mut vis);
        return;
    }

    let Some(angle) = arrows.displayed_angle else {
        // Defensive: if we somehow lost the angle while still fading,
        // hide this frame and let the next rising edge re-initialise.
        hard_reset(&mut arrows, &mut vis);
        return;
    };
    let dir_x = angle.cos();
    let dir_z = angle.sin();
    let curvature = arrows.displayed_curvature;

    // Chevrons start just past the selection-ring halo. The outer
    // radius is `inner_radius + band_width` so the depth of the
    // visible chevron train is the same for small and large
    // machines — the band grows outward in lockstep with the halo.
    let inner_radius = selection_ring_outer(state) + INNER_MARGIN;
    let radius = inner_radius + settings.band_width.max(1.0);
    if (arrows.built_for - radius).abs() > 1e-3 {
        if let Some(m) = meshes.get_mut(&arrows.mesh) {
            *m = make_disc_mesh(radius);
        }
        arrows.built_for = radius;
    }

    // Drones get the disc at their altitude so it reads as "under
    // the machine" rather than "far below on the ground".
    let arrows_y = match state.spec.drive_mode {
        DriveMode::Drone => pose.point.y as f32,
        _ => ARROWS_GROUND_OFFSET,
    };

    *vis = Visibility::Visible;
    tr.translation = Vec3::new(pose.point.x as f32, arrows_y, pose.point.z as f32);
    tr.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    tr.scale = Vec3::ONE;

    // Use the *smoothed* intensity, not the raw speed — this is what
    // gives the long coast-down. Scroll speed still uses the
    // instantaneous live speed so chevron motion stays in sync with
    // physical motion; they just thin out slowly once the throttle
    // is released.
    let speed_fade = arrows.displayed_speed_fade.clamp(0.0, 1.0);

    if let Some(mat) = materials.get_mut(&arrows.material) {
        let [r, g, b, _] = egui_to_linear(accent.0);
        mat.extension.color_r = r;
        mat.extension.color_g = g;
        mat.extension.color_b = b;
        mat.extension.time = t;
        mat.extension.dir_x = dir_x;
        mat.extension.dir_z = dir_z;
        mat.extension.speed = speed_xz.max(0.5);
        mat.extension.speed_fade = speed_fade;
        mat.extension.radius = radius;
        mat.extension.inner_radius = inner_radius;
        mat.extension.center_x = tr.translation.x;
        mat.extension.center_z = tr.translation.z;
        mat.extension.curvature = curvature;
    }
}

fn egui_to_linear(c: bevy_egui::egui::Color32) -> [f32; 4] {
    let to_f = |v: u8| (v as f32) / 255.0;
    [to_f(c.r()), to_f(c.g()), to_f(c.b()), to_f(c.a())]
}
