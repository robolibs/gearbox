//! The physical joint graph and the reporting tree. Every enabled joint is
//! physical; closures stay physical but never parent a frame; a cycle of
//! joints nobody excluded is rejected rather than broken by prim order.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::compile::Builder;
use super::diagnostics::{Code, Diagnostic};
use super::model::*;
use super::read;

pub(super) fn build(b: &mut Builder) {
    classify(b);
    reporting_tree(b);
    articulations(b);
}

fn classify(b: &mut Builder) {
    let n = b.machine.bodies.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for index in 0..b.machine.joints.len() {
        let joint = &b.machine.joints[index];
        if joint.class != JointClass::Tree {
            continue;
        }
        let class = match joint.bodies() {
            _ if joint.exclude_from_articulation => JointClass::Closure,
            (Some(_), None) | (None, Some(_)) => JointClass::WorldAnchor,
            _ => JointClass::Tree,
        };
        b.machine.joints[index].class = class;
        if class != JointClass::Tree {
            continue;
        }
        let (Some(a), Some(c)) = b.machine.joints[index].bodies() else { continue };
        let (ra, rc) = (find(&mut parent, a), find(&mut parent, c));
        if ra == rc {
            let path = b.machine.joints[index].path.clone();
            b.push(Diagnostic::error(
                Code::UnclassifiedCycle,
                Some(&path),
                "joint closes a kinematic loop; mark one joint of the loop physics:excludeFromArticulation",
            ));
        } else {
            parent[ra.max(rc)] = ra.min(rc);
        }
    }
}

fn reporting_tree(b: &mut Builder) {
    let Some(control) = b.machine.control_body else { return };
    let Some(base) = b.machine.base_frame else { return };
    let mut edges = Vec::new();
    if base != control {
        edges.push(TreeEdge { parent: base, child: control, joint: None, reversed: true });
    }

    let mut adjacency: BTreeMap<usize, Vec<(usize, usize, bool)>> = BTreeMap::new();
    for (index, joint) in b.machine.joints.iter().enumerate() {
        if joint.class != JointClass::Tree {
            continue;
        }
        if let (Some(p), Some(c)) = joint.bodies() {
            adjacency.entry(p).or_default().push((index, c, false));
            adjacency.entry(c).or_default().push((index, p, true));
        }
    }
    let mut seen: BTreeSet<usize> = BTreeSet::from([control]);
    let mut queue = VecDeque::from([control]);
    while let Some(body) = queue.pop_front() {
        let mut next = adjacency.get(&body).cloned().unwrap_or_default();
        next.sort_by(|a, c| b.machine.joints[a.0].path.cmp(&b.machine.joints[c.0].path));
        for (joint, other, reversed) in next {
            if seen.insert(other) {
                edges.push(TreeEdge { parent: body, child: other, joint: Some(joint), reversed });
                queue.push_back(other);
            }
        }
    }

    for (i, body) in b.machine.bodies.clone().iter().enumerate() {
        if seen.contains(&i) {
            continue;
        }
        let anchored = b.machine.joints.iter().any(|j| j.class == JointClass::WorldAnchor && (j.bodies().0 == Some(i) || j.bodies().1 == Some(i)));
        let closed = b.machine.joints.iter().any(|j| j.class == JointClass::Closure && (j.bodies().0 == Some(i) || j.bodies().1 == Some(i)));
        let how = if anchored {
            "anchored to the world apart from the control body"
        } else if closed {
            "joined only by loop-closure joints"
        } else {
            "joined to nothing"
        };
        b.push(Diagnostic::warning(
            Code::DisconnectedBody,
            Some(&body.path),
            format!("body is outside the reporting tree: {how}"),
        ));
    }

    for (index, joint) in b.machine.joints.clone().iter().enumerate() {
        if joint.class != JointClass::Tree {
            continue;
        }
        let Some(child) = edges.iter().find(|e| e.joint == Some(index)).map(|e| e.child) else { continue };
        let path = b.machine.bodies[child].path.clone();
        if read::first_target(b.scene, &path, "gearbox:link:parent").is_some() {
            b.error(
                Code::ConflictingParent,
                Some(&path),
                format!("gearbox:link:parent declares a static parent for a body joint {} moves", joint.path),
            );
        }
    }

    for (index, frame) in b.machine.frames.iter().enumerate() {
        if index == base || frame.role == FrameRole::Body {
            continue;
        }
        let parent = match frame.owner {
            FrameOwner::Body(i) => i,
            FrameOwner::Frame(i) => i,
        };
        if frame.publish && b.machine.frame_body(index).is_some_and(|body| seen.contains(&body)) {
            edges.push(TreeEdge { parent, child: index, joint: None, reversed: false });
        }
    }
    b.machine.tree = edges;
}

fn articulations(b: &mut Builder) {
    let n = b.machine.bodies.len();
    let mut group: Vec<usize> = (0..n).collect();
    fn find(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for joint in &b.machine.joints {
        if joint.class == JointClass::Tree
            && let (Some(a), Some(c)) = joint.bodies()
        {
            let (ra, rc) = (find(&mut group, a), find(&mut group, c));
            group[ra.max(rc)] = ra.min(rc);
        }
    }
    let mut groups: BTreeMap<usize, ArticulationGroup> = BTreeMap::new();
    for i in 0..n {
        let root = find(&mut group, i);
        groups
            .entry(root)
            .or_insert_with(|| ArticulationGroup { markers: Vec::new(), bodies: Vec::new(), joints: Vec::new(), fixed_base: false })
            .bodies
            .push(i);
    }
    for (index, joint) in b.machine.joints.iter().enumerate() {
        let Some(body) = joint.bodies().0.or(joint.bodies().1) else { continue };
        let root = find(&mut group, body);
        let entry = groups.get_mut(&root).expect("every body has a group");
        match joint.class {
            JointClass::Tree => entry.joints.push(index),
            JointClass::WorldAnchor => {
                entry.joints.push(index);
                entry.fixed_base = true;
            }
            _ => {}
        }
    }
    for marker in &b.scope.markers {
        let marked: Vec<usize> = if let Some(i) = b.machine.body_index(marker) {
            vec![i]
        } else if let Some(j) = b.machine.joint_index(marker) {
            [b.machine.joints[j].bodies().0, b.machine.joints[j].bodies().1].into_iter().flatten().collect()
        } else {
            b.machine
                .bodies
                .iter()
                .enumerate()
                .filter(|(_, body)| super::scene::Scene::is_under(&body.path, marker))
                .map(|(i, _)| i)
                .collect()
        };
        let roots: BTreeSet<usize> = marked.iter().map(|&i| find(&mut group, i)).collect();
        for root in roots {
            if let Some(g) = groups.get_mut(&root)
                && !g.markers.contains(marker)
            {
                g.markers.push(marker.clone());
            }
        }
    }
    b.machine.articulations = groups.into_values().filter(|g| !g.joints.is_empty() || !g.markers.is_empty()).collect();
}
