//! Frames: one per body, then every site, sensor, coupling, tool and link
//! prim of the machine, each owned by a body or another frame and placed
//! from composed world matrices. Also picks the control body, the reporting
//! base frame and the body-local control frame.

use std::collections::{BTreeSet, HashMap};

use super::compile::{Builder, leaf};
use super::diagnostics::{Code, Diagnostic};
use super::model::*;
use super::read;
use super::scene::Scene;

fn frame_role(b: &Builder, path: &str) -> Option<FrameRole> {
    let scene = b.scene;
    if scene.is_body(path) {
        return None;
    }
    if scene.has_schema(path, "GearboxCouplingAPI") || read::token(scene, path, "gearbox:coupling:side").is_some() {
        return Some(FrameRole::Coupling);
    }
    if scene.has_schema(path, "IsaacSiteAPI") {
        return Some(FrameRole::Site);
    }
    if read::token(scene, path, "gearbox:sensor:kind").is_some() {
        return Some(FrameRole::Sensor);
    }
    if scene.has_schema(path, "GearboxElementAPI") || read::token(scene, path, "gearbox:element:type").is_some() {
        return Some(FrameRole::Tool);
    }
    let marked = scene.has_schema(path, "GearboxLinkAPI")
        || read::token(scene, path, "gearbox:link:name").is_some()
        || read::token(scene, path, "gearbox:link:role").is_some();
    if !marked {
        return None;
    }
    Some(match read::token(scene, path, "gearbox:link:role").as_deref() {
        Some("sensor") => FrameRole::Sensor,
        Some("tool") => FrameRole::Tool,
        Some("base") => FrameRole::Base,
        _ => FrameRole::Link,
    })
}

fn frame_name(b: &Builder, path: &str) -> String {
    if let Some(name) = read::token(b.scene, path, "gearbox:link:name") {
        return name;
    }
    if b.profile == ProfileKind::Standard {
        leaf(path).to_string()
    } else {
        crate::links::sanitize_link_name(leaf(path))
    }
}

fn pick_control_body(b: &mut Builder) -> Option<usize> {
    if b.scope.explicit {
        let authored = read::first_target(b.scene, &b.scope.root, "gearbox:machine:body");
        if let Some(target) = authored {
            return match b.machine.body_index(&target) {
                Some(i) => Some(i),
                None => {
                    b.error(
                        Code::InvalidControlBody,
                        Some(&b.scope.root.clone()),
                        format!("gearbox:machine:body -> {target} is not a rigid body of this machine"),
                    );
                    None
                }
            };
        }
        let root = b.scope.root.clone();
        b.push(Diagnostic::error(Code::MissingControlBody, Some(&root), "a Gearbox machine requires gearbox:machine:body"));
        return None;
    }
    if let Some(i) = b.scope.markers.iter().find_map(|m| b.machine.body_index(m)) {
        return Some(i);
    }
    if b.scope.isaac
        && let Some(first) = read::targets(b.scene, &b.scope.root, "isaac:physics:robotLinks").and_then(|l| l.into_iter().next())
        && let Some(i) = b.machine.body_index(&first)
    {
        return Some(i);
    }
    if let Some(i) = b
        .machine
        .bodies
        .iter()
        .position(|body| read::token(b.scene, &body.path, "gearbox:link:role").as_deref() == Some("base"))
    {
        return Some(i);
    }
    let children: BTreeSet<usize> = b
        .machine
        .joints
        .iter()
        .filter(|j| j.class == JointClass::Tree && !j.exclude_from_articulation)
        .filter_map(|j| match j.bodies() {
            (Some(_), Some(c)) => Some(c),
            _ => None,
        })
        .collect();
    let anchored: Vec<usize> = b
        .machine
        .joints
        .iter()
        .filter(|j| j.class == JointClass::Tree && !j.exclude_from_articulation)
        .filter_map(|j| match j.bodies() {
            (None, Some(c)) | (Some(c), None) => Some(c),
            _ => None,
        })
        .collect();
    let roots: Vec<usize> = (0..b.machine.bodies.len()).filter(|i| !children.contains(i)).collect();
    anchored.first().copied().or_else(|| (roots.len() == 1).then(|| roots[0])).or_else(|| roots.first().copied())
}

/// A frame prim authored by relationship on the machine root: existing frame,
/// body frame, or a new frame owned by its nearest body or frame ancestor.
fn frame_for_target(b: &mut Builder, target: &str, role: FrameRole) -> Result<usize, String> {
    if let Some(i) = b.machine.frame_index(target) {
        return Ok(i);
    }
    if !b.scene.exists(target) {
        return Err(format!("{target} does not exist or is inactive"));
    }
    let owner = owner_of(b, target, None).ok_or_else(|| format!("{target} rides no body of this machine"))?;
    let transform = relative(b, owner, target);
    b.machine.frames.push(FrameDescription {
        path: target.to_string(),
        name: frame_name(b, target),
        owner,
        transform,
        role,
        publish: true,
    });
    Ok(b.machine.frames.len() - 1)
}

fn owner_path(b: &Builder, owner: FrameOwner) -> String {
    match owner {
        FrameOwner::Body(i) => b.machine.bodies[i].path.clone(),
        FrameOwner::Frame(i) => b.machine.frames[i].path.clone(),
    }
}

fn relative(b: &Builder, owner: FrameOwner, path: &str) -> RigidTransform {
    let parent = b.scene.rigid_world(&owner_path(b, owner));
    parent.inverse().mul(&b.scene.rigid_world(path))
}

/// Nearest ancestor that is a body or an already-built frame.
fn owner_of(b: &Builder, path: &str, skip: Option<usize>) -> Option<FrameOwner> {
    for ancestor in Scene::ancestors(path) {
        if let Some(i) = b.machine.body_index(ancestor) {
            return Some(FrameOwner::Body(i));
        }
        if let Some(i) = b.machine.frame_index(ancestor).filter(|i| Some(*i) != skip) {
            return Some(FrameOwner::Frame(i));
        }
    }
    None
}

pub(super) fn build(b: &mut Builder) {
    for (i, body) in b.machine.bodies.clone().iter().enumerate() {
        b.machine.frames.push(FrameDescription {
            path: body.path.clone(),
            name: body.name.clone(),
            owner: FrameOwner::Body(i),
            transform: RigidTransform::IDENTITY,
            role: FrameRole::Body,
            publish: true,
        });
    }
    let root = b.scope.root.clone();
    let candidates: Vec<(String, FrameRole)> = b
        .scene
        .prims
        .iter()
        .filter(|p| Scene::is_under(&p.path, &root) && p.path != root)
        .filter_map(|p| frame_role(b, &p.path).map(|role| (p.path.clone(), role)))
        .collect();
    let first_site = b.machine.frames.len();
    for (path, role) in &candidates {
        b.machine.frames.push(FrameDescription {
            path: path.clone(),
            name: frame_name(b, path),
            owner: FrameOwner::Body(usize::MAX),
            transform: RigidTransform::IDENTITY,
            role: *role,
            publish: true,
        });
    }

    let control = pick_control_body(b);
    b.machine.control_body = control;

    let mut owners: HashMap<usize, FrameOwner> = HashMap::new();
    for index in first_site..b.machine.frames.len() {
        let path = b.machine.frames[index].path.clone();
        let parent = read::first_target(b.scene, &path, "gearbox:link:parent");
        let owner = match parent {
            Some(target) => match (b.machine.body_index(&target), b.machine.frame_index(&target)) {
                (Some(i), _) => Some(FrameOwner::Body(i)),
                (None, Some(i)) if i != index => Some(FrameOwner::Frame(i)),
                _ => {
                    b.error(Code::ConflictingParent, Some(&path), format!("gearbox:link:parent -> {target} is not a body or frame of this machine"));
                    owner_of(b, &path, Some(index))
                }
            },
            None => owner_of(b, &path, Some(index)),
        };
        let owner = match (owner, control) {
            (Some(owner), _) => owner,
            (None, Some(control)) => {
                b.push(Diagnostic::warning(Code::UnanchoredFrame, Some(&path), "frame rides no body; held fixed to the control body"));
                FrameOwner::Body(control)
            }
            (None, None) => {
                b.push(Diagnostic::warning(Code::UnanchoredFrame, Some(&path), "frame rides no body and the machine has no control body"));
                continue;
            }
        };
        owners.insert(index, owner);
    }
    for (index, owner) in &owners {
        b.machine.frames[*index].owner = *owner;
    }
    let dangling: Vec<usize> = (first_site..b.machine.frames.len()).filter(|i| !owners.contains_key(i)).collect();
    for index in dangling.into_iter().rev() {
        b.machine.frames.remove(index);
        for frame in &mut b.machine.frames {
            if let FrameOwner::Frame(f) = &mut frame.owner
                && *f > index
            {
                *f -= 1;
            }
        }
    }
    for index in first_site..b.machine.frames.len() {
        if b.machine.frame_body(index).is_none() {
            let path = b.machine.frames[index].path.clone();
            b.error(Code::ConflictingParent, Some(&path), "frame parents form a cycle");
            if let Some(control) = control {
                b.machine.frames[index].owner = FrameOwner::Body(control);
            }
        }
        let owner = b.machine.frames[index].owner;
        let path = b.machine.frames[index].path.clone();
        b.machine.frames[index].transform = relative(b, owner, &path);
        let d = b.scene.decomposed(&path);
        if d.shear || d.reflection {
            b.push(Diagnostic::warning(Code::ShearTransform, Some(&path), "frame transform is sheared or mirrored; only its rigid part is used"));
        }
    }

    base_and_control_frames(b, control);
}

fn base_and_control_frames(b: &mut Builder, control: Option<usize>) {
    let root = b.scope.root.clone();
    let Some(control) = control else { return };
    let rides_control = |b: &Builder, frame: usize| b.machine.frame_body(frame) == Some(control);

    let base = match (b.profile, read::first_target(b.scene, &root, "gearbox:machine:baseFrame")) {
        (ProfileKind::Gearbox, Some(target)) => match frame_for_target(b, &target, FrameRole::Base) {
            Ok(i) if rides_control(b, i) => Some(i),
            Ok(_) => {
                b.push(Diagnostic::error(Code::InvalidBaseFrame, Some(&root), format!("gearbox:machine:baseFrame -> {target} does not ride the control body")));
                None
            }
            Err(e) => {
                b.push(Diagnostic::error(Code::InvalidBaseFrame, Some(&root), format!("gearbox:machine:baseFrame: {e}")));
                None
            }
        },
        _ => None,
    };
    b.machine.base_frame = Some(base.unwrap_or(control));
    if let Some(base) = base
        && b.machine.frames[base].role != FrameRole::Body
    {
        b.machine.frames[base].role = FrameRole::Base;
    }

    b.machine.control_frame = match b.profile {
        ProfileKind::Gearbox => match read::first_target(b.scene, &root, "gearbox:machine:controlFrame") {
            Some(target) => match frame_for_target(b, &target, FrameRole::Control) {
                Ok(i) if rides_control(b, i) => Some(ControlFrame { frame: i, source: Provenance::Authored }),
                Ok(_) => {
                    b.push(Diagnostic::error(Code::InvalidControlFrame, Some(&root), format!("gearbox:machine:controlFrame -> {target} does not ride the control body")));
                    None
                }
                Err(e) => {
                    b.push(Diagnostic::error(Code::InvalidControlFrame, Some(&root), format!("gearbox:machine:controlFrame: {e}")));
                    None
                }
            },
            None => b.machine.base_frame.map(|frame| ControlFrame { frame, source: Provenance::ProfileDefault }),
        },
        ProfileKind::Standard => None,
    };
}

/// Public names are unique within a machine.
pub(super) fn check_names(b: &mut Builder) {
    let mut dupes: Vec<(String, String)> = Vec::new();
    let mut check = |kind: &str, names: Vec<(String, String)>| {
        let mut seen: HashMap<String, String> = HashMap::new();
        for (name, path) in names {
            if let Some(first) = seen.insert(name.clone(), path.clone()) {
                dupes.push((path, format!("{kind} name `{name}` is also used by {first}")));
            }
        }
    };
    check("frame", b.machine.frames.iter().filter(|f| f.publish).map(|f| (f.name.clone(), f.path.clone())).collect());
    check("joint", b.machine.joints.iter().map(|j| (j.name.clone(), j.path.clone())).collect());
    check("dof", b.machine.dofs.iter().map(|d| (d.name.clone(), b.machine.joints[d.joint].path.clone())).collect());
    for (path, message) in dupes {
        b.error(Code::DuplicateName, Some(&path), message);
    }
}
