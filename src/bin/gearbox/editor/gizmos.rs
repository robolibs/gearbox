//! Selected-vehicle highlight + transform gizmo.
//!
//! Modes (cycled with `Tab`):
//!   - `Translate` → three axis-coloured arrows (default)
//!   - `Rotate`    → three axis-coloured circles in the XYZ planes
//!   - `Scale`     → three axis-coloured stubs tipped with small cubes
//!
//! Tab is ignored when egui wants the keyboard (e.g. typing in a field).

use bevy::gizmos::config::GizmoConfigStore;
use bevy::prelude::*;
use bevy_egui::EguiContexts;

use crate::viz::{GearboxSim, VehicleBody};

use super::selection::Selection;

// Axis colours (render side). Must match `editor::style::AXIS_{X,Y,Z}`.
const AXIS_X: Color = Color::srgb(0.878, 0.263, 0.231);
const AXIS_Y: Color = Color::srgb(0.498, 0.706, 0.208);
const AXIS_Z: Color = Color::srgb(0.180, 0.514, 0.902);
// Selection outline (render side) = editor::style::ACCENT (#A78BFA).
const OUTLINE: Color = Color::srgb(0.655, 0.545, 0.980);

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum GizmoMode {
    #[default]
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    fn next(self) -> Self {
        match self {
            GizmoMode::Translate => GizmoMode::Rotate,
            GizmoMode::Rotate    => GizmoMode::Scale,
            GizmoMode::Scale     => GizmoMode::Translate,
        }
    }
}

/// Startup system — bump the global gizmo line width so selection
/// arrows/circles are actually visible on a busy scene.
pub fn configure_gizmos(mut config: ResMut<GizmoConfigStore>) {
    for (_, cfg, _) in config.iter_mut() {
        cfg.line.width = 3.0;
    }
}

/// Cycle gizmo mode on `Tab`. Ignored when egui wants the keyboard
/// (e.g. user is typing in an egui field).
pub fn gizmo_mode_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<GizmoMode>,
    mut contexts: EguiContexts,
) {
    let wants_kb = contexts
        .ctx_mut()
        .map(|ctx| ctx.wants_keyboard_input())
        .unwrap_or(false);
    if wants_kb {
        return;
    }
    if keys.just_pressed(KeyCode::Tab) {
        *mode = mode.next();
    }
}

pub fn selection_gizmos_system(
    mut gizmos: Gizmos,
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    mode: Res<GizmoMode>,
    bodies: Query<(&VehicleBody, &GlobalTransform)>,
) {
    let Some(id) = selection.vehicle else { return };
    let Some(state) = sim.0.vehicle(id) else { return };
    let Some((_, gt)) = bodies.iter().find(|(vb, _)| vb.id == id) else { return };

    let tr = gt.compute_transform();
    let size = Vec3::new(
        state.spec.chassis.size.x as f32,
        state.spec.chassis.size.y as f32,
        state.spec.chassis.size.z as f32,
    );

    // Selection outline — wireframe box slightly bigger than the chassis.
    gizmos.primitive_3d(
        &Cuboid::from_size(size * 1.02),
        Isometry3d::new(tr.translation, tr.rotation),
        OUTLINE,
    );

    let local_axes = [
        (tr.rotation * Vec3::X, AXIS_X),
        (tr.rotation * Vec3::Y, AXIS_Y),
        (tr.rotation * Vec3::Z, AXIS_Z),
    ];
    let reach = size.max_element() * 0.8 + 0.5;

    match *mode {
        GizmoMode::Translate => {
            for (dir, color) in local_axes {
                let tip = tr.translation + dir * reach;
                gizmos
                    .arrow(tr.translation, tip, color)
                    .with_tip_length(reach * 0.2);
            }
        }
        GizmoMode::Rotate => {
            // Three circles centred on the vehicle, each perpendicular
            // to its axis. Rotate the default (XY-plane) circle so its
            // local Z points along `dir`.
            let r = reach * 1.05;
            for (dir, color) in local_axes {
                let rotation = Quat::from_rotation_arc(Vec3::Z, dir);
                gizmos.circle(
                    Isometry3d::new(tr.translation, rotation),
                    r,
                    color,
                );
            }
        }
        GizmoMode::Scale => {
            // Three short stubs tipped with small cubes — the
            // standard "scale" gizmo visual used by every editor.
            let cube_edge = reach * 0.12;
            for (dir, color) in local_axes {
                let tip = tr.translation + dir * reach;
                gizmos.line(tr.translation, tip, color);
                // Box at the tip, oriented along `dir` so it "faces"
                // outward along that axis.
                let rotation = Quat::from_rotation_arc(Vec3::Z, dir);
                gizmos.primitive_3d(
                    &Cuboid::from_size(Vec3::splat(cube_edge)),
                    Isometry3d::new(tip, rotation),
                    color,
                );
            }
        }
    }
}
