//! Runtime attachments (`specs/TOOLS_SPEC.md`): a slave's coupler hangs on
//! a master's hitch through a rapier joint shaped by the coupling type. The
//! master's agent answers the requests, the slave's agent refuses commands
//! while attached, and the master's `/links` grows the slave's tree.

use std::collections::HashMap;

use crate::physics::PhysicsWorld;
use bevy::prelude::*;
use gearbox_api::{
    AttachRequest, DetachRequest, GearboxBus, LinkDesc, SceneEvent, Status, ToolDesc, code,
    event_kind,
};
use peerbus::ReqReplyToken;
use rapier3d::math::{Rotation as DQuat, Vector as DVec3};
use rapier3d::prelude::{
    ColliderHandle, GenericJoint, GenericJointBuilder, Group, ImpulseJointHandle, JointAxesMask,
    JointAxis, MultibodyJointHandle, Pose, RigidBodyHandle,
};
use usd_bevy::UsdPrimRef;

use crate::controller::{
    ControllerInventory, MachineAgentKeys, MachineInstanceSpec, find_prim_entity, link_descs,
};
use crate::links::{CouplingSide, LinkSpec, LinkTree};

/// Without teleport the coupler must already be this close to the hitch.
const SNAP_DISTANCE_M: f64 = 0.25;
const SNAP_ANGLE_RAD: f64 = 30.0_f64.to_radians();

#[derive(Debug, Clone)]
pub struct Attachment {
    pub master_ns: String,
    pub slave_ns: String,
    pub hitch: String,
    pub hitch_link: String,
    pub coupler: String,
    pub kind: String,
    pub joint: HitchJoint,
    pub slave_mass_kg: f64,
    pub controlled: bool,
    /// Slave requests the master does not grant (`TOOLS_SPEC.md` §5.3).
    pub denied: Vec<String>,
    /// The slave's parking stand (`gearbox:coupling:stand` on the coupler),
    /// hidden and without collision while hitched.
    pub stand: Option<String>,
}

#[derive(Resource, Default)]
pub struct Attachments(pub Vec<Attachment>);

/// Extra mass each master tows, by machine id, for the engine force law.
#[derive(Resource, Default)]
pub struct TowedMass(pub HashMap<String, f64>);

/// An attachment authored in a world layer, waiting for both machines to
/// have agents before it is applied like a runtime attach with teleport.
#[derive(Debug, Clone)]
pub struct StaticAttachment {
    pub scene_root: Entity,
    pub hitch_prim: String,
    pub coupler_prim: String,
    pub frames_waited: u32,
}

#[derive(Resource, Default)]
pub struct PendingStaticAttachments(pub Vec<StaticAttachment>);

const STATIC_ATTACH_MAX_FRAMES: u32 = 1200;

pub struct AttachPlugin;

impl Plugin for AttachPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Attachments>()
            .init_resource::<TowedMass>()
            .init_resource::<PendingStaticAttachments>()
            .add_systems(Update, serve_attachments);
    }
}

struct Frame {
    translation: DVec3,
    rotation: DQuat,
}

impl Frame {
    fn inverse(&self) -> Self {
        let rotation = self.rotation.inverse();
        Self {
            translation: -(rotation * self.translation),
            rotation,
        }
    }

    fn then(&self, other: &Frame) -> Self {
        Self {
            translation: self.translation + self.rotation * other.translation,
            rotation: self.rotation * other.rotation,
        }
    }

    fn pose(&self) -> Pose {
        Pose {
            translation: self.translation,
            rotation: self.rotation,
        }
    }

    fn from_pose(p: &Pose) -> Self {
        Self {
            translation: p.translation,
            rotation: p.rotation,
        }
    }

    fn from_global(gt: &GlobalTransform) -> Self {
        let (_, rot, tr) = gt.to_scale_rotation_translation();
        Self {
            translation: DVec3::new(tr.x as f64, tr.y as f64, tr.z as f64),
            rotation: DQuat::from_xyzw(rot.x as f64, rot.y as f64, rot.z as f64, rot.w as f64),
        }
    }
}

struct Scene<'w, 's> {
    inventory: &'w ControllerInventory,
    keys: &'w MachineAgentKeys,
    prims: &'w Query<'w, 's, (Entity, &'static UsdPrimRef)>,
    parents: &'w Query<'w, 's, &'static ChildOf>,
    transforms: &'w Query<'w, 's, &'static GlobalTransform>,
}

impl Scene<'_, '_> {
    fn machine(&self, ns: &str) -> Option<&MachineInstanceSpec> {
        let key = self.keys.0.get(ns)?;
        self.inventory
            .machines
            .iter()
            .find(|m| m.scene_root == Some(key.scene_root) && m.id == key.machine_id)
    }

    fn entity(&self, machine: &MachineInstanceSpec, prim: &str) -> Option<Entity> {
        find_prim_entity(machine.scene_root?, prim, self.prims, self.parents)
    }

    fn frame(&self, machine: &MachineInstanceSpec, prim: &str) -> Option<Frame> {
        let entity = self.entity(machine, prim)?;
        self.transforms.get(entity).ok().map(Frame::from_global)
    }

    fn body(
        &self,
        machine: &MachineInstanceSpec,
        prim: &str,
        physics: &PhysicsWorld,
    ) -> Option<RigidBodyHandle> {
        let entity = self.entity(machine, prim)?;
        physics.entity_to_body.get(&entity).copied()
    }

    fn bodies(
        &self,
        machine: &MachineInstanceSpec,
        physics: &PhysicsWorld,
    ) -> Vec<RigidBodyHandle> {
        machine
            .links
            .links
            .iter()
            .filter_map(|l| l.body_prim.as_deref())
            .filter_map(|p| self.body(machine, p, physics))
            .collect()
    }
}

/// The rigid-body link a (possibly body-less) link rides on.
fn body_link<'a>(tree: &'a LinkTree, link: &'a LinkSpec) -> Option<&'a LinkSpec> {
    let mut cur = link;
    for _ in 0..64 {
        if cur.body_prim.is_some() {
            return Some(cur);
        }
        cur = tree.get(cur.parent.as_deref()?)?;
    }
    None
}

fn refused(message: impl Into<String>) -> Status {
    Status::err(code::REFUSED, &message.into())
}

fn not_found(message: impl Into<String>) -> Status {
    Status::err(code::NOT_FOUND, &message.into())
}

/// Pick the coupling a request means: by name, else the only free one of a
/// matching type.
fn pick_coupling<'a>(
    tree: &'a LinkTree,
    side: CouplingSide,
    name: Option<&str>,
    kind: Option<&str>,
    occupied: &dyn Fn(&str) -> bool,
    what: &str,
) -> Result<&'a LinkSpec, Status> {
    let mut candidates: Vec<&LinkSpec> = tree
        .couplings()
        .filter(|(_, c)| c.side == side)
        .filter(|(_, c)| name.is_none_or(|n| n == c.name))
        .filter(|(_, c)| kind.is_none_or(|k| k == c.kind))
        .filter(|(_, c)| name.is_some() || !occupied(&c.name))
        .map(|(l, _)| l)
        .collect();
    if let (Some(n), true) = (name, candidates.is_empty()) {
        let all: Vec<String> = tree
            .couplings()
            .filter(|(_, c)| c.side == side)
            .map(|(_, c)| format!("{} ({})", c.name, c.kind))
            .collect();
        return Err(not_found(format!(
            "no {what} named `{n}`; {what}s: {}",
            if all.is_empty() {
                "none".to_string()
            } else {
                all.join(", ")
            }
        )));
    }
    match candidates.len() {
        0 => Err(Status::err(
            code::UNSUPPORTED,
            &format!("no free {what} of a matching type"),
        )),
        1 => Ok(candidates.remove(0)),
        _ => Err(Status::err(
            code::USAGE,
            &format!(
                "several {what}s match, name one: {}",
                candidates
                    .iter()
                    .filter_map(|l| l.coupling.as_ref())
                    .map(|c| format!("{} ({})", c.name, c.kind))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

fn joint_for(kind: &str, frame1: Pose, frame2: Pose) -> GenericJoint {
    let lin = JointAxesMask::LIN_X | JointAxesMask::LIN_Y | JointAxesMask::LIN_Z;
    let deg = |d: f64| d.to_radians();
    // Coupling frames are prim frames on bodies that keep the USD basis:
    // X right, Y back, Z up. So yaw is about Z, pitch about X, roll about Y.
    let (mask, limits): (JointAxesMask, Vec<(JointAxis, f64)>) = match kind {
        "three_point_mounted" | "chassis_mounted" | "loader_carriage" => (
            lin | JointAxesMask::ANG_X | JointAxesMask::ANG_Y | JointAxesMask::ANG_Z,
            vec![],
        ),
        // Pinned at the eye, pitch and yaw free, roll locked: the tractor
        // carries the trailer's nose.
        "drawbar" => (lin | JointAxesMask::ANG_Y, vec![]),
        "clevis" => (
            lin | JointAxesMask::ANG_Y,
            vec![(JointAxis::AngX, deg(20.0))],
        ),
        "piton" | "fifth_wheel" => (
            lin | JointAxesMask::ANG_Y,
            vec![(JointAxis::AngX, deg(15.0))],
        ),
        "pivot_wagon" | "three_point_semi_mounted" => {
            (lin | JointAxesMask::ANG_X | JointAxesMask::ANG_Y, vec![])
        }
        "hitch_hook" => (
            lin,
            vec![(JointAxis::AngX, deg(25.0)), (JointAxis::AngY, deg(25.0))],
        ),
        "cuna" => (
            lin,
            vec![(JointAxis::AngX, deg(20.0)), (JointAxis::AngY, deg(20.0))],
        ),
        "ball" => (
            lin,
            vec![(JointAxis::AngX, deg(30.0)), (JointAxis::AngY, deg(30.0))],
        ),
        _ => (lin, vec![]),
    };
    let mut b = GenericJointBuilder::new(mask)
        .local_frame1(frame1)
        .local_frame2(frame2)
        .contacts_enabled(false);
    for (axis, limit) in limits {
        b = b.limits(axis, [-limit, limit]);
    }
    b.build()
}

/// A hitched pair is one vehicle: the slave's drawbar runs through the
/// master's hitch parts and wheels, so every body of one stops colliding
/// with every body of the other (and starts again on detach). Each machine
/// owns one collision group bit, so the other side's memberships are
/// masked out of each collider's filter.
fn set_cross_collisions(
    physics: &mut PhysicsWorld,
    a: &[RigidBodyHandle],
    b: &[RigidBodyHandle],
    enabled: bool,
) {
    fn colliders_of(physics: &PhysicsWorld, bodies: &[RigidBodyHandle]) -> Vec<ColliderHandle> {
        bodies
            .iter()
            .filter_map(|h| physics.bodies.get(*h))
            .flat_map(|b| b.colliders().iter().copied())
            .collect()
    }
    fn memberships(physics: &PhysicsWorld, handles: &[ColliderHandle]) -> Group {
        handles
            .iter()
            .filter_map(|h| physics.colliders.get(*h))
            .fold(Group::NONE, |acc, c| acc | c.collision_groups().memberships)
    }
    let (ca, cb) = (colliders_of(physics, a), colliders_of(physics, b));
    // Machines that are not articulations sit in every group; give each
    // one its own bit first, or there is nothing to mask out.
    let mut taken = Group::NONE;
    for (handles, bodies) in [(&ca, a), (&cb, b)] {
        let current = memberships(physics, handles);
        if current != Group::ALL {
            taken |= current;
            continue;
        }
        let seed = bodies.first().map_or(0, |h| h.into_raw_parts().0);
        let mut bit = Group::from_bits_truncate(1 << ((seed % 31) + 1));
        while taken.contains(bit) {
            bit = Group::from_bits_truncate((bit.bits() << 1).max(2) & !1);
            if bit == Group::NONE {
                bit = Group::GROUP_2;
            }
        }
        taken |= bit;
        for h in handles.iter() {
            if let Some(c) = physics.colliders.get_mut(*h) {
                let mut groups = c.collision_groups();
                groups.memberships = bit;
                c.set_collision_groups(groups);
            }
        }
    }
    let (ma, mb) = (memberships(physics, &ca), memberships(physics, &cb));
    if ma == mb {
        return;
    }
    for (handles, other) in [(&ca, mb), (&cb, ma)] {
        for h in handles {
            if let Some(c) = physics.colliders.get_mut(*h) {
                let mut groups = c.collision_groups();
                groups.filter = if enabled { groups.filter | other } else { groups.filter.difference(other) };
                c.set_collision_groups(groups);
            }
        }
    }
}

/// The physical hitch. An impulse joint: merging the two Featherstone trees
/// with a multibody joint blows up (NaN poses) and rapier only implements a
/// few joint shapes there anyway.
#[derive(Debug, Clone, Copy)]
pub enum HitchJoint {
    Multibody(MultibodyJointHandle),
    Impulse(ImpulseJointHandle),
}

fn insert_hitch_joint(
    physics: &mut PhysicsWorld,
    hitch_body: RigidBodyHandle,
    coupler_body: RigidBodyHandle,
    joint: GenericJoint,
) -> HitchJoint {
    HitchJoint::Impulse(physics.impulse_joints.insert(hitch_body, coupler_body, joint, true))
}

fn remove_hitch_joint(physics: &mut PhysicsWorld, joint: HitchJoint) {
    match joint {
        HitchJoint::Multibody(handle) => {
            physics.multibody_joints.remove(handle, true);
        }
        HitchJoint::Impulse(handle) => {
            physics.impulse_joints.remove(handle, true);
        }
    }
}

/// The prim a slave's coupler names as its parking stand.
fn stand_of(
    scene: &Scene,
    instances: Option<&usd_bevy::instance::UsdInstances>,
    slave_ns: &str,
    coupler: &str,
) -> Option<String> {
    let slave = scene.machine(slave_ns)?;
    let stage = instances?.stage(slave.scene_root?)?;
    let link = slave
        .links
        .links
        .iter()
        .find(|l| l.coupling.as_ref().is_some_and(|c| c.name == coupler))?;
    let prim = openusd::sdf::path(&link.prim_path).ok()?;
    crate::controller::read_rel_first(stage, &prim, "gearbox:coupling:stand")
}

/// Hitched, a slave selects its `coupling` variant `hitched` (the stand
/// disappears) and the stand stops colliding; parked reverses both.
fn set_stand(
    scene: &Scene,
    physics: &mut PhysicsWorld,
    overrides: &mut Query<&mut usd_bevy::instance::UsdInstanceOverrides>,
    slave_ns: &str,
    stand: &str,
    hitched: bool,
) {
    let Some(slave) = scene.machine(slave_ns) else {
        return;
    };
    if let Some(body) = scene.body(slave, stand, physics) {
        let colliders: Vec<_> = physics
            .bodies
            .get(body)
            .map(|b| b.colliders().to_vec())
            .unwrap_or_default();
        for handle in colliders {
            if let Some(c) = physics.colliders.get_mut(handle) {
                c.set_enabled(!hitched);
            }
        }
    }
    let Some(root) = slave.scene_root else {
        return;
    };
    let option = if hitched { "hitched" } else { "parked" };
    info!("gearbox-attach: `{slave_ns}` stand {stand} {option}");
    if let Ok(mut o) = overrides.get_mut(root) {
        o.variants.retain(|(prim, set, _)| !(prim == &slave.prim_path && set == "coupling"));
        o.variants.push((slave.prim_path.clone(), "coupling".to_string(), option.to_string()));
    }
}

/// Does `candidate_master` already hang, directly or through others, below
/// `slave`? Attaching would then close a loop.
fn would_loop(attachments: &[Attachment], candidate_master: &str, slave: &str) -> bool {
    let mut cur = candidate_master.to_string();
    for _ in 0..64 {
        if cur == slave {
            return true;
        }
        match attachments.iter().find(|a| a.slave_ns == cur) {
            Some(a) => cur = a.master_ns.clone(),
            None => return false,
        }
    }
    true
}

struct Attached {
    attachment: Attachment,
    event: SceneEvent,
}

fn try_attach(
    master_ns: &str,
    req: &AttachRequest,
    scene: &Scene,
    bus: &GearboxBus,
    physics: &mut PhysicsWorld,
    attachments: &[Attachment],
) -> Result<Attached, Status> {
    let slave_ns = req.slave();
    if slave_ns.is_empty() {
        return Err(Status::err(code::USAGE, "attach needs a `slave` namespace"));
    }
    if slave_ns == master_ns {
        return Err(refused("a machine cannot attach to itself"));
    }
    let master_agent = bus
        .machines
        .get(master_ns)
        .ok_or_else(|| not_found(format!("no machine `{master_ns}`")))?;
    if master_agent.session_id() != req.session {
        return Err(refused(format!(
            "session {} does not hold `{master_ns}`",
            req.session
        )));
    }
    let slave_agent = bus
        .machines
        .get(&slave_ns)
        .ok_or_else(|| not_found(format!("no machine `{slave_ns}`")))?;
    if let Some(already) = slave_agent.attached_to() {
        return Err(refused(format!(
            "`{slave_ns}` is already attached to `{already}`"
        )));
    }
    if would_loop(attachments, master_ns, &slave_ns) {
        return Err(refused(format!(
            "`{master_ns}` already hangs below `{slave_ns}`; attaching would close a loop"
        )));
    }
    let master = scene
        .machine(master_ns)
        .ok_or_else(|| not_found(format!("`{master_ns}` has no loaded machine")))?;
    let slave = scene
        .machine(&slave_ns)
        .ok_or_else(|| not_found(format!("`{slave_ns}` has no loaded machine")))?;

    let hitch_busy = |name: &str| {
        attachments
            .iter()
            .any(|a| a.master_ns == master_ns && a.hitch == name)
    };
    let coupler_kind_hint = req.coupler().and_then(|c| {
        slave
            .links
            .couplings()
            .find(|(_, k)| k.name == c)
            .map(|(_, k)| k.kind.clone())
    });
    let hitch_link = pick_coupling(
        &master.links,
        CouplingSide::Hitch,
        req.hitch().as_deref(),
        coupler_kind_hint.as_deref(),
        &hitch_busy,
        "hitch",
    )?;
    let hitch = hitch_link.coupling.as_ref().expect("coupling link");
    if hitch_busy(&hitch.name) {
        return Err(Status::with(
            code::BUSY,
            &[("message", "hitch is occupied"), ("hitch", &hitch.name)],
        ));
    }
    let coupler_link = pick_coupling(
        &slave.links,
        CouplingSide::Coupler,
        req.coupler().as_deref(),
        Some(&hitch.kind),
        &|_| false,
        "coupler",
    )?;
    let coupler = coupler_link.coupling.as_ref().expect("coupling link");
    if coupler.kind != hitch.kind {
        return Err(refused(format!(
            "hitch `{}` is {} but coupler `{}` is {}",
            hitch.name, hitch.kind, coupler.name, coupler.kind
        )));
    }

    let hitch_body_link = body_link(&master.links, hitch_link)
        .ok_or_else(|| refused(format!("hitch `{}` rides on no rigid body", hitch.name)))?;
    let coupler_body_link = body_link(&slave.links, coupler_link)
        .ok_or_else(|| refused(format!("coupler `{}` rides on no rigid body", coupler.name)))?;
    let hitch_body = scene
        .body(
            master,
            hitch_body_link.body_prim.as_deref().unwrap_or_default(),
            physics,
        )
        .ok_or_else(|| refused("hitch body has no physics yet"))?;
    let coupler_body = scene
        .body(
            slave,
            coupler_body_link.body_prim.as_deref().unwrap_or_default(),
            physics,
        )
        .ok_or_else(|| refused("coupler body has no physics yet"))?;
    let hitch_world = scene
        .frame(master, &hitch_link.prim_path)
        .ok_or_else(|| refused("hitch prim has no transform yet"))?;
    let coupler_world = scene
        .frame(slave, &coupler_link.prim_path)
        .ok_or_else(|| refused("coupler prim has no transform yet"))?;
    let hitch_body_world = Frame::from_pose(
        physics
            .bodies
            .get(hitch_body)
            .ok_or_else(|| refused("hitch body vanished"))?
            .position(),
    );
    let coupler_body_world = Frame::from_pose(
        physics
            .bodies
            .get(coupler_body)
            .ok_or_else(|| refused("coupler body vanished"))?
            .position(),
    );

    // Only the prims' positions count. Every body keeps the USD basis (X
    // right, Y back, Z up), so aligning the two bodies puts the slave behind
    // the master facing the same way; an authored prim rotation would tip
    // the whole slave over on teleport.
    let hitch_world = Frame { translation: hitch_world.translation, rotation: hitch_body_world.rotation };
    let coupler_world =
        Frame { translation: coupler_world.translation, rotation: coupler_body_world.rotation };
    let target = Frame { translation: hitch_world.translation, rotation: hitch_body_world.rotation };
    let frame1 = hitch_body_world.inverse().then(&hitch_world);
    let frame2 = coupler_body_world.inverse().then(&coupler_world);

    let slave_bodies = scene.bodies(slave, physics);
    if req.teleport != 0 {
        let delta = target.then(&coupler_world.inverse());
        for handle in &slave_bodies {
            if let Some(body) = physics.bodies.get_mut(*handle) {
                let cur = Frame::from_pose(body.position());
                let moved = delta.then(&cur);
                body.set_position(moved.pose(), true);
                body.set_linvel(DVec3::ZERO, true);
                body.set_angvel(DVec3::ZERO, true);
            }
        }
    } else {
        let gap = (coupler_world.translation - target.translation).length();
        let turn = coupler_world.rotation.angle_between(target.rotation);
        if gap > SNAP_DISTANCE_M || turn > SNAP_ANGLE_RAD {
            return Err(refused(format!(
                "coupler is {gap:.2} m and {:.0}° from the hitch; move closer or pass teleport",
                turn.to_degrees()
            )));
        }
    }

    let slave_mass_kg: f64 = slave_bodies
        .iter()
        .filter_map(|h| physics.bodies.get(*h))
        .map(|b| b.mass())
        .sum();
    let joint = joint_for(&hitch.kind, frame1.pose(), frame2.pose());
    let handle = insert_hitch_joint(physics, hitch_body, coupler_body, joint);
    let master_bodies = scene.bodies(master, physics);
    set_cross_collisions(physics, &master_bodies, &slave_bodies, false);

    let mut denied: Vec<String> = slave
        .controllers
        .iter()
        .flat_map(|c| c.requests.iter())
        .filter(|r| !master.grants.contains(*r))
        .cloned()
        .collect();
    denied.sort();
    denied.dedup();
    if !denied.is_empty() {
        warn!(
            "gearbox-attach: `{master_ns}` does not grant `{slave_ns}` these requests: {}",
            denied.join(", ")
        );
    }
    let event = SceneEvent::new(event_kind::ATTACHED, &slave_ns)
        .with_prop("master", master_ns)
        .with_prop("slave", &slave_ns)
        .with_prop("hitch", &hitch.name)
        .with_prop("coupler", &coupler.name)
        .with_prop("type", &hitch.kind)
        .with_prop("denied", &denied.join(","));
    Ok(Attached {
        attachment: Attachment {
            master_ns: master_ns.to_string(),
            slave_ns,
            hitch: hitch.name.clone(),
            hitch_link: hitch_link.name.clone(),
            coupler: coupler.name.clone(),
            kind: hitch.kind.clone(),
            joint: handle,
            slave_mass_kg,
            controlled: !slave.controllers.is_empty(),
            denied,
            stand: None,
        },
        event,
    })
}

/// The master's composite: attachments depth-first and every slave's links
/// re-parented under the hitch link, names prefixed with the slave path.
fn composite(
    master_ns: &str,
    attachments: &[Attachment],
    scene: &Scene,
) -> (Vec<ToolDesc>, Vec<LinkDesc>) {
    let mut tools = Vec::new();
    let mut links = Vec::new();
    fn walk(
        owner: &str,
        prefix: &str,
        depth: u32,
        attachments: &[Attachment],
        scene: &Scene,
        tools: &mut Vec<ToolDesc>,
        links: &mut Vec<LinkDesc>,
    ) {
        if depth > 16 {
            return;
        }
        for a in attachments.iter().filter(|a| a.master_ns == owner) {
            tools.push(ToolDesc {
                slave: a.slave_ns.clone(),
                hitch: a.hitch.clone(),
                coupler: a.coupler.clone(),
                kind: a.kind.clone(),
                controlled: a.controlled,
                depth,
                denied: a.denied.join(","),
            });
            let slave_prefix = format!("{prefix}{}/", a.slave_ns);
            let hitch_parent = format!("{prefix}{}", a.hitch_link);
            if let Some(slave) = scene.machine(&a.slave_ns) {
                for mut l in link_descs(&slave.links) {
                    l.parent = Some(match &l.parent {
                        None => hitch_parent.clone(),
                        Some(p) => format!("{slave_prefix}{p}"),
                    });
                    if l.role == "base" {
                        l.role = "link".to_string();
                    }
                    l.name = format!("{slave_prefix}{}", l.name);
                    links.push(l);
                }
            }
            walk(
                &a.slave_ns,
                &slave_prefix,
                depth + 1,
                attachments,
                scene,
                tools,
                links,
            );
        }
    }
    walk(master_ns, "", 0, attachments, scene, &mut tools, &mut links);
    (tools, links)
}

fn refresh_masters(bus: &mut GearboxBus, attachments: &[Attachment], scene: &Scene) {
    let masters: Vec<String> = bus.machines.keys().cloned().collect();
    for ns in masters {
        let (tools, links) = composite(&ns, attachments, scene);
        if let Some(agent) = bus.machines.get_mut(&ns) {
            agent.set_tools(tools, links);
        }
    }
}

fn recompute_towed(attachments: &[Attachment], scene: &Scene, towed: &mut TowedMass) {
    towed.0.clear();
    for a in attachments {
        // Everything below a master, however deep, weighs on its engine.
        let mut cur = a.master_ns.clone();
        for _ in 0..64 {
            if let Some(m) = scene.machine(&cur) {
                *towed.0.entry(m.id.clone()).or_default() += a.slave_mass_kg;
            }
            match attachments.iter().find(|x| x.slave_ns == cur) {
                Some(up) => cur = up.master_ns.clone(),
                None => break,
            }
        }
    }
}

pub(crate) fn serve_attachments(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    mut attachments: ResMut<Attachments>,
    mut towed: ResMut<TowedMass>,
    mut physics: ResMut<PhysicsWorld>,
    mut pending_static: ResMut<PendingStaticAttachments>,
    prims: Query<(Entity, &'static UsdPrimRef)>,
    parents: Query<&'static ChildOf>,
    transforms: Query<&'static GlobalTransform>,
    instances: Option<NonSend<usd_bevy::instance::UsdInstances>>,
    mut overrides: Query<&mut usd_bevy::instance::UsdInstanceOverrides>,
) {
    let Some(mut bus) = bus else { return };
    let scene = Scene {
        inventory: &inventory,
        keys: &keys,
        prims: &prims,
        parents: &parents,
        transforms: &transforms,
    };

    // Machines that left the scene take their attachments with them.
    let before = attachments.0.len();
    let mut dropped = Vec::new();
    attachments.0.retain(|a| {
        let alive =
            bus.machines.contains_key(&a.master_ns) && bus.machines.contains_key(&a.slave_ns);
        if !alive {
            dropped.push(a.clone());
        }
        alive
    });
    for a in dropped {
        remove_hitch_joint(physics.as_mut(), a.joint);
        if let Some(slave) = bus.machines.get_mut(&a.slave_ns) {
            slave.set_attached_to(None);
        }
    }
    let mut changed = attachments.0.len() != before;

    let mut attach_reqs: Vec<(String, AttachRequest, Option<ReqReplyToken>)> = Vec::new();
    let mut detach_reqs: Vec<(String, DetachRequest, ReqReplyToken)> = Vec::new();
    for (ns, agent) in bus.machines.iter_mut() {
        for (req, token) in agent.pending_attach.drain(..) {
            attach_reqs.push((ns.clone(), req, Some(token)));
        }
        for (req, token) in agent.pending_detach.drain(..) {
            detach_reqs.push((ns.clone(), req, token));
        }
    }

    // World-layer attachments join once both machines have agents.
    let mut still_pending = Vec::new();
    for mut sa in pending_static.0.drain(..) {
        let owner = |prim: &str| {
            inventory
                .machines
                .iter()
                .filter(|m| m.scene_root == Some(sa.scene_root))
                .find(|m| prim == m.prim_path || prim.starts_with(&format!("{}/", m.prim_path)))
        };
        let ns_of = |m: &MachineInstanceSpec| {
            keys.0
                .iter()
                .find(|(_, k)| Some(k.scene_root) == m.scene_root && k.machine_id == m.id)
                .map(|(ns, _)| ns.clone())
        };
        let ready = match (owner(&sa.hitch_prim), owner(&sa.coupler_prim)) {
            (Some(master), Some(slave)) => match (ns_of(master), ns_of(slave)) {
                (Some(master_ns), Some(slave_ns))
                    if bus.machines.contains_key(&master_ns)
                        && bus.machines.contains_key(&slave_ns) =>
                {
                    let hitch = master
                        .links
                        .by_prim(&sa.hitch_prim)
                        .and_then(|l| l.coupling.as_ref())
                        .map(|c| c.name.clone());
                    let coupler = slave
                        .links
                        .by_prim(&sa.coupler_prim)
                        .and_then(|l| l.coupling.as_ref())
                        .map(|c| c.name.clone());
                    match (hitch, coupler) {
                        (Some(h), Some(c)) => {
                            let req = AttachRequest::new(0, &slave_ns)
                                .with_hitch(&h)
                                .with_coupler(&c)
                                .teleporting();
                            attach_reqs.push((master_ns, req, None));
                            true
                        }
                        _ => {
                            warn!(
                                "gearbox-attach: static attachment {} -> {} names prims that are not couplings",
                                sa.hitch_prim, sa.coupler_prim
                            );
                            true
                        }
                    }
                }
                _ => false,
            },
            _ => false,
        };
        if !ready {
            sa.frames_waited += 1;
            if sa.frames_waited > STATIC_ATTACH_MAX_FRAMES {
                warn!(
                    "gearbox-attach: static attachment {} -> {} never found both machines",
                    sa.hitch_prim, sa.coupler_prim
                );
            } else {
                still_pending.push(sa);
            }
        }
    }
    pending_static.0 = still_pending;

    for (master_ns, req, token) in attach_reqs {
        let outcome = try_attach(&master_ns, &req, &scene, &bus, &mut physics, &attachments.0);
        let status = match outcome {
            Ok(mut done) => {
                let slave_ns = done.attachment.slave_ns.clone();
                done.attachment.stand =
                    stand_of(&scene, instances.as_deref(), &slave_ns, &done.attachment.coupler);
                if let Some(stand) = done.attachment.stand.clone() {
                    set_stand(&scene, physics.as_mut(), &mut overrides, &slave_ns, &stand, true);
                }
                info!(
                    "gearbox-attach: `{slave_ns}` on `{master_ns}` via {} / {} ({})",
                    done.attachment.hitch, done.attachment.coupler, done.attachment.kind
                );
                attachments.0.push(done.attachment);
                if let Some(slave) = bus.machines.get_mut(&slave_ns) {
                    slave.set_attached_to(Some(master_ns.clone()));
                }
                bus.publish_event(done.event);
                changed = true;
                Status::ok_with(&gearbox_api::Props::from_pairs(&[(
                    "slave",
                    slave_ns.as_str(),
                )]))
            }
            Err(status) => {
                warn!(
                    "gearbox-attach: `{master_ns}` refused attach of `{}`: {}",
                    req.slave(),
                    status.message()
                );
                status
            }
        };
        if let (Some(token), Some(agent)) = (token, bus.machines.get_mut(&master_ns)) {
            agent.respond_attach(token, &status);
        }
    }

    for (master_ns, req, token) in detach_reqs {
        let slave_ns = req.slave();
        let status = match bus.machines.get(&master_ns) {
            Some(agent) if agent.session_id() != req.session => refused(format!(
                "session {} does not hold `{master_ns}`",
                req.session
            )),
            _ => match attachments
                .0
                .iter()
                .position(|a| a.master_ns == master_ns && a.slave_ns == slave_ns)
            {
                None => not_found(format!("`{slave_ns}` is not attached to `{master_ns}`")),
                Some(i) => {
                    let a = attachments.0.remove(i);
                    remove_hitch_joint(physics.as_mut(), a.joint);
                    if let (Some(m), Some(s)) = (scene.machine(&master_ns), scene.machine(&slave_ns)) {
                        let (mb, sb) = (scene.bodies(m, &physics), scene.bodies(s, &physics));
                        set_cross_collisions(physics.as_mut(), &mb, &sb, true);
                    }
                    if let Some(stand) = a.stand.as_deref() {
                        set_stand(&scene, physics.as_mut(), &mut overrides, &slave_ns, stand, false);
                    }
                    if let Some(slave) = bus.machines.get_mut(&slave_ns) {
                        slave.set_attached_to(None);
                    }
                    info!("gearbox-attach: `{slave_ns}` detached from `{master_ns}`");
                    bus.publish_event(
                        SceneEvent::new(event_kind::DETACHED, &slave_ns)
                            .with_prop("master", &master_ns)
                            .with_prop("slave", &slave_ns)
                            .with_prop("hitch", &a.hitch)
                            .with_prop("coupler", &a.coupler),
                    );
                    changed = true;
                    Status::ok()
                }
            },
        };
        if let Some(agent) = bus.machines.get_mut(&master_ns) {
            agent.respond_detach(token, &status);
        }
    }

    if changed {
        refresh_masters(&mut bus, &attachments.0, &scene);
        recompute_towed(&attachments.0, &scene, &mut towed);
    }

    // Commands aimed at a slave through its master land in the slave's queue.
    let masters: Vec<String> = bus.machines.keys().cloned().collect();
    for ns in masters {
        let routed: Vec<(String, gearbox_api::ControllerCommand)> = {
            let Some(agent) = bus.machines.get_mut(&ns) else {
                continue;
            };
            let mut keep = Vec::new();
            let mut routed = Vec::new();
            for cmd in agent.commands.drain(..) {
                match cmd.props().get("tool") {
                    Some(tool) => routed.push((tool, cmd)),
                    None => keep.push(cmd),
                }
            }
            agent.commands = keep;
            routed
        };
        for (tool, cmd) in routed {
            let leaf = tool.rsplit('/').next().unwrap_or(&tool).to_string();
            let reachable = attachments.0.iter().any(|a| a.slave_ns == leaf);
            if let (true, Some(slave)) = (reachable, bus.machines.get_mut(&leaf)) {
                // The slave sees the command as its own; the route is spent.
                let mut props = gearbox_api::Props::new();
                for (k, v) in cmd.props().iter() {
                    if k != "tool" {
                        props.set(&k, &v);
                    }
                }
                slave.commands.push(gearbox_api::ControllerCommand {
                    props: props.into_bytes(),
                    ..cmd
                });
            } else {
                warn!("gearbox-attach: `{ns}` has no attached tool `{tool}`; command dropped");
            }
        }
    }
}
