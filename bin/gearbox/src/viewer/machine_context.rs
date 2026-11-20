//! Live machine cards, coupling candidates and the rings drawn round
//! hovered, selected and followed machines.

use std::collections::HashMap;

use bevy::prelude::*;
use gearbox_api::wire::draw_widgets::{Place, RingMark, machine_ring};
use gearbox_api::{DrawStoreRes, draw_flag};

use super::state::FollowTarget;
use super::systems::{Selection, SelectionRing};
use crate::{
    attach::{
        Attachments, LocalAttachmentAction, SNAP_ANGLE_RAD, SNAP_DISTANCE_M, body_link, would_loop,
    },
    controller::{ControllerInventory, MachineAgentKeys, MachineRef},
    links::CouplingSide,
    physics::PhysicsWorld,
};

#[derive(Resource, Default)]
pub(crate) struct MachineHover {
    pub hit: Option<MachineRef>,
    pub retained: Option<MachineRef>,
    pub captures_pointer: bool,
}

#[derive(Clone)]
pub(crate) struct CouplingAction {
    pub action: LocalAttachmentAction,
    pub endpoints: [Vec3; 2],
    pub label: String,
    pub detail: String,
    pub enabled: bool,
}

#[derive(Clone)]
pub(crate) struct MachineCard {
    pub machine_id: String,
    pub machine: MachineRef,
    pub ground_center: Vec3,
    pub ground_right: Vec3,
    pub ground_down: Vec3,
    pub center: Vec3,
    pub radius: f32,
    pub stopped: bool,
    pub available: bool,
    pub actions: Vec<CouplingAction>,
    pub ring: Option<RingFit>,
}

#[derive(Resource, Default)]
pub(crate) struct MachineCards(pub Vec<MachineCard>);

struct Port {
    machine: usize,
    name: String,
    kind: String,
    side: CouplingSide,
    position: Vec3,
    rotation: Quat,
}

pub(super) struct MachineContextPlugin;

impl Plugin for MachineContextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachineHover>()
            .init_resource::<MachineCards>()
            .add_systems(
                PostUpdate,
                (publish_cards, draw_rings)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

type MeshBounds = (
    &'static GlobalTransform,
    &'static bevy::camera::primitives::Aabb,
    Option<&'static InheritedVisibility>,
);

/// World corners of the boxes of the visible meshes at and under `root`.
fn subtree_corners(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Query<MeshBounds, With<Mesh3d>>,
) -> Vec<Vec3> {
    let mut corners = Vec::new();
    let subtree = std::iter::once(root).chain(children.iter_descendants(root));
    for (gt, aabb, visibility) in meshes.iter_many(subtree).flatten() {
        if visibility.is_some_and(|v| !v.get()) {
            continue;
        }
        let (centre, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        corners.extend((0..8).map(|i| {
            let side = Vec3::new([-1.0, 1.0][i & 1], [-1.0, 1.0][i >> 1 & 1], [-1.0, 1.0][i >> 2]);
            gt.transform_point(centre + half * side)
        }));
    }
    corners
}

fn min_max(points: impl Iterator<Item = Vec3>) -> (Vec3, Vec3) {
    points.fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(min, max), p| (min.min(p), max.max(p)))
}

/// Centre and reach across the ground of `corners`: the middle of their
/// bounds in `frame` (the machine's own), and the farthest one from it.
fn subtree_bounds(corners: &[Vec3], frame: &GlobalTransform) -> Option<(Vec3, f32)> {
    let to_frame = frame.affine().inverse();
    let (min, max) = min_max(corners.iter().map(|p| to_frame.transform_point3(*p)));
    let centre = frame.transform_point((min + max) * 0.5);
    let reach = corners.iter().map(|p| (*p - centre).xz().length()).fold(0.0, f32::max);
    min.is_finite().then_some((centre, reach))
}

/// A machine's ring, fitted once it settles after spawning: its pose in the
/// base link's frame, level with the ground under the machine then, and its
/// outer radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RingFit {
    pub at: Vec3,
    pub turn: Quat,
    pub radius: f32,
}

/// Frames a machine's mesh count must hold before its ring is fitted.
const RING_SETTLE_FRAMES: u32 = 30;
/// Space between a machine's widest corner and its ring, metres.
const RING_MARGIN_M: f32 = 0.35;
/// Height of a ring over the machine's lowest point, metres.
const RING_LIFT_M: f32 = 0.03;

/// The ring for a machine whose mesh box `corners` are given in the world,
/// seen from its base `link`.
fn fit_ring(link: &GlobalTransform, corners: &[Vec3]) -> Option<RingFit> {
    let (_, rotation, translation) = link.to_scale_rotation_translation();
    let turn = Quat::from_rotation_arc(Vec3::Z, (rotation.inverse() * Vec3::Y).normalize());
    let level = (rotation * turn).inverse();
    let points: Vec<Vec3> = corners.iter().map(|c| level * (*c - translation)).collect();
    let (min, max) = min_max(points.iter().copied());
    if !min.is_finite() {
        return None;
    }
    let centre = Vec3::new(0.5 * (min.x + max.x), 0.5 * (min.y + max.y), min.z + RING_LIFT_M);
    let reach = points.iter().map(|p| (p.xy() - centre.xy()).length()).fold(0.0, f32::max);
    Some(RingFit { at: turn * centre, turn, radius: (reach + RING_MARGIN_M).max(1.0) })
}

/// A spawning machine's ring: fitted once its mesh count has held still.
#[derive(Default)]
struct Settling {
    corners: usize,
    frames: u32,
    fit: Option<RingFit>,
}

impl Settling {
    /// Counts a frame in which the machine has `corners` mesh corners; true
    /// once that count has held for [`RING_SETTLE_FRAMES`].
    fn settled(&mut self, corners: usize) -> bool {
        if corners > 0 && corners == self.corners {
            self.frames += 1;
        } else {
            (self.corners, self.frames) = (corners, 0);
        }
        self.frames >= RING_SETTLE_FRAMES
    }
}

fn publish_cards(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<Res<gearbox_api::GearboxBus>>,
    attachments: Res<Attachments>,
    physics: Res<PhysicsWorld>,
    index: Res<crate::prim_index::PrimIndex>,
    children: Query<&Children>,
    transforms: Query<&GlobalTransform>,
    meshes: Query<MeshBounds, With<Mesh3d>>,
    mut cards: ResMut<MachineCards>,
    mut reaches: Local<HashMap<String, f32>>,
    mut settling: Local<HashMap<String, Settling>>,
) {
    reaches.retain(|machine_id, _| keys.0.contains_key(machine_id));
    settling.retain(|machine_id, _| keys.0.contains_key(machine_id));
    cards.0.clear();
    let mut ports = Vec::new();
    let mut agents: Vec<_> = keys.0.iter().collect();
    agents.sort_by_key(|(machine_id, _)| *machine_id);
    for (machine_id, key) in agents {
        let Some(machine) = inventory
            .machines
            .iter()
            .find(|m| m.scene_root == Some(key.scene_root) && m.id == key.machine_id)
        else {
            continue;
        };
        let entity_of = |p: &str| index.find(key.scene_root, p);
        let machine_entity = entity_of(&machine.prim_path).unwrap_or(key.scene_root);
        let Some(frame) = machine
            .body
            .as_deref()
            .and_then(entity_of)
            .or(Some(machine_entity))
            .and_then(|e| transforms.get(e).ok())
        else {
            continue;
        };
        let corners = subtree_corners(machine_entity, &children, &meshes);
        let Some((center, measured)) = subtree_bounds(&corners, frame) else {
            continue;
        };
        let ring = settling.entry(machine_id.clone()).or_default();
        if ring.fit.is_none() && ring.settled(corners.len()) {
            let base = machine.links.base().or(machine.links.links.first());
            ring.fit = base
                .and_then(|link| entity_of(&link.prim_path))
                .and_then(|e| transforms.get(e).ok())
                .and_then(|link| fit_ring(link, &corners));
        }
        let ring = ring.fit;
        // Spinning wheels sway the reach a little; the ring keeps the widest.
        let widest = reaches.entry(machine_id.clone()).or_insert(0.0);
        *widest = widest.max(measured);
        let reach = *widest;
        let body = machine
            .body
            .as_deref()
            .and_then(entity_of)
            .and_then(|e| physics.entity_to_body.get(&e))
            .and_then(|h| physics.body(*h));
        let index = cards.0.len();
        for (link, coupling) in machine.links.couplings() {
            let Some(gt) = entity_of(&link.prim_path).and_then(|e| transforms.get(e).ok()) else {
                continue;
            };
            let Some(body_gt) = body_link(&machine.links, link)
                .and_then(|l| l.body_prim.as_deref())
                .and_then(entity_of)
                .and_then(|e| transforms.get(e).ok())
            else {
                continue;
            };
            ports.push(Port {
                machine: index,
                name: coupling.name.clone(),
                kind: coupling.kind.clone(),
                side: coupling.side,
                position: gt.translation(),
                rotation: body_gt.rotation(),
            });
        }
        let right = body
            .map(|b| {
                let q = b.rotation();
                Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32) * Vec3::X
            })
            .map(|v| Vec3::new(v.x, 0.0, v.z).normalize_or(Vec3::X))
            .unwrap_or(Vec3::X);
        let radius = (reach + 0.35).max(1.0);
        cards.0.push(MachineCard {
            machine_id: machine_id.clone(),
            machine: MachineRef {
                scene_root: key.scene_root,
                machine_id: key.machine_id.clone(),
            },
            ground_center: center + right * (radius + 0.55),
            ground_right: right.cross(Vec3::Y),
            ground_down: -right,
            center,
            radius,
            stopped: body.is_some_and(|b| b.linvel().length() < 0.3 && b.angvel().length() < 0.2),
            available: bus
                .as_ref()
                .and_then(|b| b.machines.get(machine_id))
                .is_some_and(|a| a.session_id() == 0),
            actions: Vec::new(),
            ring,
        });
    }
    for attachment in &attachments.0 {
        let Some(mi) = cards.0.iter().position(|c| c.machine_id == attachment.master_id) else {
            continue;
        };
        let Some(si) = cards.0.iter().position(|c| c.machine_id == attachment.slave_id) else {
            continue;
        };
        let enabled = [mi, si]
            .into_iter()
            .all(|i| cards.0[i].stopped && cards.0[i].available);
        let Some(hitch) = ports.iter().find(|p| {
            p.machine == mi && p.side == CouplingSide::Hitch && p.name == attachment.hitch
        }) else {
            continue;
        };
        let Some(coupler) = ports.iter().find(|p| {
            p.machine == si && p.side == CouplingSide::Coupler && p.name == attachment.coupler
        }) else {
            continue;
        };
        let action = CouplingAction {
            action: LocalAttachmentAction::Disconnect {
                master: attachment.master_id.clone(),
                slave: attachment.slave_id.clone(),
            },
            endpoints: [hitch.position, coupler.position],
            label: "Disconnect".into(),
            detail: format!(
                "{} ↔ {} · {}{}",
                attachment.master_id,
                attachment.slave_id,
                attachment.kind,
                if enabled {
                    ""
                } else {
                    " · stop / release control first"
                }
            ),
            enabled,
        };
        cards.0[mi].actions.push(action.clone());
        cards.0[si].actions.push(action);
    }
    for hitch in ports.iter().filter(|p| p.side == CouplingSide::Hitch) {
        for coupler in ports.iter().filter(|p| {
            p.side == CouplingSide::Coupler && p.kind == hitch.kind && p.machine != hitch.machine
        }) {
            let (master, slave) = (&cards.0[hitch.machine], &cards.0[coupler.machine]);
            let gap = hitch.position.distance(coupler.position) as f64;
            if gap > crate::attach::COUPLING_REACH_M
                || attachments.0.iter().any(|a| {
                    a.slave_id == slave.machine_id || (a.master_id == master.machine_id && a.hitch == hitch.name)
                })
                || would_loop(&attachments.0, &master.machine_id, &slave.machine_id)
            {
                continue;
            }
            let angle = hitch.rotation.angle_between(coupler.rotation) as f64;
            let reason = if !master.available || !slave.available {
                "Release external control"
            } else if !master.stopped || !slave.stopped {
                "Stop both machines"
            } else if gap > SNAP_DISTANCE_M {
                "Move closer"
            } else if angle > SNAP_ANGLE_RAD {
                "Align machines"
            } else {
                "Ready to connect"
            };
            let action = CouplingAction {
                action: LocalAttachmentAction::Connect {
                    master: master.machine_id.clone(),
                    slave: slave.machine_id.clone(),
                    hitch: hitch.name.clone(),
                    coupler: coupler.name.clone(),
                },
                endpoints: [hitch.position, coupler.position],
                label: "Connect".into(),
                detail: format!(
                    "{} ↔ {}\n{} / {} · {:.0} cm · {:.0}°\n{}",
                    master.machine_id,
                    slave.machine_id,
                    hitch.name,
                    coupler.name,
                    gap * 100.0,
                    angle.to_degrees(),
                    reason
                ),
                enabled: reason == "Ready to connect",
            };
            cards.0[hitch.machine].actions.push(action.clone());
            cards.0[coupler.machine].actions.push(action);
        }
    }
}

pub(crate) fn ring_color(selected: bool) -> LinearRgba {
    if selected {
        LinearRgba::new(0.12, 0.8, 0.65, 0.65)
    } else {
        LinearRgba::new(0.55, 0.8, 0.95, 0.5)
    }
}

/// Draw namespace of the viewer's machine rings, one path per machine below it.
pub(crate) const RING_NS: &str = "gearbox/rings";

/// A ring as laid: its fit to the machine and its mark.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ring {
    fit: RingFit,
    mark: RingMark,
}

fn ring_root(machine_id: &str) -> String {
    format!("{RING_NS}/{machine_id}")
}

/// Puts the `wanted` rings in the draw store, in each machine's base link
/// frame, and takes the rest out. A ring is laid again only when its mark
/// changes or it went missing from the store; the link frame carries it.
fn lay_rings(
    store: &DrawStoreRes,
    laid: &mut HashMap<String, Ring>,
    wanted: HashMap<String, Ring>,
) {
    let mut store = store.lock();
    laid.retain(|id, _| {
        let keep = wanted.contains_key(id);
        if !keep {
            store.clear(&ring_root(id));
        }
        keep
    });
    for (id, ring) in wanted {
        if laid.get(&id) == Some(&ring) && store.get(&ring_root(&id), "ring").is_some() {
            continue;
        }
        let (at, turn) = (ring.fit.at.as_dvec3().to_array(), ring.fit.turn.as_dquat());
        for e in machine_ring(&Place::frame(&id), RING_NS, &id, [0.0; 3], ring.fit.radius, ring.mark, 0) {
            let _ = store.apply(e.with_pose(at, [turn.w, turn.x, turn.y, turn.z]).with_flags(draw_flag::VIEW_ONLY), 0.0);
        }
        laid.insert(id, ring);
    }
}

/// Rings round the followed machine always, and round the selected and
/// hovered ones while the simulation runs, drawn through the draw store.
fn draw_rings(
    cards: Res<MachineCards>,
    selection: Res<Selection>,
    inventory: Res<ControllerInventory>,
    hover: Res<MachineHover>,
    follow: Res<FollowTarget>,
    active: Res<gearbox_api::PhysicsActive>,
    store: Option<Res<DrawStoreRes>>,
    mut ring: ResMut<SelectionRing>,
    mut laid: Local<HashMap<String, Ring>>,
) {
    let Some(store) = store else {
        return;
    };
    let picked = selection.machine(&inventory);
    let hovered = hover.retained.as_ref().or(hover.hit.as_ref());
    let mut wanted = HashMap::new();
    for card in &cards.0 {
        let selected = picked.is_some_and(|m| card.machine.is(m));
        if selected {
            ring.outer_radius = card.radius;
        }
        let Some(fit) = card.ring else {
            continue;
        };
        let mark = if follow.machine.as_ref() == Some(&card.machine) {
            RingMark::Following
        } else if active.0 && selected {
            RingMark::Selected
        } else if active.0 && hovered == Some(&card.machine) {
            RingMark::Hover
        } else {
            continue;
        };
        wanted.insert(card.machine.machine_id.clone(), Ring { fit, mark });
    }
    lay_rings(&store, &mut laid, wanted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::primitives::Aabb;
    use bevy::ecs::system::SystemState;

    fn mesh(world: &mut World, turn: Quat, x: f32, visible: bool) -> Entity {
        world
            .spawn((
                Mesh3d(Handle::default()),
                GlobalTransform::from(Transform::from_rotation(turn) * Transform::from_xyz(x, 0.0, 0.0)),
                Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
                if visible { InheritedVisibility::VISIBLE } else { InheritedVisibility::HIDDEN },
            ))
            .id()
    }

    /// Centre and reach of a machine (a part, a wheel 4 m along and a hidden
    /// part) and of a neighbour, everything turned `turn` about up.
    fn bounds(turn: Quat) -> [(Vec3, f32); 2] {
        let mut world = World::new();
        let machine = world.spawn_empty().id();
        let part = mesh(&mut world, turn, 0.0, true);
        let wheel = mesh(&mut world, turn, 4.0, true);
        let hidden = mesh(&mut world, turn, -50.0, false);
        let neighbour = mesh(&mut world, turn, 100.0, true);
        world.entity_mut(machine).add_children(&[part, hidden]);
        world.entity_mut(part).add_child(wheel);
        let mut state =
            SystemState::<(Query<&Children>, Query<MeshBounds, With<Mesh3d>>)>::new(&mut world);
        let (children, meshes) = state.get(&world).expect("queries are valid");
        let frame = GlobalTransform::from_rotation(turn);
        [machine, neighbour].map(|root| {
            subtree_bounds(&subtree_corners(root, &children, &meshes), &frame).expect("meshes bound")
        })
    }

    #[test]
    fn a_card_bounds_only_the_visible_meshes_under_its_machine() {
        let [(centre, reach), (around, alone)] = bounds(Quat::IDENTITY);
        assert!(centre.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5));
        assert!((reach - 10f32.sqrt()).abs() < 1e-5, "the farthest corner, got {reach}");
        assert!(around.abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-4));
        assert!((alone - 2f32.sqrt()).abs() < 1e-5);
    }

    /// A 4 x 2 x 1 m machine whose base link stands 0.5 m over its lowest
    /// point, Z up (the USD up-axis fix), turned `yaw` and put at `at`.
    fn spawned(yaw: f32, at: Vec3) -> (GlobalTransform, Vec<Vec3>) {
        let link = GlobalTransform::from(
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        );
        let corners = (0..8)
            .map(|i| Vec3::new([-2.0, 2.0][i & 1], [-1.0, 1.0][i >> 1 & 1], [-0.5, 0.5][i >> 2]))
            .map(|local| link.transform_point(local))
            .collect();
        (link, corners)
    }

    #[test]
    fn a_ring_is_fitted_under_the_base_link_whatever_the_machine_faces() {
        for (yaw, at) in [(0.0, Vec3::ZERO), (0.785, Vec3::new(30.0, 2.0, -7.0)), (2.5, Vec3::new(-4.0, 0.0, 9.0))] {
            let (link, corners) = spawned(yaw, at);
            let fit = fit_ring(&link, &corners).expect("a machine with meshes is fitted");
            assert!(fit.at.abs_diff_eq(Vec3::new(0.0, 0.0, -0.5 + RING_LIFT_M), 1e-4), "{fit:?}");
            assert!(fit.turn.angle_between(Quat::IDENTITY) < 1e-4, "the ring lies in the link's level plane");
            assert!((fit.radius - (5f32.sqrt() + RING_MARGIN_M)).abs() < 1e-4, "{fit:?}");
        }
        assert_eq!(fit_ring(&GlobalTransform::IDENTITY, &[]), None);
    }

    #[test]
    fn a_ring_is_fitted_once_the_meshes_stop_arriving() {
        let mut settling = Settling::default();
        assert!(!(0..RING_SETTLE_FRAMES).any(|_| settling.settled(0)), "nothing loaded yet");
        assert!(!(0..RING_SETTLE_FRAMES).any(|_| settling.settled(8)));
        assert!(!settling.settled(16), "a new mesh starts the count again");
        assert!((0..RING_SETTLE_FRAMES).map(|_| settling.settled(16)).last().unwrap());
    }

    fn ring(mark: RingMark) -> Ring {
        Ring { fit: RingFit { at: Vec3::new(0.5, 0.0, -0.47), turn: Quat::IDENTITY, radius: 2.5 }, mark }
    }

    fn laid_ring(store: &DrawStoreRes, machine: &str) -> Option<(u64, gearbox_api::DrawEntity)> {
        let store = store.lock();
        let item = store.get(&ring_root(machine), "ring")?;
        Some((item.revision, item.entity.clone()))
    }

    #[test]
    fn rings_ride_the_base_link_and_change_only_with_their_mark() {
        let store = DrawStoreRes::default();
        let mut laid = HashMap::new();
        let wanted = |rings: &[(&str, Ring)]| rings.iter().map(|(id, r)| (id.to_string(), *r)).collect();
        let mark = |store: &DrawStoreRes| laid_ring(store, "tractor").and_then(|(_, e)| e.props().get("mark"));
        lay_rings(&store, &mut laid, wanted(&[("tractor", ring(RingMark::Selected))]));
        let (first, e) = laid_ring(&store, "tractor").expect("a selected machine is ringed");
        assert_eq!(mark(&store).as_deref(), Some("selected"));
        assert_eq!(e.frame(), gearbox_api::DrawFrame::Link("tractor".into(), String::new()));
        assert_eq!(([e.px, e.py, e.pz], e.qw, e.width), ([0.5, 0.0, -0.47f32 as f64], 1.0, 5.0));
        assert_ne!(e.flags & draw_flag::VIEW_ONLY, 0, "rings stay out of captures");
        assert_eq!(e.flags & draw_flag::DRAPE, 0, "a ring rides its link, not the terrain");

        for _ in 0..10 {
            lay_rings(&store, &mut laid, wanted(&[("tractor", ring(RingMark::Selected))]));
        }
        assert_eq!(laid_ring(&store, "tractor").unwrap().0, first, "the laid ring is never rebuilt");

        lay_rings(&store, &mut laid, wanted(&[("tractor", ring(RingMark::Following))]));
        assert_eq!(mark(&store).as_deref(), Some("following"));
        lay_rings(&store, &mut laid, wanted(&[("tractor", ring(RingMark::Hover))]));
        assert_eq!(mark(&store).as_deref(), Some("hover"));

        store.lock().clear("");
        lay_rings(&store, &mut laid, wanted(&[("tractor", ring(RingMark::Hover))]));
        assert!(laid_ring(&store, "tractor").is_some(), "a cleared ring comes back");

        lay_rings(&store, &mut laid, HashMap::new());
        assert!(laid_ring(&store, "tractor").is_none());
        assert!(laid.is_empty());
    }
}
