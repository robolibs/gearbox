//! Streamed boulder meshes and matching static collision surfaces.

use crate::{
    globe::Sites,
    physics::{
        PhysicsWorld,
        backend::{ColliderDesc, ColliderId, Pose, Shape},
    },
    terrain::ProceduralTerrain,
};
use bevy::{math::DVec3, mesh::VertexAttributeValues, prelude::*};
use gearbox_globe::{
    Body, Datum, Geodetic,
    rocks::{Rock, RockKey},
};
use std::collections::{HashMap, HashSet};

const DRAW_M: f64 = 220.0;
const CONTACT_M: f64 = 80.0;

pub struct PlanetRocksPlugin;

impl Plugin for PlanetRocksPlugin {
    fn build(&self, app: &mut App) {
        if Body::active() == Body::Earth {
            return;
        }
        app.init_resource::<RockStream>()
            .add_systems(Startup, templates)
            .add_systems(
                Update,
                stream
                    .after(crate::terrain::TerrainUpdates)
                    .before(crate::physics::step_physics),
            );
    }
}

struct Template {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    vertices: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
}

#[derive(Resource)]
struct Templates(Vec<Template>);

struct Resident {
    entity: Entity,
    collider: Option<ColliderId>,
    transform: Transform,
    template: usize,
    frame: Datum,
}

#[derive(Resource, Default)]
struct RockStream {
    entries: HashMap<(usize, RockKey), Resident>,
    elapsed: f32,
    reported: (usize, usize),
}

fn rock_mesh(index: usize) -> (Mesh, Vec<DVec3>, Vec<[u32; 3]>) {
    let mut mesh = Sphere::new(1.0).mesh().ico(2).unwrap();
    let Some(VertexAttributeValues::Float32x3(points)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        unreachable!()
    };
    for point in points.iter_mut() {
        let p = Vec3::from_array(*point);
        let phase = index as f32 * 1.73;
        let radius = 1.0
            + 0.12 * (p.x * 5.0 + phase).sin() * (p.z * 4.0 - phase).cos()
            + 0.06 * (p.y * 7.0 + phase).sin();
        *point = (p * radius).to_array();
    }
    let vertices = points
        .iter()
        .map(|p| Vec3::from_array(*p).as_dvec3())
        .collect();
    let indices: Vec<_> = mesh.indices().unwrap().iter().map(|i| i as u32).collect();
    let triangles = indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    mesh.compute_smooth_normals();
    (mesh, vertices, triangles)
}

fn templates(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let moon = Body::active() == Body::Moon;
    let rocks = (0..16)
        .map(|index| {
            let (mesh, vertices, triangles) = rock_mesh(index);
            let grey = 0.055 + index as f32 * 0.005;
            let color = if moon {
                Color::linear_rgb(grey, grey * 0.98, grey * 0.95)
            } else {
                Color::linear_rgb(grey * 1.15, grey * 0.85, grey * 0.65)
            };
            Template {
                mesh: meshes.add(mesh),
                vertices,
                triangles,
                material: materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.97,
                    ..default()
                }),
            }
        })
        .collect();
    commands.insert_resource(Templates(rocks));
}

fn local_transform(rock: &Rock, site: &Datum, land: &gearbox_globe::Terrain) -> Transform {
    let ground = rock.ground.ecef_on(site.body);
    let surface = ground.normalize() * (site.body.radius_m() + land.height(ground) as f64);
    let local = site.from_ecef(surface);
    let rock_frame = Datum::on(site.body, rock.ground.latitude, rock.ground.longitude);
    let rotation =
        (site.rotation.inverse() * rock_frame.rotation).as_quat() * Quat::from_rotation_y(rock.yaw);
    Transform {
        translation: local.as_vec3() + rotation * Vec3::Y * rock.scale.y * 0.2,
        rotation,
        scale: rock.scale,
    }
}

fn collider(template: &Template, transform: &Transform, site: usize) -> ColliderDesc {
    ColliderDesc::new(Shape::TriMesh {
        vertices: template
            .vertices
            .iter()
            .map(|v| glam::DVec3::from_array((*v * transform.scale.as_dvec3()).to_array()))
            .collect(),
        indices: template.triangles.clone(),
    })
    .pose(Pose::new(
        glam::DVec3::from_array(
            (transform.translation.as_dvec3() + gearbox_globe::physics_offset(site)).to_array(),
        ),
        glam::DQuat::from_array(transform.rotation.as_dquat().to_array()),
    ))
    .friction(0.85)
    .restitution(0.0)
}

fn stream(
    mut commands: Commands,
    mut state: ResMut<RockStream>,
    templates: Res<Templates>,
    sites: Res<Sites>,
    terrain: Option<Res<ProceduralTerrain>>,
    view: Res<crate::viewer::camera::View>,
    mut physics: ResMut<PhysicsWorld>,
    time: Res<Time>,
) {
    if terrain.is_none() {
        for (_, resident) in state.entries.drain() {
            commands.entity(resident.entity).despawn();
            if let Some(id) = resident.collider {
                physics.remove_collider(id, true);
            }
        }
        return;
    }
    state.elapsed += time.delta_secs();
    if state.elapsed < 0.4 {
        return;
    }
    state.elapsed = 0.0;
    let mut sources: Vec<(usize, Geodetic, bool)> = Vec::new();
    for id in physics.bodies() {
        let Some(body) = physics.body(id) else {
            continue;
        };
        if !body.is_dynamic() {
            continue;
        }
        let at = body.translation();
        let site = gearbox_globe::region_of_physics(at.x);
        if site >= sites.list.len() {
            continue;
        }
        let point = sites.list[site]
            .frame
            .geodetic(DVec3::from_array(at.to_array()) - gearbox_globe::physics_offset(site));
        if sources.iter().any(|(s, p, _)| {
            *s == site
                && p.ecef_on(sites.land.body())
                    .distance(point.ecef_on(sites.land.body()))
                    < 12.0
        }) {
            continue;
        }
        sources.push((site, point, true));
    }
    if view.eye_height_m() < 600.0 {
        sources.push((sites.current, view.at, false));
    }
    let mut desired = HashMap::new();
    let mut touching = HashSet::new();
    for (site, point, dynamic) in sources {
        let point = Geodetic::new(point.latitude, point.longitude, 0.0);
        let origin = point.ecef_on(sites.land.body());
        for rock in gearbox_globe::rocks::near(sites.land.body(), point, DRAW_M) {
            let ground = rock.ground.ecef_on(sites.land.body());
            if ground.distance(sites.home().frame.origin) < 8.0 {
                continue;
            }
            let key = (site, rock.key);
            if dynamic && ground.distance(origin) < CONTACT_M {
                touching.insert(key);
            }
            desired.insert(key, rock);
        }
    }
    state.entries.retain(|key, resident| {
        if desired.contains_key(key) {
            return true;
        }
        commands.entity(resident.entity).despawn();
        if let Some(id) = resident.collider {
            physics.remove_collider(id, true);
        }
        false
    });
    for (key, rock) in desired {
        let resident = state.entries.entry(key).or_insert_with(|| {
            let transform = local_transform(&rock, &sites.list[key.0].frame, &sites.land);
            let template = &templates.0[rock.template];
            let entity = commands
                .spawn((
                    Name::new(format!(
                        "{} boulder {}:{}",
                        sites.land.body().name(),
                        rock.key.0,
                        rock.key.1
                    )),
                    Mesh3d(template.mesh.clone()),
                    MeshMaterial3d(template.material.clone()),
                    transform,
                    ChildOf(sites.list[key.0].entity),
                ))
                .id();
            Resident {
                entity,
                collider: None,
                transform,
                template: rock.template,
                frame: sites.list[key.0].frame,
            }
        });
        if resident.frame != sites.list[key.0].frame {
            resident.frame = sites.list[key.0].frame;
            resident.transform = local_transform(&rock, &resident.frame, &sites.land);
            commands.entity(resident.entity).insert(resident.transform);
            if let Some(id) = resident.collider.take() {
                physics.remove_collider(id, true);
            }
        }
        if touching.contains(&key) && resident.collider.is_none() {
            let id = physics.insert_collider(collider(
                &templates.0[resident.template],
                &resident.transform,
                key.0,
            ));
            if let Some(id) = id {
                if let Err(error) = physics.register_wheel_ground(id, None) {
                    warn!("boulder wheel contact: {error}");
                }
                resident.collider = Some(id);
            }
        } else if !touching.contains(&key)
            && let Some(id) = resident.collider.take()
        {
            physics.remove_collider(id, true);
        }
    }
    let counts = (
        state.entries.len(),
        state
            .entries
            .values()
            .filter(|r| r.collider.is_some())
            .count(),
    );
    if counts != state.reported {
        info!(
            "planet-rocks: {} meshes, {} wheel-contact colliders",
            counts.0, counts.1
        );
        state.reported = counts;
    }
}

#[cfg(test)]
mod planet_tests {
    use super::*;

    #[test]
    fn native_boulder_rays_hit_the_visible_triangles() {
        for index in 0..16 {
            let (_, vertices, triangles) = rock_mesh(index);
            let template = Template {
                mesh: default(),
                material: default(),
                vertices,
                triangles,
            };
            let transform = Transform::from_xyz(3.0, 2.0, -4.0)
                .with_scale(Vec3::new(1.1, 0.65, 0.8))
                .with_rotation(Quat::from_rotation_y(index as f32));
            let origin = transform.translation.as_dvec3()
                + gearbox_globe::physics_offset(3)
                + DVec3::Y * 10.0;
            let direction = DVec3::NEG_Y;
            let points: Vec<_> = template
                .vertices
                .iter()
                .map(|v| {
                    transform.transform_point(v.as_vec3()).as_dvec3()
                        + gearbox_globe::physics_offset(3)
                })
                .collect();
            let expected = template
                .triangles
                .iter()
                .filter_map(|triangle| {
                    let [a, b, c] = triangle.map(|i| points[i as usize]);
                    let (e1, e2) = (b - a, c - a);
                    let p = direction.cross(e2);
                    let det = e1.dot(p);
                    if det.abs() < 1e-10 {
                        return None;
                    }
                    let t = origin - a;
                    let u = t.dot(p) / det;
                    let q = t.cross(e1);
                    let v = direction.dot(q) / det;
                    let distance = e2.dot(q) / det;
                    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && distance >= 0.0).then_some(distance)
                })
                .reduce(f64::min)
                .unwrap();
            let mut physics = PhysicsWorld::default();
            let id = physics
                .insert_collider(collider(&template, &transform, 3))
                .unwrap();
            physics.register_wheel_ground(id, None).unwrap();
            let (hit, distance) = physics
                .cast_ray(
                    glam::DVec3::from_array(origin.to_array()),
                    glam::DVec3::from_array(direction.to_array()),
                    20.0,
                )
                .unwrap();
            assert_eq!(hit, id);
            assert!(
                (distance - expected).abs() < 1e-5,
                "template {index}: {distance} != {expected}"
            );
        }
    }

    #[test]
    fn boulder_collision_vertices_match_rendered_shapes_across_sites() {
        for body in [Body::Mars, Body::Moon] {
            let (lat, lon) = body.home();
            let home = Datum::on(body, lat, lon);
            let other = home.travelled(70.0, 10_000.0);
            let land = gearbox_globe::Terrain::new(&home);
            let rock = gearbox_globe::rocks::near(body, Geodetic::new(lat, lon, 0.0), 100.0)[0];
            let a = local_transform(&rock, &home, &land);
            let b = local_transform(&rock, &other, &land);
            let (mesh, vertices, triangles) = rock_mesh(rock.template);
            assert!(!triangles.is_empty());
            let VertexAttributeValues::Float32x3(rendered) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                unreachable!()
            };
            for (vertex, rendered) in vertices.iter().zip(rendered) {
                assert!((*vertex - Vec3::from_array(*rendered).as_dvec3()).length() < 1e-7);
                let pa = a.transform_point(vertex.as_vec3()).as_dvec3();
                let pb = b.transform_point(vertex.as_vec3()).as_dvec3();
                assert!(home.to_ecef(pa).distance(other.to_ecef(pb)) < 0.003);
            }
        }
    }
}
