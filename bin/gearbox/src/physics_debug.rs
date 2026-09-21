//! `GEARBOX_CONTACT_LOG=1` prints, once a second, every pair of rigid
//! bodies with an active contact, named by prim, so a shaking machine can
//! be traced to the colliders that touch.

use std::collections::HashMap;

use crate::physics::PhysicsWorld;
use bevy::prelude::*;
use crate::physics::backend::{BodyId, ColliderId};
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
    let names: HashMap<BodyId, String> = physics
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
    // Deepest point of every collider pair in active contact.
    let mut depths: HashMap<(ColliderId, ColliderId), f64> = HashMap::new();
    for manifold in physics.contacts().iter().filter(|m| m.active) {
        let depth = depths
            .entry((manifold.collider1, manifold.collider2))
            .or_insert(0.0);
        for point in &manifold.points {
            *depth = depth.min(point.dist);
        }
    }
    let body = |c: ColliderId| {
        physics
            .collider(c)
            .and_then(|col| col.parent())
            .and_then(|h| names.get(&h).cloned())
            .unwrap_or_else(|| "static".to_string())
    };
    let mut lines: Vec<String> = depths
        .iter()
        .map(|((c1, c2), depth)| format!("{} <> {} depth {:.4}", body(*c1), body(*c2), depth))
        .collect();
    lines.sort();
    // Closed loops that cannot close leave a permanent anchor gap that the
    // solver fights every step.
    let mut gaps: Vec<(f64, String)> = Vec::new();
    for id in physics.joints() {
        if physics.joint_is_reduced(id) {
            continue;
        }
        let (Some((body1, body2)), Some(joint)) = (physics.joint_bodies(id), physics.joint(id))
        else {
            continue;
        };
        let (Some(b1), Some(b2)) = (physics.body(body1), physics.body(body2)) else {
            continue;
        };
        let a1 = b1.position().transform_point(joint.frame1().translation);
        let a2 = b2.position().transform_point(joint.frame2().translation);
        let gap = (a1 - a2).length();
        if gap > 0.002 {
            gaps.push((
                gap,
                format!(
                    "{} - {}",
                    names.get(&body1).cloned().unwrap_or_default(),
                    names.get(&body2).cloned().unwrap_or_default()
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
