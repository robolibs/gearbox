//! Stage → resolved machines: scope discovery, body/joint/DOF resolution and
//! unit normalization. Graph classification, frames, bindings and
//! capabilities run on the result in their own modules.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy::math::{DQuat, DVec3};
use openusd::usd::Stage;

use super::diagnostics::{self, Code, Diagnostic, Severity};
use super::model::*;
use super::read;
use super::scene::Scene;
use super::units::read_stage_units;
use super::xform::decompose;

#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    /// One asset: the given entry prim, else the stage's defaultPrim, else everything.
    Asset { entry: Option<String> },
    /// A world: every machine on the stage, whatever the defaultPrim says.
    World,
}

#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub selection: Selection,
    pub load_instance: String,
    pub generation: u64,
    pub meters_per_unit_override: Option<f64>,
    pub physics_rate_hz: f64,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            selection: Selection::Asset { entry: None },
            load_instance: "load".to_string(),
            generation: 1,
            meters_per_unit_override: None,
            physics_rate_hz: 120.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompileResult {
    pub units: StageUnits,
    pub scan_root: String,
    pub machines: Vec<MachineDescription>,
    /// Stage-level findings that belong to no machine.
    pub diagnostics: Vec<Diagnostic>,
}

impl CompileResult {
    pub fn machine(&self, root: &str) -> Option<&MachineDescription> {
        self.machines.iter().find(|m| m.root == root)
    }
}

/// Prims that declare a Gearbox machine.
pub(super) fn is_explicit_root(scene: &Scene, path: &str) -> bool {
    scene.has_schema(path, "GearboxMachineAPI")
        || read::token(scene, path, "gearbox:machine:kind").is_some()
        || read::token(scene, path, "gearbox:machine:idPolicy").is_some()
        || read::first_target(scene, path, "gearbox:machine:body").is_some()
}

#[derive(Clone, Debug, Default)]
pub(super) struct Scope {
    pub root: String,
    pub explicit: bool,
    pub isaac: bool,
    pub bodies: BTreeSet<String>,
    pub joints: BTreeSet<String>,
    pub markers: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Body a joint relationship target reaches: itself or its nearest body ancestor.
fn target_body(scene: &Scene, target: &str) -> Option<String> {
    if scene.is_body(target) {
        Some(target.to_string())
    } else if scene.exists(target) {
        scene.nearest_body_ancestor(target)
    } else {
        None
    }
}

fn joint_target(scene: &Scene, joint: &str, rel: &str) -> Option<String> {
    read::first_target(scene, joint, rel)
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn find(&mut self, i: usize) -> usize {
        let mut i = i;
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[b.max(a)] = a.min(b);
        }
    }
}

fn lowest_common_ancestor<'p>(paths: impl Iterator<Item = &'p String>) -> String {
    let mut prefix: Option<Vec<&str>> = None;
    for path in paths {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        prefix = Some(match prefix {
            None => parts,
            Some(p) => p.iter().zip(parts.iter()).take_while(|(a, b)| a == b).map(|(a, _)| *a).collect(),
        });
    }
    let parts = prefix.unwrap_or_default();
    if parts.is_empty() { "/".to_string() } else { format!("/{}", parts.join("/")) }
}

pub(super) fn discover_scopes(scene: &Scene) -> Vec<Scope> {
    let paths: Vec<&str> = scene.prims.iter().map(|p| p.path.as_str()).collect();
    let explicit: Vec<String> = paths.iter().filter(|p| is_explicit_root(scene, p)).map(|p| p.to_string()).collect();
    let bodies: Vec<String> = paths.iter().filter(|p| scene.is_body(p)).map(|p| p.to_string()).collect();
    let joints: Vec<String> = paths.iter().filter(|p| scene.is_joint(p)).map(|p| p.to_string()).collect();
    let body_index: HashMap<&str, usize> = bodies.iter().enumerate().map(|(i, b)| (b.as_str(), i)).collect();

    let mut uf = UnionFind((0..bodies.len()).collect());
    let mut joint_bodies: HashMap<&str, (Option<String>, Option<String>)> = HashMap::new();
    for joint in &joints {
        let b0 = joint_target(scene, joint, "physics:body0").and_then(|t| target_body(scene, &t));
        let b1 = joint_target(scene, joint, "physics:body1").and_then(|t| target_body(scene, &t));
        let enabled = read::boolean(scene, joint, "physics:jointEnabled").unwrap_or(true);
        if enabled
            && let (Some(a), Some(b)) = (b0.as_deref().and_then(|b| body_index.get(b)), b1.as_deref().and_then(|b| body_index.get(b)))
        {
            uf.union(*a, *b);
        }
        joint_bodies.insert(joint.as_str(), (b0, b1));
    }

    let mut scopes = Vec::new();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for root in &explicit {
        let mut scope = Scope { root: root.clone(), explicit: true, ..Default::default() };
        for other in &explicit {
            if other != root && (Scene::is_under(root, other) || Scene::is_under(other, root)) {
                scope.diagnostics.push(Diagnostic::error(
                    Code::NestedMachineRoot,
                    Some(root),
                    format!("machine root nests with machine root {other}; machine roots must not nest"),
                ));
            }
        }
        scope.bodies = bodies.iter().filter(|b| Scene::is_under(b, root)).cloned().collect();
        scope.joints = joints.iter().filter(|j| Scene::is_under(j, root)).cloned().collect();
        scope.markers = paths
            .iter()
            .filter(|p| Scene::is_under(p, root) && scene.has_schema(p, "PhysicsArticulationRootAPI"))
            .map(|p| p.to_string())
            .collect();
        claimed.extend(scope.bodies.iter().cloned());
        scopes.push(scope);
    }

    let inside_explicit = |p: &str| explicit.iter().any(|r| Scene::is_under(p, r));
    let component_of = |uf: &mut UnionFind, body: &str| body_index.get(body).map(|&i| uf.find(i));

    let add_standard = |scopes: &mut Vec<Scope>, claimed: &mut BTreeSet<String>, root: String, isaac: bool, seeds: BTreeSet<String>, markers: Vec<String>, uf: &mut UnionFind| {
        let components: BTreeSet<usize> = seeds.iter().filter_map(|b| component_of(uf, b)).collect();
        let mut scope = Scope { root: root.clone(), isaac, markers, ..Default::default() };
        for (i, body) in bodies.iter().enumerate() {
            let in_component = components.contains(&uf.find(i));
            if (in_component || Scene::is_under(body, &root)) && !claimed.contains(body) {
                scope.bodies.insert(body.clone());
            } else if in_component && claimed.contains(body) {
                scope.diagnostics.push(Diagnostic::error(
                    Code::CrossMachineJoint,
                    Some(body),
                    "body joined to this articulation belongs to another machine",
                ));
            }
        }
        for joint in &joints {
            let (b0, b1) = &joint_bodies[joint.as_str()];
            let touches = [b0, b1].iter().any(|b| b.as_ref().is_some_and(|b| scope.bodies.contains(b)));
            if (touches || Scene::is_under(joint, &root)) && !inside_explicit(joint) {
                scope.joints.insert(joint.clone());
            }
        }
        claimed.extend(scope.bodies.iter().cloned());
        scopes.push(scope);
    };

    // Isaac robots that are not Gearbox machines.
    let isaac_roots: Vec<String> = paths
        .iter()
        .filter(|p| scene.has_schema(p, "IsaacRobotAPI") && !inside_explicit(p))
        .filter(|p| !Scene::ancestors(p).any(|a| scene.has_schema(a, "IsaacRobotAPI")))
        .map(|p| p.to_string())
        .collect();
    for root in &isaac_roots {
        let mut seeds: BTreeSet<String> = bodies.iter().filter(|b| Scene::is_under(b, root)).cloned().collect();
        for link in read::targets(scene, root, "isaac:physics:robotLinks").unwrap_or_default() {
            if let Some(body) = target_body(scene, &link) {
                seeds.insert(body);
            }
        }
        let markers = paths
            .iter()
            .filter(|p| Scene::is_under(p, root) && scene.has_schema(p, "PhysicsArticulationRootAPI"))
            .map(|p| p.to_string())
            .collect();
        add_standard(&mut scopes, &mut claimed, root.clone(), true, seeds, markers, &mut uf);
    }

    // Articulation markers outside every explicit and Isaac scope, grouped by
    // the articulation they mark.
    let mut groups: BTreeMap<usize, (Vec<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut seedless: Vec<String> = Vec::new();
    for marker in paths.iter().filter(|p| scene.has_schema(p, "PhysicsArticulationRootAPI")) {
        if inside_explicit(marker) || isaac_roots.iter().any(|r| Scene::is_under(marker, r)) {
            continue;
        }
        let mut seeds: BTreeSet<String> = BTreeSet::new();
        if scene.is_body(marker) {
            seeds.insert(marker.to_string());
        } else if scene.is_joint(marker) {
            let (b0, b1) = &joint_bodies[*marker];
            seeds.extend(b0.iter().chain(b1.iter()).cloned());
        } else {
            seeds.extend(bodies.iter().filter(|b| Scene::is_under(b, marker)).cloned());
        }
        let Some(component) = seeds.iter().filter_map(|b| component_of(&mut uf, b)).min() else {
            seedless.push(marker.to_string());
            continue;
        };
        let entry = groups.entry(component).or_default();
        entry.0.push(marker.to_string());
        entry.1.extend(seeds);
    }
    for (component, (markers, seeds)) in groups {
        let members: Vec<String> = bodies
            .iter()
            .enumerate()
            .filter(|(i, b)| uf.find(*i) == component && !claimed.contains(*b))
            .map(|(_, b)| b.clone())
            .collect();
        let root = match markers.as_slice() {
            [single] if !scene.is_body(single) && !scene.is_joint(single) => single.clone(),
            _ => {
                let lca = lowest_common_ancestor(markers.iter().chain(members.iter()));
                if lca == "/" { markers[0].clone() } else { lca }
            }
        };
        let mut duplicate = Vec::new();
        if markers.len() > 1 {
            duplicate.push(Diagnostic::warning(
                Code::DuplicateArticulationMarker,
                Some(&root),
                format!("one articulation carries {} PhysicsArticulationRootAPI markers: {}", markers.len(), markers.join(", ")),
            ));
        }
        add_standard(&mut scopes, &mut claimed, root, false, seeds, markers, &mut uf);
        scopes.last_mut().expect("just pushed").diagnostics.extend(duplicate);
    }
    for marker in seedless {
        let mut scope = Scope { root: marker.clone(), markers: vec![marker.clone()], ..Default::default() };
        scope.diagnostics.push(Diagnostic::warning(
            Code::DisconnectedBody,
            Some(&marker),
            "articulation marker reaches no rigid body",
        ));
        scopes.push(scope);
    }
    scopes.sort_by(|a, b| a.root.cmp(&b.root));
    scopes
}

pub fn compile(stage: &Stage, options: &CompileOptions) -> CompileResult {
    let mut stage_diagnostics = Vec::new();
    let units = read_stage_units(stage, options.meters_per_unit_override, &mut stage_diagnostics);
    let scan_root = match &options.selection {
        Selection::Asset { entry: Some(entry) } => entry.clone(),
        Selection::Asset { entry: None } => stage.default_prim().map(|p| format!("/{p}")).unwrap_or_else(|| "/".to_string()),
        Selection::World => "/".to_string(),
    };
    let scene = Scene::new(stage, &scan_root, units.clone(), stage_diagnostics);
    let scopes = discover_scopes(&scene);
    let mut machines: Vec<MachineDescription> = scopes.into_iter().map(|scope| build_machine(&scene, scope, options)).collect();

    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in machines.iter().enumerate() {
        by_id.entry(m.machine_id.clone()).or_default().push(i);
    }
    for indices in by_id.values().filter(|v| v.len() > 1) {
        for &i in indices {
            let others: Vec<String> = indices.iter().filter(|&&j| j != i).map(|&j| machines[j].root.clone()).collect();
            let m = &mut machines[i];
            m.diagnostics.push(Diagnostic::error(
                Code::DuplicateMachineId,
                Some(&m.root),
                format!("machine id `{}` is also used by {}", m.machine_id, others.join(", ")),
            ));
        }
    }
    // Invalid stage units invalidate every machine measured in them.
    let unit_errors: Vec<Diagnostic> =
        scene.diagnostics.iter().filter(|d| d.code == Code::InvalidUnits && d.severity == Severity::Error).cloned().collect();
    for m in &mut machines {
        m.diagnostics.extend(unit_errors.iter().cloned());
        diagnostics::sort(&mut m.diagnostics);
    }
    let mut diagnostics = scene.diagnostics.clone();
    if machines.is_empty() {
        diagnostics.push(Diagnostic::info(Code::NoMachines, Some(&scan_root), "no machine on the scanned stage"));
    }
    diagnostics::sort(&mut diagnostics);
    CompileResult { units, scan_root, machines, diagnostics }
}

pub fn derive_machine_id(prim_path: &str) -> String {
    let mut out = String::new();
    for ch in prim_path.trim_matches('/').chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() { "machine".to_string() } else { out }
}

pub fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Shared state while one machine is built.
pub(super) struct Builder<'s, 'a> {
    pub scene: &'s Scene<'a>,
    pub scope: Scope,
    pub profile: ProfileKind,
    pub machine: MachineDescription,
}

impl Builder<'_, '_> {
    pub fn error(&mut self, code: Code, path: Option<&str>, message: impl Into<String>) {
        self.machine.diagnostics.push(Diagnostic::error(code, path, message));
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.machine.diagnostics.push(diagnostic);
    }

    fn check_numeric(&mut self, path: &str, what: &str, v: f64) -> bool {
        if v.is_finite() {
            return true;
        }
        self.push(Diagnostic::error(Code::InvalidNumeric, Some(path), format!("{what} = {v} is not finite")));
        false
    }

    fn check_vec(&mut self, path: &str, what: &str, v: DVec3) -> bool {
        if v.is_finite() {
            return true;
        }
        self.push(Diagnostic::error(Code::InvalidNumeric, Some(path), format!("{what} = {v:?} is not finite")));
        false
    }
}

fn build_machine(scene: &Scene, mut scope: Scope, options: &CompileOptions) -> MachineDescription {
    let diagnostics = std::mem::take(&mut scope.diagnostics);
    let profile = if scope.explicit { ProfileKind::Gearbox } else { ProfileKind::Standard };
    let machine_id = if scope.explicit {
        read::token(scene, &scope.root, "gearbox:machine:id").unwrap_or_else(|| derive_machine_id(&scope.root))
    } else {
        derive_machine_id(&scope.root)
    };
    let kind = scope.explicit.then(|| read::token(scene, &scope.root, "gearbox:machine:kind")).flatten();
    let machine = MachineDescription {
        load_instance: options.load_instance.clone(),
        root: scope.root.clone(),
        machine_id,
        generation: options.generation,
        profile,
        kind,
        isaac: scope.isaac,
        units: scene.units.clone(),
        control_body: None,
        base_frame: None,
        control_frame: None,
        bodies: Vec::new(),
        joints: Vec::new(),
        dofs: Vec::new(),
        frames: Vec::new(),
        tree: Vec::new(),
        articulations: Vec::new(),
        controllers: Vec::new(),
        owners: BTreeMap::new(),
        reporting: None,
        capabilities: Vec::new(),
        diagnostics,
        command_validity_ms: DEFAULT_COMMAND_VALIDITY_MS,
    };
    let mut b = Builder { scene, scope, profile, machine };

    let body_paths: Vec<String> = b.scope.bodies.iter().cloned().collect();
    for path in &body_paths {
        let body = read_body(&mut b, path);
        b.machine.bodies.push(body);
    }
    let joint_paths: Vec<String> = b.scope.joints.iter().cloned().collect();
    for path in &joint_paths {
        read_joint(&mut b, path);
    }
    super::isaac::apply(&mut b);
    super::frames::build(&mut b);
    super::graph::build(&mut b);
    super::bindings::build(&mut b, options.physics_rate_hz);
    super::frames::check_names(&mut b);
    super::capabilities::evaluate(&mut b);
    b.machine
}

fn read_body(b: &mut Builder, path: &str) -> BodyDescription {
    let scene = b.scene;
    let units = scene.units.clone();
    let mode = if read::boolean(scene, path, "physics:rigidBodyEnabled") == Some(false) {
        BodyMode::Disabled
    } else if read::boolean(scene, path, "physics:kinematicEnabled") == Some(true) {
        BodyMode::Kinematic
    } else {
        BodyMode::Dynamic
    };
    let d = scene.decomposed(path);
    if d.shear {
        b.error(Code::ShearTransform, Some(path), "rigid body transform is sheared; physics cannot represent it");
    }
    if d.reflection {
        b.error(Code::ReflectionTransform, Some(path), "rigid body transform mirrors its axes; physics cannot represent it");
    }
    let scale = match d.uniform_scale() {
        Some(s) => s,
        None => {
            b.error(
                Code::NonUniformScale,
                Some(path),
                format!("rigid body has nonuniform scale {:?}; author the scale on its meshes instead", d.scale),
            );
            (d.scale.x + d.scale.y + d.scale.z) / 3.0
        }
    };
    for op in scene.unsupported_ops(path) {
        b.push(Diagnostic::warning(Code::UnsupportedXformOp, Some(path), format!("xform op ignored: {op}")));
    }

    let mut mass_value = read::float(scene, path, "physics:mass").filter(|m| *m != 0.0);
    if let Some(m) = mass_value
        && !(m.is_finite() && m > 0.0)
    {
        b.push(Diagnostic::error(Code::InvalidMass, Some(path), format!("physics:mass = {m} must be positive and finite")));
        mass_value = None;
    }
    let density = read::float(scene, path, "physics:density").filter(|d| *d > 0.0);
    let com = read::vec3(scene, path, "physics:centerOfMass");
    if let Some(c) = com {
        b.check_vec(path, "physics:centerOfMass", c);
    }
    let inertia = read::vec3(scene, path, "physics:diagonalInertia").filter(|i| *i != DVec3::ZERO);
    let slack = |i: DVec3| 1e-5 * i.max_element();
    if let Some(i) = inertia
        && (!i.is_finite()
            || i.min_element() < 0.0
            || i.x + i.y < i.z - slack(i)
            || i.y + i.z < i.x - slack(i)
            || i.z + i.x < i.y - slack(i))
    {
        b.push(Diagnostic::error(
            Code::InvalidInertia,
            Some(path),
            format!("physics:diagonalInertia = {i:?} is not a physical inertia (nonnegative, triangle inequality)"),
        ));
    }
    let axes = match read::quat(scene, path, "physics:principalAxes") {
        Some(Ok(q)) => Valued::authored(q),
        Some(Err(e)) => {
            b.push(Diagnostic::error(Code::InvalidInertia, Some(path), e));
            Valued::schema(DQuat::IDENTITY)
        }
        None => Valued::schema(DQuat::IDENTITY),
    };
    if mass_value.is_some() && inertia.is_none() {
        b.push(Diagnostic::warning(
            Code::MissingInertia,
            Some(path),
            "explicit mass without diagonalInertia: the backend derives inertia from colliders",
        ));
    }

    let parent_rotation = Scene::ancestors(path)
        .next()
        .map(|parent| decompose(&scene.world_matrix(parent)).rigid.rotation)
        .unwrap_or(DQuat::IDENTITY);
    let velocity = read::vec3(scene, path, "physics:velocity");
    let angular = read::vec3(scene, path, "physics:angularVelocity");
    let to_world = |v: DVec3, scale: f64| scene.up * (parent_rotation * v) * scale;
    let initial_linear_velocity = match velocity {
        Some(v) => Valued::authored(to_world(v, units.l())),
        None => Valued::schema(DVec3::ZERO),
    };
    let initial_angular_velocity = match angular {
        Some(v) => Valued::authored(to_world(v, std::f64::consts::PI / 180.0)),
        None => Valued::schema(DVec3::ZERO),
    };

    let mut damping = |attr: &str, schema: f64| match read::float(scene, path, attr) {
        Some(v) if v.is_finite() && v >= 0.0 => Valued::authored(v),
        Some(v) => {
            b.push(Diagnostic::error(Code::InvalidNumeric, Some(path), format!("{attr} = {v} must be nonnegative and finite")));
            Valued::schema(schema)
        }
        None => Valued::schema(schema),
    };
    let linear_damping = damping("physxRigidBody:linearDamping", SCHEMA_LINEAR_DAMPING);
    let angular_damping = damping("physxRigidBody:angularDamping", SCHEMA_ANGULAR_DAMPING);

    let colliders = read_colliders(scene, path);
    let name = read::token(scene, path, "gearbox:link:name")
        .or_else(|| read::token(scene, path, "isaac:nameOverride").filter(|n| !n.is_empty()))
        .unwrap_or_else(|| leaf(path).to_string());
    BodyDescription {
        path: path.to_string(),
        name,
        mode,
        mass: MassProperties {
            mass: mass_value.map(|m| Valued::authored(units.mass(m))),
            density: density.map(|d| Valued::authored(units.density(d))),
            center_of_mass: com.map(|c| Valued::authored(c * units.l() * scale)).unwrap_or_else(|| Valued::schema(DVec3::ZERO)),
            diagonal_inertia: inertia.map(|i| Valued::authored(i * units.inertia(1.0))),
            principal_axes: axes,
        },
        initial_pose: scene.rigid_world(path),
        scale,
        initial_linear_velocity,
        initial_angular_velocity,
        linear_damping,
        angular_damping,
        colliders,
    }
}

fn read_colliders(scene: &Scene, body: &str) -> Vec<ColliderBinding> {
    scene
        .prims
        .iter()
        .filter(|p| Scene::is_under(&p.path, body) && p.schemas.iter().any(|s| s == "PhysicsCollisionAPI"))
        .filter(|p| {
            let owner = if scene.is_body(&p.path) { Some(p.path.clone()) } else { scene.nearest_body_ancestor(&p.path) };
            owner.as_deref() == Some(body)
        })
        .map(|p| ColliderBinding {
            path: p.path.clone(),
            shape: p.type_name.clone(),
            approximation: read::token(scene, &p.path, "physics:approximation"),
            enabled: read::boolean(scene, &p.path, "physics:collisionEnabled").unwrap_or(true),
        })
        .collect()
}

/// Where a joint's local frame is expressed.
enum Space {
    Body(usize, String),
    World,
}

fn resolve_endpoint(b: &mut Builder, joint: &str, rel: &str) -> (EndpointRecord, Option<(Space, Option<String>)>) {
    let Some(authored) = read::first_target(b.scene, joint, rel) else {
        return (EndpointRecord { authored: None, endpoint: Endpoint::World, via: None }, Some((Space::World, None)));
    };
    let target = authored.clone();
    let invalid = |reason: &str| EndpointRecord {
        authored: Some(authored.clone()),
        endpoint: Endpoint::Invalid { target: target.clone(), reason: reason.to_string() },
        via: None,
    };
    if !b.scene.exists(&target) {
        let reason = if b.scene.defined(&target) { "target prim is inactive" } else { "target prim does not exist" };
        b.push(Diagnostic::error(Code::InvalidEndpoint, Some(joint), format!("{rel} -> {authored}: {reason}")));
        return (invalid(reason), None);
    }
    let body = if b.scene.is_body(&target) { Some(target.clone()) } else { b.scene.nearest_body_ancestor(&target) };
    match body {
        Some(body) => match b.machine.body_index(&body) {
            Some(i) => {
                let via = (body != target).then(|| format!("ancestor:{body}"));
                if via.is_some() {
                    b.push(Diagnostic::info(Code::EndpointViaAncestor, Some(joint), format!("{rel} -> {target} rides body {body}")));
                }
                let frame_prim = (body != target).then(|| target.clone());
                (EndpointRecord { authored: Some(authored), endpoint: Endpoint::Body(i), via }, Some((Space::Body(i, body), frame_prim)))
            }
            None => {
                b.error(Code::CrossMachineJoint, Some(joint), format!("{rel} -> {target}: body {body} is outside machine {}", b.scope.root));
                (invalid("body outside this machine"), None)
            }
        },
        None => {
            b.push(Diagnostic::info(Code::StaticEndpoint, Some(joint), format!("{rel} -> {target} is not under a rigid body: world-fixed frame")));
            (
                EndpointRecord { authored: Some(authored), endpoint: Endpoint::World, via: Some(format!("static:{target}")) },
                Some((Space::World, Some(target))),
            )
        }
    }
}

/// The joint frame in its endpoint's space, SI. `local` is in stage units.
fn local_frame(scene: &Scene, space: &Space, frame_prim: Option<&str>, local_pos: DVec3, local_rot: DQuat) -> RigidTransform {
    match space {
        Space::Body(_, body) => {
            let wb = scene.world_matrix(body);
            let m = match frame_prim {
                Some(p) => wb.inverse() * scene.world_matrix(p),
                None => bevy::math::DMat4::IDENTITY,
            };
            let scale = decompose(&wb).scale;
            let p = m.transform_point3(local_pos) * scale * scene.units.l();
            let r = decompose(&m).rigid.rotation * local_rot;
            RigidTransform::new(p, r.normalize())
        }
        Space::World => {
            let m = frame_prim.map(|p| scene.world_matrix(p)).unwrap_or(bevy::math::DMat4::IDENTITY);
            let p = m.transform_point3(local_pos);
            let r = decompose(&m).rigid.rotation * local_rot;
            scene.to_world(RigidTransform::new(p, r))
        }
    }
}

fn read_joint(b: &mut Builder, path: &str) {
    let scene = b.scene;
    let type_name = scene.info(path).map(|p| p.type_name.clone()).unwrap_or_default();
    let Some(kind) = JointKind::from_type_name(&type_name) else { return };
    if let JointKind::Unknown(name) = &kind {
        b.error(Code::UnsupportedJointType, Some(path), format!("joint type {name} is not a UsdPhysics joint"));
    }
    let enabled = read::boolean(scene, path, "physics:jointEnabled").unwrap_or(true);
    let (body0, space0) = resolve_endpoint(b, path, "physics:body0");
    let (body1, space1) = resolve_endpoint(b, path, "physics:body1");

    let pos = |b: &mut Builder, name: &str| {
        let v = read::vec3(scene, path, name).unwrap_or(DVec3::ZERO);
        if b.check_vec(path, name, v) { v } else { DVec3::ZERO }
    };
    let rot = |b: &mut Builder, name: &str| match read::quat(scene, path, name) {
        Some(Ok(q)) => q,
        Some(Err(e)) => {
            b.push(Diagnostic::error(Code::InvalidNumeric, Some(path), e));
            DQuat::IDENTITY
        }
        None => DQuat::IDENTITY,
    };
    let (p0, r0, p1, r1) = (pos(b, "physics:localPos0"), rot(b, "physics:localRot0"), pos(b, "physics:localPos1"), rot(b, "physics:localRot1"));
    let local0 = space0.as_ref().map(|(s, f)| local_frame(scene, s, f.as_deref(), p0, r0)).unwrap_or_default();
    let local1 = space1.as_ref().map(|(s, f)| local_frame(scene, s, f.as_deref(), p1, r1)).unwrap_or_default();

    let axis_token = read::token(scene, path, "physics:axis");
    let axis = match (&kind, axis_token.as_deref()) {
        (JointKind::Revolute | JointKind::Prismatic | JointKind::Spherical, Some(t)) => match Axis::parse(t) {
            Some(a) => Some(a),
            None => {
                b.push(Diagnostic::error(Code::InvalidNumeric, Some(path), format!("physics:axis `{t}` is not X, Y or Z")));
                Some(Axis::X)
            }
        },
        (JointKind::Revolute | JointKind::Prismatic | JointKind::Spherical, None) => Some(Axis::X),
        _ => None,
    };

    let both_world = matches!((&body0.endpoint, &body1.endpoint), (Endpoint::World, Endpoint::World));
    if both_world {
        b.push(Diagnostic::error(Code::BothEndpointsWorld, Some(path), "neither endpoint resolves to a body of this machine"));
    }
    if let (Some(a), Some(c)) = (body0.endpoint.body(), body1.endpoint.body())
        && a == c
    {
        b.push(Diagnostic::error(Code::SameBodyEndpoints, Some(path), "body0 and body1 are the same body"));
    }
    let invalid = matches!(body0.endpoint, Endpoint::Invalid { .. }) || matches!(body1.endpoint, Endpoint::Invalid { .. }) || both_world;
    let class = if invalid {
        JointClass::Invalid
    } else if !enabled {
        b.push(Diagnostic::info(Code::DisabledJoint, Some(path), "jointEnabled = false: described, not simulated"));
        JointClass::Disabled
    } else {
        JointClass::Tree
    };
    let units = scene.units.clone();
    let effort_limit = |b: &mut Builder, name: &str, rotational: bool| {
        let v = read::float(scene, path, name)?;
        if v.is_nan() || v < 0.0 {
            b.push(Diagnostic::error(Code::InvalidNumeric, Some(path), format!("{name} = {v} must be nonnegative")));
            return None;
        }
        v.is_finite().then(|| units.effort(rotational, v))
    };
    let break_force = effort_limit(b, "physics:breakForce", false);
    let break_torque = effort_limit(b, "physics:breakTorque", true);

    let joint_index = b.machine.joints.len();
    let name = read::token(scene, path, "isaac:NameOverride")
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| leaf(path).to_string());
    b.machine.joints.push(JointDescription {
        path: path.to_string(),
        name,
        kind: kind.clone(),
        enabled,
        collision_enabled: read::boolean(scene, path, "physics:collisionEnabled").unwrap_or(false),
        exclude_from_articulation: read::boolean(scene, path, "physics:excludeFromArticulation").unwrap_or(false),
        body0,
        body1,
        local0,
        local1,
        axis,
        dofs: Vec::new(),
        class,
        break_force,
        break_torque,
    });
    let tokens: Vec<DofToken> = match kind {
        JointKind::Revolute => vec![DofToken::Angular],
        JointKind::Prismatic => vec![DofToken::Linear],
        JointKind::Distance => vec![DofToken::Distance],
        JointKind::D6 => DofToken::D6.to_vec(),
        _ => Vec::new(),
    };
    let multi = tokens.len() > 1;
    for token in tokens {
        if let Some(dof) = read_dof(b, joint_index, token, multi) {
            let index = b.machine.dofs.len();
            b.machine.dofs.push(dof);
            b.machine.joints[joint_index].dofs.push(index);
        }
    }
    let joint = b.machine.joints[joint_index].clone();
    let all_tokens = [DofToken::Angular, DofToken::Linear, DofToken::Distance].into_iter().chain(DofToken::D6);
    for token in all_tokens {
        let name = format!("gearbox:joint:{}:commandInterfaces", token.as_str());
        let has_dof = joint.dofs.iter().any(|&d| b.machine.dofs[d].token == token);
        if !has_dof && read::tokens(scene, path, &name).is_some() {
            b.push(Diagnostic::error(Code::InvalidInterface, Some(path), format!("{name}: this {} joint has no such DOF to command", joint.kind.as_str())));
        }
    }
    for applied in scene.schema_instances(path, "PhysicsDriveAPI") {
        let used = joint.dofs.iter().any(|&d| b.machine.dofs[d].drive.is_some() && drive_token_for(&b.machine.dofs[d], &applied));
        if !used {
            b.error(
                Code::InvalidDrive,
                Some(path),
                format!("PhysicsDriveAPI:{applied} names no free DOF of this {} joint", joint.kind.as_str()),
            );
        }
    }
}

fn drive_token_for(dof: &DofDescription, applied: &str) -> bool {
    applied == dof.token.as_str()
        || (dof.token == DofToken::Angular && matches!(applied, "rotX" | "rotY" | "rotZ"))
        || (dof.token == DofToken::Linear && matches!(applied, "transX" | "transY" | "transZ"))
}

fn read_limit(b: &mut Builder, path: &str, token: DofToken, joint_kind: &JointKind) -> Limit {
    let scene = b.scene;
    let rotational = token.rotational();
    let (low_name, high_name) = match (joint_kind, token) {
        (JointKind::Revolute | JointKind::Prismatic, _) => ("physics:lowerLimit".to_string(), "physics:upperLimit".to_string()),
        (JointKind::Distance, _) => ("physics:minDistance".to_string(), "physics:maxDistance".to_string()),
        (_, t) => (format!("limit:{}:physics:low", t.as_str()), format!("limit:{}:physics:high", t.as_str())),
    };
    let low = read::float(scene, path, &low_name);
    let high = read::float(scene, path, &high_name);
    if low.is_none() && high.is_none() {
        return Limit::unbounded(Provenance::SchemaDefault);
    }
    if low.is_some_and(f64::is_nan) || high.is_some_and(f64::is_nan) {
        b.push(Diagnostic::error(Code::InvalidLimit, Some(path), format!("{low_name}/{high_name} is NaN")));
        return Limit::unbounded(Provenance::Authored);
    }
    let convert = |v: Option<f64>| {
        let v = v?;
        if matches!(joint_kind, JointKind::Distance) && v < 0.0 {
            return None;
        }
        v.is_finite().then(|| scene.units.coordinate(rotational, v))
    };
    let (lower, upper) = (convert(low), convert(high));
    let locked = matches!(joint_kind, JointKind::D6) && matches!((lower, upper), (Some(l), Some(h)) if l > h);
    if !locked && let (Some(l), Some(h)) = (lower, upper) && l > h {
        b.error(Code::InvalidLimit, Some(path), format!("{low_name} {l} exceeds {high_name} {h}"));
    }
    Limit { lower, upper, locked, source: Provenance::Authored }
}

fn read_drive(b: &mut Builder, path: &str, token: DofToken) -> Option<DriveDescription> {
    let scene = b.scene;
    let applied = scene.schema_instances(path, "PhysicsDriveAPI");
    let chosen = if applied.iter().any(|a| a == token.as_str()) {
        token.as_str().to_string()
    } else {
        let alternates: &[&str] = match token {
            DofToken::Angular => &["rotX", "rotY", "rotZ"],
            DofToken::Linear => &["transX", "transY", "transZ"],
            _ => &[],
        };
        let alt = applied.iter().find(|a| alternates.contains(&a.as_str()))?.clone();
        b.error(
            Code::NonStandardDriveToken,
            Some(path),
            format!("PhysicsDriveAPI:{alt} on a {} DOF; author PhysicsDriveAPI:{}", token.as_str(), token.as_str()),
        );
        alt
    };
    let units = scene.units.clone();
    let rotational = token.rotational();
    let attr = |n: &str| format!("drive:{chosen}:physics:{n}");
    let drive_type = match read::token(scene, path, &attr("type")).as_deref() {
        Some("force") => Valued::authored(DriveType::Force),
        Some("acceleration") => Valued::authored(DriveType::Acceleration),
        Some(other) => {
            b.push(Diagnostic::error(Code::InvalidDrive, Some(path), format!("{} = `{other}`", attr("type"))));
            Valued::authored(DriveType::Force)
        }
        None => Valued::schema(DriveType::Force),
    };
    let gain = |b: &mut Builder, n: &str| match read::float(scene, path, &attr(n)) {
        Some(v) if v.is_finite() && v >= 0.0 => Valued::authored(units.drive_gain(rotational, drive_type.value, v)),
        Some(v) => {
            b.push(Diagnostic::error(Code::InvalidDrive, Some(path), format!("{} = {v} must be finite and nonnegative", attr(n))));
            Valued::authored(0.0)
        }
        None => Valued::schema(0.0),
    };
    let stiffness = gain(b, "stiffness");
    let damping = gain(b, "damping");
    let max_force = match read::float(scene, path, &attr("maxForce")) {
        Some(v) if v.is_infinite() && v > 0.0 => Valued::authored(None),
        Some(v) if v.is_finite() && v >= 0.0 => Valued::authored(Some(units.effort(rotational, v))),
        Some(v) => {
            b.push(Diagnostic::error(Code::InvalidDrive, Some(path), format!("{} = {v} must be nonnegative", attr("maxForce"))));
            Valued::authored(Some(0.0))
        }
        None => Valued::schema(None),
    };
    let target = |b: &mut Builder, n: &str| match read::float(scene, path, &attr(n)) {
        Some(v) if v.is_finite() => Valued::authored(units.coordinate(rotational, v)),
        Some(v) => {
            b.push(Diagnostic::error(Code::InvalidDrive, Some(path), format!("{} = {v} must be finite", attr(n))));
            Valued::authored(0.0)
        }
        None => Valued::schema(0.0),
    };
    let target_position = target(b, "targetPosition");
    let target_velocity = target(b, "targetVelocity");
    Some(DriveDescription { drive_type, stiffness, damping, max_force, target_position, target_velocity })
}

fn read_dof(b: &mut Builder, joint: usize, token: DofToken, multi: bool) -> Option<DofDescription> {
    let scene = b.scene;
    let j = b.machine.joints[joint].clone();
    let path = j.path.as_str();
    let limit = read_limit(b, path, token, &j.kind);
    if limit.locked {
        return None;
    }
    let drive = read_drive(b, path, token);
    let passive = drive.as_ref().map_or(PassiveKind::NoDrive, DriveDescription::kind);
    let rotational = token.rotational();

    let interfaces_name = format!("gearbox:joint:{}:commandInterfaces", token.as_str());
    let interfaces = match read::tokens(scene, path, &interfaces_name) {
        Some(list) => {
            let mut parsed = Vec::new();
            for t in &list {
                match Interface::parse(t) {
                    Some(i) if !parsed.contains(&i) => parsed.push(i),
                    Some(_) => {}
                    None => b.push(Diagnostic::error(Code::InvalidInterface, Some(path), format!("{interfaces_name}: unknown interface `{t}`"))),
                }
            }
            parsed.sort();
            Valued::authored(parsed)
        }
        None => Valued::profile(Vec::new()),
    };
    let mode_name = format!("gearbox:joint:{}:defaultMode", token.as_str());
    let default_mode = match read::token(scene, path, &mode_name) {
        Some(t) => match DefaultMode::parse(&t) {
            Some(m) => Valued::authored(m),
            None => {
                b.push(Diagnostic::error(Code::InvalidInterface, Some(path), format!("{mode_name}: unknown mode `{t}`")));
                Valued::authored(DefaultMode::Disabled)
            }
        },
        None => Valued::profile(DefaultMode::Disabled),
    };

    let state_position = read::float(scene, path, &format!("state:{}:physics:position", token.as_str()));
    let state_velocity = read::float(scene, path, &format!("state:{}:physics:velocity", token.as_str()));
    let derived = pose_coordinate(&b.machine, &j, token);
    let initial_position = match state_position {
        Some(v) if v.is_finite() => {
            let q = scene.units.coordinate(rotational, v);
            if let Some(d) = derived
                && (q - d).abs() > 1e-3
            {
                b.push(Diagnostic::warning(
                    Code::InitialStateMismatch,
                    Some(path),
                    format!("authored {} state {q:.6} differs from the pose-derived coordinate {d:.6}", token.as_str()),
                ));
            }
            Valued::authored(q)
        }
        _ => Valued::derived(derived.unwrap_or(0.0)),
    };
    let initial_velocity = match state_velocity {
        Some(v) if v.is_finite() => Valued::authored(scene.units.coordinate(rotational, v)),
        _ => Valued::schema(0.0),
    };
    let name = if multi { format!("{}:{}", j.name, token.as_str()) } else { j.name.clone() };
    Some(DofDescription {
        joint,
        token,
        name,
        limit,
        drive,
        passive,
        interfaces,
        default_mode,
        initial_position,
        initial_velocity,
        isaac_actuator: None,
    })
}

/// World pose of a joint frame from its endpoint's initial pose.
pub(super) fn frame_world(machine: &MachineDescription, endpoint: &Endpoint, local: &RigidTransform) -> Option<RigidTransform> {
    match endpoint {
        Endpoint::World => Some(*local),
        Endpoint::Body(i) => Some(machine.bodies[*i].initial_pose.mul(local)),
        Endpoint::Invalid { .. } => None,
    }
}

fn twist_angle(q: DQuat, axis: DVec3) -> f64 {
    let v = DVec3::new(q.x, q.y, q.z);
    let angle = 2.0 * v.dot(axis).atan2(q.w);
    let wrapped = (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    if wrapped.abs() < 1e-12 { 0.0 } else { wrapped }
}

/// The coordinate of body1's joint frame relative to body0's at load.
pub(super) fn pose_coordinate(machine: &MachineDescription, joint: &JointDescription, token: DofToken) -> Option<f64> {
    let f0 = frame_world(machine, &joint.body0.endpoint, &joint.local0)?;
    let f1 = frame_world(machine, &joint.body1.endpoint, &joint.local1)?;
    let rel = f0.inverse().mul(&f1);
    let axis = joint.axis.unwrap_or(Axis::X).vector();
    let value = match token {
        DofToken::Angular => twist_angle(rel.rotation, axis),
        DofToken::Linear => rel.translation.dot(axis),
        DofToken::RotX => twist_angle(rel.rotation, DVec3::X),
        DofToken::RotY => twist_angle(rel.rotation, DVec3::Y),
        DofToken::RotZ => twist_angle(rel.rotation, DVec3::Z),
        DofToken::TransX => rel.translation.x,
        DofToken::TransY => rel.translation.y,
        DofToken::TransZ => rel.translation.z,
        DofToken::Distance => rel.translation.length(),
    };
    Some(if value.abs() < 1e-12 { 0.0 } else { value })
}
