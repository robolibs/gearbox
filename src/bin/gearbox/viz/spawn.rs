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

/// Marker for all meshes that make up the currently-dragging ghost
/// spawn preview. Despawning the tagged root with `despawn_recursive`
/// removes every child mesh as well.
#[derive(Component)]
pub struct GhostTag;

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

/// Non-physics translucent preview of a vehicle — same meshes/parts
/// as the real one, but with alpha-blended materials and no
/// `VehicleBody` / rapier tagging. Used for the "drag-to-place" UX:
/// the ghost follows the cursor until the user commits with a click.
pub fn spawn_vehicle_ghost(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    spec: &VehicleSpec,
    big_space_root: Entity,
) -> Entity {
    let alpha = 0.45;
    let [r, g, b] = spec.chassis.color;
    let chassis_color = Color::srgba(r, g, b, alpha);
    let wheel_color   = Color::srgba(0.08, 0.08, 0.08, alpha);

    let chassis_mesh = meshes.add(Cuboid::new(
        spec.chassis.size.x as f32,
        spec.chassis.size.y as f32,
        spec.chassis.size.z as f32,
    ));
    let chassis_mat = materials.add(StandardMaterial {
        base_color: chassis_color,
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.7,
        metallic: 0.1,
        ..default()
    });
    let root = commands
        .spawn((
            Name::new(format!("{}-ghost", spec.name)),
            BigSpatialBundle::default(),
            Mesh3d(chassis_mesh),
            MeshMaterial3d(chassis_mat),
            GhostTag,
        ))
        .insert(ChildOf(big_space_root))
        .id();

    // Wheels as children of the ghost root — at rest (suspension
    // fully extended) so the silhouette reads as a settled vehicle.
    // Cylinder default axis is +Y; rotate 90° around Z so the axle
    // lies along X.
    for wheel in &spec.wheels {
        let mesh = meshes.add(Cylinder::new(wheel.radius, wheel.width));
        let mat = materials.add(StandardMaterial {
            base_color: wheel_color,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let wy = (wheel.chassis_connection.y - wheel.suspension_rest_length as f64) as f32;
        commands
            .spawn((
                Transform::from_xyz(
                    wheel.chassis_connection.x as f32,
                    wy,
                    wheel.chassis_connection.z as f32,
                )
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                GhostTag,
            ))
            .insert(ChildOf(root));
    }

    // Body parts — children of the chassis root with local offsets.
    for part in &spec.parts {
        let [pr, pg, pb] = part.color;
        let p_color = Color::srgba(pr, pg, pb, alpha);
        let mesh = meshes.add(Cuboid::new(
            part.size.x as f32,
            part.size.y as f32,
            part.size.z as f32,
        ));
        let mat = materials.add(StandardMaterial {
            base_color: p_color,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: match part.kind {
                PartKind::Hitch => 0.3,
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
                Transform::from_xyz(
                    part.position.x as f32,
                    part.position.y as f32,
                    part.position.z as f32,
                ),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                GhostTag,
            ))
            .insert(ChildOf(root));
    }

    root
}
