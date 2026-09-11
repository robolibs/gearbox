//! Runtime attachments (`specs/TOOLS_SPEC.md`): a slave's coupler hangs on
//! a master's hitch through a rapier joint shaped by the coupling type. The
//! master's agent answers the requests, the slave's agent refuses commands
//! while attached, and the master's `/links` grows the slave's tree.

use std::collections::HashMap;

use bevy::prelude::*;
use gearbox_api::{
    AttachRequest, DetachRequest, GearboxBus, LinkDesc, SceneEvent, Status, ToolDesc, code,
    event_kind,
};
use peerbus::ReqReplyToken;
use rapier3d::math::{Rotation as DQuat, Vector as DVec3};
use rapier3d::prelude::{
    GenericJoint, GenericJointBuilder, ImpulseJointHandle, JointAxesMask, JointAxis, Pose,
    RigidBodyHandle,
};
use usd_bevy::UsdPrimRef;
use usd_bevy::physics::PhysicsWorld;

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
    pub joint: ImpulseJointHandle,
    pub slave_mass_kg: f64,
    pub controlled: bool,
}

#[derive(Resource, Default)]
pub struct Attachments(pub Vec<Attachment>);

/// Extra mass each master tows, by machine id, for the engine force law.
#[derive(Resource, Default)]
pub struct TowedMass(pub HashMap<String, f64>);

pub struct AttachPlugin;

impl Plugin for AttachPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Attachments>()
            .init_resource::<TowedMass>()
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
    // Joint frames are Y up: yaw about Y, pitch about Z, roll about X.
    let (mask, limits): (JointAxesMask, Vec<(JointAxis, f64)>) = match kind {
        "three_point_mounted" | "chassis_mounted" | "loader_carriage" => (
            lin | JointAxesMask::ANG_X | JointAxesMask::ANG_Y | JointAxesMask::ANG_Z,
            vec![],
        ),
        "drawbar" => (
            lin,
            vec![(JointAxis::AngZ, deg(20.0)), (JointAxis::AngX, deg(10.0))],
        ),
        "clevis" => (
            lin | JointAxesMask::ANG_X,
            vec![(JointAxis::AngZ, deg(20.0))],
        ),
        "piton" | "fifth_wheel" => (
            lin | JointAxesMask::ANG_X,
            vec![(JointAxis::AngZ, deg(15.0))],
        ),
        "pivot_wagon" | "three_point_semi_mounted" => {
            (lin | JointAxesMask::ANG_X | JointAxesMask::ANG_Z, vec![])
        }
        "hitch_hook" => (
            lin,
            vec![(JointAxis::AngX, deg(25.0)), (JointAxis::AngZ, deg(25.0))],
        ),
        "cuna" => (
            lin,
            vec![(JointAxis::AngX, deg(20.0)), (JointAxis::AngZ, deg(20.0))],
        ),
        "ball" => (
            lin,
            vec![(JointAxis::AngX, deg(30.0)), (JointAxis::AngZ, deg(30.0))],
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

    // The coupler must sit on the hitch, facing the same way as the master.
    let target = Frame {
        translation: hitch_world.translation,
        rotation: hitch_world.rotation * DQuat::from_rotation_y(std::f64::consts::PI),
    };
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
    let handle = physics
        .impulse_joints
        .insert(hitch_body, coupler_body, joint, true);

    let event = SceneEvent::new(event_kind::ATTACHED, &slave_ns)
        .with_prop("master", master_ns)
        .with_prop("slave", &slave_ns)
        .with_prop("hitch", &hitch.name)
        .with_prop("coupler", &coupler.name)
        .with_prop("type", &hitch.kind);
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

fn serve_attachments(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    mut attachments: ResMut<Attachments>,
    mut towed: ResMut<TowedMass>,
    mut physics: ResMut<PhysicsWorld>,
    prims: Query<(Entity, &'static UsdPrimRef)>,
    parents: Query<&'static ChildOf>,
    transforms: Query<&'static GlobalTransform>,
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
        physics.impulse_joints.remove(a.joint, true);
        if let Some(slave) = bus.machines.get_mut(&a.slave_ns) {
            slave.set_attached_to(None);
        }
    }
    let mut changed = attachments.0.len() != before;

    let mut attach_reqs: Vec<(String, AttachRequest, ReqReplyToken)> = Vec::new();
    let mut detach_reqs: Vec<(String, DetachRequest, ReqReplyToken)> = Vec::new();
    for (ns, agent) in bus.machines.iter_mut() {
        for (req, token) in agent.pending_attach.drain(..) {
            attach_reqs.push((ns.clone(), req, token));
        }
        for (req, token) in agent.pending_detach.drain(..) {
            detach_reqs.push((ns.clone(), req, token));
        }
    }

    for (master_ns, req, token) in attach_reqs {
        let outcome = try_attach(&master_ns, &req, &scene, &bus, &mut physics, &attachments.0);
        let status = match outcome {
            Ok(done) => {
                let slave_ns = done.attachment.slave_ns.clone();
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
        if let Some(agent) = bus.machines.get_mut(&master_ns) {
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
                    physics.impulse_joints.remove(a.joint, true);
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
                slave.commands.push(cmd);
            } else {
                warn!("gearbox-attach: `{ns}` has no attached tool `{tool}`; command dropped");
            }
        }
    }
}
