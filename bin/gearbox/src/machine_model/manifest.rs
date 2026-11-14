//! Canonical JSON of a compiled machine: sorted keys, 12 significant digits,
//! no negative zero, quaternions with w ≥ 0, unbounded values as null.

use bevy::math::{DQuat, DVec3};
use serde_json::{Map, Value, json};

use super::compile::CompileResult;
use super::diagnostics::Diagnostic;
use super::model::*;

pub fn canon(v: f64) -> Value {
    if !v.is_finite() {
        return Value::Null;
    }
    if v == 0.0 {
        return json!(0.0);
    }
    let rounded: f64 = format!("{v:.11e}").parse().unwrap_or(v);
    json!(if rounded == 0.0 { 0.0 } else { rounded })
}

fn opt(v: Option<f64>) -> Value {
    v.map_or(Value::Null, canon)
}

fn vec3(v: DVec3) -> Value {
    json!([canon(v.x), canon(v.y), canon(v.z)])
}

/// `[w, x, y, z]` with w ≥ 0.
fn quat(q: DQuat) -> Value {
    let q = if q.w < 0.0 { -q } else { q };
    json!([canon(q.w), canon(q.x), canon(q.y), canon(q.z)])
}

fn transform(t: &RigidTransform) -> Value {
    json!({ "translation": vec3(t.translation), "rotation_wxyz": quat(t.rotation) })
}

fn valued<T>(v: &Valued<T>, f: impl Fn(&T) -> Value) -> Value {
    json!({ "value": f(&v.value), "source": v.source.as_str() })
}

pub fn diagnostic(d: &Diagnostic) -> Value {
    json!({ "code": d.code.as_str(), "severity": d.severity.as_str(), "path": d.path, "message": d.message })
}

fn endpoint(m: &MachineDescription, e: &EndpointRecord) -> Value {
    let resolved = match &e.endpoint {
        Endpoint::World => json!({ "kind": "world" }),
        Endpoint::Body(i) => json!({ "kind": "body", "body": m.bodies[*i].path }),
        Endpoint::Invalid { target, reason } => json!({ "kind": "invalid", "target": target, "reason": reason }),
    };
    json!({ "authored": e.authored, "resolved": resolved, "via": e.via })
}

fn frame_name(m: &MachineDescription, i: usize) -> Value {
    m.frames.get(i).map_or(Value::Null, |f| json!(f.name))
}

pub fn machine(m: &MachineDescription) -> Value {
    let units = json!({
        "meters_per_unit": valued(&m.units.meters_per_unit, |v| canon(*v)),
        "kilograms_per_unit": valued(&m.units.kilograms_per_unit, |v| canon(*v)),
        "up_axis": valued(&m.units.up_axis, |v| json!(match v { UpAxis::Y => "Y", UpAxis::Z => "Z" })),
    });
    let bodies: Vec<Value> = m
        .bodies
        .iter()
        .map(|b| {
            json!({
                "path": b.path,
                "name": b.name,
                "mode": b.mode.as_str(),
                "mass": b.mass.mass.as_ref().map_or(Value::Null, |v| valued(v, |x| canon(*x))),
                "density": b.mass.density.as_ref().map_or(Value::Null, |v| valued(v, |x| canon(*x))),
                "center_of_mass": valued(&b.mass.center_of_mass, |v| vec3(*v)),
                "diagonal_inertia": b.mass.diagonal_inertia.as_ref().map_or(Value::Null, |v| valued(v, |x| vec3(*x))),
                "principal_axes": valued(&b.mass.principal_axes, |q| quat(*q)),
                "initial_pose": transform(&b.initial_pose),
                "scale": canon(b.scale),
                "initial_linear_velocity": valued(&b.initial_linear_velocity, |v| vec3(*v)),
                "initial_angular_velocity": valued(&b.initial_angular_velocity, |v| vec3(*v)),
                "linear_damping": valued(&b.linear_damping, |x| canon(*x)),
                "angular_damping": valued(&b.angular_damping, |x| canon(*x)),
                "colliders": b.colliders.iter().map(|c| json!({
                    "path": c.path, "shape": c.shape, "approximation": c.approximation, "enabled": c.enabled,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let joints: Vec<Value> = m
        .joints
        .iter()
        .map(|j| {
            json!({
                "path": j.path,
                "name": j.name,
                "kind": j.kind.as_str(),
                "class": j.class.as_str(),
                "enabled": j.enabled,
                "collision_enabled": j.collision_enabled,
                "exclude_from_articulation": j.exclude_from_articulation,
                "body0": endpoint(m, &j.body0),
                "body1": endpoint(m, &j.body1),
                "local0": transform(&j.local0),
                "local1": transform(&j.local1),
                "axis": j.axis.map(Axis::as_str),
                "dofs": j.dofs.iter().map(|d| json!(m.dofs[*d].name)).collect::<Vec<_>>(),
                "break_force": opt(j.break_force),
                "break_torque": opt(j.break_torque),
            })
        })
        .collect();
    let dofs: Vec<Value> = m
        .dofs
        .iter()
        .map(|d| {
            let drive = d.drive.as_ref().map_or(Value::Null, |drive| {
                json!({
                    "type": valued(&drive.drive_type, |t| json!(t.as_str())),
                    "stiffness": valued(&drive.stiffness, |v| canon(*v)),
                    "damping": valued(&drive.damping, |v| canon(*v)),
                    "max_force": valued(&drive.max_force, |v| opt(*v)),
                    "target_position": valued(&drive.target_position, |v| canon(*v)),
                    "target_velocity": valued(&drive.target_velocity, |v| canon(*v)),
                })
            });
            json!({
                "name": d.name,
                "joint": m.joints[d.joint].path,
                "token": d.token.as_str(),
                "unit": if d.token.rotational() { "rad" } else { "m" },
                "coordinate": "body1 joint frame relative to body0 joint frame",
                "limit": {
                    "lower": opt(d.limit.lower),
                    "upper": opt(d.limit.upper),
                    "source": d.limit.source.as_str(),
                },
                "drive": drive,
                "passive": d.passive.as_str(),
                "interfaces": valued(&d.interfaces, |v| json!(v.iter().map(|i| i.as_str()).collect::<Vec<_>>())),
                "default_mode": valued(&d.default_mode, |v| json!(v.as_str())),
                "initial_position": valued(&d.initial_position, |v| canon(*v)),
                "initial_velocity": valued(&d.initial_velocity, |v| canon(*v)),
                "isaac_actuator": d.isaac_actuator,
            })
        })
        .collect();
    let frames: Vec<Value> = m
        .frames
        .iter()
        .map(|f| {
            let owner = match f.owner {
                FrameOwner::Body(i) => json!({ "body": m.bodies.get(i).map(|b| b.path.clone()) }),
                FrameOwner::Frame(i) => json!({ "frame": frame_name(m, i) }),
            };
            json!({
                "path": f.path, "name": f.name, "owner": owner, "transform": transform(&f.transform),
                "role": f.role.as_str(), "publish": f.publish,
            })
        })
        .collect();
    let tree: Vec<Value> = m
        .tree
        .iter()
        .map(|e| {
            json!({
                "parent": frame_name(m, e.parent),
                "child": frame_name(m, e.child),
                "joint": e.joint.map(|j| m.joints[j].path.clone()),
                "reversed": e.reversed,
            })
        })
        .collect();
    let articulations: Vec<Value> = m
        .articulations
        .iter()
        .map(|a| {
            json!({
                "markers": a.markers,
                "bodies": a.bodies.iter().map(|&i| m.bodies[i].path.clone()).collect::<Vec<_>>(),
                "joints": a.joints.iter().map(|&i| m.joints[i].path.clone()).collect::<Vec<_>>(),
                "fixed_base": a.fixed_base,
            })
        })
        .collect();
    let dof_name = |r: &DofRef| json!(m.dofs[r.dof].name);
    let controllers: Vec<Value> = m
        .controllers
        .iter()
        .map(|c| {
            let roles: Map<String, Value> =
                c.roles.iter().map(|(k, v)| (k.clone(), json!(v.iter().map(dof_name).collect::<Vec<_>>()))).collect();
            json!({
                "instance": c.instance, "type": c.controller_type, "enabled": c.enabled,
                "update_rate_hz": valued(&c.update_rate_hz, |v| canon(*v)),
                "authority": c.authority, "roles": roles, "parameters": c.parameters,
            })
        })
        .collect();
    let owners: Vec<Value> = m
        .owners
        .iter()
        .map(|(dof, o)| json!({ "dof": m.dofs[*dof].name, "owner": o.owner, "kind": o.kind.as_str(), "device": o.device }))
        .collect();
    let capabilities: Vec<Value> = m
        .capabilities
        .iter()
        .map(|c| {
            json!({
                "feature": c.feature, "subject": c.subject, "status": c.status.as_str(),
                "source_property": c.source_property, "reason": c.reason, "evidence": c.evidence,
            })
        })
        .collect();
    let reporting = m.reporting.as_ref().map_or(Value::Null, |r| {
        json!({
            "links": r.bodies.iter().map(|&i| m.bodies[i].path.clone()).collect::<Vec<_>>(),
            "joints": r.joints.iter().map(|&i| m.joints[i].path.clone()).collect::<Vec<_>>(),
        })
    });
    json!({
        "manifest_version": MANIFEST_VERSION,
        "convention": WORLD_CONVENTION,
        "load_instance": m.load_instance,
        "root": m.root,
        "machine_id": m.machine_id,
        "generation": m.generation,
        "profile": m.profile.as_str(),
        "kind": m.kind,
        "isaac": m.isaac,
        "accepted": m.accepted(),
        "units": units,
        "control_body": m.control_body.map(|i| m.bodies[i].path.clone()),
        "base_frame": m.base_frame.map_or(Value::Null, |i| frame_name(m, i)),
        "control_frame": m.control_frame.as_ref().map_or(Value::Null, |c| json!({ "frame": frame_name(m, c.frame), "source": c.source.as_str() })),
        "command_validity_ms": m.command_validity_ms,
        "bodies": bodies,
        "joints": joints,
        "dofs": dofs,
        "frames": frames,
        "tree": tree,
        "articulations": articulations,
        "controllers": controllers,
        "owners": owners,
        "reporting": reporting,
        "capabilities": capabilities,
        "diagnostics": m.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
    })
}

pub fn result(r: &CompileResult) -> Value {
    json!({
        "manifest_version": MANIFEST_VERSION,
        "scan_root": r.scan_root,
        "machines": r.machines.iter().map(machine).collect::<Vec<_>>(),
        "diagnostics": r.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
    })
}

pub fn to_string(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default() + "\n"
}
