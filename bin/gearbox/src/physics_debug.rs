//! `GEARBOX_CONTACT_LOG=1` prints, once a second, every pair of rigid
//! bodies with an active contact, named by prim, so a shaking machine can
//! be traced to the colliders that touch.

use std::collections::HashMap;

use crate::physics::PhysicsWorld;
use bevy::prelude::*;
use rapier3d::prelude::RigidBodyHandle;
use usd_bevy::UsdPrimRef;

pub struct PhysicsDebugPlugin;

impl Plugin for PhysicsDebugPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("GEARBOX_CONTACT_LOG").is_some() {
            app.insert_resource(Tick(Timer::from_seconds(1.0, TimerMode::Repeating)))
                .add_systems(Update, log_contacts);
        }
    }
}

#[derive(Resource)]
struct Tick(Timer);

fn log_contacts(
    time: Res<Time>,
    mut tick: ResMut<Tick>,
    physics: Res<PhysicsWorld>,
    prims: Query<&UsdPrimRef>,
) {
    if !tick.0.tick(time.delta()).just_finished() {
        return;
    }
    let names: HashMap<RigidBodyHandle, String> = physics
        .entity_to_body
        .iter()
        .map(|(entity, handle)| {
            let name = prims
                .get(*entity)
                .map(|p| p.path.clone())
                .unwrap_or_else(|_| format!("{entity:?}"));
            (*handle, name)
        })
        .collect();
    let mut lines = Vec::new();
    for pair in physics.narrow_phase.contact_pairs() {
        if !pair.has_any_active_contact() {
            continue;
        }
        let body = |c| {
            physics
                .colliders
                .get(c)
                .and_then(|col| col.parent())
                .and_then(|h| names.get(&h).cloned())
                .unwrap_or_else(|| "static".to_string())
        };
        let depth = pair
            .manifolds
            .iter()
            .flat_map(|m| m.points.iter().map(|p| p.dist))
            .fold(0.0_f64, f64::min);
        lines.push(format!(
            "{} <> {} depth {:.4}",
            body(pair.collider1),
            body(pair.collider2),
            depth
        ));
    }
    lines.sort();
    // Closed loops that cannot close leave a permanent anchor gap that the
    // solver fights every step.
    let mut gaps: Vec<(f64, String)> = Vec::new();
    for (_, joint) in physics.impulse_joints.iter() {
        let (Some(b1), Some(b2)) = (
            physics.bodies.get(joint.body1),
            physics.bodies.get(joint.body2),
        ) else {
            continue;
        };
        let a1 = b1
            .position()
            .transform_point(joint.data.local_frame1.translation);
        let a2 = b2
            .position()
            .transform_point(joint.data.local_frame2.translation);
        let gap = (a1 - a2).length();
        if gap > 0.002 {
            gaps.push((
                gap,
                format!(
                    "{} - {}",
                    names.get(&joint.body1).cloned().unwrap_or_default(),
                    names.get(&joint.body2).cloned().unwrap_or_default()
                ),
            ));
        }
    }
    gaps.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let gaps: Vec<String> = gaps
        .iter()
        .take(12)
        .map(|(g, n)| format!("gap {:.4} m {n}", g))
        .collect();
    info!(
        "gearbox-joints: {} anchors off by >2 mm\n  {}",
        gaps.len(),
        gaps.join("\n  ")
    );
    info!(
        "gearbox-contacts: {} active pairs, {} filtered\n  {}",
        lines.len(),
        physics.filtered_pairs.len() / 2,
        lines.join("\n  ")
    );
}
