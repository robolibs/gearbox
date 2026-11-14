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
        if gearbox_globe::Body::active() != gearbox_globe::Body::Earth {
            warn!(
                "gearbox-physics: planetary gravity retained; ignoring imported USD scene gravity"
            );
            continue;
        }
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

#[cfg(test)]
mod planet_tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, ColliderDesc, Pose, Shape};

    #[test]
    fn planet_gravity_all_bodies() {
        for planet in ["earth", "mars", "moon"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .env("GEARBOX_PLANET",planet)
                .args(["--exact","physics::scene::planet_tests::planet_runtime_gravity_survives_import_and_accelerates_bodies","--ignored","--nocapture"])
                .status().unwrap();
            assert!(status.success(), "{planet} gravity subprocess failed");
        }
    }

    #[test]
    #[ignore = "run each GEARBOX_PLANET in a separate process with oslo make test-planet-gravity"]
    fn planet_runtime_gravity_survives_import_and_accelerates_bodies() {
        let body = gearbox_globe::Body::active();
        let expected = body.gravity_mps2();
        let mut app = App::new();
        app.init_resource::<PhysicsWorld>()
            .add_systems(Update, sync_gravity_from_usd_scene);
        assert_eq!(
            app.world().resource::<PhysicsWorld>().gravity(),
            DVec3::new(0.0, -expected, 0.0)
        );
        app.world_mut().spawn(UsdPhysicsScene {
            gravity_direction: Vec3::NEG_Y,
            gravity_magnitude: 9.81,
        });
        app.update();
        app.world_mut().spawn(UsdPhysicsScene {
            gravity_direction: Vec3::X,
            gravity_magnitude: 30.0,
        });
        app.update();
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        assert!((physics.gravity() - DVec3::new(0.0, -expected, 0.0)).length() < 1e-6);
        let falling =
            physics.insert_body(BodyDesc::dynamic().pose(Pose::from_translation(DVec3::Y * 10.0)));
        physics
            .insert_collider(
                ColliderDesc::new(Shape::Ball { radius: 0.1 })
                    .density(1000.0)
                    .parent(falling),
            )
            .unwrap();
        let steps = (1.0 / physics.dt()).round() as usize;
        let duration = steps as f64 * physics.dt();
        for _ in 0..steps {
            physics.step();
        }
        let fallen = physics.body(falling).unwrap();
        let measured = -fallen.linvel().y / duration;
        assert!(
            (measured - expected).abs() < 0.001,
            "{body:?}: measured {measured}, expected {expected}"
        );
        assert!(
            (fallen.translation().y - (10.0 - 0.5 * expected * duration * duration)).abs()
                < expected * physics.dt()
        );
        assert!(fallen.translation().x.abs() + fallen.translation().z.abs() < 1e-9);
        println!(
            "{body:?}: measured {measured:.6} m/s² after two imported gravity scenes; y={:.6}",
            fallen.translation().y
        );
    }
}
