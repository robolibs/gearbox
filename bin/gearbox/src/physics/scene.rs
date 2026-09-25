//! `UsdPhysicsScene` → `PhysicsWorld.gravity`. First scene wins;
//! subsequent ones warn (the sim runs one world).

use super::markers::UsdPhysicsScene;
use bevy::prelude::*;
use glam::DVec3;

use super::world::PhysicsWorld;

pub fn sync_gravity_from_usd_scene(
    scenes: Query<&UsdPhysicsScene, Added<UsdPhysicsScene>>,
    mut world: ResMut<PhysicsWorld>,
    mut applied: Local<bool>,
) {
    for scene in &scenes {
        if *applied {
            warn!(
                "gearbox-physics: multiple UsdPhysicsScene prims; ignoring extras (gravity already applied)"
            );
            continue;
        }
        let g = scene.gravity_direction.normalize_or_zero() * scene.gravity_magnitude;
        world.set_gravity(DVec3::new(g.x as f64, g.y as f64, g.z as f64));
        info!(
            "gearbox-physics: gravity set to {:?} m/s² (from UsdPhysicsScene)",
            world.gravity()
        );
        *applied = true;
    }
}
