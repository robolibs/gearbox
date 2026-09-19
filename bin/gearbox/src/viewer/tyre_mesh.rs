use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::MeshAabb;
use bevy::math::Affine3A;
use bevy::mesh::{PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use molla_vehicle::real_tire::LoadedTireEnvelope;
use usd_bevy::UsdPrimRef;

use crate::physics::PhysicsWorld;
use crate::physics::backend::{BodyId, WheelForceOutput};

pub(super) struct TyreMeshPlugin;

impl Plugin for TyreMeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TyreMeshes>().add_systems(
            PostUpdate,
            update_tyre_meshes
                .after(bevy::transform::TransformSystems::Propagate)
                .before(bevy::camera::visibility::VisibilitySystems::CalculateBounds)
                .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        );
    }
}

#[derive(Resource, Default)]
struct TyreMeshes(HashMap<Entity, RubberMesh>);

struct RubberMesh {
    source: Handle<Mesh>,
    private: Handle<Mesh>,
    positions: Vec<[f32; 3]>,
    body: BodyId,
    bead_radius: f64,
    last_frame: Option<(Affine3A, LoadedTireEnvelope)>,
}

fn rubber_name(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    if rigid_name(&name) {
        return false;
    }
    ["tyre", "tire", "tread", "mould_line", "bkt_fl630"]
        .iter()
        .any(|s| name.contains(s))
}

fn rigid_name(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    ["rim", "hub", "collision"].iter().any(|s| name.contains(s))
}

fn contact_frame(sample: WheelForceOutput) -> Option<Affine3A> {
    let pressure = sample.pressure?;
    let vector = |v: glam::DVec3| Vec3::from_array(v.to_array().map(|v| v as f32));
    let up = if sample.in_contact {
        vector(sample.normal)
    } else {
        Vec3::Y
    };
    let up = up.try_normalize()?;
    let forward = vector(pressure.forward);
    let forward = (forward - up * forward.dot(up)).try_normalize()?;
    Some(Affine3A::from_mat3_translation(
        Mat3::from_cols(forward, up, forward.cross(up)),
        vector(pressure.hub),
    ))
}

fn reference_positions(mesh: &Mesh) -> Option<Vec<[f32; 3]>> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList
        || !mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)
        || mesh.indices().is_some_and(|indices| indices.is_empty())
    {
        return None;
    }
    match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) if positions.len() >= 3 => {
            Some(positions.clone())
        }
        _ => None,
    }
}

fn update_tyre_meshes(
    mut commands: Commands,
    physics: Res<PhysicsWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rubber: ResMut<TyreMeshes>,
    mut visuals: Query<(Entity, &mut Mesh3d, &GlobalTransform)>,
    hierarchy: Query<(
        Option<&ChildOf>,
        Option<&UsdPrimRef>,
        Option<&Name>,
        Option<&GlobalTransform>,
    )>,
) {
    let samples: HashMap<_, _> = physics
        .entity_to_body
        .iter()
        .filter_map(|(entity, body)| {
            let sample = physics.wheel_output(*body)?;
            let mut frame = contact_frame(sample)?;
            if let Ok((_, _, _, Some(global))) = hierarchy.get(*entity) {
                let pose = physics.body(*body)?.position();
                let physical = Affine3A::from_rotation_translation(
                    Quat::from_array(pose.rotation.to_array().map(|v| v as f32)),
                    Vec3::from_array(pose.translation.to_array().map(|v| v as f32)),
                );
                frame = global.affine() * physical.inverse() * frame;
            }
            Some((*body, (sample, frame)))
        })
        .collect();
    if samples.is_empty() && rubber.0.is_empty() {
        return;
    }
    rubber.0.retain(|entity, entry| {
        let Ok((_, mut mesh, _)) = visuals.get_mut(*entity) else {
            return false;
        };
        if mesh.0 != entry.private {
            return false;
        }
        if !samples.contains_key(&entry.body) {
            mesh.0 = entry.source.clone();
            if let Some(bounds) = meshes.get(&entry.source).and_then(Mesh::compute_aabb) {
                commands.entity(*entity).insert(bounds);
            }
            return false;
        }
        true
    });
    for (entity, mut mesh, global) in &mut visuals {
        if rubber.0.contains_key(&entity) {
            continue;
        }
        let mut current = entity;
        let mut is_rubber = false;
        let body = loop {
            if let Some(body) = physics.entity_to_body.get(&current) {
                break Some(*body);
            }
            let Ok((parent, prim, name, _)) = hierarchy.get(current) else {
                break None;
            };
            if prim.is_some_and(|p| rigid_name(&p.path))
                || name.is_some_and(|n| rigid_name(n.as_str()))
            {
                break None;
            }
            is_rubber |= prim.is_some_and(|p| rubber_name(&p.path))
                || name.is_some_and(|n| rubber_name(n.as_str()));
            let Some(parent) = parent else { break None };
            current = parent.parent();
        };
        let Some(body) = body.filter(|_| is_rubber) else {
            continue;
        };
        let Some((sample, frame)) = samples.get(&body).copied() else {
            continue;
        };
        let Some(source) = meshes.get(&mesh.0) else {
            continue;
        };
        let Some(positions) = reference_positions(source) else {
            continue;
        };
        let radius = sample.pressure.unwrap().radius;
        let to_frame = frame.inverse() * global.affine();
        if !to_frame.is_finite() || to_frame.matrix3.determinant().abs() < 1e-12 {
            continue;
        }
        let bead_radius = positions
            .iter()
            .map(|p| {
                let p = to_frame.transform_point3(Vec3::from_array(*p));
                f64::from(p.x.hypot(p.y))
            })
            .fold(radius, f64::min)
            .clamp(0.2 * radius, 0.9 * radius);
        let mut private = source.clone();
        private.final_aabb = None;
        private.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        let private = meshes.add(private);
        rubber.0.insert(
            entity,
            RubberMesh {
                source: mesh.0.clone(),
                private: private.clone(),
                positions,
                body,
                bead_radius,
                last_frame: None,
            },
        );
        mesh.0 = private;
    }
    let mut beads = HashMap::<BodyId, f64>::new();
    for entry in rubber.0.values() {
        beads
            .entry(entry.body)
            .and_modify(|r| *r = r.min(entry.bead_radius))
            .or_insert(entry.bead_radius);
    }
    for (entity, entry) in &mut rubber.0 {
        let Ok((_, _, global)) = visuals.get(*entity) else {
            continue;
        };
        let Some((sample, frame)) = samples.get(&entry.body).copied() else {
            continue;
        };
        let pressure = sample.pressure.unwrap();
        let envelope = LoadedTireEnvelope {
            radius: pressure.radius,
            bead_radius: beads[&entry.body],
            width: pressure.width,
            loaded_radius: if sample.in_contact {
                pressure.loaded_radius
            } else {
                pressure.radius
            },
        };
        let to_frame = frame.inverse() * global.affine();
        if !to_frame.is_finite() || to_frame.matrix3.determinant().abs() < 1e-12 {
            continue;
        }
        if entry.last_frame == Some((to_frame, envelope)) {
            continue;
        }
        let Some(mut mesh) = meshes.get_mut(&entry.private) else {
            continue;
        };
        let from_frame = to_frame.inverse();
        let positions: Vec<_> = entry
            .positions
            .iter()
            .map(|p| {
                let p = to_frame.transform_point3(Vec3::from_array(*p));
                let p = envelope.deform(p.to_array().map(f64::from));
                from_frame
                    .transform_point3(Vec3::from_array(p.map(|v| v as f32)))
                    .to_array()
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.compute_normals();
        if mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT) {
            let _ = mesh.generate_tangents();
        }
        if let Some(bounds) = mesh.compute_aabb() {
            commands.entity(*entity).insert(bounds);
        }
        entry.last_frame = Some((to_frame, envelope));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::MollaBackend;
    use crate::physics::backend::*;
    use glam::DVec3;

    #[test]
    fn empty_subset_parents_are_not_uploaded_as_private_meshes() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
        assert!(reference_positions(&mesh).is_none());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 3]);
        mesh.insert_indices(bevy::mesh::Indices::U32(vec![]));
        assert!(reference_positions(&mesh).is_none());
        mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
        assert!(reference_positions(&mesh).is_some());
        mesh.asset_usage = RenderAssetUsages::MAIN_WORLD;
        assert!(reference_positions(&mesh).is_none());
    }

    #[test]
    fn private_meshes_restore_without_touching_shared_sources_or_rims() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .add_plugins(TyreMeshPlugin);
        let mut backend = MollaBackend::default();
        let pose = Pose::from_translation(DVec3::Y * 0.8);
        let bearing = backend.insert_body(BodyDesc::fixed().pose(pose));
        let mut wheel_desc = BodyDesc::dynamic().pose(pose);
        wheel_desc.additional_mass = Some(MassProps {
            local_com: DVec3::ZERO,
            mass: 100.0,
            inertia: Inertia::Principal(DVec3::ONE),
        });
        let wheel = backend.insert_body(wheel_desc);
        let joint = backend.insert_joint(
            bearing,
            wheel,
            JointDesc::new(
                JointKind::Revolute { axis: DVec3::Z },
                Pose::IDENTITY,
                Pose::IDENTITY,
            ),
        );
        let ground = backend
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(10.0, 0.1, 10.0),
                })
                .translation(-DVec3::Y * 0.1),
            )
            .unwrap();
        backend.register_wheel_ground(ground, None).unwrap();
        backend
            .configure_wheel(WheelForceDesc {
                body: wheel,
                joint,
                local_hub: DVec3::ZERO,
                forward: DVec3::X,
                radius: 1.0,
                supported_mass: 100.0,
                tyre: Some(PressureTyreDesc::reference(0.4)),
            })
            .unwrap();
        backend.step(&|_, _| false);
        assert!(backend.wheel_output(wheel).unwrap().in_contact);
        let body = app
            .world_mut()
            .spawn(GlobalTransform::from_translation(Vec3::Y * 0.8))
            .id();
        let mut physics = PhysicsWorld::with_backend(Box::new(backend));
        physics.entity_to_body.insert(body, wheel);
        app.insert_resource(physics);
        let original = vec![[0.0, -1.0, 0.2], [0.1, -0.98, 0.2], [0.0, -0.5, 0.2]];
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, original.clone());
        let source = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let mut spawn = |name: &str| {
            app.world_mut()
                .spawn((
                    Mesh3d(source.clone()),
                    Name::new(name.to_owned()),
                    ChildOf(body),
                    GlobalTransform::from_translation(Vec3::Y * 0.8),
                ))
                .id()
        };
        let tyre = spawn("radial_tyre");
        let tread = spawn("traction_tread");
        let rim = spawn("OuterRim");
        app.update();
        let private = app.world().get::<Mesh3d>(tyre).unwrap().0.clone();
        assert_ne!(private, source);
        assert_ne!(private, app.world().get::<Mesh3d>(tread).unwrap().0);
        assert_eq!(app.world().get::<Mesh3d>(rim).unwrap().0, source);
        let assets = app.world().resource::<Assets<Mesh>>();
        assert_eq!(
            reference_positions(assets.get(&source).unwrap()).unwrap(),
            original
        );
        let deformed = reference_positions(assets.get(&private).unwrap()).unwrap();
        assert!((deformed[0][1] + 0.8).abs() < 1e-5);
        app.update();
        assert_eq!(
            reference_positions(
                app.world()
                    .resource::<Assets<Mesh>>()
                    .get(&private)
                    .unwrap()
            )
            .unwrap(),
            deformed
        );
        let shifted = GlobalTransform::from_translation(Vec3::new(2.0, 3.8, -1.0));
        for entity in [body, tyre, tread, rim] {
            *app.world_mut().get_mut::<GlobalTransform>(entity).unwrap() = shifted;
        }
        app.update();
        let shifted_shape = reference_positions(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&private)
                .unwrap(),
        )
        .unwrap();
        for (a, b) in shifted_shape
            .iter()
            .flatten()
            .zip(deformed.iter().flatten())
        {
            assert!((a - b).abs() < 1e-6);
        }
        {
            let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
            physics.collider_mut(ground).unwrap().set_enabled(false);
            physics.backend.step(&|_, _| false);
        }
        app.update();
        let restored = reference_positions(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&private)
                .unwrap(),
        )
        .unwrap();
        for (a, b) in restored.iter().flatten().zip(original.iter().flatten()) {
            assert!((a - b).abs() < 1e-6);
        }
        app.world_mut()
            .resource_mut::<PhysicsWorld>()
            .remove_body(wheel);
        app.update();
        assert_eq!(app.world().get::<Mesh3d>(tyre).unwrap().0, source);
        assert!(app.world().resource::<TyreMeshes>().0.is_empty());
    }

    #[test]
    fn rubber_selection_does_not_select_wheel_assemblies_or_rims() {
        for name in [
            "radial_tyre_001",
            "swept_traction_tread",
            "mould_line_0_54",
            "_0010_12_BKT_FL630_650_55R26_5_004",
        ] {
            assert!(rubber_name(name));
        }
        for name in [
            "wheel_front_left",
            "OuterRimBackLeft",
            "hubFrontLeft",
            "/tyre/collision",
            "/tyre/InnerRim",
        ] {
            assert!(!rubber_name(name));
        }
    }
}
