//! Controller bindings to exact DOFs, the default owner of every commanded
//! DOF, and the command interfaces each DOF admits. Standard machines get no
//! controller: name-based role guesses become diagnostics only.

use std::collections::BTreeMap;

use super::compile::Builder;
use super::diagnostics::{Code, Diagnostic};
use super::model::*;
use super::read;

const DRIVE_TYPES: [&str; 3] = ["builtin:ackermann_cmd_vel", "builtin:diff_drive_cmd_vel", "builtin:tracked_cmd_vel"];

const CONTROLLER_ROLES: [(&str, &str); 11] = [
    ("driveWheelJoints", "drive_wheels"),
    ("passiveWheelJoints", "passive_wheels"),
    ("wheelJoints", "wheels"),
    ("steerJoints", "steer"),
    ("steerLeftJoint", "steer"),
    ("steerRightJoint", "steer"),
    ("frontLeftWheelJoint", "corner_wheels"),
    ("frontRightWheelJoint", "corner_wheels"),
    ("rearLeftWheelJoint", "corner_wheels"),
    ("rearRightWheelJoint", "corner_wheels"),
    ("target", "target"),
];

const MACHINE_ROLES: [(&str, &str); 6] = [
    ("poweredWheelJoints", "powered_wheels"),
    ("passiveWheelJoints", "passive_wheels"),
    ("steeringJoints", "steering"),
    ("brakeJoints", "brake"),
    ("suspensionJoints", "suspension"),
    ("toolJoints", "tool"),
];

/// The DOFs `targets` name. An unresolved target of a disabled controller,
/// which never runs, only warns.
fn resolve_dofs(b: &mut Builder, source: &str, targets: Vec<String>, enabled: bool) -> Vec<DofRef> {
    let mut out = Vec::new();
    for target in targets {
        let path = b.resolve_target(&target);
        match b.machine.dof(&path, None) {
            Some(dof) => {
                if !out.contains(&dof) {
                    out.push(dof);
                }
            }
            None => {
                let why = if b.machine.joint_index(&path).is_some() { "is not a single-DOF joint" } else { "is not a joint of this machine" };
                let message = format!("{source}: {target} {why}");
                if enabled {
                    b.strict_or_warn(Code::UnresolvedControllerJoint, Some(&path), message);
                } else {
                    b.push(Diagnostic::warning(Code::UnresolvedControllerJoint, Some(&path), format!("{message} (controller disabled)")));
                }
            }
        }
    }
    out
}

fn interfaces_for(controller_type: &str, role: &str) -> Vec<Interface> {
    match (controller_type, role) {
        (t, "steer" | "steering") if DRIVE_TYPES.contains(&t) => vec![Interface::Position],
        (t, _) if DRIVE_TYPES.contains(&t) => vec![Interface::Velocity],
        ("builtin:hitch" | "builtin:joint_position" | "builtin:trailer_steer", _) => vec![Interface::Position],
        ("builtin:pto" | "builtin:hydraulic_valve" | "builtin:joint_velocity", _) => vec![Interface::Velocity],
        ("device", _) => Interface::ALL.to_vec(),
        _ => Vec::new(),
    }
}

pub(super) fn build(b: &mut Builder, physics_rate_hz: f64) {
    if !b.scope.explicit {
        suggest_roles(b);
        validate_interfaces(b);
        return;
    }
    let root = b.scope.root.clone();
    let mut machine_roles: BTreeMap<String, Vec<DofRef>> = BTreeMap::new();
    for (rel, role) in MACHINE_ROLES {
        let name = format!("gearbox:machine:role:{rel}");
        if let Some(targets) = read::targets(b.scene, &root, &name) {
            let dofs = resolve_dofs(b, &name, targets, true);
            machine_roles.insert(role.to_string(), dofs);
        }
    }

    let Some(sdf) = b.scene.sdf(&root) else { return };
    let schemas = b.scene.info(&root).map(|p| p.schemas.clone()).unwrap_or_default();
    let specs = crate::controller::discover_controllers(b.scene.stage, &sdf, &schemas, &b.machine.machine_id.clone(), &root);
    for spec in specs {
        let prefix = format!("gearbox:controller:{}:", spec.instance);
        let mut roles: BTreeMap<String, Vec<DofRef>> = BTreeMap::new();
        let mut steer_joints: Vec<DofRef> = Vec::new();
        for (rel, role) in CONTROLLER_ROLES {
            if let Some(targets) = read::targets(b.scene, &root, &format!("{prefix}{rel}")) {
                if role == "target" && !crate::services::SERVICE_TYPES.contains(&spec.controller_type.as_str()) {
                    continue;
                }
                let dofs = resolve_dofs(b, &format!("{prefix}{rel}"), targets, spec.enabled);
                if rel == "steerJoints" {
                    steer_joints = dofs.clone();
                }
                let entry = roles.entry(role.to_string()).or_default();
                for dof in dofs {
                    if !entry.contains(&dof) {
                        entry.push(dof);
                    }
                }
            }
        }
        let is_drive = DRIVE_TYPES.contains(&spec.controller_type.as_str());
        let is_service = crate::services::SERVICE_TYPES.contains(&spec.controller_type.as_str());
        // A drive controller owns its own wheel and steer joints together
        // with the machine's roles: the runtime drives both.
        if is_drive {
            let fills = [("drive_wheels", "powered_wheels"), ("passive_wheels", "passive_wheels"), ("steer", "steering")];
            for (own, machine_role) in fills {
                let Some(dofs) = machine_roles.get(machine_role).filter(|d| !d.is_empty()) else { continue };
                let entry = roles.entry(own.to_string()).or_default();
                if entry.is_empty() {
                    b.push(Diagnostic::info(
                        Code::LegacyInference,
                        Some(&root),
                        format!("controller `{}` {own} taken from gearbox:machine:role", spec.instance),
                    ));
                }
                for dof in dofs {
                    if !entry.contains(dof) {
                        entry.push(*dof);
                    }
                }
            }
        }
        // A brake drives its wheelJoints, else the brake role, else every
        // wheel; a trailer steer its steerJoints, else the steering role:
        // the joints the runtime drives, whatever `target` says.
        let own = |name: &str| roles.get(name).filter(|d| !d.is_empty()).cloned();
        let machine_role = |name: &str| machine_roles.get(name).filter(|d| !d.is_empty()).cloned();
        let wheels = || {
            let all: Vec<DofRef> = ["powered_wheels", "passive_wheels"].iter().filter_map(|r| machine_roles.get(*r)).flatten().copied().collect();
            (!all.is_empty()).then_some(all)
        };
        let runtime = match spec.controller_type.as_str() {
            "builtin:brake" => Some(own("wheels").map(|d| (d, false)).or_else(|| machine_role("brake").or_else(wheels).map(|d| (d, true)))),
            "builtin:trailer_steer" => Some((!steer_joints.is_empty()).then(|| (steer_joints.clone(), false)).or_else(|| machine_role("steering").map(|d| (d, true)))),
            _ => None,
        };
        if let Some(found) = runtime {
            roles.remove("target");
            if let Some((dofs, inferred)) = found {
                roles.insert("target".to_string(), dofs);
                if inferred {
                    b.push(Diagnostic::info(
                        Code::LegacyInference,
                        Some(&root),
                        format!("controller `{}` target taken from gearbox:machine:role", spec.instance),
                    ));
                }
            }
        } else if is_service && roles.get("target").is_none_or(Vec::is_empty) {
            let fallback = machine_roles.get("tool").and_then(|t| t.first().map(|d| vec![*d]));
            if let Some(dofs) = fallback.filter(|d| !d.is_empty()) {
                roles.insert("target".to_string(), dofs);
                b.push(Diagnostic::info(
                    Code::LegacyInference,
                    Some(&root),
                    format!("controller `{}` target taken from gearbox:machine:role", spec.instance),
                ));
            }
        }

        if spec.controller_type == "builtin:tracked_cmd_vel" {
            let dofs = track_dofs(b, &sdf);
            if !dofs.is_empty() {
                roles.insert("track".to_string(), dofs);
            }
        }

        let rate_name = format!("{prefix}updateRateHz");
        let update_rate_hz = match read::float(b.scene, &root, &rate_name) {
            Some(rate) if !(rate.is_finite() && rate > 0.0) => {
                b.push(Diagnostic::error(Code::InvalidControllerRate, Some(&root), format!("{rate_name} = {rate} must be positive and finite")));
                Valued::authored(physics_rate_hz)
            }
            Some(rate) if rate > physics_rate_hz + 1e-9 => {
                b.strict_or_warn(
                    Code::InvalidControllerRate,
                    Some(&root),
                    format!("{rate_name} = {rate} exceeds the {physics_rate_hz} Hz physics rate"),
                );
                Valued::authored(rate.min(physics_rate_hz))
            }
            Some(rate) => Valued::authored(rate),
            None => Valued::profile(physics_rate_hz),
        };
        let mut parameters = BTreeMap::new();
        let mut put = |k: &str, v: Option<String>| {
            if let Some(v) = v {
                parameters.insert(k.to_string(), v);
            }
        };
        put("command_interface", spec.command_interface.clone());
        put("wheel_base", spec.wheel_base.map(|v| v.to_string()));
        put("wheel_radius", spec.wheel_radius.map(|v| v.to_string()));
        put("track_width", spec.track_width.map(|v| v.to_string()));
        put("max_steer_deg", spec.max_steer_deg.map(|v| v.to_string()));
        put("steering_geometry", spec.steering_geometry.clone());
        put("drive_mode", spec.drive_mode.clone());
        put("executable", spec.executable.clone());
        b.machine.controllers.push(ControllerBinding {
            instance: spec.instance.clone(),
            controller_type: spec.controller_type.clone(),
            enabled: spec.enabled,
            update_rate_hz,
            authority: "exclusive".to_string(),
            roles,
            parameters,
        });
    }

    assign_owners(b, &machine_roles);
    validate_interfaces(b);
}

/// The sprocket and roller joints of a tracked machine's belts: every joint
/// between a carrier and its sprocket or a body with `rolling_radius_m`.
fn track_dofs(b: &mut Builder, root: &openusd::sdf::Path) -> Vec<DofRef> {
    let Ok(tracks) = crate::controller::tracked::discover(b.scene.stage, root) else { return Vec::new() };
    let mut out = Vec::new();
    for track in tracks {
        let carrier = b.resolve_target(&track.carrier);
        let sprocket = b.resolve_target(&track.sprocket);
        let (Some(carrier), Some(sprocket)) = (b.machine.body_index(&carrier), b.machine.body_index(&sprocket)) else { continue };
        for (j, joint) in b.machine.joints.iter().enumerate() {
            let other = match joint.bodies() {
                (Some(a), Some(c)) if a == carrier => c,
                (Some(a), Some(c)) if c == carrier => a,
                _ => continue,
            };
            let roller = read::float(b.scene, &b.machine.bodies[other].path, "gearbox:value:rolling_radius_m").is_some_and(|r| r > 0.0);
            if other != sprocket && !roller {
                continue;
            }
            if let [dof] = joint.dofs.as_slice() {
                let r = DofRef { joint: j, dof: *dof };
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Claims {
    owners: BTreeMap<usize, DofOwner>,
    conflicts: Vec<(usize, String, String)>,
    interfaces: BTreeMap<usize, Vec<Interface>>,
}

impl Claims {
    fn claim(&mut self, dof: usize, owner: DofOwner, granted: Vec<Interface>) {
        match self.owners.get(&dof) {
            Some(existing) if existing.owner != owner.owner => {
                self.conflicts.push((dof, existing.owner.clone(), owner.owner.clone()))
            }
            Some(_) => self.interfaces.entry(dof).or_default().extend(granted),
            None => {
                self.owners.insert(dof, owner);
                self.interfaces.entry(dof).or_default().extend(granted);
            }
        }
    }
}

fn assign_owners(b: &mut Builder, machine_roles: &BTreeMap<String, Vec<DofRef>>) {
    let root = b.scope.root.clone();
    let mut claims = Claims::default();

    let controllers = b.machine.controllers.clone();
    for c in controllers.iter().filter(|c| c.enabled && DRIVE_TYPES.contains(&c.controller_type.as_str())) {
        for (role, dofs) in &c.roles {
            for d in dofs {
                let owner = DofOwner { owner: c.instance.clone(), kind: OwnerKind::Controller, device: None };
                claims.claim(d.dof, owner, interfaces_for(&c.controller_type, role));
            }
        }
    }
    for c in controllers.iter().filter(|c| c.enabled && crate::services::SERVICE_TYPES.contains(&c.controller_type.as_str())) {
        for d in c.roles.get("target").into_iter().flatten() {
            let held_by_drive = claims.owners.get(&d.dof).is_some_and(|o| o.kind == OwnerKind::Controller);
            if c.controller_type == "builtin:brake" && held_by_drive {
                continue;
            }
            let owner = DofOwner { owner: c.instance.clone(), kind: OwnerKind::Service, device: None };
            claims.claim(d.dof, owner, interfaces_for(&c.controller_type, "target"));
        }
    }

    let couplers: Vec<String> = b
        .scene
        .prims
        .iter()
        .filter(|p| super::scene::Scene::is_under(&p.path, &root))
        .filter(|p| read::token(b.scene, &p.path, "gearbox:coupling:side").as_deref() == Some("coupler"))
        .map(|p| p.path.clone())
        .collect();
    for coupler in couplers {
        let mut bound: Vec<String> = read::targets(b.scene, &coupler, "gearbox:coupling:ptoJoint").unwrap_or_default();
        bound.extend(read::targets(b.scene, &coupler, "gearbox:coupling:valveJoints").unwrap_or_default());
        for target in bound {
            let path = b.resolve_target(&target);
            if let Some(dof) = b.machine.dof(&path, None)
                && !claims.owners.contains_key(&dof.dof)
            {
                let owner = DofOwner { owner: "coupling".to_string(), kind: OwnerKind::Service, device: None };
                claims.claim(dof.dof, owner, vec![Interface::Velocity]);
            }
        }
    }

    let prims: Vec<openusd::sdf::Path> = b.scene.prims.iter().filter_map(|p| b.scene.sdf(&p.path)).collect();
    let (devices, device_errors) = match b.scene.sdf(&root) {
        Some(sdf) => crate::devices::discover(b.scene.stage, &sdf, &prims),
        None => (Vec::new(), Vec::new()),
    };
    for error in device_errors {
        b.strict_or_warn(Code::LegacyDevice, Some(&root), error);
    }
    let suspension: Vec<usize> = machine_roles.get("suspension").into_iter().flatten().map(|d| d.dof).collect();
    for device in devices {
        let crate::devices::DeviceKind::Joint { motor: Some(_), .. } = device.kind else { continue };
        let Some(dof) = b.machine.dof(&device.prim, None) else { continue };
        if suspension.contains(&dof.dof) {
            b.push(Diagnostic::error(
                Code::DuplicateOwner,
                Some(&device.prim),
                format!("motor device `{}` sits on a suspension joint", device.name),
            ));
            continue;
        }
        match claims.owners.get_mut(&dof.dof) {
            Some(owner) if owner.kind == OwnerKind::Service => owner.device = Some(device.name.clone()),
            Some(owner) => {
                let first = owner.owner.clone();
                claims.conflicts.push((dof.dof, first, format!("device:{}", device.name)));
            }
            None => {
                let owner = DofOwner { owner: format!("device:{}", device.name), kind: OwnerKind::Device, device: Some(device.name.clone()) };
                claims.claim(dof.dof, owner, interfaces_for("device", ""));
            }
        }
    }
    let Claims { owners, conflicts, interfaces } = claims;
    for (dof, first, second) in conflicts {
        let path = b.machine.joints[b.machine.dofs[dof].joint].path.clone();
        b.push(Diagnostic::error(
            Code::DuplicateOwner,
            Some(&path),
            format!("DOF `{}` has two default owners: `{first}` and `{second}`", b.machine.dofs[dof].name),
        ));
    }
    for (dof, granted) in interfaces {
        let strict = b.profile.strict();
        let entry = &mut b.machine.dofs[dof].interfaces;
        if entry.source != Provenance::Authored && !strict {
            let mut granted = granted;
            granted.sort();
            granted.dedup();
            *entry = Valued::legacy(granted);
        }
    }
    b.machine.owners = owners;
}

/// An authored interface the DOF cannot realize is an error, not ignored.
fn validate_interfaces(b: &mut Builder) {
    for index in 0..b.machine.dofs.len() {
        let dof = b.machine.dofs[index].clone();
        let joint = b.machine.joints[dof.joint].clone();
        let path = joint.path.clone();
        let scalar = matches!(joint.kind, JointKind::Revolute | JointKind::Prismatic | JointKind::D6);
        let authored = dof.interfaces.source == Provenance::Authored;
        for interface in dof.interfaces.value.iter().filter(|_| authored) {
            let problem = match interface {
                _ if !scalar => Some(format!("{} joints take no commands", joint.kind.as_str())),
                Interface::Position if dof.drive.as_ref().is_none_or(|d| d.stiffness.value <= 0.0) => {
                    Some("position needs a PhysicsDriveAPI with stiffness > 0".to_string())
                }
                Interface::Velocity if dof.drive.as_ref().is_none_or(|d| d.damping.value <= 0.0) => {
                    Some("velocity needs a PhysicsDriveAPI with damping > 0".to_string())
                }
                _ => None,
            };
            if let Some(problem) = problem {
                b.push(Diagnostic::error(Code::InvalidInterface, Some(&path), format!("{}: {}: {problem}", dof.name, interface.as_str())));
            }
        }
        let mode = dof.default_mode.value;
        let needed = match mode {
            DefaultMode::Disabled => None,
            DefaultMode::Position => Some(Interface::Position),
            DefaultMode::Velocity => Some(Interface::Velocity),
            DefaultMode::Effort => Some(Interface::Effort),
        };
        if let Some(needed) = needed
            && !dof.interfaces.value.contains(&needed)
        {
            b.push(Diagnostic::error(
                Code::InvalidInterface,
                Some(&path),
                format!("{}: defaultMode {} is not among its command interfaces", dof.name, mode.as_str()),
            ));
        }
    }
}

/// Wheel- and steer-looking joint names, and Isaac controller joint arrays
/// found under this machine, reported as suggestions only.
fn suggest_roles(b: &mut Builder) {
    let root = b.scope.root.clone();
    let mut suggestions: Vec<(String, &str)> = Vec::new();
    for joint in &b.machine.joints {
        let lower = joint.path.to_ascii_lowercase();
        if lower.contains("steer") || lower.contains("knuckle") || lower.contains("upright") {
            suggestions.push((joint.path.clone(), "steering"));
        } else if lower.contains("wheel") {
            suggestions.push((joint.path.clone(), "wheel"));
        }
    }
    for prim in b.scene.prims.iter().filter(|p| super::scene::Scene::is_under(&p.path, &root)) {
        for prop in read::property_names(b.scene, &prim.path) {
            let lower = prop.to_ascii_lowercase();
            if !(lower.contains("jointnames") || lower.contains("joint_names") || lower.contains("dofnames") || lower.contains("dof_names")) {
                continue;
            }
            let names = read::tokens(b.scene, &prim.path, &prop).unwrap_or_default();
            for name in names {
                if let Some(joint) = b.machine.joints.iter().find(|j| super::compile::leaf(&j.path) == name || j.name == name) {
                    suggestions.push((joint.path.clone(), "named in a controller joint array"));
                }
            }
        }
    }
    suggestions.sort();
    suggestions.dedup();
    for (path, role) in suggestions {
        b.push(Diagnostic::info(Code::SuggestedRole, Some(&path), format!("looks like a {role} joint; no controller is created from names")));
    }
}
