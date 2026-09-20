//! Runtime attachments (`specs/TOOLS_SPEC.md`): a slave's coupler hangs on
//! a master's hitch through a rapier joint shaped by the coupling type. The
//! master's agent answers the requests, the slave's agent refuses commands
//! while attached, and the master's `/links` grows the slave's tree.

use crate::physics::PhysicsWorld;
use bevy::prelude::*;
use gearbox_api::{
    AttachRequest, DetachRequest, GearboxBus, LinkDesc, SceneEvent, Status, ToolDesc, code,
    event_kind,
};
use peerbus::ReqReplyToken;
use crate::physics::backend::{
    BodyId, DQuat, DVec3, JointAxes, JointAxis, JointDesc, JointId, JointKind, Pose,
};
use usd_bevy::UsdPrimRef;

use crate::controller::{
    ControllerInventory, MachineAgentKeys, MachineInstanceSpec, find_prim_entity, link_descs,
};
use crate::links::{CouplingSide, LinkSpec, LinkTree};

/// Without teleport the coupler must already be this close to the hitch.
pub(crate) const SNAP_DISTANCE_M: f64 = 0.5;
pub(crate) const SNAP_ANGLE_RAD: f64 = 30.0_f64.to_radians();

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum LocalAttachmentAction {
    Connect {
        master: String,
        slave: String,
        hitch: String,
        coupler: String,
    },
    Disconnect {
        master: String,
        slave: String,
    },
}

#[derive(Resource, Default)]
pub(crate) struct LocalAttachments {
    pub pending: Vec<LocalAttachmentAction>,
    pub feedback: Option<(bool, String)>,
}

enum Reply {
    Remote(ReqReplyToken),
    Local,
    Static,
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub master_id: String,
    pub slave_id: String,
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
            .init_resource::<LocalAttachments>()
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
    fn machine(&self, machine_id: &str) -> Option<&MachineInstanceSpec> {
        let key = self.keys.0.get(machine_id)?;
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
    ) -> Option<BodyId> {
        let entity = self.entity(machine, prim)?;
        physics.entity_to_body.get(&entity).copied()
    }

    fn bodies(
        &self,
        machine: &MachineInstanceSpec,
        physics: &PhysicsWorld,
    ) -> Vec<BodyId> {
        machine
            .links
            .links
            .iter()
            .filter_map(|l| l.body_prim.as_deref())
            .filter_map(|p| self.body(machine, p, physics))
            .collect()
    }
}

/// A parked trailer stands nose-up on its stand; levelled onto the hitch its
/// axles swing into the ground. Pitch the slave about the hitch until no tyre
/// is below the terrain.
fn lift_tyres_out_of_terrain(
    physics: &mut PhysicsWorld,
    bodies: &[BodyId],
    wheels: &[BodyId],
    pivot: DVec3,
    axis: DVec3,
) -> Result<(), String> {
    for _ in 0..3 {
        let deepest = wheels
            .iter()
            .filter_map(|w| {
                let radius = crate::controller::body_max_collider_radius(physics, *w)?;
                let p = physics.body(*w)?.position().translation;
                let ground = crate::globe::ground_height_at_physics(p.x, p.z);
                let offset = p - pivot;
                let reach = (offset - axis * offset.dot(axis)).length();
                (reach > 0.5).then_some((ground - (p.y - radius), reach, offset))
            })
            .max_by(|a, b| (a.0 / a.1).total_cmp(&(b.0 / b.1)));
        let Some((depth, reach, offset)) = deepest else {
            return Ok(());
        };
        if depth <= 0.005 {
            return Ok(());
        }
        let up = DQuat::from_axis_angle(axis, (depth / reach).atan());
        let turn = if (up * offset).y > offset.y {
            up
        } else {
            up.inverse()
        };
        let poses: Vec<_> = bodies.iter().filter_map(|handle| {
            physics.body(*handle).map(|body| {
                let pose = body.position();
                let moved = Pose {
                    translation: pivot + turn * (pose.translation - pivot),
                    rotation: turn * pose.rotation,
                };
                (*handle, moved)
            })
        }).collect();
        physics.set_body_poses(&poses, true)?;
    }
    Ok(())
}

/// The rigid-body link a (possibly body-less) link rides on.
pub(crate) fn body_link<'a>(tree: &'a LinkTree, link: &'a LinkSpec) -> Option<&'a LinkSpec> {
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

fn joint_for(kind: &str, frame1: Pose, frame2: Pose) -> JointDesc {
    let lin = JointAxes::LIN;
    let deg = |d: f64| d.to_radians();
    // Coupling frames are prim frames on bodies that keep the USD basis:
    // X right, Y back, Z up. So yaw is about Z, pitch about X, roll about Y.
    let (mask, limits): (JointAxes, Vec<(JointAxis, f64)>) = match kind {
        "three_point_mounted" | "chassis_mounted" | "loader_carriage" => (JointAxes::ALL, vec![]),
        // Pinned at the eye, pitch and yaw free, roll locked: the tractor
        // carries the trailer's nose.
        "drawbar" => (lin.with(JointAxis::AngY), vec![]),
        "clevis" => (
            lin.with(JointAxis::AngY),
            vec![(JointAxis::AngX, deg(20.0))],
        ),
        "piton" | "fifth_wheel" => (
            lin.with(JointAxis::AngY),
            vec![(JointAxis::AngX, deg(15.0))],
        ),
        "pivot_wagon" | "three_point_semi_mounted" => {
            (lin.with(JointAxis::AngX).with(JointAxis::AngY), vec![])
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
    let mut desc = JointDesc::new(JointKind::Generic { locked: mask }, frame1, frame2);
    desc.softness = Some((30.0, 1.0));
    desc.limits = limits
        .into_iter()
        .map(|(axis, limit)| (axis, [-limit, limit]))
        .collect();
    desc
}

/// Suppress only the connected machines' body pairs; keep authored filters unchanged.
fn set_cross_collisions(
    physics: &mut PhysicsWorld,
    a: &[BodyId],
    b: &[BodyId],
    enabled: bool,
) {
    for &a in a {
        for &b in b {
            for pair in [(a, b), (b, a)] {
                if enabled {
                    physics.attachment_filtered_pairs.remove(&pair);
                } else {
                    physics.attachment_filtered_pairs.insert(pair);
                }
            }
        }
    }
}

/// The physical hitch. A constraint joint (`reduced` stays off): merging the
/// two machines' reduced-coordinate trees blows up (NaN poses), and few joint
/// shapes exist there anyway.
pub type HitchJoint = JointId;

fn insert_hitch_joint(
    physics: &mut PhysicsWorld,
    hitch_body: BodyId,
    coupler_body: BodyId,
    joint: JointDesc,
) -> HitchJoint {
    physics.insert_joint(hitch_body, coupler_body, joint)
}

fn remove_hitch_joint(physics: &mut PhysicsWorld, joint: HitchJoint) {
    physics.remove_joint(joint);
}

/// The prim a slave's coupler names as its parking stand.
fn stand_of(
    scene: &Scene,
    instances: Option<&usd_bevy::instance::UsdInstances>,
    slave_id: &str,
    coupler: &str,
) -> Option<String> {
    let slave = scene.machine(slave_id)?;
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
    slave_id: &str,
    stand: &str,
    hitched: bool,
) {
    let Some(slave) = scene.machine(slave_id) else {
        return;
    };
    if let Some(body) = scene.body(slave, stand, physics) {
        let colliders = physics.body(body).map(|b| b.colliders()).unwrap_or_default();
        for handle in colliders {
            if let Some(c) = physics.collider_mut(handle) {
                c.set_enabled(!hitched);
            }
        }
    }
    let Some(root) = slave.scene_root else {
        return;
    };
    let option = if hitched { "hitched" } else { "parked" };
    info!("gearbox-attach: `{slave_id}` stand {stand} {option}");
    if let Ok(mut o) = overrides.get_mut(root) {
        o.variants
            .retain(|(prim, set, _)| !(prim == &slave.prim_path && set == "coupling"));
        o.variants.push((
            slave.prim_path.clone(),
            "coupling".to_string(),
            option.to_string(),
        ));
    }
}

/// Does `candidate_master` already hang, directly or through others, below
/// `slave`? Attaching would then close a loop.
pub(crate) fn would_loop(attachments: &[Attachment], candidate_master: &str, slave: &str) -> bool {
    let mut cur = candidate_master.to_string();
    for _ in 0..64 {
        if cur == slave {
            return true;
        }
        match attachments.iter().find(|a| a.slave_id == cur) {
            Some(a) => cur = a.master_id.clone(),
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
    master_id: &str,
    req: &AttachRequest,
    scene: &Scene,
    bus: &GearboxBus,
    physics: &mut PhysicsWorld,
    attachments: &[Attachment],
) -> Result<Attached, Status> {
    let slave_id = req.slave();
    if slave_id.is_empty() {
        return Err(Status::err(code::USAGE, "attach needs a `slave` id"));
    }
    if slave_id == master_id {
        return Err(refused("a machine cannot attach to itself"));
    }
    let master_agent = bus
        .machines
        .get(master_id)
        .ok_or_else(|| not_found(format!("no machine `{master_id}`")))?;
    if master_agent.session_id() != req.session {
        return Err(refused(format!(
            "session {} does not hold `{master_id}`",
            req.session
        )));
    }
    let slave_agent = bus
        .machines
        .get(&slave_id)
        .ok_or_else(|| not_found(format!("no machine `{slave_id}`")))?;
    if let Some(already) = slave_agent.attached_to() {
        return Err(refused(format!(
            "`{slave_id}` is already attached to `{already}`"
        )));
    }
    if would_loop(attachments, master_id, &slave_id) {
        return Err(refused(format!(
            "`{master_id}` already hangs below `{slave_id}`; attaching would close a loop"
        )));
    }
    let master = scene
        .machine(master_id)
        .ok_or_else(|| not_found(format!("`{master_id}` has no loaded machine")))?;
    let slave = scene
        .machine(&slave_id)
        .ok_or_else(|| not_found(format!("`{slave_id}` has no loaded machine")))?;

    let hitch_busy = |name: &str| {
        attachments
            .iter()
            .any(|a| a.master_id == master_id && a.hitch == name)
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
        &physics
            .body(hitch_body)
            .ok_or_else(|| refused("hitch body vanished"))?
            .position(),
    );
    let coupler_body_world = Frame::from_pose(
        &physics
            .body(coupler_body)
            .ok_or_else(|| refused("coupler body vanished"))?
            .position(),
    );

    let hitch_scene_body = scene
        .frame(
            master,
            hitch_body_link.body_prim.as_deref().unwrap_or_default(),
        )
        .ok_or_else(|| refused("hitch body transform is unavailable"))?;
    let coupler_scene_body = scene
        .frame(
            slave,
            coupler_body_link.body_prim.as_deref().unwrap_or_default(),
        )
        .ok_or_else(|| refused("coupler body transform is unavailable"))?;
    let hitch_world = hitch_body_world.then(&hitch_scene_body.inverse().then(&hitch_world));
    let coupler_world = coupler_body_world.then(&coupler_scene_body.inverse().then(&coupler_world));

    // Only the prims' positions count. Every body keeps the USD basis (X
    // right, Y back, Z up), so aligning the two bodies puts the slave behind
    // the master facing the same way; an authored prim rotation would tip
    // the whole slave over on teleport.
    let hitch_world = Frame {
        translation: hitch_world.translation,
        rotation: hitch_body_world.rotation,
    };
    let coupler_world = Frame {
        translation: coupler_world.translation,
        rotation: coupler_body_world.rotation,
    };
    let target = Frame {
        translation: hitch_world.translation,
        rotation: hitch_body_world.rotation,
    };
    let frame1 = hitch_body_world.inverse().then(&hitch_world);
    let frame2 = coupler_body_world.inverse().then(&coupler_world);

    let slave_bodies = scene.bodies(slave, physics);
    if req.teleport != 0 {
        let delta = target.then(&coupler_world.inverse());
        let poses: Vec<_> = slave_bodies.iter().filter_map(|handle| {
            physics.body(*handle).map(|body| {
                let cur = Frame::from_pose(&body.position());
                let moved = delta.then(&cur);
                (*handle, moved.pose())
            })
        }).collect();
        physics.set_body_poses(&poses, true).map_err(refused)?;
        let wheels: Vec<BodyId> = slave
            .links
            .links
            .iter()
            .filter(|l| l.role == crate::links::LinkRole::Wheel)
            .filter_map(|l| l.body_prim.as_deref())
            .filter_map(|p| scene.body(slave, p, physics))
            .collect();
        lift_tyres_out_of_terrain(
            &mut *physics,
            &slave_bodies,
            &wheels,
            target.translation,
            target.rotation * DVec3::X,
        ).map_err(refused)?;
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
        .filter_map(|h| physics.body(*h))
        .map(|b| b.mass())
        .sum();
    let (Some(hitch_pose), Some(coupler_pose)) = (
        physics.body(hitch_body).map(|b| b.position()),
        physics.body(coupler_body).map(|b| b.position()),
    ) else {
        return Err(refused("the hitch or the coupler has no physics body".to_string()));
    };
    let current_hitch = Frame::from_pose(&hitch_pose).then(&frame1);
    let initial_frame2 = Frame::from_pose(&coupler_pose).inverse().then(&current_hitch);
    let mut joint = joint_for(&hitch.kind, frame1.pose(), initial_frame2.pose());
    joint.softness = Some((8.0, 1.0));
    let handle = insert_hitch_joint(physics, hitch_body, coupler_body, joint);
    physics.capture_hitch(handle, frame2.pose());
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
            "gearbox-attach: `{master_id}` does not grant `{slave_id}` these requests: {}",
            denied.join(", ")
        );
    }
    let event = SceneEvent::new(event_kind::ATTACHED, &slave_id)
        .with_prop("master", master_id)
        .with_prop("slave", &slave_id)
        .with_prop("hitch", &hitch.name)
        .with_prop("coupler", &coupler.name)
        .with_prop("type", &hitch.kind)
        .with_prop("denied", &denied.join(","));
    Ok(Attached {
        attachment: Attachment {
            master_id: master_id.to_string(),
            slave_id,
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
    master_id: &str,
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
        for a in attachments.iter().filter(|a| a.master_id == owner) {
            tools.push(ToolDesc {
                slave: a.slave_id.clone(),
                hitch: a.hitch.clone(),
                coupler: a.coupler.clone(),
                kind: a.kind.clone(),
                controlled: a.controlled,
                depth,
                denied: a.denied.join(","),
            });
            let slave_prefix = format!("{prefix}{}/", a.slave_id);
            let hitch_parent = format!("{prefix}{}", a.hitch_link);
            if let Some(slave) = scene.machine(&a.slave_id) {
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
                &a.slave_id,
                &slave_prefix,
                depth + 1,
                attachments,
                scene,
                tools,
                links,
            );
        }
    }
    walk(master_id, "", 0, attachments, scene, &mut tools, &mut links);
    (tools, links)
}

fn refresh_masters(bus: &mut GearboxBus, attachments: &[Attachment], scene: &Scene) {
    let masters: Vec<String> = bus.machines.keys().cloned().collect();
    for machine_id in masters {
        let (tools, links) = composite(&machine_id, attachments, scene);
        if let Some(agent) = bus.machines.get_mut(&machine_id) {
            agent.set_tools(tools, links);
        }
    }
}

pub(crate) fn serve_attachments(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    mut attachments: ResMut<Attachments>,

    mut physics: ResMut<PhysicsWorld>,
    mut pending_static: ResMut<PendingStaticAttachments>,
    prims: Query<(Entity, &'static UsdPrimRef)>,
    parents: Query<&'static ChildOf>,
    transforms: Query<&'static GlobalTransform>,
    instances: Option<NonSend<usd_bevy::instance::UsdInstances>>,
    mut overrides: Query<&mut usd_bevy::instance::UsdInstanceOverrides>,
    mut local: ResMut<LocalAttachments>,
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
            bus.machines.contains_key(&a.master_id) && bus.machines.contains_key(&a.slave_id);
        if !alive {
            dropped.push(a.clone());
        }
        alive
    });
    for a in dropped {
        remove_hitch_joint(physics.as_mut(), a.joint);
        if let Some(slave) = bus.machines.get_mut(&a.slave_id) {
            slave.set_attached_to(None);
        }
    }
    if attachments.0.len() != before {
        let physics = physics.as_mut();
        let backend = &physics.backend;
        physics
            .attachment_filtered_pairs
            .retain(|(a, b)| backend.contains_body(*a) && backend.contains_body(*b));
    }
    let mut changed = attachments.0.len() != before;

    let mut attach_reqs: Vec<(String, AttachRequest, Reply)> = Vec::new();
    let mut detach_reqs: Vec<(String, DetachRequest, Reply)> = Vec::new();
    for (machine_id, agent) in bus.machines.iter_mut() {
        for (req, token) in agent.pending_attach.drain(..) {
            attach_reqs.push((machine_id.clone(), req, Reply::Remote(token)));
        }
        for (req, token) in agent.pending_detach.drain(..) {
            detach_reqs.push((machine_id.clone(), req, Reply::Remote(token)));
        }
    }

    for action in std::mem::take(&mut local.pending) {
        let (master, slave) = match &action {
            LocalAttachmentAction::Connect { master, slave, .. }
            | LocalAttachmentAction::Disconnect { master, slave } => (master, slave),
        };
        let safe = [master, slave].into_iter().all(|machine_id| {
            let Some(agent) = bus.machines.get(machine_id) else {
                return false;
            };
            let Some(machine) = scene.machine(machine_id) else {
                return false;
            };
            let Some(body) = machine
                .body
                .as_deref()
                .and_then(|p| scene.body(machine, p, &physics))
                .and_then(|h| physics.body(h))
            else {
                return false;
            };
            agent.session_id() == 0 && body.linvel().length() < 0.3 && body.angvel().length() < 0.2
        });
        if !safe {
            local.feedback = Some((
                false,
                "Stop both machines and release external control first.".into(),
            ));
            continue;
        }
        match action {
            LocalAttachmentAction::Connect {
                master,
                slave,
                hitch,
                coupler,
            } => {
                attach_reqs.push((
                    master,
                    AttachRequest::new(0, &slave)
                        .with_hitch(&hitch)
                        .with_coupler(&coupler),
                    Reply::Local,
                ));
            }
            LocalAttachmentAction::Disconnect { master, slave } => {
                detach_reqs.push((master, DetachRequest::new(0, &slave), Reply::Local));
            }
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
                .map(|(machine_id, _)| machine_id.clone())
        };
        let ready = match (owner(&sa.hitch_prim), owner(&sa.coupler_prim)) {
            (Some(master), Some(slave)) => match (ns_of(master), ns_of(slave)) {
                (Some(master_id), Some(slave_id))
                    if bus.machines.contains_key(&master_id)
                        && bus.machines.contains_key(&slave_id) =>
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
                            let req = AttachRequest::new(0, &slave_id)
                                .with_hitch(&h)
                                .with_coupler(&c)
                                .teleporting();
                            attach_reqs.push((master_id, req, Reply::Static));
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

    for (master_id, req, token) in attach_reqs {
        let outcome = try_attach(&master_id, &req, &scene, &bus, &mut physics, &attachments.0);
        let status = match outcome {
            Ok(mut done) => {
                let slave_id = done.attachment.slave_id.clone();
                done.attachment.stand = stand_of(
                    &scene,
                    instances.as_deref(),
                    &slave_id,
                    &done.attachment.coupler,
                );
                if let Some(stand) = done.attachment.stand.clone() {
                    set_stand(
                        &scene,
                        physics.as_mut(),
                        &mut overrides,
                        &slave_id,
                        &stand,
                        true,
                    );
                }
                info!(
                    "gearbox-attach: `{slave_id}` on `{master_id}` via {} / {} ({})",
                    done.attachment.hitch, done.attachment.coupler, done.attachment.kind
                );
                attachments.0.push(done.attachment);
                if let Some(slave) = bus.machines.get_mut(&slave_id) {
                    slave.set_attached_to(Some(master_id.clone()));
                }
                bus.publish_event(done.event);
                changed = true;
                Status::ok_with(&gearbox_api::Props::from_pairs(&[(
                    "slave",
                    slave_id.as_str(),
                )]))
            }
            Err(status) => {
                warn!(
                    "gearbox-attach: `{master_id}` refused attach of `{}`: {}",
                    req.slave(),
                    status.message()
                );
                status
            }
        };
        match token {
            Reply::Remote(token) => {
                if let Some(agent) = bus.machines.get_mut(&master_id) {
                    agent.respond_attach(token, &status);
                }
            }
            Reply::Local => {
                local.feedback = Some((
                    status.is_ok(),
                    if status.is_ok() {
                        format!("{} connected to {master_id}", req.slave())
                    } else {
                        status.message()
                    },
                ))
            }
            Reply::Static => {}
        }
    }

    for (master_id, req, token) in detach_reqs {
        let slave_id = req.slave();
        let status = match bus.machines.get(&master_id) {
            Some(agent) if agent.session_id() != req.session => refused(format!(
                "session {} does not hold `{master_id}`",
                req.session
            )),
            _ => match attachments
                .0
                .iter()
                .position(|a| a.master_id == master_id && a.slave_id == slave_id)
            {
                None => not_found(format!("`{slave_id}` is not attached to `{master_id}`")),
                Some(i) => {
                    let a = attachments.0.remove(i);
                    remove_hitch_joint(physics.as_mut(), a.joint);
                    if let (Some(m), Some(s)) =
                        (scene.machine(&master_id), scene.machine(&slave_id))
                    {
                        let (mb, sb) = (scene.bodies(m, &physics), scene.bodies(s, &physics));
                        set_cross_collisions(physics.as_mut(), &mb, &sb, true);
                    }
                    if let Some(stand) = a.stand.as_deref() {
                        set_stand(
                            &scene,
                            physics.as_mut(),
                            &mut overrides,
                            &slave_id,
                            stand,
                            false,
                        );
                    }
                    if let Some(slave) = bus.machines.get_mut(&slave_id) {
                        slave.set_attached_to(None);
                    }
                    info!("gearbox-attach: `{slave_id}` detached from `{master_id}`");
                    bus.publish_event(
                        SceneEvent::new(event_kind::DETACHED, &slave_id)
                            .with_prop("master", &master_id)
                            .with_prop("slave", &slave_id)
                            .with_prop("hitch", &a.hitch)
                            .with_prop("coupler", &a.coupler),
                    );
                    changed = true;
                    Status::ok()
                }
            },
        };
        match token {
            Reply::Remote(token) => {
                if let Some(agent) = bus.machines.get_mut(&master_id) {
                    agent.respond_detach(token, &status);
                }
            }
            Reply::Local => {
                local.feedback = Some((
                    status.is_ok(),
                    if status.is_ok() {
                        format!("{slave_id} disconnected")
                    } else {
                        status.message()
                    },
                ))
            }
            Reply::Static => {}
        }
    }

    if changed {
        refresh_masters(&mut bus, &attachments.0, &scene);
    }

    // Commands aimed at a slave through its master land in the slave's queue.
    let masters: Vec<String> = bus.machines.keys().cloned().collect();
    for machine_id in masters {
        let routed: Vec<(String, gearbox_api::ControllerCommand)> = {
            let Some(agent) = bus.machines.get_mut(&machine_id) else {
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
            let reachable = attachments.0.iter().any(|a| a.slave_id == leaf);
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
                warn!("gearbox-attach: `{machine_id}` has no attached tool `{tool}`; command dropped");
            }
        }
    }
}
