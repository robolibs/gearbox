//! Link trees and couplings discovered from a machine's USD prims
//! (`specs/CONTROLLER_SPEC.md` §7, `specs/TOOLS_SPEC.md` §2).
//!
//! A machine that authors no `GearboxLinkAPI` gets a tree derived from its
//! rigid bodies and joints, with a warning. One authored mark switches the
//! machine to strict validation; any failure is recorded in `errors` and the
//! machine gets no agent.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::math::{DMat4, DQuat, DVec3, EulerRot};
use openusd::sdf::{Path as SdfPath, Value};

use crate::controller::{
    read_attr, read_bool, read_float, read_rel_first, read_token, read_token_array,
    rebase_asset_root_target, type_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRole {
    Base,
    Link,
    Wheel,
    Steer,
    Sensor,
    Tool,
}

impl LinkRole {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "base" => Self::Base,
            "link" => Self::Link,
            "wheel" => Self::Wheel,
            "steer" => Self::Steer,
            "sensor" => Self::Sensor,
            "tool" => Self::Tool,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Link => "link",
            Self::Wheel => "wheel",
            Self::Steer => "steer",
            Self::Sensor => "sensor",
            Self::Tool => "tool",
        }
    }
}

/// Static transform of a link in its parent link's frame, asset (Z-up) axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticOffset {
    pub translation: DVec3,
    pub rotation: DQuat,
}

impl Default for StaticOffset {
    fn default() -> Self {
        Self {
            translation: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CouplingSide {
    Hitch,
    Coupler,
}

impl CouplingSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hitch => "hitch",
            Self::Coupler => "coupler",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CouplingSpec {
    pub name: String,
    pub side: CouplingSide,
    pub kind: String,
    pub iso_category: Option<u8>,
    pub capacity_kg: Option<f32>,
    pub services: Vec<String>,
    pub lift_joint: Option<String>,
    pub excludes: Vec<String>,
    /// Coupler side: the slave joint a bound master PTO spins.
    pub pto_joint: Option<String>,
    /// Coupler side: slave joints bound to the master's hydraulic valves, in
    /// valve order.
    pub valve_joints: Vec<String>,
}

pub const COUPLING_TYPES: [&str; 12] = [
    "drawbar",
    "three_point_semi_mounted",
    "three_point_mounted",
    "hitch_hook",
    "clevis",
    "piton",
    "cuna",
    "ball",
    "chassis_mounted",
    "pivot_wagon",
    "fifth_wheel",
    "loader_carriage",
];

#[derive(Debug, Clone, PartialEq)]
pub struct LinkSpec {
    pub name: String,
    pub prim_path: String,
    pub role: LinkRole,
    pub parent: Option<String>,
    pub joint_prim: Option<String>,
    pub body_prim: Option<String>,
    pub static_offset: Option<StaticOffset>,
    pub coupling: Option<CouplingSpec>,
    /// Working-part element this link is (function, bin, section, ...).
    pub element: Option<ElementInfo>,
    /// Named values authored on the link (`gearbox:value:<Name>`).
    pub values: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkTree {
    pub links: Vec<LinkSpec>,
    pub derived: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl LinkTree {
    pub fn base(&self) -> Option<&LinkSpec> {
        self.links.iter().find(|l| l.role == LinkRole::Base)
    }

    pub fn get(&self, name: &str) -> Option<&LinkSpec> {
        self.links.iter().find(|l| l.name == name)
    }

    pub fn by_prim(&self, prim: &str) -> Option<&LinkSpec> {
        self.links.iter().find(|l| l.prim_path == prim)
    }

    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn couplings(&self) -> impl Iterator<Item = (&LinkSpec, &CouplingSpec)> {
        self.links
            .iter()
            .filter_map(|l| l.coupling.as_ref().map(|c| (l, c)))
    }
}

struct JointRecord {
    prim: String,
    body0: Option<String>,
    body1: Option<String>,
    excluded: bool,
}

/// Discover the link tree of the machine rooted at `machine_prim`.
/// `machine_body` is the `gearbox:machine:body` target when authored.
pub fn discover_link_tree(
    stage: &openusd::Stage,
    machine_prim: &SdfPath,
    machine_body: Option<&str>,
    prims: &[SdfPath],
) -> LinkTree {
    let root = machine_prim.as_str();
    let inside: Vec<&SdfPath> = prims
        .iter()
        .filter(|p| p.as_str() == root || p.as_str().starts_with(&format!("{root}/")))
        .collect();

    let mut bodies: Vec<String> = Vec::new();
    let mut marked: Vec<String> = Vec::new();
    let mut couplings: Vec<String> = Vec::new();
    let mut elements: Vec<String> = Vec::new();
    let mut joints: Vec<JointRecord> = Vec::new();
    for prim in &inside {
        let schemas = stage.api_schemas(prim).unwrap_or_default();
        let path = prim.as_str().to_string();
        if schemas.iter().any(|s| s == "PhysicsRigidBodyAPI") {
            bodies.push(path.clone());
        }
        if schemas.iter().any(|s| s == "GearboxLinkAPI")
            || read_token(stage, prim, "gearbox:link:name").is_some()
            || read_token(stage, prim, "gearbox:link:role").is_some()
        {
            marked.push(path.clone());
        }
        if schemas.iter().any(|s| s == "GearboxCouplingAPI")
            || read_token(stage, prim, "gearbox:coupling:side").is_some()
        {
            couplings.push(path.clone());
        }
        if schemas.iter().any(|s| s == "GearboxElementAPI")
            || read_token(stage, prim, "gearbox:element:type").is_some()
        {
            elements.push(path.clone());
        }
        let ty = type_name(stage, prim).unwrap_or_default();
        if ty.starts_with("Physics") && ty.ends_with("Joint") {
            joints.push(JointRecord {
                prim: path.clone(),
                body0: read_rel_first(stage, prim, "physics:body0")
                    .map(|t| rebase_asset_root_target(root, &t)),
                body1: read_rel_first(stage, prim, "physics:body1")
                    .map(|t| rebase_asset_root_target(root, &t)),
                excluded: read_bool(stage, prim, "physics:excludeFromArticulation")
                    .unwrap_or(false),
            });
        }
    }

    let mut tree = LinkTree {
        derived: marked.is_empty(),
        ..Default::default()
    };

    // Which prims are links, and what they are called.
    let mut link_prims: Vec<String> = if tree.derived {
        let mut v = bodies.clone();
        for c in &couplings {
            if !v.contains(c) {
                v.push(c.clone());
            }
        }
        v
    } else {
        let mut v = marked.clone();
        for c in &couplings {
            if !v.contains(c) {
                v.push(c.clone());
            }
        }
        v
    };
    for e in &elements {
        if !link_prims.contains(e) {
            link_prims.push(e.clone());
        }
    }
    link_prims.sort();
    link_prims.dedup();

    if tree.derived {
        if !bodies.is_empty() {
            tree.warnings.push(
                "link tree derived from rigid bodies and joints; author GearboxLinkAPI to make it explicit"
                    .to_string(),
            );
        }
    } else {
        for body in &bodies {
            if !link_prims.contains(body) {
                tree.errors
                    .push(format!("rigid body {body} has no GearboxLinkAPI"));
            }
        }
        for c in &couplings {
            if !marked.contains(c) {
                tree.errors.push(format!(
                    "coupling {c} is not a link (missing GearboxLinkAPI)"
                ));
            }
        }
    }

    // The base link: authored role, else the machine body, else the only
    // body that is never a joint child.
    let derived_base: Option<String> = machine_body
        .map(|b| rebase_asset_root_target(root, b))
        .filter(|b| link_prims.contains(b))
        .or_else(|| {
            let children: HashSet<&String> =
                joints.iter().filter_map(|j| j.body1.as_ref()).collect();
            let roots: Vec<&String> = bodies.iter().filter(|b| !children.contains(b)).collect();
            (roots.len() == 1).then(|| roots[0].clone())
        });

    let mut links: Vec<LinkSpec> = Vec::new();
    for prim in &link_prims {
        let sdf = match openusd::sdf::path(prim) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let authored_role = read_token(stage, &sdf, "gearbox:link:role");
        let role = match authored_role.as_deref() {
            Some(token) => match LinkRole::parse(token) {
                Some(r) => r,
                None => {
                    tree.errors
                        .push(format!("{prim}: unknown gearbox:link:role `{token}`"));
                    LinkRole::Link
                }
            },
            None if tree.derived && derived_base.as_deref() == Some(prim.as_str()) => {
                LinkRole::Base
            }
            None if couplings.contains(prim) => LinkRole::Tool,
            None if elements.contains(prim) => LinkRole::Tool,
            None => LinkRole::Link,
        };
        let name = match read_token(stage, &sdf, "gearbox:link:name") {
            Some(n) => n,
            None if role == LinkRole::Base => "base_link".to_string(),
            None => sanitize_link_name(leaf(prim)),
        };
        let parent_override = read_rel_first(stage, &sdf, "gearbox:link:parent")
            .map(|t| rebase_asset_root_target(root, &t));
        let coupling = couplings
            .contains(prim)
            .then(|| read_coupling(stage, &sdf, root, &mut tree.errors))
            .flatten();
        links.push(LinkSpec {
            name,
            prim_path: prim.clone(),
            role,
            parent: parent_override,
            joint_prim: None,
            body_prim: bodies.contains(prim).then(|| prim.clone()),
            static_offset: None,
            coupling,
            element: read_element(stage, &sdf, &mut tree.errors),
            values: read_values(stage, &sdf),
        });
    }

    // Names: valid and unique.
    let mut seen: HashSet<String> = HashSet::new();
    for l in &links {
        if !is_valid_link_name(&l.name) {
            tree.errors.push(format!(
                "{}: link name `{}` must be lowercase [a-z0-9_]",
                l.prim_path, l.name
            ));
        }
        if !seen.insert(l.name.clone()) {
            tree.errors
                .push(format!("link name `{}` is used twice", l.name));
        }
    }

    // Exactly one base, called base_link.
    let bases: Vec<&LinkSpec> = links.iter().filter(|l| l.role == LinkRole::Base).collect();
    match bases.len() {
        0 if !links.is_empty() => tree.errors.push("no link has role = base".to_string()),
        1 => {
            if bases[0].name != "base_link" {
                tree.errors.push(format!(
                    "{}: the base link must be named base_link, not `{}`",
                    bases[0].prim_path, bases[0].name
                ));
            }
        }
        n if n > 1 => tree.errors.push(format!(
            "{n} links have role = base: {}",
            bases
                .iter()
                .map(|b| b.prim_path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => {}
    }

    // Joints connect rigid-body links: body1 is the child of body0.
    let prim_to_index: HashMap<String, usize> = links
        .iter()
        .enumerate()
        .map(|(i, l)| (l.prim_path.clone(), i))
        .collect();
    let mut child_joint_count: HashMap<usize, usize> = HashMap::new();
    for j in &joints {
        let (Some(b0), Some(b1)) = (&j.body0, &j.body1) else {
            continue;
        };
        let inside_machine = |p: &String| p == root || p.starts_with(&format!("{root}/"));
        let i0 = prim_to_index.get(b0);
        let i1 = prim_to_index.get(b1);
        if !tree.derived {
            if i0.is_none() && inside_machine(b0) || i0.is_none() && !inside_machine(b0) {
                tree.errors.push(format!(
                    "joint {}: physics:body0 {b0} is not a link of this machine",
                    j.prim
                ));
            }
            if i1.is_none() {
                tree.errors.push(format!(
                    "joint {}: physics:body1 {b1} is not a link of this machine",
                    j.prim
                ));
            }
        }
        let (Some(&i0), Some(&i1)) = (i0, i1) else {
            continue;
        };
        if !j.excluded {
            *child_joint_count.entry(i1).or_default() += 1;
        }
        if links[i1].parent.is_none() && links[i1].role != LinkRole::Base {
            links[i1].parent = Some(links[i0].name.clone());
            links[i1].joint_prim = Some(j.prim.clone());
        }
    }
    for (i, count) in &child_joint_count {
        if *count > 1 {
            tree.errors.push(format!(
                "{}: child of {count} joints (kinematic loop); mark all but one physics:excludeFromArticulation",
                links[*i].prim_path
            ));
        }
    }

    // Parent overrides are given as prim paths; turn them into names.
    for i in 0..links.len() {
        if let Some(p) = links[i].parent.clone()
            && p.starts_with('/')
        {
            match prim_to_index.get(&p) {
                Some(&pi) if pi != i => {
                    let name = links[pi].name.clone();
                    links[i].parent = Some(name);
                }
                _ => {
                    tree.errors.push(format!(
                        "{}: gearbox:link:parent {p} is not a link of this machine",
                        links[i].prim_path
                    ));
                    links[i].parent = None;
                }
            }
        }
    }

    // Links without a joint parent hang from the nearest ancestor link with
    // a static offset. Base never has a parent.
    for i in 0..links.len() {
        if links[i].role == LinkRole::Base {
            links[i].parent = None;
            links[i].joint_prim = None;
            continue;
        }
        if links[i].parent.is_some() && links[i].joint_prim.is_some() {
            continue;
        }
        let ancestor = links[i].parent.clone().and_then(|n| {
            links
                .iter()
                .find(|l| l.name == n)
                .map(|l| l.prim_path.clone())
        });
        let ancestor =
            ancestor.or_else(|| nearest_ancestor_link(&links[i].prim_path, &prim_to_index, i));
        if let Some(anc) = ancestor {
            let offset = local_offset_between(stage, &anc, &links[i].prim_path);
            let name = links[prim_to_index[&anc]].name.clone();
            links[i].parent = Some(name);
            links[i].static_offset = Some(offset);
        } else if links.iter().any(|l| l.role == LinkRole::Base) {
            tree.errors.push(format!(
                "{}: not connected to base_link by a joint or an ancestor link",
                links[i].prim_path
            ));
        }
    }

    // Reachability and cycles from base_link.
    if let Some(base_i) = links.iter().position(|l| l.role == LinkRole::Base) {
        let name_to_index: HashMap<String, usize> = links
            .iter()
            .enumerate()
            .map(|(i, l)| (l.name.clone(), i))
            .collect();
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, l) in links.iter().enumerate() {
            if let Some(p) = &l.parent
                && let Some(&pi) = name_to_index.get(p)
            {
                children.entry(pi).or_default().push(i);
            }
        }
        let mut order = vec![base_i];
        let mut seen: HashSet<usize> = HashSet::from([base_i]);
        let mut queue = VecDeque::from([base_i]);
        while let Some(i) = queue.pop_front() {
            let mut kids = children.get(&i).cloned().unwrap_or_default();
            kids.sort_by(|a, b| links[*a].name.cmp(&links[*b].name));
            for k in kids {
                if seen.insert(k) {
                    order.push(k);
                    queue.push_back(k);
                }
            }
        }
        for (i, l) in links.iter().enumerate() {
            if !seen.contains(&i) {
                let via = l.parent.as_deref().unwrap_or("nothing");
                tree.errors.push(format!(
                    "{}: not reachable from base_link (parent {via} forms a cycle or is missing)",
                    l.prim_path
                ));
            }
        }
        let mut sorted: Vec<LinkSpec> = order.iter().map(|&i| links[i].clone()).collect();
        for (i, l) in links.into_iter().enumerate() {
            if !seen.contains(&i) {
                sorted.push(l);
            }
        }
        links = sorted;
    }

    tree.links = links;
    tree
}

fn read_coupling(
    stage: &openusd::Stage,
    prim: &SdfPath,
    root: &str,
    errors: &mut Vec<String>,
) -> Option<CouplingSpec> {
    let path = prim.as_str();
    let side = match read_token(stage, prim, "gearbox:coupling:side").as_deref() {
        Some("hitch") => CouplingSide::Hitch,
        Some("coupler") => CouplingSide::Coupler,
        Some(other) => {
            errors.push(format!(
                "{path}: gearbox:coupling:side must be hitch or coupler, not `{other}`"
            ));
            return None;
        }
        None => {
            errors.push(format!("{path}: coupling has no gearbox:coupling:side"));
            return None;
        }
    };
    let kind = match read_token(stage, prim, "gearbox:coupling:type") {
        Some(k) if COUPLING_TYPES.contains(&k.as_str()) => k,
        Some(k) => {
            errors.push(format!("{path}: unknown gearbox:coupling:type `{k}`"));
            return None;
        }
        None => {
            errors.push(format!("{path}: coupling has no gearbox:coupling:type"));
            return None;
        }
    };
    let iso_category = match read_attr(stage, prim, "gearbox:coupling:isoCategory") {
        Some(Value::Int(v)) => Some(v.clamp(0, 4) as u8),
        Some(Value::Uint(v)) => Some(v.min(4) as u8),
        _ => None,
    };
    Some(CouplingSpec {
        name: read_token(stage, prim, "gearbox:coupling:name")
            .unwrap_or_else(|| sanitize_link_name(leaf(path))),
        side,
        kind,
        iso_category,
        capacity_kg: read_float(stage, prim, "gearbox:coupling:capacityKg"),
        services: read_token_array(stage, prim, "gearbox:coupling:services"),
        lift_joint: read_rel_first(stage, prim, "gearbox:coupling:lift")
            .map(|t| rebase_asset_root_target(root, &t)),
        excludes: read_token_array(stage, prim, "gearbox:coupling:excludes"),
        pto_joint: read_rel_first(stage, prim, "gearbox:coupling:ptoJoint")
            .map(|t| rebase_asset_root_target(root, &t)),
        valve_joints: crate::controller::read_rel_targets(
            stage,
            prim,
            "gearbox:coupling:valveJoints",
        )
        .into_iter()
        .map(|t| rebase_asset_root_target(root, &t))
        .collect(),
    })
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn sanitize_link_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "link".to_string()
    } else {
        out
    }
}

fn is_valid_link_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The closest ancestor prim of `prim` that is itself a link.
fn nearest_ancestor_link(
    prim: &str,
    prim_to_index: &HashMap<String, usize>,
    this: usize,
) -> Option<String> {
    let mut cur = prim.to_string();
    while let Some(pos) = cur.rfind('/') {
        cur.truncate(pos);
        if cur.is_empty() {
            break;
        }
        if let Some(&i) = prim_to_index.get(&cur)
            && i != this
        {
            return Some(cur);
        }
    }
    None
}

/// Composed local transform of `descendant` in `ancestor`'s frame.
pub(crate) fn local_offset_between(
    stage: &openusd::Stage,
    ancestor: &str,
    descendant: &str,
) -> StaticOffset {
    let Some(rel) = descendant.strip_prefix(ancestor) else {
        return StaticOffset::default();
    };
    let mut m = DMat4::IDENTITY;
    let mut cur = ancestor.to_string();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        cur = format!("{cur}/{part}");
        if let Ok(p) = openusd::sdf::path(&cur) {
            m *= prim_local_matrix(stage, &p);
        }
    }
    let (_, rotation, translation) = m.to_scale_rotation_translation();
    StaticOffset {
        translation,
        rotation: rotation.normalize(),
    }
}

/// The prim's own xform from its `xformOpOrder`, ignoring scale and pivots.
pub fn prim_local_matrix(stage: &openusd::Stage, prim: &SdfPath) -> DMat4 {
    let order = read_token_array(stage, prim, "xformOpOrder");
    let ops: Vec<String> = if order.is_empty() {
        [
            "xformOp:translate",
            "xformOp:orient",
            "xformOp:rotateXYZ",
            "xformOp:transform",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        order
    };
    let mut m = DMat4::IDENTITY;
    for op in ops {
        let inverted = op.starts_with("!invert!");
        let name = op.trim_start_matches("!invert!");
        let Some(value) = read_attr(stage, prim, name) else {
            continue;
        };
        let base = name.split(':').take(2).collect::<Vec<_>>().join(":");
        let step = match (base.as_str(), value) {
            ("xformOp:translate", Value::Vec3d(v)) => DMat4::from_translation(DVec3::from(v)),
            ("xformOp:translate", Value::Vec3f(v)) => {
                DMat4::from_translation(DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64))
            }
            ("xformOp:orient", Value::Quatd(q)) => DMat4::from_quat(quat_wxyz(q)),
            ("xformOp:orient", Value::Quatf(q)) => DMat4::from_quat(quat_wxyz([
                q[0] as f64,
                q[1] as f64,
                q[2] as f64,
                q[3] as f64,
            ])),
            ("xformOp:rotateXYZ", Value::Vec3f(v)) => DMat4::from_quat(DQuat::from_euler(
                EulerRot::XYZ,
                (v[0] as f64).to_radians(),
                (v[1] as f64).to_radians(),
                (v[2] as f64).to_radians(),
            )),
            ("xformOp:rotateXYZ", Value::Vec3d(v)) => DMat4::from_quat(DQuat::from_euler(
                EulerRot::XYZ,
                v[0].to_radians(),
                v[1].to_radians(),
                v[2].to_radians(),
            )),
            ("xformOp:rotateZ", Value::Float(a)) => DMat4::from_rotation_z((a as f64).to_radians()),
            ("xformOp:rotateZ", Value::Double(a)) => DMat4::from_rotation_z(a.to_radians()),
            ("xformOp:rotateX", Value::Float(a)) => DMat4::from_rotation_x((a as f64).to_radians()),
            ("xformOp:rotateX", Value::Double(a)) => DMat4::from_rotation_x(a.to_radians()),
            ("xformOp:rotateY", Value::Float(a)) => DMat4::from_rotation_y((a as f64).to_radians()),
            ("xformOp:rotateY", Value::Double(a)) => DMat4::from_rotation_y(a.to_radians()),
            ("xformOp:transform", Value::Matrix4d(v)) => {
                // USD matrices are row-major with the translation in the
                // last row; glam wants columns.
                DMat4::from_cols_array(&v).transpose()
            }
            _ => continue,
        };
        let step = if inverted { step.inverse() } else { step };
        m *= step;
    }
    m
}

fn quat_wxyz(q: [f64; 4]) -> DQuat {
    DQuat::from_xyzw(q[1], q[2], q[3], q[0]).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::discover_machines_from_usd;
    use std::path::Path;

    fn write_temp(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("gearbox-link-tests");
        std::fs::create_dir_all(&dir).unwrap();
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("{name}-{}-{n}.usda", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn machine_with(links: &str, joints: &str) -> String {
        format!(
            r#"#usda 1.0
(
    defaultPrim = "robot"
    upAxis = "Z"
)

def Xform "robot" (
    prepend apiSchemas = ["GearboxMachineAPI"]
)
{{
    token gearbox:machine:kind = "test"
    rel gearbox:machine:body = </robot/chassis>
{links}
    def Scope "Joints"
    {{
{joints}
    }}
}}
"#
        )
    }

    const CHASSIS_AND_WHEEL: &str = r#"
    def Xform "chassis" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:name = "base_link"
        token gearbox:link:role = "base"
        def Xform "imu" (prepend apiSchemas = ["GearboxLinkAPI"])
        {
            token gearbox:link:role = "sensor"
            double3 xformOp:translate = (0.5, 0, 1.2)
            uniform token[] xformOpOrder = ["xformOp:translate"]
        }
    }
    def Xform "wheel_left" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:role = "wheel"
    }
"#;

    const WHEEL_JOINT: &str = r#"
        def PhysicsRevoluteJoint "rev_left"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/wheel_left>
        }
"#;

    fn discover(body: &str) -> LinkTree {
        let path = write_temp("m", body);
        let machines = discover_machines_from_usd(&path).expect("scan");
        let _ = std::fs::remove_file(&path);
        machines.into_iter().next().expect("one machine").links
    }

    #[test]
    fn strict_tree_with_sensor_offset() {
        let tree = discover(&machine_with(CHASSIS_AND_WHEEL, WHEEL_JOINT));
        assert!(tree.is_valid(), "{:?}", tree.errors);
        assert!(!tree.derived);
        let names: Vec<&str> = tree.links.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["base_link", "imu", "wheel_left"]);
        let imu = tree.get("imu").unwrap();
        assert_eq!(imu.parent.as_deref(), Some("base_link"));
        assert_eq!(imu.role, LinkRole::Sensor);
        let off = imu.static_offset.unwrap();
        assert!((off.translation - DVec3::new(0.5, 0.0, 1.2)).length() < 1e-9);
        let wheel = tree.get("wheel_left").unwrap();
        assert_eq!(wheel.parent.as_deref(), Some("base_link"));
        assert_eq!(wheel.joint_prim.as_deref(), Some("/robot/Joints/rev_left"));
    }

    #[test]
    fn unmarked_body_is_an_error_in_strict_mode() {
        let links = format!(
            "{CHASSIS_AND_WHEEL}\n    def Xform \"loose\" (prepend apiSchemas = [\"PhysicsRigidBodyAPI\"]) {{}}\n"
        );
        let tree = discover(&machine_with(&links, WHEEL_JOINT));
        assert!(
            tree.errors
                .iter()
                .any(|e| e.contains("/robot/loose") && e.contains("GearboxLinkAPI"))
        );
    }

    #[test]
    fn two_bases_and_bad_names_are_errors() {
        let links = r#"
    def Xform "chassis" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:name = "Chassis-Main"
        token gearbox:link:role = "base"
    }
    def Xform "other" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:name = "base_link"
        token gearbox:link:role = "base"
    }
"#;
        let tree = discover(&machine_with(links, ""));
        assert!(
            tree.errors
                .iter()
                .any(|e| e.contains("2 links have role = base"))
        );
        assert!(tree.errors.iter().any(|e| e.contains("lowercase")));
    }

    #[test]
    fn loop_and_disconnected_are_errors() {
        let links = format!(
            "{CHASSIS_AND_WHEEL}\n    def Xform \"floating\" (prepend apiSchemas = [\"PhysicsRigidBodyAPI\", \"GearboxLinkAPI\"]) {{}}\n"
        );
        let joints = format!(
            "{WHEEL_JOINT}\n        def PhysicsFixedJoint \"second\"\n        {{\n            rel physics:body0 = </robot/floating>\n            rel physics:body1 = </robot/wheel_left>\n        }}\n"
        );
        let tree = discover(&machine_with(&links, &joints));
        assert!(
            tree.errors.iter().any(|e| e.contains("kinematic loop")),
            "{:?}",
            tree.errors
        );
        assert!(
            tree.errors
                .iter()
                .any(|e| e.contains("/robot/floating") && e.contains("not reachable")),
            "{:?}",
            tree.errors
        );
    }

    #[test]
    fn joint_to_foreign_prim_is_an_error() {
        let joints = format!(
            "{WHEEL_JOINT}\n        def PhysicsFixedJoint \"ext\"\n        {{\n            rel physics:body0 = </robot/chassis>\n            rel physics:body1 = </elsewhere/thing>\n        }}\n"
        );
        let tree = discover(&machine_with(CHASSIS_AND_WHEEL, &joints));
        assert!(tree.errors.iter().any(|e| e.contains("/elsewhere/thing")));
    }

    #[test]
    fn coupling_needs_a_link_and_a_known_type() {
        let links = format!(
            r#"{CHASSIS_AND_WHEEL}
    def Xform "hitch" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
    {{
        token gearbox:link:role = "tool"
        token gearbox:coupling:side = "hitch"
        token gearbox:coupling:type = "drawbar"
        token gearbox:coupling:name = "rear_drawbar"
        double3 xformOp:translate = (-1.9, 0, 0.45)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }}
    def Xform "bad" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
    {{
        token gearbox:coupling:side = "hitch"
        token gearbox:coupling:type = "rope"
    }}
"#
        );
        let tree = discover(&machine_with(&links, WHEEL_JOINT));
        let (link, c) = tree
            .couplings()
            .find(|(_, c)| c.name == "rear_drawbar")
            .expect("drawbar coupling");
        assert_eq!(c.side, CouplingSide::Hitch);
        // A tool link directly under the machine prim has no rigid-body
        // ancestor, so it is reported as disconnected.
        assert!(link.parent.is_none());
        assert!(
            tree.errors
                .iter()
                .any(|e| e.contains("/robot/hitch") && e.contains("not connected"))
        );
        assert!(tree.errors.iter().any(|e| e.contains("rope")));
    }

    #[test]
    fn unmarked_asset_gets_a_derived_tree() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/oxbo.usd");
        let machines = discover_machines_from_usd(&path).expect("oxbo.usd should scan");
        let tree = &machines[0].links;
        assert!(tree.is_valid(), "{:?}", tree.errors);
        assert!(tree.derived);
        assert_eq!(tree.links[0].name, "base_link");
        assert!(tree.links.len() > 1);
        assert!(tree.links.iter().skip(1).all(|l| l.parent.is_some()));
    }

    #[test]
    fn tractor_is_marked_and_strict() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/tractor.usd");
        let machines = discover_machines_from_usd(&path).expect("tractor.usd should scan");
        let tree = &machines[0].links;
        assert!(tree.is_valid(), "{:?}", tree.errors);
        assert!(!tree.derived);
        assert_eq!(
            tree.base().map(|b| b.prim_path.as_str()),
            Some("/robot/chassis")
        );
        assert_eq!(
            tree.links
                .iter()
                .filter(|l| l.role == LinkRole::Wheel)
                .count(),
            4
        );
        assert_eq!(
            tree.links
                .iter()
                .filter(|l| l.role == LinkRole::Steer)
                .count(),
            2
        );
        let steer = tree.get("steer_front_left").unwrap();
        assert_eq!(steer.parent.as_deref(), Some("base_link"));
        let wheel = tree.get("wheel_front_left").unwrap();
        assert_eq!(wheel.parent.as_deref(), Some("steer_front_left"));
        assert!(tree.couplings().any(|(_, c)| c.name == "rear_drawbar"));
    }

    #[test]
    fn trailer_is_a_strict_slave_with_coupler_and_hitch() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/trailer.usd");
        let machines = discover_machines_from_usd(&path).expect("trailer.usd should scan");
        let machine = &machines[0];
        assert_eq!(machine.kind.as_deref(), Some("trailer"));
        assert!(machine.controllers.is_empty());
        let tree = &machine.links;
        assert!(tree.is_valid(), "{:?}", tree.errors);
        assert!(!tree.derived);
        let (eye_link, eye) = tree
            .couplings()
            .find(|(_, c)| c.side == CouplingSide::Coupler)
            .expect("coupler");
        assert_eq!(eye.name, "eye");
        assert_eq!(eye.kind, "drawbar");
        assert_eq!(eye_link.parent.as_deref(), Some("base_link"));
        let off = eye_link.static_offset.unwrap();
        assert!((off.translation.y + 2.25).abs() < 1e-6, "{off:?}");
        assert!(
            off.rotation
                .angle_between(DQuat::from_rotation_z(std::f64::consts::PI))
                < 1e-6
        );
        assert!(tree.couplings().any(|(_, c)| c.side == CouplingSide::Hitch));
        assert_eq!(
            tree.links
                .iter()
                .filter(|l| l.role == LinkRole::Wheel)
                .count(),
            2
        );
    }

    #[test]
    fn yard_composes_two_machines_and_a_static_attachment() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/world/yard.usda");
        let machines = discover_machines_from_usd(&path).expect("yard.usda should scan");
        let kinds: Vec<&str> = machines.iter().filter_map(|m| m.kind.as_deref()).collect();
        assert!(
            kinds.contains(&"tractor") && kinds.contains(&"trailer"),
            "{kinds:?}"
        );
        for m in &machines {
            assert!(m.links.is_valid(), "{}: {:?}", m.prim_path, m.links.errors);
        }
        let pairs = crate::controller::discover_static_attachments_from_usd(&path);
        assert_eq!(
            pairs,
            vec![(
                "/World/Tractor_01/chassis/rear_hitch".to_string(),
                "/World/Trailer_01/chassis/drawbar_eye".to_string()
            )]
        );
        let tractor = machines
            .iter()
            .find(|m| m.kind.as_deref() == Some("tractor"))
            .unwrap();
        assert!(
            tractor.links.by_prim(&pairs[0].0).is_some(),
            "hitch prim is a link"
        );
    }

    #[test]
    fn sprayer_is_a_controlled_slave_with_requests() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sprayer.usd");
        let machines = discover_machines_from_usd(&path).expect("sprayer.usd should scan");
        let machine = &machines[0];
        assert_eq!(machine.kind.as_deref(), Some("sprayer"));
        assert!(machine.links.is_valid(), "{:?}", machine.links.errors);
        let boom = machine
            .controllers
            .iter()
            .find(|c| c.instance == "boom")
            .expect("boom controller");
        assert_eq!(boom.controller_type, "builtin:joint_position");
        assert_eq!(boom.target.as_deref(), Some("/robot/Joints/boom_fold"));
        assert_eq!(
            boom.requests,
            vec!["speed".to_string(), "hitch:rear_lift".to_string()]
        );
        let (_, coupler) = machine
            .links
            .couplings()
            .find(|(_, c)| c.side == CouplingSide::Coupler)
            .expect("three-point coupler");
        assert_eq!(coupler.kind, "three_point_mounted");
        let boom_link = machine.links.get("boom").expect("boom link");
        assert_eq!(boom_link.parent.as_deref(), Some("base_link"));
        assert_eq!(
            boom_link.joint_prim.as_deref(),
            Some("/robot/Joints/boom_fold")
        );
        assert_eq!(
            boom_link.element.as_ref().map(|e| e.kind.as_str()),
            Some("function")
        );
        let left = machine.links.get("section_left").expect("section link");
        assert_eq!(left.parent.as_deref(), Some("boom"));
        assert_eq!(
            left.element.as_ref().map(|e| e.kind.as_str()),
            Some("section")
        );
        assert!(
            left.values
                .contains(&("ActualWorkingWidth".to_string(), 3.0)),
            "{:?}",
            left.values
        );
        let tank = machine.links.get("tank").expect("tank link");
        assert_eq!(tank.parent.as_deref(), Some("base_link"));
        assert_eq!(tank.element.as_ref().map(|e| e.kind.as_str()), Some("bin"));

        let tractor = discover_machines_from_usd(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/tractor.usd"),
        )
        .expect("tractor");
        assert_eq!(
            tractor[0].grants,
            vec![
                "speed".to_string(),
                "steering".to_string(),
                "pto:pto".to_string()
            ]
        );
        assert!(
            tractor[0]
                .links
                .couplings()
                .any(|(_, c)| c.name == "rear_three_point" && c.kind == "three_point_mounted")
        );
    }
}

/// A link that is also a working part of the machine (`TOOLS_SPEC.md`
/// §7.3): the tree stays the link tree, the element is a label on it.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementInfo {
    pub kind: String,
    pub number: Option<u32>,
    pub designator: String,
}

pub const ELEMENT_KINDS: [&str; 7] = [
    "device",
    "function",
    "bin",
    "section",
    "unit",
    "connector",
    "navigation",
];

fn read_element(
    stage: &openusd::Stage,
    prim: &SdfPath,
    errors: &mut Vec<String>,
) -> Option<ElementInfo> {
    let kind = read_token(stage, prim, "gearbox:element:type")?;
    if !ELEMENT_KINDS.contains(&kind.as_str()) {
        errors.push(format!(
            "{}: unknown gearbox:element:type `{kind}`",
            prim.as_str()
        ));
        return None;
    }
    let number = match read_attr(stage, prim, "gearbox:element:number") {
        Some(Value::Int(n)) if n >= 0 => Some(n as u32),
        Some(Value::Uint(n)) => Some(n),
        _ => None,
    };
    let designator = match read_attr(stage, prim, "gearbox:element:designator") {
        Some(Value::String(s)) | Some(Value::Token(s)) => s,
        _ => leaf(prim.as_str()).to_string(),
    };
    Some(ElementInfo {
        kind,
        number,
        designator,
    })
}

/// Every `gearbox:value:*` attribute on a prim, sorted by name.
fn read_values(stage: &openusd::Stage, prim: &SdfPath) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for name in stage.prim_properties(prim.clone()).unwrap_or_default() {
        let Some(key) = name.strip_prefix("gearbox:value:") else {
            continue;
        };
        let value = match read_attr(stage, prim, &name) {
            Some(Value::Float(v)) => v as f64,
            Some(Value::Double(v)) => v,
            Some(Value::Int(v)) => v as f64,
            Some(Value::Uint(v)) => v as f64,
            Some(Value::Bool(b)) => b as u8 as f64,
            _ => continue,
        };
        out.push((key.to_string(), value));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
