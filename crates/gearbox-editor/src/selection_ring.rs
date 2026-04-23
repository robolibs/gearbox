//! Animated selection ring — ported from astrocraft's placement-
//! preview shader. Spins a fine striped pattern around the ring so
//! the "you have this vehicle selected" cue reads even at a glance.
//!
//! Height is vehicle-aware: ground vehicles get a ring just above
//! `y = 0`, drones (or anything flying) get a ring at the vehicle's
//! actual altitude so the marker follows the machine in 3-D.

use bevy::light::NotShadowCaster;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use gearbox_physics::DriveMode;

use gearbox_viz::{GearboxSim, SimClock};

use super::selection::Selection;
use super::style::AccentColor;

pub type SelectionRingMaterial = ExtendedMaterial<StandardMaterial, SelectionRingExtension>;

/// Uniform block for the spinning-ring shader. Fields are packed
/// manually (three `_pad` floats) so the WGSL struct matches the
/// WebGPU std140 layout.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SelectionRingExtension {
    #[uniform(100)]
    pub color_r: f32,
    #[uniform(100)]
    pub color_g: f32,
    #[uniform(100)]
    pub color_b: f32,
    #[uniform(100)]
    pub time: f32,
    #[uniform(100)]
    pub pulse_speed: f32,
    #[uniform(100)]
    pub pulse_count: f32,
    #[uniform(100)]
    pub alpha: f32,
    #[uniform(100)]
    pub center_x: f32,
    #[uniform(100)]
    pub center_z: f32,
    #[uniform(100)]
    pub fine_mult: f32,
    #[uniform(100)]
    pub _pad2: f32,
    #[uniform(100)]
    pub _pad3: f32,
}

impl Default for SelectionRingExtension {
    fn default() -> Self {
        Self {
            color_r: 0.5,
            color_g: 0.5,
            color_b: 0.5,
            time: 0.0,
            pulse_speed: 3.0,
            pulse_count: 8.0,
            alpha: 1.0,
            center_x: 0.0,
            center_z: 0.0,
            fine_mult: 4.0,
            _pad2: 0.0,
            _pad3: 0.0,
        }
    }
}

impl MaterialExtension for SelectionRingExtension {
    fn fragment_shader() -> ShaderRef {
        // Resolved from the `embedded://` asset source registered by
        // this crate's plugin (see `SelectionRingPlugin::build`). No
        // on-disk `assets/` directory is consulted at runtime — the
        // WGSL file is baked into the binary via `embedded_asset!`.
        "embedded://gearbox_editor/shaders/selection_ring.wgsl".into()
    }
}

/// Registers the `MaterialPlugin` + ring-settings resource. Call
/// from `EditorPlugin::build`.
pub struct SelectionRingPlugin;

impl Plugin for SelectionRingPlugin {
    fn build(&self, app: &mut App) {
        // `embedded_asset!` calls `include_bytes!` on the WGSL source
        // at compile time and publishes it under the
        // `embedded://gearbox_editor/shaders/selection_ring.wgsl`
        // AssetPath — so the shipped binary has no `assets/` runtime
        // dependency.
        bevy::asset::embedded_asset!(app, "shaders/selection_ring.wgsl");
        app.add_plugins(MaterialPlugin::<SelectionRingMaterial>::default())
            .init_resource::<SelectionRingSettings>();
    }
}

/// User-tweakable ring look. `thickness` is the **world-space** band
/// width — because the mesh is rebuilt per-vehicle (see
/// `update_selection_ring`) the thickness stays constant regardless
/// of how big the machine is, instead of scaling with the radius.
#[derive(Resource, Copy, Clone, Debug)]
pub struct SelectionRingSettings {
    pub thickness: f32,
}

impl Default for SelectionRingSettings {
    fn default() -> Self {
        // Tractor-thickness by default — reads well without being noisy.
        Self { thickness: 0.15 }
    }
}

#[derive(Component)]
pub struct SelectionRing {
    pub material: Handle<SelectionRingMaterial>,
    pub mesh: Handle<Mesh>,
    /// Last-built mesh dimensions (outer_radius, thickness) so we
    /// only rebuild when either changes.
    pub built_for: (f32, f32),
}

/// Height above the ground for ground vehicles. Drones override this
/// with their actual world-Y.
const RING_GROUND_OFFSET: f32 = 0.05;

/// Ring sizing uses a *fixed padding* rather than a multiplier so
/// very long machines (oxbo harvester with its forward head) don't
/// get a ring that's 10 m wider than their silhouette. The ring
/// ends up `max_reach + padding` metres from the chassis centre,
/// plus a per-mode floor so tiny vehicles still get a visible ring.
const RING_PADDING_GROUND: f32 = 0.65;
const RING_PADDING_DRONE: f32 = 0.3;
const MIN_OUTER_RADIUS_GROUND: f32 = 1.3;
const MIN_OUTER_RADIUS_DRONE: f32 = 0.9;
/// Reference diameter that produces the "drone" look: 8 coarse
/// segments. Bigger rings scale only the COARSE count linearly with
/// diameter so the big-gap cadence stays the same around any ring;
/// the fine-stripe count **per segment** is fixed so each
/// illuminated segment always looks the same density (the user
/// complaint: varying fine count made the tractor / drone look
/// inconsistent).
const REF_DIAMETER: f32 = 2.5;
const REF_PULSE_COUNT: f32 = 8.0;
/// Stripes per coarse segment, same on every ring.
const FINE_MULT: f32 = 4.0;

/// Angular resolution of the annulus mesh.
const RING_RESOLUTION: u32 = 128;

fn make_ring_mesh(outer: f32, thickness: f32) -> Mesh {
    let inner = (outer - thickness).max(0.01);
    Annulus::new(inner, outer).mesh().resolution(RING_RESOLUTION).build()
}

pub fn setup_selection_ring(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SelectionRingMaterial>>,
    settings: Res<SelectionRingSettings>,
) {
    // Annulus lies in its own XY plane; rotate -90° around X so the
    // disc is flat on the world XZ plane (normal facing +Y).
    let initial_outer = 1.0f32;
    let initial_thickness = settings.thickness;
    let mesh = meshes.add(make_ring_mesh(initial_outer, initial_thickness));
    let mat = materials.add(SelectionRingMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: SelectionRingExtension::default(),
    });

    commands.spawn((
        Name::new("SelectionRing"),
        SelectionRing {
            material: mat.clone(),
            mesh: mesh.clone(),
            built_for: (initial_outer, initial_thickness),
        },
        Transform {
            translation: Vec3::new(0.0, RING_GROUND_OFFSET, 0.0),
            rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        },
        Mesh3d(mesh),
        MeshMaterial3d(mat),
        Visibility::Hidden,
        NotShadowCaster,
    ));
}

pub fn update_selection_ring(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    clock: Res<SimClock>,
    accent: Res<AccentColor>,
    settings: Res<SelectionRingSettings>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SelectionRingMaterial>>,
    mut q: Query<(&mut SelectionRing, &mut Transform, &mut Visibility)>,
) {
    let Ok((mut ring, mut tr, mut vis)) = q.single_mut() else { return };

    // Advance the shader clock every frame regardless of visibility,
    // so a just-shown ring isn't phase-snapping.
    let t = time.elapsed_secs();

    // The ring is a drive-mode marker; edit-mode uses transform gizmos.
    if clock.paused {
        *vis = Visibility::Hidden;
        return;
    }

    let Some(id) = selection.vehicle else {
        *vis = Visibility::Hidden;
        return;
    };
    let Some(state) = sim.0.vehicle(id) else {
        *vis = Visibility::Hidden;
        return;
    };

    let pose = sim.0.vehicle_pose(id);
    // Outer radius = farthest point of the whole machine from the
    // chassis origin (top-down), scaled up a bit so the ring sits
    // clear of the silhouette.
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
    // Per-mode padding + floor. Padding is constant in metres, so
    // the ring doesn't balloon on long machines (oxbo) — the
    // harvest head gets ~50 cm of clearance, not 40 % extra radius.
    let (padding, min_outer) = match state.spec.drive_mode {
        DriveMode::Drone => (RING_PADDING_DRONE, MIN_OUTER_RADIUS_DRONE),
        _ => (RING_PADDING_GROUND, MIN_OUTER_RADIUS_GROUND),
    };
    let outer = (max_reach + padding).max(min_outer);
    let thickness = settings.thickness.max(0.01);

    // Coarse pulse count scales with diameter — big gaps are the
    // same *arc length* on every ring. Rounded to an even integer
    // so on/off pairs stay symmetric around the seam. Fine-stripe
    // count is a fixed per-segment constant.
    let diameter = outer * 2.0;
    let ratio = (diameter / REF_DIAMETER).max(1.0);
    let mut pulse_count = (REF_PULSE_COUNT * ratio).round().max(2.0);
    if (pulse_count as i32) & 1 == 1 {
        pulse_count += 1.0;
    }
    let fine_mult = FINE_MULT;

    // Rebuild the mesh only if the radius or thickness actually
    // changed — which happens at most on selection or slider moves,
    // not per frame.
    let key = (outer, thickness);
    if (ring.built_for.0 - key.0).abs() > 1e-3 || (ring.built_for.1 - key.1).abs() > 1e-3 {
        if let Some(m) = meshes.get_mut(&ring.mesh) {
            *m = make_ring_mesh(outer, thickness);
        }
        ring.built_for = key;
    }

    // Drones ride at the vehicle's altitude so the marker floats with
    // the machine; ground vehicles get a small lift above y = 0 so the
    // ring doesn't z-fight the ground patch.
    let ring_y = match state.spec.drive_mode {
        DriveMode::Drone => pose.point.y as f32,
        _ => RING_GROUND_OFFSET,
    };

    *vis = Visibility::Visible;
    tr.translation = Vec3::new(pose.point.x as f32, ring_y, pose.point.z as f32);
    tr.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    // Scale = 1.0 — the mesh is already built in world units, so
    // thickness stays constant regardless of the machine's size.
    tr.scale = Vec3::ONE;

    // Drive shader uniforms from the live accent + ring centre so the
    // spinning pattern stays in the same world-space phase relative to
    // the vehicle. Pulse / fine counts are size-scaled so a tiny
    // drone ring doesn't look crowded.
    if let Some(mat) = materials.get_mut(&ring.material) {
        let [r, g, b, _] = egui_to_linear(accent.0);
        mat.extension.color_r = r;
        mat.extension.color_g = g;
        mat.extension.color_b = b;
        mat.extension.time = t;
        mat.extension.alpha = 0.9;
        mat.extension.center_x = tr.translation.x;
        mat.extension.center_z = tr.translation.z;
        mat.extension.pulse_count = pulse_count;
        mat.extension.fine_mult = fine_mult;
    }
}

fn egui_to_linear(c: bevy_egui::egui::Color32) -> [f32; 4] {
    let to_f = |v: u8| (v as f32) / 255.0;
    [to_f(c.r()), to_f(c.g()), to_f(c.b()), to_f(c.a())]
}
