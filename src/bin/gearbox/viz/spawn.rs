//! Spawn helpers for turning a `VehicleSpec` into Bevy entities.
//!
//! The returned root entity has the chassis mesh on it and each wheel is a
//! top-level entity — not a child of the chassis — because each wheel's
//! pose is computed in world space directly by the vehicle controller.

use bevy::prelude::*;
use big_space::prelude::BigSpatialBundle;

use gearbox::{VehicleId, VehicleSpec};

use super::{VehicleBody, VehicleWheel};

/// Spawn the chassis + wheel visuals for a vehicle that has already been
/// created inside [`gearbox::Sim`]. Returns the root (chassis) entity.
///
/// `big_space_root` is the entity ID of the enclosing `BigSpace`; every
/// spawned mesh is parented to it so big_space's floating-origin
/// transform propagation picks them up.
pub fn spawn_vehicle_visuals(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    id: VehicleId,
    spec: &VehicleSpec,
    big_space_root: Entity,
) -> Entity {
    let chassis_color = Color::srgb(0.25, 0.55, 0.22); // tractor green
    let wheel_color = Color::srgb(0.08, 0.08, 0.08);

    let chassis_mesh = meshes.add(Cuboid::new(
        spec.chassis.size.x as f32,
        spec.chassis.size.y as f32,
        spec.chassis.size.z as f32,
    ));
    let chassis_mat = materials.add(StandardMaterial {
        base_color: chassis_color,
        perceptual_roughness: 0.6,
        metallic: 0.1,
        ..default()
    });
    let root = commands
        .spawn((
            Name::new(spec.name.clone()),
            BigSpatialBundle::default(),
            Mesh3d(chassis_mesh),
            MeshMaterial3d(chassis_mat),
            VehicleBody { id },
        ))
        .insert(ChildOf(big_space_root))
        .id();

    for (idx, wheel) in spec.wheels.iter().enumerate() {
        let mesh = meshes.add(Cylinder::new(wheel.radius, wheel.width));
        let mat = materials.add(StandardMaterial {
            base_color: wheel_color,
            perceptual_roughness: 0.9,
            metallic: 0.0,
            ..default()
        });
        commands
            .spawn((
                Name::new(format!("{}::wheel[{}]", spec.name, idx)),
                BigSpatialBundle::default(),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                VehicleWheel { id, index: idx },
            ))
            .insert(ChildOf(big_space_root));
    }

    root
}
