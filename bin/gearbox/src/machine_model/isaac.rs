//! Isaac robot-schema metadata read without Isaac: `IsaacRobotAPI`
//! reporting lists, `IsaacJointAPI` per-DOF arrays. Lists may be subsets of
//! the physical graph; nothing they leave out is dropped, and an actuator
//! flag grants no command interface.

use super::compile::Builder;
use super::diagnostics::{Code, Diagnostic};
use super::model::*;
use super::read;
use super::scene::Scene;

/// Bodies a `robotLinks` entry names: a body, a site's body, or every body
/// of a listed sub-assembly.
fn listed_bodies(b: &Builder, target: &str) -> Option<Vec<usize>> {
    if let Some(i) = b.machine.body_index(target) {
        return Some(vec![i]);
    }
    if b.scene.has_schema(target, "IsaacRobotAPI") || !b.scene.is_joint(target) {
        let inside: Vec<usize> = b
            .machine
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, body)| Scene::is_under(&body.path, target))
            .map(|(i, _)| i)
            .collect();
        if !inside.is_empty() {
            return Some(inside);
        }
        return b.scene.nearest_body_ancestor(target).and_then(|a| b.machine.body_index(&a)).map(|i| vec![i]);
    }
    None
}

pub(super) fn apply(b: &mut Builder) {
    let root = b.scope.root.clone();
    if b.scene.has_schema(&root, "IsaacRobotAPI") {
        let mut order = ReportingOrder::default();
        for target in read::targets(b.scene, &root, "isaac:physics:robotLinks").unwrap_or_default() {
            match listed_bodies(b, &target) {
                Some(bodies) => {
                    for body in bodies {
                        if !order.bodies.contains(&body) {
                            order.bodies.push(body);
                        }
                    }
                }
                None if b.scene.exists(&target) => b.push(Diagnostic::warning(
                    Code::IsaacOutsideScope,
                    Some(&target),
                    "isaac:physics:robotLinks entry is not a body of this robot",
                )),
                None => b.push(Diagnostic::error(Code::IsaacUnknownTarget, Some(&target), "isaac:physics:robotLinks entry does not exist")),
            }
        }
        for target in read::targets(b.scene, &root, "isaac:physics:robotJoints").unwrap_or_default() {
            let joints: Vec<usize> = match b.machine.joint_index(&target) {
                Some(j) => vec![j],
                None => b
                    .machine
                    .joints
                    .iter()
                    .enumerate()
                    .filter(|(_, j)| Scene::is_under(&j.path, &target) && target != j.path)
                    .map(|(i, _)| i)
                    .collect(),
            };
            if joints.is_empty() {
                let code = if b.scene.exists(&target) { Code::IsaacOutsideScope } else { Code::IsaacUnknownTarget };
                b.push(Diagnostic::warning(code, Some(&target), "isaac:physics:robotJoints entry is not a joint of this robot"));
            }
            for joint in joints {
                if !order.joints.contains(&joint) {
                    order.joints.push(joint);
                }
            }
        }
        b.machine.reporting = Some(order);
    }

    for index in 0..b.machine.joints.len() {
        let path = b.machine.joints[index].path.clone();
        let dofs = b.machine.joints[index].dofs.clone();
        let order = read::tokens(b.scene, &path, "isaac:physics:DofOffsetOpOrder");
        let ordered: Vec<usize> = match &order {
            Some(tokens) if !tokens.is_empty() => {
                let mut out = Vec::new();
                for token in tokens {
                    match dofs.iter().find(|&&d| b.machine.dofs[d].token.as_str().eq_ignore_ascii_case(token)) {
                        Some(&d) => out.push(d),
                        None => b.push(Diagnostic::error(
                            Code::IsaacArrayLength,
                            Some(&path),
                            format!("isaac:physics:DofOffsetOpOrder names `{token}`, which is not a free DOF of this joint"),
                        )),
                    }
                }
                if out.len() != dofs.len() {
                    b.push(Diagnostic::error(
                        Code::IsaacArrayLength,
                        Some(&path),
                        format!("isaac:physics:DofOffsetOpOrder lists {} DOFs, the joint has {}", tokens.len(), dofs.len()),
                    ));
                }
                out
            }
            _ => dofs.clone(),
        };
        if let Some(flags) = read::bools(b.scene, &path, "isaac:actuator") {
            if flags.len() != dofs.len() {
                b.push(Diagnostic::error(
                    Code::IsaacArrayLength,
                    Some(&path),
                    format!("isaac:actuator has {} entries, the joint has {} DOFs", flags.len(), dofs.len()),
                ));
            } else {
                for (dof, flag) in ordered.iter().zip(flags) {
                    b.machine.dofs[*dof].isaac_actuator = Some(flag);
                }
            }
        }
    }
}
