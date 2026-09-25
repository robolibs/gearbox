//! Receiver and emitter links of every machine on one medium: packets that
//! controllers hand to an emitter reach the receivers Molla's radio model
//! says hear them, as receiver measurements.

use gearbox_api::{Measurement, Props, measurement_kind as kind};
use molla_math::{Quat as MQuat, Real, Transform as MTransform, Vec3 as MVec3};
use molla_sensors::{RadioEndpoint, RadioKind, RadioMedium};

use crate::links::{SensorSpec, SensorVariant};
use crate::physics::backend::{BodyId, ColliderId, DVec3, PhysicsBackend};

/// A receiver or emitter link where it is this frame.
pub struct RadioLink {
    /// Bus key of the machine that carries it.
    pub machine: String,
    pub mount: usize,
    pub name: String,
    pub link_index: u32,
    pub body: BodyId,
    pub world: MTransform,
    pub endpoint: RadioEndpoint,
}

pub fn endpoint(spec: &SensorSpec) -> RadioEndpoint {
    RadioEndpoint {
        kind: match spec.variant {
            SensorVariant::Infrared => RadioKind::Infrared,
            SensorVariant::Serial => RadioKind::Serial,
            _ => RadioKind::Radio,
        },
        channel: spec.channel,
        range: spec.range_m as Real,
        aperture: spec.aperture as Real,
    }
}

/// World pose of a link mounted at `local` on `body`.
pub fn world_pose(physics: &dyn PhysicsBackend, body: BodyId, local: &MTransform) -> Option<MTransform> {
    let pose = physics.body(body)?.position();
    let (t, r) = (pose.translation, pose.rotation);
    let body = MTransform::new(MVec3::new(t.x, t.y, t.z), MQuat::from_xyzw(r.x, r.y, r.z, r.w));
    Some(body.compose(local))
}

/// Whether nothing but the two end bodies lies between `from` and `to`.
pub fn line_of_sight(physics: &dyn PhysicsBackend, ends: [BodyId; 2], from: MVec3, to: MVec3) -> bool {
    let (from, to) = (DVec3::new(from.x, from.y, from.z), DVec3::new(to.x, to.y, to.z));
    let length = from.distance(to);
    if length == 0.0 {
        return true;
    }
    let own: Vec<ColliderId> = ends
        .iter()
        .filter_map(|&body| physics.body(body))
        .flat_map(|body| body.colliders())
        .collect();
    physics
        .cast_ray_filtered(from, (to - from) / length, length, &|c| !own.contains(&c))
        .is_none()
}

/// Hands each `(emitter, payload)` of `sent` to the receivers that hear it;
/// returns receiver indices with their measurements.
pub fn exchange(
    physics: &dyn PhysicsBackend,
    emitters: &[RadioLink],
    receivers: &[RadioLink],
    sent: Vec<(usize, Vec<u8>)>,
    sim_time: f64,
) -> Vec<(usize, Measurement)> {
    let mut medium = RadioMedium::new(
        emitters.iter().map(|l| l.endpoint).collect(),
        receivers.iter().map(|l| l.endpoint).collect(),
    );
    for (emitter, data) in sent {
        medium.send(emitter, data);
    }
    let tx: Vec<MTransform> = emitters.iter().map(|l| l.world).collect();
    let rx: Vec<MTransform> = receivers.iter().map(|l| l.world).collect();
    // Infra-red line of sight between the end bodies at the two positions.
    let body_at = |links: &[RadioLink], at: MVec3| links.iter().find(|l| l.world.position == at).map(|l| l.body);
    let mut clear = |from: MVec3, to: MVec3| match (body_at(emitters, from), body_at(receivers, to)) {
        (Some(a), Some(b)) => line_of_sight(physics, [a, b], from, to),
        _ => false,
    };
    medium
        .deliver(&tx, &rx, &mut clear)
        .into_iter()
        .map(|d| {
            let (to, from) = (&receivers[d.receiver], &emitters[d.emitter]);
            let origin = format!("{}/{}", from.machine, from.name);
            let v = d.direction;
            let measurement = Measurement {
                sim_time_s: sim_time,
                link_index: to.link_index,
                kind: kind::RECEIVER,
                count: 1,
                values: vec![d.channel as f64, d.signal_strength, v.x, v.y, v.z],
                data: d.data,
                props: Props::from_pairs(&[("name", to.name.as_str()), ("from", origin.as_str())]).into_bytes(),
                ..Default::default()
            };
            (d.receiver, measurement)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::PhysicsWorld;
    use crate::physics::backend::{BodyDesc, ColliderDesc, Pose, Shape};
    use crate::physics::molla::MollaBackend;

    fn link(world: &PhysicsWorld, name: &str, body: BodyId, endpoint: RadioEndpoint) -> RadioLink {
        RadioLink {
            machine: "m".into(),
            mount: 0,
            name: name.into(),
            link_index: 1,
            body,
            world: world_pose(&*world.backend, body, &MTransform::IDENTITY).unwrap(),
            endpoint,
        }
    }

    /// Emitter at the origin facing +X; receivers 5 m ahead in the open,
    /// behind a wall, and out of radio range.
    #[test]
    fn packets_reach_receivers_that_hear_them() {
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let at = |world: &mut PhysicsWorld, x: f64, z: f64, half: f64| {
            let body = world.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::new(x, 1.0, z))));
            world
                .insert_collider(ColliderDesc::new(Shape::Ball { radius: half }).parent(body))
                .unwrap();
            body
        };
        let tx = at(&mut world, 0.0, 0.0, 0.2);
        let open = at(&mut world, 5.0, 0.0, 0.2);
        let hidden = at(&mut world, 5.0, 4.0, 0.2);
        let far = at(&mut world, 40.0, 0.0, 0.2);
        let wall = world.insert_body(BodyDesc::fixed());
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(0.1, 2.0, 1.0),
                })
                .parent(wall)
                .translation(DVec3::new(2.5, 1.0, 2.0)),
            )
            .unwrap();
        let radio = RadioEndpoint {
            range: 20.0,
            ..Default::default()
        };
        let infrared = RadioEndpoint {
            kind: RadioKind::Infrared,
            aperture: std::f64::consts::TAU,
            ..radio
        };
        for (endpoint, heard) in [(radio, vec!["open", "hidden"]), (infrared, vec!["open"])] {
            let emitters = [link(&world, "tx", tx, endpoint)];
            let receivers = [
                link(&world, "open", open, endpoint),
                link(&world, "hidden", hidden, endpoint),
                link(&world, "far", far, endpoint),
            ];
            let out = exchange(&*world.backend, &emitters, &receivers, vec![(0, b"hi".to_vec())], 2.0);
            let names: Vec<String> = out.iter().map(|(_, m)| m.name()).collect();
            assert_eq!(names, heard, "{:?}", endpoint.kind);
            let (_, first) = &out[0];
            assert_eq!((first.kind, first.data.as_slice()), (kind::RECEIVER, &b"hi"[..]));
            assert_eq!(first.props().get("from").as_deref(), Some("m/tx"));
            assert!((first.values[1] - 1.0 / 25.0).abs() < 1e-9 && first.values[2] < -0.99);
        }
    }
}
