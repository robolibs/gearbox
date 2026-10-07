//! Soil tools working the ground.
//!
//! A machine whose `gearbox:machine:kind` names a soil tool — a disc harrow —
//! works the field where its soil-engaging parts touch it. Those parts are
//! the prims named for them (its discs), and what says a part is working is
//! its collider against the ground: a physics contact, or the collider within
//! a few centimetres of the surface between the solver's contacts as the
//! frame bounces over it. A part lifted clear works nothing, and on a turn
//! each part works the path it takes itself, the outer ones further than the
//! inner. A part works the ground from itself halfway to its neighbours in
//! its own gang: one gang of angled discs cuts the whole width.

use std::collections::HashMap;

use bevy::prelude::*;
use gearbox_fields::{ToolContact, ToolContacts, ToolKind};
use usd_bevy::UsdPrimRef;

use super::ControllerInventory;
use crate::physics::PhysicsWorld;
use crate::physics::backend::ColliderId;

/// Slower than this along the ground and the tool is taken as standing.
const MOVING_M_S: f32 = 0.15;
/// How far the tool has to get before its speed is measured again: far
/// enough that a tool settling in place, shaking back and forth a few
/// centimetres, is never taken as going anywhere.
const STRIDE_M: f32 = 0.2;
/// Longer than this to cover a stride and the tool has stopped.
const STALL_S: f32 = 1.5;
/// Further than this in one frame is a jump — the tool put somewhere — not
/// travel over the ground.
const JUMP_M: f32 = 1.0;
/// How deep a disc harrow works.
const DISC_DEPTH_M: f32 = 0.10;
/// A part's collider this close to the ground is still in it.
const TOUCH_M: f32 = 0.05;
/// How far along the ground a disc cuts either side of its middle.
const CUT_M: f32 = 0.15;
/// How far across a part works at most, either side of itself.
const REACH_M: f32 = 0.3;
/// Parts further apart than this along the way the tool goes stand in
/// different gangs.
const GANG_GAP_M: f32 = 0.2;
/// A ground contact pushes the part at least this upright.
const UPRIGHT: f64 = 0.3;
/// How much of the sideways wander of the tool's middle each frame keeps: the
/// ridges are laid out from it, and would wobble with the physics settling.
const STEADY_SIDEWAYS: f32 = 0.9;

/// What a tool remembers between frames: its working parts' colliders, where
/// each part in the soil was, the tool's middle, where and when its last
/// stride began, and the line its parts stand across.
#[derive(Default)]
pub(super) struct ToolTrack {
    parts: Vec<(Entity, ColliderId)>,
    in_soil: HashMap<ColliderId, Vec2>,
    middle: Option<Vec2>,
    stride: Option<(Vec2, f32)>,
    moving: bool,
    axle: Option<Vec2>,
}

/// The names a soil-engaging part goes by.
fn works_soil(prim_path: &str) -> bool {
    let leaf = prim_path.rsplit('/').next().unwrap_or(prim_path).to_ascii_lowercase();
    leaf.contains("disc") || leaf.contains("tine") || leaf.contains("share")
}

/// Every descendant of `root`, the root included.
fn descendants(root: Entity, children: &Query<&Children>) -> Vec<Entity> {
    let mut out = vec![root];
    let mut at = 0;
    while at < out.len() {
        if let Ok(kids) = children.get(out[at]) {
            out.extend(kids.iter());
        }
        at += 1;
    }
    out
}

/// The way the points spread furthest, through their middle: the line a row
/// of parts stands across. Turned to agree with `held`, so it never flips.
fn spread(points: &[Vec2], middle: Vec2, held: Option<Vec2>) -> Option<Vec2> {
    let (mut xx, mut xz, mut zz) = (0.0, 0.0, 0.0);
    for point in points {
        let d = *point - middle;
        xx += d.x * d.x;
        xz += d.x * d.y;
        zz += d.y * d.y;
    }
    if xx + zz < 1.0e-6 {
        return None;
    }
    let axis = Vec2::from_angle(0.5 * (2.0 * xz).atan2(xx - zz));
    Some(if held.is_some_and(|held| held.dot(axis) < 0.0) { -axis } else { axis })
}

/// Whether the physics has a part's collider on the ground: a contact with
/// anything that is not a moving body, pushing the part up.
fn on_ground(physics: &PhysicsWorld, collider: ColliderId) -> bool {
    physics.contacts_with(collider).iter().any(|manifold| {
        let (other, toward) = if manifold.collider1 == collider {
            (manifold.collider2, -1.0)
        } else {
            (manifold.collider1, 1.0)
        };
        let moving = physics
            .collider(other)
            .and_then(|other| other.parent())
            .and_then(|body| physics.body(body))
            .is_some_and(|body| body.is_dynamic());
        !moving
            && (manifold.normal * toward).y >= UPRIGHT
            && manifold.points.iter().any(|point| point.dist <= 0.01)
    })
}

/// How far either side of each part it works, across: halfway to its
/// neighbours in its own gang, an end part as far out as in, and never
/// beyond `REACH_M`. Gangs are told apart by where the parts lie along.
fn bands(across: &[f32], along: &[f32]) -> Vec<(f32, f32)> {
    let mut by_along: Vec<usize> = (0..along.len()).collect();
    by_along.sort_by(|a, b| along[*a].total_cmp(&along[*b]));
    let mut out = vec![(REACH_M, REACH_M); across.len()];
    for gang in by_along.chunk_by(|a, b| along[*b] - along[*a] <= GANG_GAP_M) {
        let mut order = gang.to_vec();
        order.sort_by(|a, b| across[*a].total_cmp(&across[*b]));
        for (rank, &part) in order.iter().enumerate() {
            let gap = |other: Option<&usize>| other.map(|other| (across[*other] - across[part]).abs() * 0.5);
            let before = rank.checked_sub(1).and_then(|rank| order.get(rank));
            let (low, high) = (gap(before), gap(order.get(rank + 1)));
            let low = low.or(high).unwrap_or(REACH_M).min(REACH_M);
            let high = high.unwrap_or(low).min(REACH_M);
            out[part] = (low, high);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_soil_work(
    inventory: Res<ControllerInventory>,
    active: Res<gearbox_api::PhysicsActive>,
    physics: Res<PhysicsWorld>,
    time: Res<Time>,
    prims: Query<&UsdPrimRef>,
    children: Query<&Children>,
    transforms: Query<&GlobalTransform>,
    heights: Option<Res<gearbox_fields::CoverHeights>>,
    mut contacts: ResMut<ToolContacts>,
    mut tracks: Local<HashMap<String, ToolTrack>>,
) {
    if !active.0 {
        return;
    }
    let Some(heights) = heights else {
        return;
    };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut seen = Vec::new();
    for machine in &inventory.machines {
        let (Some(scene_root), Some(kind)) = (
            machine.scene_root,
            machine.kind.as_deref().and_then(ToolKind::of_machine),
        ) else {
            continue;
        };
        seen.push(machine.id.clone());
        let track = tracks.entry(machine.id.clone()).or_default();
        // The working parts' colliders, found once and again if one is gone.
        if track.parts.is_empty() || track.parts.iter().any(|(_, collider)| physics.collider(*collider).is_none()) {
            let mut parts: Vec<(Entity, ColliderId)> = descendants(scene_root, &children)
                .into_iter()
                .filter(|entity| {
                    prims.get(*entity).is_ok_and(|prim| {
                        prim.path.starts_with(&machine.prim_path) && works_soil(&prim.path)
                    })
                })
                .flat_map(|part| descendants(part, &children))
                .filter_map(|entity| physics.entity_to_collider.get(&entity).map(|collider| (entity, *collider)))
                .collect();
            parts.sort_unstable_by_key(|(entity, _)| *entity);
            parts.dedup();
            track.parts = parts;
            track.in_soil.clear();
        }
        let places: Vec<Vec2> = track
            .parts
            .iter()
            .filter_map(|(entity, _)| transforms.get(*entity).ok())
            .map(|transform| transform.translation().xz())
            .collect();
        if places.len() != track.parts.len() || places.is_empty() {
            continue;
        }
        let middle = places.iter().sum::<Vec2>() / places.len() as f32;
        let Some(mut axle) = spread(&places, middle, track.axle) else {
            continue;
        };
        let now = time.elapsed_secs();
        let previous = track.middle.replace(middle);
        let Some(previous) = previous.filter(|previous| previous.distance(middle) <= JUMP_M) else {
            track.stride = Some((middle, now));
            track.moving = false;
            track.in_soil.clear();
            track.axle = Some(axle);
            continue;
        };
        // Whether the tool is going anywhere is measured a stride at a time;
        // which way it goes turns the line its parts stand across forward.
        let (start, since) = *track.stride.get_or_insert((middle, now));
        let travel = middle - start;
        if travel.length() >= STRIDE_M {
            track.moving = travel.length() / (now - since).max(dt) >= MOVING_M_S;
            if axle.perp().dot(travel) < 0.0 {
                axle = -axle;
            }
            track.stride = Some((middle, now));
        } else if now - since > STALL_S {
            track.moving = false;
            track.stride = Some((middle, now));
        }
        track.axle = Some(axle);
        if !track.moving {
            track.in_soil.clear();
            continue;
        }
        let heading = axle.perp();
        // The ridges are laid out across from a steady middle line: what
        // wanders sideways frame to frame is the physics settling.
        let moved = middle - previous;
        let steady = previous + heading * moved.dot(heading) + axle * moved.dot(axle) * (1.0 - STEADY_SIDEWAYS);
        track.middle = Some(steady);
        let across: Vec<f32> = places.iter().map(|place| (*place - steady).dot(axle)).collect();
        let along: Vec<f32> = places.iter().map(|place| (*place - steady).dot(heading)).collect();
        let bands = bands(&across, &along);
        // Each part in the soil works from where it was last frame to where
        // it is now, and as far again as its disc cuts.
        let mut in_soil = HashMap::new();
        let first = contacts.contacts.len();
        let mut edges = Vec2::new(f32::MAX, f32::MIN);
        for (index, (_, collider)) in track.parts.iter().enumerate() {
            let place = places[index];
            let Some(bottom) = physics.collider(*collider).map(|collider| collider.aabb().mins.y as f32) else {
                continue;
            };
            let ground = heights.0.height(place.x, place.y);
            if bottom - ground > TOUCH_M && !on_ground(&physics, *collider) {
                continue;
            }
            in_soil.insert(*collider, place);
            let back = track
                .in_soil
                .get(collider)
                .filter(|last| last.distance(place) <= JUMP_M)
                .map_or(0.0, |last| (place - *last).dot(heading).max(0.0));
            let (low, high) = bands[index];
            let side = (high - low) * 0.5;
            let centre = place - heading * back * 0.5 + axle * side;
            edges = Vec2::new(edges.x.min(across[index] - low), edges.y.max(across[index] + high));
            contacts.contacts.push(ToolContact {
                kind,
                position: Vec3::new(centre.x, ground, centre.y),
                direction: heading,
                across: across[index] + side,
                width: low + high,
                edges: Vec2::ZERO,
                length: back + CUT_M * 2.0,
                depth: DISC_DEPTH_M,
            });
        }
        // The strip's edges are those of the outermost parts in the soil.
        for contact in &mut contacts.contacts[first..] {
            contact.edges = edges;
        }
        track.in_soil = in_soil;
    }
    tracks.retain(|id, _| seen.contains(id));
}

#[cfg(test)]
mod tests {
    use super::{REACH_M, bands, spread, works_soil};
    use bevy::prelude::Vec2;

    #[test]
    fn discs_tines_and_shares_work_the_soil_and_nothing_else_does() {
        assert!(works_soil("/robot/chassis/visual_074_centerDiscs_part00/centerDiscs_part00_001"));
        assert!(works_soil("/robot/chassis/extraDisc_001"));
        assert!(works_soil("/robot/left_tine_carrier/tine_04"));
        assert!(!works_soil("/robot/chassis/frame_beam"));
        // Only the prim's own name counts, not a parent's.
        assert!(!works_soil("/robot/disc_frame/bolt_02"));
    }

    #[test]
    fn each_part_works_halfway_to_its_neighbours_in_its_own_gang() {
        let bands = bands(&[0.5, -0.5, 0.0, 2.0, 0.25], &[0.4, 0.4, 0.4, 0.4, -0.4]);
        assert_eq!(bands[2], (0.25, 0.25));
        assert_eq!(bands[1], (0.25, 0.25), "an end part reaches out as far as in");
        assert_eq!(bands[0], (0.25, REACH_M), "a wide gap is not filled");
        assert_eq!(bands[3], (REACH_M, REACH_M));
        assert_eq!(bands[4], (REACH_M, REACH_M), "the other gang's parts are not neighbours");
    }

    #[test]
    fn a_row_of_parts_stands_across_the_line_it_spreads_along() {
        let row: Vec<Vec2> = (0..12)
            .map(|i| Vec2::new(i as f32 * 0.5, 0.3 * (i % 2) as f32))
            .map(|p| Vec2::from_angle(0.4).rotate(p))
            .collect();
        let middle = row.iter().sum::<Vec2>() / row.len() as f32;
        let axle = spread(&row, middle, None).unwrap();
        assert!(axle.dot(Vec2::from_angle(0.4)).abs() > 0.99, "{axle}");
        let held = spread(&row, middle, Some(-axle)).unwrap();
        assert!(held.dot(-axle) > 0.99, "keeps to the way it was held");
    }
}
