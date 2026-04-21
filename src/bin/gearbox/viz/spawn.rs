//! Spawn helpers for turning a `VehicleSpec` into Bevy entities.
//!
//! - Chassis is the root; each wheel is a top-level sibling (rapier's
//!   vehicle controller computes wheel poses in world space directly).
//! - Body **parts** (hitches, karosseries, tanks) are children of the
//!   chassis — they have a fixed local offset, so Bevy's transform
//!   propagation keeps them glued to the chassis for free.

use bevy::prelude::*;
use big_space::prelude::BigSpatialBundle;

use gearbox::{PartKind, VehicleId, VehicleSpec};

use super::{VehicleBody, VehicleWheel};

pub fn spawn_vehicle_visuals(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    id: VehicleId,
    spec: &VehicleSpec,
    big_space_root: Entity,
) -> Entity {
    let [r, g, b] = spec.chassis.color;
    let chassis_color = Color::srgb(r, g, b);
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

    // Wheels — tracked separately, not parented (pose from controller).
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

    // Parts — parented to the chassis so they inherit its pose.
    for part in &spec.parts {
        let [pr, pg, pb] = part.color;
        let p_color = Color::srgb(pr, pg, pb);
        let mesh = meshes.add(Cuboid::new(
            part.size.x as f32,
            part.size.y as f32,
            part.size.z as f32,
        ));
        let mat = materials.add(StandardMaterial {
            base_color: p_color,
            perceptual_roughness: match part.kind {
                PartKind::Hitch => 0.3, // slightly glossy marker
                _ => 0.7,
            },
            metallic: match part.kind {
                PartKind::Hitch => 0.4,
                _ => 0.1,
            },
            ..default()
        });
        commands
            .spawn((
                Name::new(format!("{}::{}", spec.name, part.name)),
                Transform::from_xyz(
                    part.position.x as f32,
                    part.position.y as f32,
                    part.position.z as f32,
                ),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
            ))
            .insert(ChildOf(root));
    }

    root
}

/// Pick a spawn Y that guarantees the chassis starts a bit above the
/// ground (wheels hang down, settle on contact). Dynamic so we don't
/// hard-code 1.4 for every preset regardless of size.
pub fn spawn_height_for(spec: &VehicleSpec) -> f64 {
    // 0.6 m clearance under the chassis bottom — enough for wheels to
    // hang in air at rest regardless of radius / suspension tuning.
    spec.chassis.size.y * 0.5 + 0.6
}
