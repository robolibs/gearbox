//! Selected-vehicle highlight. Uses the vehicle's `GlobalTransform` (which
//! big_space has already rebased into render space), so gizmos appear in
//! the right place regardless of how far the camera has zoomed.

use bevy::color::palettes::css;
use bevy::prelude::*;

use crate::viz::{GearboxSim, VehicleBody};

use super::selection::Selection;

pub fn selection_gizmos_system(
    mut gizmos: Gizmos,
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    bodies: Query<(&VehicleBody, &GlobalTransform)>,
) {
    let Some(id) = selection.vehicle else { return };
    let Some(state) = sim.0.vehicle(id) else { return };

    let Some((_, gt)) = bodies.iter().find(|(vb, _)| vb.id == id) else { return };
    let tr = gt.compute_transform();
    let half = Vec3::new(
        (state.spec.chassis.size.x * 0.5) as f32,
        (state.spec.chassis.size.y * 0.5) as f32,
        (state.spec.chassis.size.z * 0.5) as f32,
    );

    gizmos.primitive_3d(
        &Cuboid::from_size(half * 2.0),
        Isometry3d::new(tr.translation, tr.rotation),
        Color::from(css::ORANGE),
    );

    // Forward arrow in render space.
    let forward = tr.rotation * Vec3::Z;
    let tip = tr.translation + forward * (half.z + 1.0);
    gizmos.arrow(tr.translation, tip, Color::from(css::ORANGE_RED));
}
