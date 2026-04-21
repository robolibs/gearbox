//! RTS-style selection ring.
//!
//! A single flat, accent-coloured annulus that sits on the ground plane
//! under the currently selected vehicle. Hidden when nothing's selected.
//! Not a transform gizmo — it doesn't rotate with the vehicle and
//! doesn't try to offer drag handles. It's a non-janky "this one is
//! selected" marker that reads at any camera angle.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use big_space::prelude::BigSpatialBundle;

use crate::viz::{GearboxSim, SimClock};
use crate::BigSpaceRoot;

use super::selection::Selection;
use super::style::AccentColor;

/// Tag for the ring entity. Only one exists in the scene.
#[derive(Component)]
pub struct SelectionRing {
    mat: Handle<StandardMaterial>,
}

/// Ring dimensions, in local mesh units. The entity's Transform
/// scales this up to the selected vehicle's footprint each frame.
const RING_INNER: f32 = 0.92;
const RING_OUTER: f32 = 1.00;

/// Ring sits this far above `y = 0` so it doesn't z-fight with the
/// ground plane.
const RING_GROUND_OFFSET: f32 = 0.03;

/// Ring outer radius = max(chassis.x, chassis.z) * 0.5 * this.
const RING_FOOTPRINT_SCALE: f32 = 1.35;

pub fn setup_selection_ring(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    big_space_root: Res<BigSpaceRoot>,
) {
    // Annulus is 2D in the XY plane; rotate -90° around X so it sits
    // flat on the ground (XZ plane, normal pointing +Y).
    let mesh = meshes.add(Annulus::new(RING_INNER, RING_OUTER).mesh().resolution(128));
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.9),
        emissive: LinearRgba::new(1.0, 1.0, 1.0, 1.0) * 2.0,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    });

    commands
        .spawn((
            Name::new("SelectionRing"),
            SelectionRing { mat: mat.clone() },
            BigSpatialBundle {
                transform: Transform {
                    translation: Vec3::new(0.0, RING_GROUND_OFFSET, 0.0),
                    rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                    scale: Vec3::ONE,
                },
                ..default()
            },
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Visibility::Hidden,
            NotShadowCaster,
        ))
        .insert(ChildOf(big_space_root.0));
}

pub fn update_selection_ring(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    clock: Res<SimClock>,
    accent: Res<AccentColor>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&SelectionRing, &mut Transform, &mut Visibility)>,
) {
    let Ok((ring, mut tr, mut vis)) = q.single_mut() else { return };

    // The ring is a "which one is selected" marker for drive-mode. In
    // edit-mode the 3-D transform gizmos take over the selection cue.
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
    let footprint =
        state.spec.chassis.size.x.max(state.spec.chassis.size.z) as f32 * 0.5 * RING_FOOTPRINT_SCALE;

    *vis = Visibility::Visible;

    // Place ring flat on the ground under the vehicle. No rotation
    // tracking — deliberately static horizontally so it reads the same
    // at any vehicle heading (RTS convention).
    tr.translation = Vec3::new(pose.point.x as f32, RING_GROUND_OFFSET, pose.point.z as f32);
    tr.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    tr.scale = Vec3::splat(footprint);

    // Drive material colour from the live accent. The unlit + emissive
    // combo keeps the ring readable regardless of lighting / fog.
    if let Some(mat) = materials.get_mut(&ring.mat) {
        let [r, g, b, _] = egui_to_linear(accent.0);
        mat.base_color = Color::srgba(r, g, b, 0.9);
        mat.emissive = LinearRgba::new(r, g, b, 1.0) * 2.0;
    }
}

fn egui_to_linear(c: bevy_egui::egui::Color32) -> [f32; 4] {
    let to_f = |v: u8| (v as f32) / 255.0;
    [to_f(c.r()), to_f(c.g()), to_f(c.b()), to_f(c.a())]
}
