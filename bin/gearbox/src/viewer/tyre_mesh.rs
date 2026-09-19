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
use crate::controller::wheel_forces::{rigid_name, rubber_name};

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
    last_frame: Option<(Affine3A, LoadedTireEnvelope, Option<f64>)>,
}

fn contact_frame(sample: WheelForceOutput) -> Option<Affine3A> {
    let pressure = sample.pressure?;
    let vector = |v: glam::DVec3| Vec3::from_array(v.to_array().map(|v| v as f32));
    let up = pressure.ground.map_or(Vec3::Y, |ground| vector(ground.normal));
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

fn supported_vertex(envelope: LoadedTireEnvelope, point: [f64; 3], ground_distance: Option<f64>) -> [f64; 3] {
    let mut point = envelope.deform(point);
    if let Some(distance) = ground_distance {
        point[1] = point[1].max(-distance);
    }
    point
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
    mut trace_at: Local<Option<std::time::Instant>>,
) {
    let started = std::time::Instant::now();
    let mut timings = [0.0; 3];
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
        let ground_distance = pressure.ground.map(|ground| (pressure.hub - ground.point).dot(ground.normal));
        let envelope = LoadedTireEnvelope {
            radius: pressure.radius,
            bead_radius: beads[&entry.body],
            width: pressure.width,
            loaded_radius: pressure.loaded_radius,
        };
        let to_frame = frame.inverse() * global.affine();
        if !to_frame.is_finite() || to_frame.matrix3.determinant().abs() < 1e-12 {
            continue;
        }
        if entry.last_frame == Some((to_frame, envelope, ground_distance)) {
            continue;
        }
        let Some(mut mesh) = meshes.get_mut(&entry.private) else {
            continue;
        };
        let from_frame = to_frame.inverse();
        let deform_at = std::time::Instant::now();
        let positions: Vec<_> = entry
            .positions
            .iter()
            .map(|p| {
                let p = to_frame.transform_point3(Vec3::from_array(*p));
                let p = supported_vertex(envelope, p.to_array().map(f64::from), ground_distance);
                from_frame
                    .transform_point3(Vec3::from_array(p.map(|v| v as f32)))
                    .to_array()
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        timings[0] += deform_at.elapsed().as_secs_f64();
        let normals_at = std::time::Instant::now();
        mesh.compute_normals();
        timings[1] += normals_at.elapsed().as_secs_f64();
        let tangents_at = std::time::Instant::now();
        if mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT) {
            let _ = mesh.generate_tangents();
        }
        timings[2] += tangents_at.elapsed().as_secs_f64();
        if let Some(bounds) = mesh.compute_aabb() {
            commands.entity(*entity).insert(bounds);
        }
        entry.last_frame = Some((to_frame, envelope, ground_distance));
    }
    if std::env::var_os("GEARBOX_TYRE_MESH_TRACE").is_some()
        && trace_at.is_none_or(|at| at.elapsed().as_secs() >= 3)
    {
        *trace_at = Some(std::time::Instant::now());
        info!("tyre-mesh timing: total_ms={} deform_ms={} normals_ms={} tangents_ms={}", started.elapsed().as_secs_f64() * 1000.0, timings[0] * 1000.0, timings[1] * 1000.0, timings[2] * 1000.0);
        for (entity, entry) in &rubber.0 {
            let Ok((_, _, global)) = visuals.get(*entity) else { continue };
            let Some(mesh) = meshes.get(&entry.private) else { continue };
            let Some(VertexAttributeValues::Float32x3(current)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { continue };
            let Some((frame, envelope, _)) = entry.last_frame else { continue };
            let mut moved = 0;
            let mut maximum = 0.0_f32;
            let mut bottom = f32::INFINITY;
            let mut world_bottom = f32::INFINITY;
            for (reference, deformed) in entry.positions.iter().zip(current) {
                let delta = global.affine().transform_vector3(Vec3::from_array(*deformed) - Vec3::from_array(*reference)).length();
                moved += usize::from(delta > 0.0001);
                maximum = maximum.max(delta);
                bottom = bottom.min(frame.transform_point3(Vec3::from_array(*reference)).y);
                world_bottom = world_bottom.min(global.transform_point(Vec3::from_array(*deformed)).y);
            }
            let name = hierarchy.get(*entity).ok().and_then(|(_, prim, name, _)| prim.map(|p| p.path.as_str()).or(name.map(|n| n.as_str()))).unwrap_or("subset");
            let pressure = samples[&entry.body].0.pressure.unwrap();
            let ground_y = pressure.ground.map_or(f64::NAN, |ground| ground.point.y);
            info!("tyre-mesh: {name} body={:?} vertices={} moved={moved} maximum_m={maximum} reference_bottom={bottom} world_bottom={world_bottom} ground_y={ground_y} hub_y={} radius={} bead={} loaded={}", entry.body, current.len(), pressure.hub.y, envelope.radius, envelope.bead_radius, envelope.loaded_radius);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::MollaBackend;
    use crate::physics::backend::*;
    use glam::DVec3;

    #[test]
    fn pressure_changes_private_rubber_in_both_directions() {
        let mut backend = MollaBackend::default();
        let ground = backend.insert_collider(ColliderDesc::new(Shape::Cuboid {
            half_extents: DVec3::new(5.0, 0.1, 5.0),
        }).translation(-DVec3::Y * 0.1)).unwrap();
        backend.register_wheel_ground(ground, None).unwrap();
        let anchor = backend.insert_body(BodyDesc::fixed());
        let dynamic = |mass| {
            let mut desc = BodyDesc::dynamic().pose(Pose::from_translation(DVec3::Y * 0.48));
            desc.additional_mass = Some(MassProps {
                local_com: DVec3::ZERO, mass, inertia: Inertia::Principal(DVec3::ONE),
            });
            desc
        };
        let chassis = backend.insert_body(dynamic(100.0));
        let wheel = backend.insert_body(dynamic(1.0));
        backend.insert_joint(anchor, chassis, JointDesc::new(
            JointKind::Prismatic { axis: DVec3::Y },
            Pose::from_translation(DVec3::Y * 0.48), Pose::IDENTITY,
        ));
        let joint = backend.insert_joint(chassis, wheel, JointDesc::new(
            JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY,
        ));
        backend.configure_wheel(WheelForceDesc {
            body: wheel, joint, local_hub: DVec3::ZERO, forward: DVec3::X,
            radius: 0.5, supported_mass: 101.0,
            tyre: Some(PressureTyreDesc { pressure_pa: 50_000.0, ..PressureTyreDesc::reference(0.3) }),
        }).unwrap();
        let steps = (8.0 / backend.settings().dt).ceil() as usize;
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().add_plugins(TyreMeshPlugin);
        let body = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
        let mut physics = PhysicsWorld::with_backend(Box::new(backend));
        physics.entity_to_body.insert(body, wheel);
        app.insert_resource(physics);
        let original = vec![[0.0, -0.5, 0.1], [0.1, -0.49, 0.1], [0.0, -0.25, 0.1]];
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, original.clone());
        let source = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let rubber = app.world_mut().spawn((
            ChildOf(body), Name::new("radial_tyre"), Mesh3d(source.clone()), GlobalTransform::IDENTITY,
        )).id();
        let sample = |app: &mut App, target| {
            let hub = {
                let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
                physics.set_wheel_pressures(&[(wheel, target)]).unwrap();
                for _ in 0..steps { physics.backend.step(&|_, _| false); }
                let pressure = physics.wheel_output(wheel).unwrap().pressure.unwrap();
                assert_eq!(pressure.pressure_pa, target);
                pressure.hub
            };
            let global = GlobalTransform::from_translation(Vec3::from_array(hub.to_array().map(|v| v as f32)));
            for entity in [body, rubber] {
                *app.world_mut().get_mut::<GlobalTransform>(entity).unwrap() = global;
            }
            app.update();
            let handle = &app.world().get::<Mesh3d>(rubber).unwrap().0;
            assert_ne!(*handle, source);
            let meshes = app.world().resource::<Assets<Mesh>>();
            assert_eq!(reference_positions(meshes.get(&source).unwrap()).unwrap(), original);
            let positions = reference_positions(meshes.get(handle).unwrap()).unwrap();
            assert!((f64::from(positions[0][1]) + hub.y).abs() < 1e-6);
            positions
        };
        let low = sample(&mut app, 50_000.0);
        let high = sample(&mut app, 400_000.0);
        assert!(high[0][1] < low[0][1] - 0.005);
        let returned = sample(&mut app, 50_000.0);
        for (a, b) in returned.iter().flatten().zip(low.iter().flatten()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn zero_force_rubber_stays_above_the_ground_plane_including_tread_lugs() {
        for normal in [DVec3::Y, DVec3::new(0.0, 1.0, 0.2).normalize()] {
            for distance in [0.8_f64, 1.02] {
                let ground = TyreGroundPlane { point: DVec3::new(1.0, 0.3, 2.0), normal };
                let pressure = PressureTyreOutput {
                    hub: ground.point + normal * distance, forward: DVec3::X,
                    radius: 1.0, loaded_radius: distance.min(1.0), width: 0.4,
                    ground: Some(ground), ..Default::default()
                };
                let frame = contact_frame(WheelForceOutput { pressure: Some(pressure), ..Default::default() }).unwrap();
                let envelope = LoadedTireEnvelope { radius: 1.0, bead_radius: 0.5, width: 0.4, loaded_radius: pressure.loaded_radius };
                let point = supported_vertex(envelope, [0.0, -1.05, 0.2], Some(distance));
                let world = frame.transform_point3(Vec3::from_array(point.map(|v| v as f32)));
                assert!((DVec3::from_array(world.to_array().map(f64::from)) - ground.point).dot(normal).abs() < 1e-6);
            }
        }
        assert!(!rubber_name("tire_collider"));
    }

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
