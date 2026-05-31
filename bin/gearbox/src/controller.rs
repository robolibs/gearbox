//! USD-authored machine/controller discovery.
//!
//! This is the first runtime slice of `PLAN.md`: after a USD file is loaded,
//! reopen the composed stage, find prims annotated with the prototype
//! `GearboxMachineAPI` / `GearboxControllerAPI:<name>` vocabulary, resolve the
//! authored relationships, and store/log a typed discovery snapshot. The first
//! builtin controller path is intentionally conservative: it binds authored
//! wheel/steer joint relationships when Rapier exposes those joints as impulse
//! joints, and keeps a body-force fallback while the upstream adapter grows a
//! stable USD-joint-to-Rapier-handle index.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use openusd::sdf::{Path as SdfPath, Value};
use rapier3d::control::{DynamicRayCastVehicleController, WheelTuning};
use rapier3d::pipeline::QueryFilter;
use rapier3d::prelude::{
    CoefficientCombineRule, JointAxis, MultibodyJointHandle, RigidBodyHandle, Vector,
};
use serde::{Deserialize, Serialize};
use usd_bevy::UsdPrimRef;
use zenoh::Wait;

/// All USD-authored machine/controller specs discovered from loaded assets.
#[derive(Resource, Debug, Default, Clone)]
pub struct ControllerInventory {
    pub machines: Vec<MachineInstanceSpec>,
}

impl ControllerInventory {
    pub fn push_loaded_asset(
        &mut self,
        scene_root: Entity,
        asset_label: impl Into<String>,
        source_path: impl Into<String>,
        mut machines: Vec<MachineInstanceSpec>,
    ) {
        let asset_label = asset_label.into();
        let source_path = source_path.into();
        for machine in &mut machines {
            machine.scene_root = Some(scene_root);
            machine.asset_label = asset_label.clone();
            machine.source_path = source_path.clone();
        }
        self.machines.extend(machines);
    }
}

/// Internal command buffer keyed by discovered controller instance.
/// UI/keyboard/zenoh bridges write here; builtin controllers consume it.
#[derive(Resource, Debug, Default, Clone)]
pub struct ControllerCommands {
    pub cmd_vel: HashMap<ControllerKey, CmdVel>,
}

/// Runtime policy for USD-authored `external:process` controllers.
///
/// Deny-by-default: set `GEARBOX_ALLOW_USD_CONTROLLER_PROCESS=1` and
/// `GEARBOX_CONTROLLER_ALLOWLIST=/abs/dir[:/abs/other]` before launching
/// gearbox. The executable must canonicalize under one of those directories.
#[derive(Resource, Debug, Clone)]
pub struct ExternalControllerPolicy {
    pub allow_processes: bool,
    pub allowlist_dirs: Vec<std::path::PathBuf>,
}

impl Default for ExternalControllerPolicy {
    fn default() -> Self {
        let allow_processes = matches!(
            std::env::var("GEARBOX_ALLOW_USD_CONTROLLER_PROCESS").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        let allowlist_dirs = std::env::var_os("GEARBOX_CONTROLLER_ALLOWLIST")
            .map(|v| std::env::split_paths(&v).collect())
            .unwrap_or_default();
        Self {
            allow_processes,
            allowlist_dirs,
        }
    }
}

#[derive(Resource, Default)]
pub struct ExternalControllerProcesses {
    pub children: HashMap<ControllerKey, Child>,
    pub status: HashMap<ControllerKey, String>,
}

#[derive(Resource, Debug, Default, Clone)]
pub struct ControllerStates {
    pub states: HashMap<ControllerKey, ControllerState>,
}

#[derive(Resource, Debug, Default, Clone)]
struct ControllerRuntimeState {
    applied_cmd_vel: HashMap<ControllerKey, CmdVel>,
    logged_empty_tire_pairs: HashSet<ControllerKey>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ControllerState {
    pub position_m: [f64; 3],
    pub heading_rad: f64,
    pub linear_speed_mps: f64,
    pub yaw_rate_rps: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControllerKey {
    pub scene_root: Entity,
    pub machine_id: String,
    pub controller_instance: String,
}

impl ControllerKey {
    pub fn new(
        scene_root: Entity,
        machine_id: impl Into<String>,
        controller_instance: impl Into<String>,
    ) -> Self {
        Self {
            scene_root,
            machine_id: machine_id.into(),
            controller_instance: controller_instance.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CmdVel {
    /// Desired forward speed in m/s.
    pub linear_mps: f32,
    /// Desired yaw rate in rad/s.
    pub angular_rps: f32,
}

impl Default for CmdVel {
    fn default() -> Self {
        Self {
            linear_mps: 0.0,
            angular_rps: 0.0,
        }
    }
}

/// Prototype plugin that owns the discovery inventory resource.
pub struct ControllerDiscoveryPlugin;

impl Plugin for ControllerDiscoveryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInventory>()
            .init_resource::<ControllerCommands>()
            .init_resource::<ExternalControllerPolicy>()
            .init_resource::<ExternalControllerProcesses>()
            .init_resource::<ControllerStates>()
            .init_resource::<ControllerRuntimeState>()
            .add_systems(
                Update,
                (
                    sync_machine_controller_api_topics,
                    reconcile_external_process_controllers,
                    apply_machine_controller_api_commands,
                    apply_builtin_ackermann_cmd_vel,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, publish_machine_controller_states);

        match MachineControllerApi::open() {
            Ok(api) => {
                app.insert_resource(api);
                info!("gearbox-control: machine controller zenoh API ready");
            }
            Err(err) => {
                warn!("gearbox-control: machine controller zenoh API disabled: {err}");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MachineCmdVelWire {
    pub linear: [f64; 3],
    pub angular: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineStateWire {
    pub machine_id: String,
    pub controller: String,
    pub position: [f64; 3],
    pub heading_rad: f64,
    pub linear_speed_mps: f64,
    pub yaw_rate_rps: f64,
}

#[derive(Resource)]
pub struct MachineControllerApi {
    session: Arc<zenoh::Session>,
    subscribers: Mutex<HashMap<ControllerKey, zenoh::pubsub::Subscriber<()>>>,
    pending_cmd_vel: Arc<Mutex<HashMap<ControllerKey, MachineCmdVelWire>>>,
}

impl MachineControllerApi {
    fn open() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let session = Arc::new(zenoh::open(zenoh::Config::default()).wait()?);
        Ok(Self {
            session,
            subscribers: Mutex::new(HashMap::new()),
            pending_cmd_vel: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn register_cmd_vel(&self, key: ControllerKey, namespace: &str) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        if subscribers.contains_key(&key) {
            return;
        }
        let topic = format!("gearbox/machines/{namespace}/cmd_vel");
        let topic_for_cb = topic.clone();
        let pending = Arc::clone(&self.pending_cmd_vel);
        let key_for_cb = key.clone();
        let result = self
            .session
            .declare_subscriber(topic.clone())
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<MachineCmdVelWire>(bytes.as_ref()) {
                    Ok(cmd) => {
                        if let Ok(mut q) = pending.lock() {
                            q.insert(key_for_cb.clone(), cmd);
                        }
                    }
                    Err(err) => {
                        eprintln!("gearbox-control: bad cmd_vel payload on {topic_for_cb}: {err}");
                    }
                }
            })
            .wait();
        match result {
            Ok(sub) => {
                subscribers.insert(key, sub);
            }
            Err(err) => {
                warn!("gearbox-control: failed to subscribe {topic}: {err}");
            }
        }
    }

    fn snapshot_cmd_vel(&self) -> HashMap<ControllerKey, MachineCmdVelWire> {
        self.pending_cmd_vel
            .lock()
            .map(|q| q.clone())
            .unwrap_or_default()
    }

    fn publish_state(&self, namespace: &str, state: &MachineStateWire) {
        let Ok(bytes) = encode(state) else {
            return;
        };
        let topic = format!("gearbox/machines/{namespace}/state");
        if let Err(err) = self.session.put(topic.clone(), bytes).wait() {
            warn!("gearbox-control: failed to publish {topic}: {err}");
        }
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)?;
    Ok(buf)
}

fn decode<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, ciborium::de::Error<std::io::Error>> {
    ciborium::from_reader(bytes)
}

/// A single composed machine prim plus all controller instances authored on it.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MachineInstanceSpec {
    /// Filled by the Bevy loader: entity that owns the `SceneRoot`.
    pub scene_root: Option<Entity>,
    /// Human label of the loaded USD asset that produced this discovery.
    pub asset_label: String,
    /// Filesystem path of the loaded USD asset.
    pub source_path: String,
    /// Composed USD prim path of the machine root.
    pub prim_path: String,
    /// Stable runtime id after applying `gearbox:machine:id` /
    /// `gearbox:machine:idPolicy`.
    pub id: String,
    pub kind: Option<String>,
    pub interface_version: Option<String>,
    pub id_policy: String,
    pub up_axis: Option<String>,
    pub body: Option<String>,
    pub visuals: Vec<String>,
    pub colliders: Vec<String>,
    pub sensors: Vec<String>,
    pub powered_wheel_joints: Vec<String>,
    pub passive_wheel_joints: Vec<String>,
    pub steering_joints: Vec<String>,
    pub brake_joints: Vec<String>,
    pub tool_joints: Vec<String>,
    pub controllers: Vec<ControllerSpec>,
}

/// One `GearboxControllerAPI:<instance>` application.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ControllerSpec {
    pub instance: String,
    pub enabled: bool,
    pub controller_type: String,
    pub namespace: String,
    pub namespace_policy: String,
    pub update_rate_hz: f32,
    pub command_interface: Option<String>,
    pub state_interfaces: Vec<String>,
    pub frame_convention: Option<String>,
    pub target: Option<String>,
    pub body: Option<String>,
    pub drive_wheels: Vec<String>,
    pub steer_joints: Vec<String>,
    pub steer_left_joint: Option<String>,
    pub steer_right_joint: Option<String>,
    pub wheel_joints: Vec<String>,
    pub drive_wheel_joints: Vec<String>,
    pub passive_wheel_joints: Vec<String>,
    pub front_left_wheel_joint: Option<String>,
    pub front_right_wheel_joint: Option<String>,
    pub rear_left_wheel_joint: Option<String>,
    pub rear_right_wheel_joint: Option<String>,
    pub wheel_base: Option<f32>,
    pub wheel_radius: Option<f32>,
    pub track_width: Option<f32>,
    pub front_track_width: Option<f32>,
    pub rear_track_width: Option<f32>,
    pub max_steer_deg: Option<f32>,
    pub steering_geometry: Option<String>,
    pub uses_roles: Vec<String>,
    pub executable: Option<String>,
    pub args: Vec<String>,
    pub transport: Option<String>,
}

/// Reopen `usd_path` and discover gearbox machine/controller metadata.
pub fn discover_machines_from_usd(usd_path: &Path) -> Result<Vec<MachineInstanceSpec>, String> {
    let stage = open_stage_for_discovery(usd_path)?;
    let mut prims = Vec::new();
    let scan_root = stage
        .default_prim()
        .and_then(|name| openusd::sdf::path(&format!("/{name}")).ok())
        .unwrap_or_else(SdfPath::abs_root);
    walk_stage(&stage, scan_root, &mut prims);

    let mut machines = Vec::new();
    for prim in &prims {
        let api_schemas = stage.api_schemas(&prim).unwrap_or_default();
        let is_machine = api_schemas.iter().any(|api| api == "GearboxMachineAPI")
            || read_token(&stage, &prim, "gearbox:machine:kind").is_some()
            || read_token(&stage, &prim, "gearbox:machine:idPolicy").is_some()
            || read_rel_first(&stage, &prim, "gearbox:machine:body").is_some();
        if !is_machine {
            continue;
        }

        let prim_path = prim.as_str().to_string();
        let id_policy = read_token(&stage, &prim, "gearbox:machine:idPolicy")
            .unwrap_or_else(|| "prim_path".to_string());
        let id = read_token(&stage, &prim, "gearbox:machine:id")
            .unwrap_or_else(|| derive_machine_id(&prim_path));
        let namespace_default = id.clone();

        let machine_prim = prim.as_str();
        let controllers = discover_controllers(
            &stage,
            &prim,
            &api_schemas,
            &namespace_default,
            machine_prim,
        );
        machines.push(MachineInstanceSpec {
            scene_root: None,
            asset_label: String::new(),
            source_path: String::new(),
            prim_path,
            id,
            kind: read_token(&stage, &prim, "gearbox:machine:kind"),
            interface_version: read_token(&stage, &prim, "gearbox:machine:interfaceVersion"),
            id_policy,
            up_axis: read_token(&stage, &prim, "gearbox:machine:upAxis"),
            body: read_rel_first(&stage, &prim, "gearbox:machine:body")
                .map(|p| rebase_asset_root_target(machine_prim, &p)),
            visuals: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:visuals",
                machine_prim,
            ),
            colliders: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:colliders",
                machine_prim,
            ),
            sensors: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:sensors",
                machine_prim,
            ),
            powered_wheel_joints: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:role:poweredWheelJoints",
                machine_prim,
            ),
            passive_wheel_joints: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:role:passiveWheelJoints",
                machine_prim,
            ),
            steering_joints: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:role:steeringJoints",
                machine_prim,
            ),
            brake_joints: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:role:brakeJoints",
                machine_prim,
            ),
            tool_joints: read_rel_targets_rebased(
                &stage,
                &prim,
                "gearbox:machine:role:toolJoints",
                machine_prim,
            ),
            controllers,
        });
    }

    append_isaac_compat_machines(&stage, &prims, &mut machines);

    Ok(machines)
}

fn open_stage_for_discovery(usd_path: &Path) -> Result<openusd::Stage, String> {
    let bytes = std::fs::read(usd_path).map_err(|e| e.to_string())?;
    let ext = usd_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("usd");
    let is_text_usd = ext.eq_ignore_ascii_case("usda")
        || (ext.eq_ignore_ascii_case("usd") && is_text_usd(&bytes));

    // Match usd_bevy's tolerance for USDA files that contain metadata tokens
    // openusd-rs cannot parse directly yet. We only need authored gearbox
    // control metadata from the root layer, so a stripped temp layer is enough.
    let open_path = if is_text_usd {
        let final_bytes =
            usd_schema::third_party::strip_metadata::strip_unsupported_prim_metadata(&bytes);
        let tmp = discovery_temp_path(usd_path, ext);
        std::fs::write(&tmp, final_bytes).map_err(|e| e.to_string())?;
        tmp
    } else {
        usd_path.to_path_buf()
    };

    let mut search = Vec::new();
    if let Some(parent) = usd_path.parent() {
        search.push(parent.to_path_buf());
    }
    if let Some(parent) = open_path.parent() {
        search.push(parent.to_path_buf());
    }

    let open_str = open_path
        .to_str()
        .ok_or_else(|| "non-UTF-8 USD discovery path".to_string())?;
    openusd::Stage::builder()
        .resolver(
            usd_schema::third_party::resolver::StripMetadataResolver::with_search_paths(search),
        )
        .on_error(|err| {
            bevy::log::warn!("gearbox-control USD composition: {err}");
            Ok(())
        })
        .open(open_str)
        .map_err(|e| e.to_string())
}

fn discovery_temp_path(usd_path: &Path, ext: &str) -> std::path::PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    usd_path.hash(&mut hasher);
    std::env::temp_dir().join(format!(
        ".gearbox_control_scan_{:016x}.{}",
        hasher.finish(),
        ext
    ))
}

fn is_text_usd(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0xEF | 0xBB | 0xBF))
        .unwrap_or(bytes.len());
    bytes[start..].starts_with(b"#usda")
}

fn append_isaac_compat_machines(
    stage: &openusd::Stage,
    prims: &[SdfPath],
    machines: &mut Vec<MachineInstanceSpec>,
) {
    let gearbox_roots = machines
        .iter()
        .map(|machine| machine.prim_path.clone())
        .collect::<HashSet<_>>();
    let isaac_joint_groups = discover_isaac_joint_groups(stage, prims);

    for prim in prims {
        let prim_path = prim.as_str();
        if gearbox_roots.contains(prim_path) {
            continue;
        }
        let api_schemas = stage.api_schemas(prim).unwrap_or_default();
        if !api_schemas
            .iter()
            .any(|api| api == "PhysicsArticulationRootAPI")
        {
            continue;
        }

        let joint_paths = physics_joint_paths_under(stage, prims, prim_path);
        if joint_paths.is_empty() {
            continue;
        }
        let wheel_joints = joint_paths
            .iter()
            .filter(|path| looks_like_wheel_joint(path))
            .cloned()
            .collect::<Vec<_>>();
        let fallback_steer_joints = joint_paths
            .iter()
            .filter(|path| looks_like_steer_joint(path))
            .cloned()
            .collect::<Vec<_>>();

        let mut steer_joints =
            resolve_isaac_joint_names(prim_path, &joint_paths, &isaac_joint_groups.steer);
        if steer_joints.is_empty() {
            steer_joints = fallback_steer_joints;
        }
        let mut drive_wheel_joints =
            resolve_isaac_joint_names(prim_path, &joint_paths, &isaac_joint_groups.drive);
        if drive_wheel_joints.is_empty() {
            drive_wheel_joints = wheel_joints.clone();
        }

        if wheel_joints.is_empty() && steer_joints.is_empty() && drive_wheel_joints.is_empty() {
            continue;
        }

        let drive_set = drive_wheel_joints.iter().cloned().collect::<HashSet<_>>();
        let passive_wheel_joints = wheel_joints
            .iter()
            .filter(|path| !drive_set.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        let (steer_left_joint, steer_right_joint) = steering_side_targets(&steer_joints);
        let body = first_rigid_body_under(stage, prims, prim_path);
        let id = derive_machine_id(prim_path);

        machines.push(MachineInstanceSpec {
            scene_root: None,
            asset_label: String::new(),
            source_path: String::new(),
            prim_path: prim_path.to_string(),
            id: id.clone(),
            kind: Some("isaac_articulation".to_string()),
            interface_version: Some("isaac_compat:v0".to_string()),
            id_policy: "prim_path".to_string(),
            up_axis: None,
            body: body.clone(),
            visuals: Vec::new(),
            colliders: Vec::new(),
            sensors: Vec::new(),
            powered_wheel_joints: drive_wheel_joints.clone(),
            passive_wheel_joints: passive_wheel_joints.clone(),
            steering_joints: steer_joints.clone(),
            brake_joints: Vec::new(),
            tool_joints: Vec::new(),
            controllers: vec![ControllerSpec {
                instance: "drive".to_string(),
                enabled: true,
                controller_type: "builtin:ackermann_cmd_vel".to_string(),
                namespace: id,
                namespace_policy: "machine_id".to_string(),
                update_rate_hz: 60.0,
                command_interface: Some("cmd_vel".to_string()),
                state_interfaces: vec![
                    "pose".to_string(),
                    "velocity".to_string(),
                    "joint_state".to_string(),
                ],
                frame_convention: Some("usd_z_up".to_string()),
                target: Some(prim_path.to_string()),
                body,
                drive_wheels: Vec::new(),
                steer_joints,
                steer_left_joint,
                steer_right_joint,
                wheel_joints,
                drive_wheel_joints,
                passive_wheel_joints,
                front_left_wheel_joint: None,
                front_right_wheel_joint: None,
                rear_left_wheel_joint: None,
                rear_right_wheel_joint: None,
                wheel_base: None,
                wheel_radius: None,
                track_width: None,
                front_track_width: None,
                rear_track_width: None,
                max_steer_deg: Some(45.0),
                steering_geometry: Some("ackermann".to_string()),
                uses_roles: vec![
                    "poweredWheelJoints".to_string(),
                    "steeringJoints".to_string(),
                ],
                executable: None,
                args: Vec::new(),
                transport: None,
            }],
        });
    }
}

#[derive(Default)]
struct IsaacJointGroups {
    steer: Vec<String>,
    drive: Vec<String>,
}

fn discover_isaac_joint_groups(stage: &openusd::Stage, prims: &[SdfPath]) -> IsaacJointGroups {
    let mut groups = IsaacJointGroups::default();
    for prim in prims {
        let prop_names = stage.prim_properties(prim.clone()).unwrap_or_default();
        if prop_names.is_empty() {
            continue;
        }

        let type_name = type_name(stage, prim).unwrap_or_default();
        let mut context = format!("{} {}", prim.as_str(), type_name).to_ascii_lowercase();
        let mut joint_names = Vec::new();
        for prop in prop_names {
            context.push(' ');
            context.push_str(&prop.to_ascii_lowercase());
            if let Some(value) = read_attr(stage, prim, &prop) {
                append_value_context(&mut context, &value);
            }
            if looks_like_joint_names_attr(&prop) {
                joint_names.extend(read_name_array(stage, prim, &prop));
            }
        }
        if joint_names.is_empty() {
            continue;
        }

        // Isaac/OmniGraph action graphs usually contain ArticulationController
        // nodes with a `jointNames` input. The node/prim/property names tell us
        // whether those names receive position targets (steering) or velocity
        // targets (powered wheels). Keep this intentionally permissive so USDs
        // exported by different Isaac versions still normalize into Gearbox.
        if context.contains("steer") || context.contains("position") {
            groups.steer.extend(joint_names.clone());
        }
        if context.contains("wheel") || context.contains("velocity") || context.contains("drive") {
            groups.drive.extend(joint_names);
        }
    }
    dedup_strings(&mut groups.steer);
    dedup_strings(&mut groups.drive);
    groups
}

fn physics_joint_paths_under(stage: &openusd::Stage, prims: &[SdfPath], root: &str) -> Vec<String> {
    prims
        .iter()
        .filter(|prim| path_is_under(prim.as_str(), root))
        .filter(|prim| {
            type_name(stage, prim)
                .map(|ty| ty.contains("Joint"))
                .unwrap_or(false)
                && !read_rel_targets(stage, prim, "physics:body0").is_empty()
                && !read_rel_targets(stage, prim, "physics:body1").is_empty()
        })
        .map(|prim| prim.as_str().to_string())
        .collect()
}

fn first_rigid_body_under(stage: &openusd::Stage, prims: &[SdfPath], root: &str) -> Option<String> {
    prims
        .iter()
        .filter(|prim| path_is_under(prim.as_str(), root))
        .filter(|prim| {
            stage
                .api_schemas(prim)
                .unwrap_or_default()
                .iter()
                .any(|api| api == "PhysicsRigidBodyAPI")
        })
        .min_by_key(|prim| {
            let path = prim.as_str();
            let wheel_or_steer_penalty =
                if looks_like_wheel_joint(path) || looks_like_steer_joint(path) {
                    1000
                } else {
                    0
                };
            path.matches('/').count() + wheel_or_steer_penalty
        })
        .map(|prim| prim.as_str().to_string())
}

fn resolve_isaac_joint_names(_root: &str, joint_paths: &[String], names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('/') {
            if joint_paths.iter().any(|path| path == trimmed) {
                out.push(trimmed.to_string());
            }
            continue;
        }
        if let Some(path) = joint_paths
            .iter()
            .find(|path| prim_leaf_name(path) == trimmed)
            .or_else(|| {
                joint_paths.iter().find(|path| {
                    path.to_ascii_lowercase()
                        .ends_with(&trimmed.to_ascii_lowercase())
                })
            })
        {
            out.push(path.clone());
        }
    }
    dedup_strings(&mut out);
    out
}

fn steering_side_targets(steer_joints: &[String]) -> (Option<String>, Option<String>) {
    let left = steer_joints
        .iter()
        .find(|path| path.to_ascii_lowercase().contains("left"))
        .cloned();
    let right = steer_joints
        .iter()
        .find(|path| path.to_ascii_lowercase().contains("right"))
        .cloned();
    (left, right)
}

fn read_name_array(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Vec<String> {
    match read_attr(stage, prim, name) {
        Some(Value::TokenVec(v)) | Some(Value::StringVec(v)) => v,
        Some(Value::Token(v)) | Some(Value::String(v)) => v
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn looks_like_joint_names_attr(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("jointnames")
        || lower.contains("joint_names")
        || lower.contains("dofnames")
        || lower.contains("dof_names")
}

fn looks_like_wheel_joint(path: &str) -> bool {
    path.to_ascii_lowercase().contains("wheel")
}

fn looks_like_steer_joint(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("steer") || lower.contains("knuckle") || lower.contains("upright")
}

fn path_is_under(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn prim_leaf_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn type_name(stage: &openusd::Stage, prim: &SdfPath) -> Option<String> {
    stage
        .field::<String>(prim.clone(), "typeName")
        .ok()
        .flatten()
}

fn append_value_context(context: &mut String, value: &Value) {
    match value {
        Value::String(v) | Value::Token(v) | Value::AssetPath(v) => {
            context.push(' ');
            context.push_str(&v.to_ascii_lowercase());
        }
        Value::StringVec(v) | Value::TokenVec(v) => {
            for item in v {
                context.push(' ');
                context.push_str(&item.to_ascii_lowercase());
            }
        }
        _ => {}
    }
}

fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

pub fn log_discovered_machines(label: &str, machines: &[MachineInstanceSpec]) {
    if machines.is_empty() {
        info!("gearbox-control: no USD-authored machines discovered in {label}");
        return;
    }
    for machine in machines {
        info!(
            "gearbox-control: machine id={} kind={:?} prim={} controllers={} source={}",
            machine.id,
            machine.kind,
            machine.prim_path,
            machine.controllers.len(),
            label,
        );
        for controller in &machine.controllers {
            info!(
                "gearbox-control:   controller:{} type={} enabled={} ns={} target={:?} powered_wheel_joints={} steering_joints={} drive_wheel_overrides={}",
                controller.instance,
                controller.controller_type,
                controller.enabled,
                controller.namespace,
                controller.target,
                machine.powered_wheel_joints.len(),
                machine.steering_joints.len(),
                controller.drive_wheel_joints.len(),
            );
        }
    }
}

fn sync_machine_controller_api_topics(
    inventory: Res<ControllerInventory>,
    api: Option<Res<MachineControllerApi>>,
) {
    let Some(api) = api else {
        return;
    };
    for machine in &inventory.machines {
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        for controller in &machine.controllers {
            if !controller.enabled || controller.command_interface.as_deref() != Some("cmd_vel") {
                continue;
            }
            let key = ControllerKey::new(scene_root, &machine.id, &controller.instance);
            api.register_cmd_vel(key, &controller.namespace);
        }
    }
}

fn apply_machine_controller_api_commands(
    api: Option<Res<MachineControllerApi>>,
    mut commands: ResMut<ControllerCommands>,
) {
    let Some(api) = api else {
        return;
    };
    for (key, wire) in api.snapshot_cmd_vel() {
        commands.cmd_vel.insert(
            key,
            CmdVel {
                linear_mps: wire.linear[0] as f32,
                angular_rps: cmd_vel_yaw_rate(&wire),
            },
        );
    }
}

fn cmd_vel_yaw_rate(wire: &MachineCmdVelWire) -> f32 {
    // Public cmd_vel follows ROS/base_link convention: yaw is angular.z.
    // Bevy/Rapier internals are Y-up, and during manual debugging it is easy
    // to publish angular.y instead. Accept angular.y as a fallback when
    // angular.z is zero so either convention turns the tractor.
    let yaw_z = wire.angular[2] as f32;
    if yaw_z.abs() > 1e-9 {
        yaw_z
    } else {
        wire.angular[1] as f32
    }
}

fn publish_machine_controller_states(
    inventory: Res<ControllerInventory>,
    states: Res<ControllerStates>,
    api: Option<Res<MachineControllerApi>>,
) {
    let Some(api) = api else {
        return;
    };
    if states.states.is_empty() {
        return;
    }
    for machine in &inventory.machines {
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        for controller in &machine.controllers {
            if !controller
                .state_interfaces
                .iter()
                .any(|iface| iface == "pose" || iface == "velocity")
            {
                continue;
            }
            let key = ControllerKey::new(scene_root, &machine.id, &controller.instance);
            let Some(state) = states.states.get(&key) else {
                continue;
            };
            api.publish_state(
                &controller.namespace,
                &MachineStateWire {
                    machine_id: machine.id.clone(),
                    controller: controller.instance.clone(),
                    position: state.position_m,
                    heading_rad: state.heading_rad,
                    linear_speed_mps: state.linear_speed_mps,
                    yaw_rate_rps: state.yaw_rate_rps,
                },
            );
        }
    }
}

fn reconcile_external_process_controllers(
    inventory: Res<ControllerInventory>,
    policy: Res<ExternalControllerPolicy>,
    mut processes: ResMut<ExternalControllerProcesses>,
) {
    let mut desired = HashSet::new();
    for machine in &inventory.machines {
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        for controller in &machine.controllers {
            if !controller.enabled || controller.controller_type != "external:process" {
                continue;
            }
            let key = ControllerKey::new(scene_root, &machine.id, &controller.instance);
            desired.insert(key.clone());

            if let Some(child) = processes.children.get_mut(&key) {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        processes.children.remove(&key);
                        processes
                            .status
                            .insert(key.clone(), format!("exited: {status}"));
                    }
                    Ok(None) => {
                        processes.status.insert(key.clone(), "running".to_string());
                    }
                    Err(err) => {
                        processes
                            .status
                            .insert(key.clone(), format!("status error: {err}"));
                    }
                }
                continue;
            }

            if !policy.allow_processes {
                processes.status.insert(
                    key,
                    "blocked: set GEARBOX_ALLOW_USD_CONTROLLER_PROCESS=1".into(),
                );
                continue;
            }
            let Some(executable) = controller.executable.as_deref() else {
                processes
                    .status
                    .insert(key, "blocked: no executable authored".into());
                continue;
            };
            let executable_path = Path::new(executable);
            let Ok(canonical) = executable_path.canonicalize() else {
                processes
                    .status
                    .insert(key, format!("blocked: executable not found: {executable}"));
                continue;
            };
            if !is_allowlisted(&canonical, &policy.allowlist_dirs) {
                processes.status.insert(
                    key,
                    format!(
                        "blocked: executable outside GEARBOX_CONTROLLER_ALLOWLIST: {executable}"
                    ),
                );
                continue;
            }

            let mut cmd = Command::new(&canonical);
            cmd.args(&controller.args)
                .env("GEARBOX_MACHINE_ID", &machine.id)
                .env("GEARBOX_CONTROLLER", &controller.instance)
                .env("GEARBOX_NAMESPACE", &controller.namespace)
                .env(
                    "GEARBOX_TRANSPORT",
                    controller.transport.as_deref().unwrap_or("zenoh"),
                );
            match cmd.spawn() {
                Ok(child) => {
                    processes.children.insert(key.clone(), child);
                    processes.status.insert(key, "running".to_string());
                }
                Err(err) => {
                    processes.status.insert(key, format!("spawn failed: {err}"));
                }
            }
        }
    }

    // Stop children whose controller disappeared or was disabled. This is a
    // controlled cleanup path, not user-requested destructive filesystem work.
    let stale = processes
        .children
        .keys()
        .filter(|key| !desired.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    for key in stale {
        if let Some(mut child) = processes.children.remove(&key) {
            let _ = child.kill();
            let _ = child.wait();
        }
        processes
            .status
            .insert(key, "stopped: controller removed".into());
    }
}

fn is_allowlisted(executable: &Path, allowlist_dirs: &[std::path::PathBuf]) -> bool {
    if allowlist_dirs.is_empty() {
        return false;
    }
    allowlist_dirs.iter().any(|dir| {
        dir.canonicalize()
            .map(|allowed| executable.starts_with(allowed))
            .unwrap_or(false)
    })
}

/// First builtin controller: consume `cmd_vel`, bind authored wheel/steer joint
/// relationships to Rapier impulse-joint motors where possible, publish
/// chassis pose/velocity state, and keep a conservative body-force fallback for
/// joint shapes that are not externally addressable yet.
fn apply_builtin_ackermann_cmd_vel(
    inventory: Res<ControllerInventory>,
    commands: Res<ControllerCommands>,
    time: Res<Time>,
    mut runtime: ResMut<ControllerRuntimeState>,
    mut states: ResMut<ControllerStates>,
    active: Res<usd_bevy::physics::PhysicsActive>,
    prims: Query<(Entity, &UsdPrimRef)>,
    joints: Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: Query<&ChildOf>,
    mut physics: ResMut<usd_bevy::physics::PhysicsWorld>,
) {
    if !active.0 || inventory.machines.is_empty() {
        return;
    }
    let dt = time.delta_secs().clamp(1.0 / 240.0, 1.0 / 20.0);

    for machine in &inventory.machines {
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        for controller in &machine.controllers {
            if !controller.enabled || controller.controller_type != "builtin:ackermann_cmd_vel" {
                continue;
            }
            let key = ControllerKey::new(scene_root, &machine.id, &controller.instance);
            let requested = commands.cmd_vel.get(&key).copied().unwrap_or_default();
            let cmd = stable_cmd_vel(&key, requested, dt, &mut runtime);
            let Some(body_path) = controller.body.as_ref().or(machine.body.as_ref()) else {
                continue;
            };
            let Some(body_entity) = find_prim_entity(scene_root, body_path, &prims, &parents)
            else {
                continue;
            };
            let Some(body_handle) = physics.entity_to_body.get(&body_entity).copied() else {
                continue;
            };

            let body_heading = physics
                .bodies
                .get(body_handle)
                .map(machine_heading_rad)
                .unwrap_or(0.0);

            {
                let Some(body) = physics.bodies.get_mut(body_handle) else {
                    continue;
                };

                // Always publish state for discovered active controllers,
                // including when the command is zero. This lets the UI/API show
                // that the controller is alive without needing movement.
                let pos = body.translation();
                states.states.insert(
                    key.clone(),
                    ControllerState {
                        position_m: [pos.x, pos.y, pos.z],
                        heading_rad: body_heading,
                        linear_speed_mps: body.linvel().length(),
                        yaw_rate_rps: body.angvel().y,
                    },
                );
            }

            let wheel_radius_m = controller.wheel_radius.unwrap_or(0.45) as f64;
            let wheel_base_m = controller.wheel_base.unwrap_or(2.37);
            let track_width_m = controller
                .front_track_width
                .or(controller.track_width)
                .unwrap_or(1.5675);
            let max_steer_deg = controller.max_steer_deg.unwrap_or(45.0);
            let steer_target_rad = steering_target_radians(
                cmd.linear_mps,
                cmd.angular_rps,
                wheel_base_m,
                max_steer_deg,
            );
            let geometry = controller
                .steering_geometry
                .as_deref()
                .unwrap_or("ackermann");
            let steer_targets = steering_joint_targets(
                scene_root,
                controller,
                machine,
                &joints,
                &parents,
                &physics,
                geometry,
                steer_target_rad,
                wheel_base_m,
                track_width_m,
                max_steer_deg,
            );
            let traction_track_width_m = controller
                .rear_track_width
                .or(controller.track_width)
                .unwrap_or(track_width_m);
            let mut wheel_targets = wheel_joint_targets(
                scene_root,
                controller,
                machine,
                &joints,
                &parents,
                &physics,
                body_handle,
                geometry,
                cmd.linear_mps as f64,
                steer_target_rad,
                wheel_base_m,
                traction_track_width_m,
                wheel_radius_m,
            );
            if cmd.linear_mps.abs() < 0.05 {
                wheel_targets.extend(parking_brake_wheel_targets(
                    scene_root, controller, machine, &joints, &parents, &physics,
                ));
            }
            let tire_pairs =
                tire_joint_pairs(scene_root, controller, machine, &joints, &parents, &physics);
            let raycast_specs = raycast_vehicle_wheel_specs_for_controller(
                scene_root,
                controller,
                machine,
                &joints,
                &parents,
                &physics,
                body_handle,
                wheel_radius_m,
            );
            if tire_pairs.is_empty() && runtime.logged_empty_tire_pairs.insert(key.clone()) {
                warn!(
                    "gearbox-control: no wheel joint pairs found for machine={} controller={}; raycast vehicle cannot drive",
                    machine.id, controller.instance
                );
            }
            ensure_tire_grip(&mut physics, body_handle, &tire_pairs);
            // Drive physics with the same Bullet/Rapier raycast vehicle model
            // that the old working `main` tractor used. USD joints/colliders
            // are kept for visuals/body collisions; tire traction/suspension is
            // controller-owned, not raw cylinder-contact-owned.
            let using_raycast_vehicle = apply_rapier_raycast_vehicle_controller(
                &mut physics,
                body_handle,
                &tire_pairs,
                &raycast_specs,
                cmd,
                steer_target_rad,
            );
            if using_raycast_vehicle {
                // Raycast traction moves the chassis; the USD wheel rigid
                // bodies are visual only. Therefore their spin must be derived
                // from the actual chassis motion at each wheel, not from the
                // requested cmd_vel. If the tractor is still accelerating,
                // braking, turning, or briefly sliding, command-based wheel
                // spin makes the tyres look like they are slipping on ice.
                wheel_targets = visual_wheel_spin_targets(
                    scene_root,
                    controller,
                    machine,
                    &joints,
                    &parents,
                    &physics,
                    body_handle,
                    steer_target_rad,
                    wheel_radius_m,
                );
            }
            wake_vehicle_for_command(
                &mut physics,
                body_handle,
                &tire_pairs,
                cmd,
                steer_target_rad,
            );
            apply_articulation_or_impulse_joint_motors(
                &mut physics,
                &wheel_targets,
                &steer_targets,
            );
        }
    }
}

fn stable_cmd_vel(
    key: &ControllerKey,
    requested: CmdVel,
    dt: f32,
    runtime: &mut ControllerRuntimeState,
) -> CmdVel {
    let requested = sanitize_cmd_vel(requested);
    let previous = runtime
        .applied_cmd_vel
        .get(key)
        .copied()
        .unwrap_or_default();
    let next = CmdVel {
        linear_mps: slew(previous.linear_mps, requested.linear_mps, 20.0 * dt),
        angular_rps: slew(previous.angular_rps, requested.angular_rps, 1.5 * dt),
    };
    runtime.applied_cmd_vel.insert(key.clone(), next);
    next
}

fn machine_heading_rad(body: &rapier3d::prelude::RigidBody) -> f64 {
    // Rapier/Bevy runs Y-up, so the drive plane is X/Z and yaw is around +Y.
    // The USD stage is Z-up and `usd_bevy` converts vectors with -90° about X:
    // (usd X, usd Y, usd Z) -> (bevy X, bevy Y=usd Z, bevy Z=-usd Y).
    // The chassis rigid body keeps the USD-authored local basis. In that basis
    // +Z is up and the tractor's visual front is -Y (front wheels have lower
    // USD Y than rear wheels). The loader's body rotation maps this local -Y
    // into Bevy/world +Z.
    // Publish heading in the same convention that bale_run.py uses:
    // 0 rad = +Z, +pi/2 = +X.
    let Some(forward) = body_forward_vector(body) else {
        return 0.0;
    };
    forward.x.atan2(forward.z)
}

fn body_forward_vector(body: &rapier3d::prelude::RigidBody) -> Option<Vector> {
    let mut forward = body.rotation() * Vector::new(0.0, -1.0, 0.0);
    forward.y = 0.0;
    (forward.length_squared() > 1e-9).then(|| forward.normalize())
}

fn sanitize_cmd_vel(cmd: CmdVel) -> CmdVel {
    CmdVel {
        linear_mps: deadband(cmd.linear_mps, 0.03).clamp(-4.0, 4.0),
        angular_rps: deadband(cmd.angular_rps, 0.02).clamp(-1.2, 1.2),
    }
}

fn deadband(value: f32, threshold: f32) -> f32 {
    if value.abs() < threshold { 0.0 } else { value }
}

fn slew(current: f32, target: f32, max_delta: f32) -> f32 {
    let delta = (target - current).clamp(-max_delta, max_delta);
    current + delta
}

fn steering_target_radians(
    linear_mps: f32,
    angular_rps: f32,
    wheel_base_m: f32,
    max_steer_deg: f32,
) -> f64 {
    // Isaac's Ackermann controller takes steeringAngle and speed as separate
    // inputs. When we adapt cmd_vel, steering is defined relative to the
    // vehicle's forward frame, so reverse must not flip the visual steering
    // direction. Only wheel/base speed changes sign. Also: steering angle is
    // allowed to move while stopped; at zero speed cmd_vel's yaw-rate field is
    // treated as a steering request against a nominal walking-speed reference
    // instead of forcing the wheels straight.
    const STOPPED_STEERING_REFERENCE_SPEED_MPS: f32 = 2.4;

    if angular_rps.abs() < 1e-3 {
        return 0.0;
    }
    let speed_for_steering = linear_mps.abs().max(STOPPED_STEERING_REFERENCE_SPEED_MPS);
    let max = max_steer_deg.to_radians() as f64;
    ((wheel_base_m as f64 * angular_rps as f64) / speed_for_steering as f64)
        .atan()
        .clamp(-max, max)
}

fn ackermann_steering_angles(
    center_steer_rad: f64,
    _wheel_base_m: f32,
    _track_width_m: f32,
    max_steer_deg: f32,
) -> (f64, f64) {
    let max = max_steer_deg.to_radians() as f64;
    let angle = center_steer_rad.clamp(-max, max);
    // The current USD tractor has mechanically tied front steering. Do NOT
    // command separate inner/outer Ackermann angles here: with the present
    // joint/collider setup that made the two front wheels toe inward/outward
    // and scrub instead of rolling. Both steering links are locked to the exact
    // same target angle.
    (angle, angle)
}

/// Drive-motor damping for the wheel velocity motor.
const WHEEL_DRIVE_DAMPING: f64 = 240.0;
/// Torque cap (N·m) on the wheel drive motor. Kept deliberately *below*
/// the tyre's grip limit (`friction × normal_load × radius`) so the
/// motor can never out-torque the contact patch — the wheel rolls
/// instead of spinning out. A wheel torque cap above the grip limit is
/// exactly what makes a driven wheel "run on ice". Lower = grippier
/// (and gentler acceleration); raise only if the tractor feels weak.
const WHEEL_DRIVE_MAX_TORQUE: f64 = 6000.0;
/// Raycast mode already moves the chassis; the USD wheel joints are only
/// visual tyres. Keep these visual motors deliberately soft so they don't feed
/// big reaction torques back into the chassis and make the tractor look like it
/// is fighting invisible contact wheels.
const WHEEL_VISUAL_DAMPING: f64 = 60.0;
const WHEEL_VISUAL_MAX_TORQUE: f64 = 350.0;
/// Steering position-motor gains + torque cap (N·m). Stiffer than the
/// old values so the steered wheels hold their angle against the tyre
/// scrub forces that now actually turn the vehicle.
const STEER_STIFFNESS: f64 = 300.0;
const STEER_DAMPING: f64 = 90.0;
const STEER_MAX_TORQUE: f64 = 1200.0;
/// Friction coefficient forced onto wheel colliders. Imported USD
/// colliders default to 0.5 when the asset authors no physics material
/// — far too slippery for a driven tyre. Applied with a `Max` combine
/// rule so the *grippier* of (tyre, ground) wins, guaranteeing a high
/// effective friction whatever the ground collider is authored at.
const TIRE_FRICTION: f64 = 2.4;

const RAYCAST_ENGINE_FORCE_GAIN_PER_REAR_WHEEL: f64 = 3_500.0;
const RAYCAST_ENGINE_FORCE_MAX_PER_REAR_WHEEL: f64 = 8_500.0;
const RAYCAST_BRAKE_IMPULSE: f64 = 1_800.0;
const RAYCAST_SUSPENSION_REST_LENGTH: f64 = 0.22;

/// Largest collider half-extent of a body — a wheel's tyre radius, a
/// chassis's bounding half-size. `None` if the body has no collider.
fn body_max_collider_radius(
    physics: &usd_bevy::physics::PhysicsWorld,
    body: RigidBodyHandle,
) -> Option<f64> {
    let body = physics.bodies.get(body)?;
    body.colliders()
        .iter()
        .filter_map(|ch| physics.colliders.get(*ch))
        .map(|c| c.compute_aabb().half_extents().max_element())
        .fold(None, |acc: Option<f64>, r| {
            Some(acc.map_or(r, |a| a.max(r)))
        })
}

/// The wheel rigid body of a wheel joint's body pair: whichever body
/// isn't the chassis, or — for a knuckle↔wheel joint where neither is
/// the chassis — the larger-collider body (the tyre, not the knuckle).
fn wheel_body_of(
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    pair: (RigidBodyHandle, RigidBodyHandle),
) -> Option<RigidBodyHandle> {
    let (a, b) = pair;
    if a == chassis {
        return Some(b);
    }
    if b == chassis {
        return Some(a);
    }
    match (
        body_max_collider_radius(physics, a),
        body_max_collider_radius(physics, b),
    ) {
        (Some(ra), Some(rb)) => Some(if ra >= rb { a } else { b }),
        (Some(_), None) => Some(a),
        (None, Some(_)) => Some(b),
        (None, None) => None,
    }
}

/// Measure a wheel's rolling radius from its collider. Returns `None`
/// when no usable collider is found, so callers can fall back.
///
/// The wheel-speed command is `linear_speed / radius`: a wrong radius
/// makes the tyres spin at the wrong rate and slip against the ground.
/// Condition every wheel collider so the tyres can actually hold the
/// ground: grippy friction, and zero restitution so a hard contact load
/// doesn't bounce the wheel (and hop the whole tractor). This applies to
/// powered and passive tyres; passive front tyres still need grip to
/// steer instead of sliding sideways. Idempotent.
fn ensure_tire_grip(
    physics: &mut usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    tire_pairs: &[(RigidBodyHandle, RigidBodyHandle)],
) {
    for pair in tire_pairs {
        let Some(wheel) = wheel_body_of(physics, chassis, *pair) else {
            continue;
        };
        let handles = physics
            .bodies
            .get(wheel)
            .map(|b| b.colliders().to_vec())
            .unwrap_or_default();
        for ch in handles {
            if let Some(col) = physics.colliders.get_mut(ch) {
                if col.friction() < TIRE_FRICTION {
                    col.set_friction(TIRE_FRICTION);
                }
                // "Stickiest wins" — the tyre's high friction holds
                // regardless of what the ground collider is authored at.
                col.set_friction_combine_rule(CoefficientCombineRule::Max);
                col.set_restitution(0.0);
            }
        }
    }
}

fn set_wheel_colliders_sensor(
    physics: &mut usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    tire_pairs: &[(RigidBodyHandle, RigidBodyHandle)],
    sensor: bool,
) {
    for pair in tire_pairs {
        let Some(wheel) = wheel_body_of(physics, chassis, *pair) else {
            continue;
        };
        let handles = physics
            .bodies
            .get(wheel)
            .map(|b| b.colliders().to_vec())
            .unwrap_or_default();
        for ch in handles {
            if let Some(col) = physics.colliders.get_mut(ch) {
                col.set_sensor(sensor);
            }
        }
    }
}

fn apply_rapier_raycast_vehicle_controller(
    physics: &mut usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    tire_pairs: &[(RigidBodyHandle, RigidBodyHandle)],
    wheel_specs: &[RaycastVehicleWheelSpec],
    cmd: CmdVel,
    steer_target_rad: f64,
) -> bool {
    if tire_pairs.is_empty() || wheel_specs.is_empty() {
        return false;
    }

    set_wheel_colliders_sensor(physics, chassis, tire_pairs, true);

    let (current_speed, force_per_rear) = {
        let Some(body) = physics.bodies.get(chassis) else {
            return false;
        };
        let Some(forward) = body_forward_vector(body) else {
            return false;
        };
        let current_speed = body.linvel().dot(forward);
        let target_speed = cmd.linear_mps as f64;
        let speed_error = target_speed - current_speed;
        let force = if target_speed.abs() < 0.05 && speed_error.abs() < 0.05 {
            0.0
        } else {
            // On hills the old fixed 2.5 kN/wheel force could be smaller
            // than gravity's component along the slope for this ~2.7 t
            // tractor, so it would just sit and spin/slide. Use a proper
            // speed servo plus feed-forward slope compensation in the chassis
            // forward axis. This keeps flat-ground behavior smooth while
            // giving enough push to climb modest field rolls.
            let forward_3d = body.rotation() * Vector::new(0.0, -1.0, 0.0);
            let slope_compensation_per_rear = body.mass() * 9.81 * forward_3d.y / 2.0;
            speed_error * RAYCAST_ENGINE_FORCE_GAIN_PER_REAR_WHEEL + slope_compensation_per_rear
        };
        (
            current_speed,
            force.clamp(
                -RAYCAST_ENGINE_FORCE_MAX_PER_REAR_WHEEL,
                RAYCAST_ENGINE_FORCE_MAX_PER_REAR_WHEEL,
            ),
        )
    };

    let mut tuning = WheelTuning::default();
    tuning.suspension_stiffness = 90.0;
    tuning.suspension_compression = 7.0;
    tuning.suspension_damping = 7.0;
    tuning.max_suspension_travel = 0.5;
    tuning.side_friction_stiffness = 1.0;
    tuning.friction_slip = 24.0;
    tuning.max_suspension_force = 30_000.0;

    let mut vehicle = DynamicRayCastVehicleController::new(chassis);
    vehicle.index_up_axis = 2;
    vehicle.index_forward_axis = 1;

    let suspension = Vector::new(0.0, 0.0, -1.0);
    // In the chassis/USD local basis the wheel axle is X and suspension is Z.
    // Use -X so normal.cross(axle) points along the tractor's local -Y front
    // after the loader rotates the body into Bevy's Y-up world.
    let axle = Vector::new(-1.0, 0.0, 0.0);

    // Adapted from the old working `main` tractor preset. Those hard-points
    // were authored in Bevy's Y-up body frame. This USD chassis rigid body
    // keeps USD's local frame instead: X = right, Y = back, Z = up. Therefore
    // the front/rear hard-points use the authored USD Y coordinates directly,
    // and their local Z is picked so the raycast wheel bottoms sit on terrain
    // at body height 0:
    //
    //   connection_z - rest_length - radius == 0
    //
    // Feeding Bevy-local Y-up points here makes the rays cast sideways/upward,
    // so the controller never supports or drives the chassis.
    for spec in wheel_specs {
        let wheel = vehicle.add_wheel(
            spec.chassis_connection,
            suspension,
            axle,
            RAYCAST_SUSPENSION_REST_LENGTH,
            spec.radius,
            &tuning,
        );
        if spec.steering_multiplier.abs() > f64::EPSILON {
            wheel.steering = steer_target_rad * spec.steering_multiplier;
        }
        if spec.driven {
            wheel.engine_force = force_per_rear;
        }
    }

    if cmd.linear_mps.abs() < 0.05 {
        let brake = (current_speed.abs() * 900.0).clamp(0.0, RAYCAST_BRAKE_IMPULSE);
        for wheel in vehicle.wheels_mut() {
            wheel.engine_force = 0.0;
            wheel.brake = brake;
        }
    }

    let filter = QueryFilter::new()
        .exclude_rigid_body(chassis)
        .exclude_sensors();
    let queries = physics.broad_phase.as_query_pipeline_mut(
        physics.narrow_phase.query_dispatcher(),
        &mut physics.bodies,
        &mut physics.colliders,
        filter,
    );
    vehicle.update_vehicle(physics.integration_parameters.dt as f64, queries);
    true
}

#[derive(Debug, Clone, Copy)]
struct RaycastVehicleWheelSpec {
    chassis_connection: Vector,
    radius: f64,
    driven: bool,
    steered: bool,
    steering_multiplier: f64,
}

fn raycast_vehicle_wheel_specs() -> [RaycastVehicleWheelSpec; 4] {
    [
        RaycastVehicleWheelSpec {
            chassis_connection: Vector::new(0.79, -1.23, RAYCAST_SUSPENSION_REST_LENGTH + 0.525),
            radius: 0.525,
            driven: false,
            steered: true,
            steering_multiplier: 1.0,
        },
        RaycastVehicleWheelSpec {
            chassis_connection: Vector::new(-0.79, -1.23, RAYCAST_SUSPENSION_REST_LENGTH + 0.525),
            radius: 0.525,
            driven: false,
            steered: true,
            steering_multiplier: 1.0,
        },
        RaycastVehicleWheelSpec {
            chassis_connection: Vector::new(0.8475, 1.14, RAYCAST_SUSPENSION_REST_LENGTH + 0.755),
            radius: 0.755,
            driven: true,
            steered: false,
            steering_multiplier: 0.0,
        },
        RaycastVehicleWheelSpec {
            chassis_connection: Vector::new(-0.8475, 1.14, RAYCAST_SUSPENSION_REST_LENGTH + 0.755),
            radius: 0.755,
            driven: true,
            steered: false,
            steering_multiplier: 0.0,
        },
    ]
}

#[allow(clippy::too_many_arguments)]
fn raycast_vehicle_wheel_specs_for_controller(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    wheel_radius_fallback_m: f64,
) -> Vec<RaycastVehicleWheelSpec> {
    let Some(chassis_body) = physics.bodies.get(chassis) else {
        return Vec::new();
    };

    let powered_paths = powered_wheel_joint_paths(machine, controller);
    let geometry = controller
        .steering_geometry
        .as_deref()
        .unwrap_or("ackermann");
    let mut specs = Vec::new();
    for path in all_wheel_joint_paths(machine, controller) {
        let Some(pair) = joint_pair(scene_root, &path, joints, parents, physics) else {
            continue;
        };
        let Some(wheel) = wheel_body_of(physics, chassis, pair) else {
            continue;
        };
        let Some(wheel_body) = physics.bodies.get(wheel) else {
            continue;
        };
        let radius = body_max_collider_radius(physics, wheel)
            .filter(|r| *r > 0.05)
            .unwrap_or(wheel_radius_fallback_m);
        let world_offset = wheel_body.translation() - chassis_body.translation();
        let local_center = chassis_body.rotation().inverse() * world_offset;
        let steering_multiplier = steering_multiplier_for_wheel_path(&path, machine, geometry);
        specs.push(RaycastVehicleWheelSpec {
            chassis_connection: Vector::new(
                local_center.x,
                local_center.y,
                local_center.z + RAYCAST_SUSPENSION_REST_LENGTH,
            ),
            radius,
            driven: powered_paths.contains(&path),
            steered: steering_multiplier.abs() > f64::EPSILON,
            steering_multiplier,
        });
    }
    specs
}

fn all_wheel_joint_paths(
    machine: &MachineInstanceSpec,
    controller: &ControllerSpec,
) -> Vec<String> {
    let mut out = Vec::new();
    for path in machine
        .powered_wheel_joints
        .iter()
        .chain(machine.passive_wheel_joints.iter())
        .chain(controller.drive_wheel_joints.iter())
        .chain(controller.passive_wheel_joints.iter())
        .chain(controller.wheel_joints.iter())
    {
        push_unique_string(&mut out, path);
    }
    for path in [
        controller.front_left_wheel_joint.as_ref(),
        controller.front_right_wheel_joint.as_ref(),
        controller.rear_left_wheel_joint.as_ref(),
        controller.rear_right_wheel_joint.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        push_unique_string(&mut out, path);
    }
    out
}

fn powered_wheel_joint_paths(
    machine: &MachineInstanceSpec,
    controller: &ControllerSpec,
) -> HashSet<String> {
    machine
        .powered_wheel_joints
        .iter()
        .chain(controller.drive_wheel_joints.iter())
        .chain(controller.rear_left_wheel_joint.iter())
        .chain(controller.rear_right_wheel_joint.iter())
        .cloned()
        .collect()
}

fn push_unique_string(out: &mut Vec<String>, value: &str) {
    if !out.iter().any(|existing| existing == value) {
        out.push(value.to_string());
    }
}

fn steering_multiplier_for_wheel_path(
    wheel_path: &str,
    machine: &MachineInstanceSpec,
    geometry: &str,
) -> f64 {
    let steered = machine
        .steering_joints
        .iter()
        .any(|steer_path| wheel_path_matches_steer_path(wheel_path, steer_path));
    if !steered {
        return 0.0;
    }
    if geometry == "crab" && is_rear_path(wheel_path) {
        -1.0
    } else {
        1.0
    }
}

fn wheel_path_matches_steer_path(wheel_path: &str, steer_path: &str) -> bool {
    let wheel_side = side_hint(wheel_path);
    let steer_side = side_hint(steer_path);
    if wheel_side.is_some() && steer_side.is_some() && wheel_side != steer_side {
        return false;
    }
    let wheel_axle = axle_hint(wheel_path);
    let steer_axle = axle_hint(steer_path);
    wheel_axle.is_some() && steer_axle.is_some() && wheel_axle == steer_axle
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideHint {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxleHint {
    Front,
    Middle,
    Rear,
}

fn side_hint(path: &str) -> Option<SideHint> {
    let lower = path.to_ascii_lowercase();
    if lower.contains("left") || lower.contains("_l") {
        Some(SideHint::Left)
    } else if lower.contains("right") || lower.contains("_r") {
        Some(SideHint::Right)
    } else {
        None
    }
}

fn axle_hint(path: &str) -> Option<AxleHint> {
    let lower = path.to_ascii_lowercase();
    if lower.contains("front") || lower.contains("fwd") {
        Some(AxleHint::Front)
    } else if lower.contains("middle") || lower.contains("mid") {
        Some(AxleHint::Middle)
    } else if lower.contains("rear") || lower.contains("back") {
        Some(AxleHint::Rear)
    } else {
        None
    }
}

fn is_rear_path(path: &str) -> bool {
    axle_hint(path) == Some(AxleHint::Rear)
}

fn raycast_wheel_spec_for_path(path: &str) -> Option<RaycastVehicleWheelSpec> {
    let lower = path.to_ascii_lowercase();
    let specs = raycast_vehicle_wheel_specs();
    if lower.contains("front") && lower.contains("left") {
        Some(specs[0])
    } else if lower.contains("front") && lower.contains("right") {
        Some(specs[1])
    } else if lower.contains("back") && lower.contains("left")
        || lower.contains("rear") && lower.contains("left")
    {
        Some(specs[2])
    } else if lower.contains("back") && lower.contains("right")
        || lower.contains("rear") && lower.contains("right")
    {
        Some(specs[3])
    } else {
        None
    }
}

fn wake_vehicle_for_command(
    physics: &mut usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    tire_pairs: &[(RigidBodyHandle, RigidBodyHandle)],
    cmd: CmdVel,
    steer_target_rad: f64,
) {
    if cmd.linear_mps.abs() < 0.03 && cmd.angular_rps.abs() < 0.02 && steer_target_rad.abs() < 1e-4
    {
        return;
    }

    if let Some(body) = physics.bodies.get_mut(chassis) {
        body.wake_up(true);
    }
    for pair in tire_pairs {
        if let Some(body) = physics.bodies.get_mut(pair.0) {
            body.wake_up(true);
        }
        if let Some(body) = physics.bodies.get_mut(pair.1) {
            body.wake_up(true);
        }
    }
}

fn tire_joint_pairs(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
) -> Vec<(RigidBodyHandle, RigidBodyHandle)> {
    let mut pairs = Vec::new();
    for path in machine
        .powered_wheel_joints
        .iter()
        .chain(machine.passive_wheel_joints.iter())
        .chain(controller.drive_wheel_joints.iter())
        .chain(controller.passive_wheel_joints.iter())
        .chain(controller.wheel_joints.iter())
    {
        if let Some(pair) = joint_pair(scene_root, path, joints, parents, physics) {
            push_unique_pair(&mut pairs, pair);
        }
    }
    for path in [
        controller.front_left_wheel_joint.as_deref(),
        controller.front_right_wheel_joint.as_deref(),
        controller.rear_left_wheel_joint.as_deref(),
        controller.rear_right_wheel_joint.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(pair) = joint_pair(scene_root, path, joints, parents, physics) {
            push_unique_pair(&mut pairs, pair);
        }
    }
    pairs
}

fn push_unique_pair(
    pairs: &mut Vec<(RigidBodyHandle, RigidBodyHandle)>,
    pair: (RigidBodyHandle, RigidBodyHandle),
) {
    if !pairs
        .iter()
        .any(|existing| rigid_body_pair_matches(*existing, pair.0, pair.1))
    {
        pairs.push(pair);
    }
}

#[derive(Debug, Clone, Copy)]
struct JointPositionTarget {
    pair: (RigidBodyHandle, RigidBodyHandle),
    position: f64,
}

#[derive(Debug, Clone, Copy)]
struct JointVelocityTarget {
    pair: (RigidBodyHandle, RigidBodyHandle),
    velocity: f64,
    damping: f64,
    max_torque: f64,
}

fn drive_wheel_velocity_target(
    pair: (RigidBodyHandle, RigidBodyHandle),
    velocity: f64,
) -> JointVelocityTarget {
    JointVelocityTarget {
        pair,
        velocity,
        damping: WHEEL_DRIVE_DAMPING,
        max_torque: WHEEL_DRIVE_MAX_TORQUE,
    }
}

fn visual_wheel_velocity_target(
    pair: (RigidBodyHandle, RigidBodyHandle),
    velocity: f64,
) -> JointVelocityTarget {
    JointVelocityTarget {
        pair,
        velocity,
        damping: WHEEL_VISUAL_DAMPING,
        max_torque: WHEEL_VISUAL_MAX_TORQUE,
    }
}

#[allow(clippy::too_many_arguments)]
fn steering_joint_targets(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    geometry: &str,
    center_steer_rad: f64,
    wheel_base_m: f32,
    track_width_m: f32,
    max_steer_deg: f32,
) -> Vec<JointPositionTarget> {
    if geometry == "ackermann" {
        let (left, right) =
            ackermann_steering_angles(center_steer_rad, wheel_base_m, track_width_m, max_steer_deg);
        let targets = explicit_steering_targets(
            scene_root, controller, joints, parents, physics, left, right,
        );
        if !targets.is_empty() {
            return targets;
        }
        let targets =
            role_steering_targets(scene_root, machine, joints, parents, physics, left, right);
        if !targets.is_empty() {
            return targets;
        }
    }

    let targets = explicit_steering_targets(
        scene_root,
        controller,
        joints,
        parents,
        physics,
        center_steer_rad,
        center_steer_rad,
    );
    if !targets.is_empty() {
        return targets;
    }

    if geometry == "crab" || geometry == "parallel" {
        let targets = all_role_steering_targets(
            scene_root,
            machine,
            joints,
            parents,
            physics,
            center_steer_rad,
            geometry,
        );
        if !targets.is_empty() {
            return targets;
        }
    }

    let targets = role_steering_targets(
        scene_root,
        machine,
        joints,
        parents,
        physics,
        center_steer_rad,
        center_steer_rad,
    );
    if !targets.is_empty() {
        return targets;
    }

    controller
        .steer_joints
        .iter()
        .chain(machine.steering_joints.iter())
        .filter_map(|path| joint_pair(scene_root, path, joints, parents, physics))
        .map(|pair| JointPositionTarget {
            pair,
            position: center_steer_rad,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn explicit_steering_targets(
    scene_root: Entity,
    controller: &ControllerSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    left_position: f64,
    right_position: f64,
) -> Vec<JointPositionTarget> {
    let mut targets = Vec::new();
    if let Some(pair) = controller
        .steer_left_joint
        .as_deref()
        .and_then(|path| joint_pair(scene_root, path, joints, parents, physics))
    {
        targets.push(JointPositionTarget {
            pair,
            position: left_position,
        });
    }
    if let Some(pair) = controller
        .steer_right_joint
        .as_deref()
        .and_then(|path| joint_pair(scene_root, path, joints, parents, physics))
    {
        targets.push(JointPositionTarget {
            pair,
            position: right_position,
        });
    }
    targets
}

#[allow(clippy::too_many_arguments)]
fn all_role_steering_targets(
    scene_root: Entity,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    center_position: f64,
    geometry: &str,
) -> Vec<JointPositionTarget> {
    machine
        .steering_joints
        .iter()
        .filter_map(|path| {
            let pair = joint_pair(scene_root, path, joints, parents, physics)?;
            let multiplier = if geometry == "crab" && is_rear_path(path) {
                -1.0
            } else {
                1.0
            };
            Some(JointPositionTarget {
                pair,
                position: center_position * multiplier,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn role_steering_targets(
    scene_root: Entity,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    left_position: f64,
    right_position: f64,
) -> Vec<JointPositionTarget> {
    let (left_path, right_path) = steering_role_sides(&machine.steering_joints);
    let mut targets = Vec::new();
    if let Some(pair) =
        left_path.and_then(|path| joint_pair(scene_root, path, joints, parents, physics))
    {
        targets.push(JointPositionTarget {
            pair,
            position: left_position,
        });
    }
    if let Some(pair) =
        right_path.and_then(|path| joint_pair(scene_root, path, joints, parents, physics))
    {
        targets.push(JointPositionTarget {
            pair,
            position: right_position,
        });
    }
    targets
}

fn steering_role_sides(paths: &[String]) -> (Option<&str>, Option<&str>) {
    let left = paths
        .iter()
        .find(|path| path.to_ascii_lowercase().contains("left"))
        .map(String::as_str);
    let right = paths
        .iter()
        .find(|path| path.to_ascii_lowercase().contains("right"))
        .map(String::as_str);
    (left, right)
}

#[allow(clippy::too_many_arguments)]
fn wheel_joint_targets(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    geometry: &str,
    linear_mps: f64,
    _center_steer_rad: f64,
    _wheel_base_m: f32,
    _traction_track_width_m: f32,
    wheel_radius_fallback_m: f64,
) -> Vec<JointVelocityTarget> {
    // Wheel angular-velocity target for one joint pair. The spin rate is
    // `ground_speed / wheel_radius`, using the wheel's *actual* collider
    // radius — a wrong radius makes the tyre over- or under-spin and slip.
    //
    // Keep all powered wheels at the same angular speed. The current tractor
    // has no authored differential, and the user's expectation is hard-locked
    // axle behavior: if one powered wheel spins, its mate spins exactly the
    // same way.
    let target = |_path: &str, pair: (RigidBodyHandle, RigidBodyHandle)| {
        let radius = wheel_body_of(physics, chassis, pair)
            .and_then(|wheel| body_max_collider_radius(physics, wheel))
            .filter(|r| *r > 0.05)
            .unwrap_or(wheel_radius_fallback_m);
        let ground_speed = linear_mps;
        drive_wheel_velocity_target(pair, ground_speed / radius)
    };

    if !controller.drive_wheel_joints.is_empty() {
        return controller
            .drive_wheel_joints
            .iter()
            .filter_map(|path| {
                let pair = joint_pair(scene_root, path, joints, parents, physics)?;
                Some(target(path, pair))
            })
            .collect();
    }
    if !machine.powered_wheel_joints.is_empty() {
        return machine
            .powered_wheel_joints
            .iter()
            .filter_map(|path| {
                let pair = joint_pair(scene_root, path, joints, parents, physics)?;
                Some(target(path, pair))
            })
            .collect();
    }

    if geometry == "ackermann" {
        let authored = [
            controller.front_left_wheel_joint.as_deref(),
            controller.front_right_wheel_joint.as_deref(),
            controller.rear_left_wheel_joint.as_deref(),
            controller.rear_right_wheel_joint.as_deref(),
        ];
        let targets = authored
            .into_iter()
            .filter_map(|path| {
                let path = path?;
                let pair = joint_pair(scene_root, path, joints, parents, physics)?;
                Some(target(path, pair))
            })
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            return targets;
        }
    }

    controller
        .wheel_joints
        .iter()
        .chain(machine.powered_wheel_joints.iter())
        .filter_map(|path| {
            let pair = joint_pair(scene_root, path, joints, parents, physics)?;
            Some(target(path, pair))
        })
        .collect()
}

fn parking_brake_wheel_targets(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
) -> Vec<JointVelocityTarget> {
    let mut targets = Vec::new();
    for path in machine
        .passive_wheel_joints
        .iter()
        .chain(controller.passive_wheel_joints.iter())
        .chain(controller.wheel_joints.iter())
    {
        let Some(pair) = joint_pair(scene_root, path, joints, parents, physics) else {
            continue;
        };
        if targets.iter().any(|target: &JointVelocityTarget| {
            rigid_body_pair_matches(target.pair, pair.0, pair.1)
        }) {
            continue;
        }
        targets.push(drive_wheel_velocity_target(pair, 0.0));
    }
    targets
}

#[allow(clippy::too_many_arguments)]
fn visual_wheel_spin_targets(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    steer_target_rad: f64,
    wheel_radius_fallback_m: f64,
) -> Vec<JointVelocityTarget> {
    let mut targets = Vec::new();
    for path in machine
        .powered_wheel_joints
        .iter()
        .chain(machine.passive_wheel_joints.iter())
        .chain(controller.drive_wheel_joints.iter())
        .chain(controller.passive_wheel_joints.iter())
        .chain(controller.wheel_joints.iter())
    {
        push_visual_wheel_spin_target(
            &mut targets,
            scene_root,
            path,
            joints,
            parents,
            physics,
            chassis,
            steer_target_rad,
            wheel_radius_fallback_m,
        );
    }
    for path in [
        controller.front_left_wheel_joint.as_deref(),
        controller.front_right_wheel_joint.as_deref(),
        controller.rear_left_wheel_joint.as_deref(),
        controller.rear_right_wheel_joint.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        push_visual_wheel_spin_target(
            &mut targets,
            scene_root,
            path,
            joints,
            parents,
            physics,
            chassis,
            steer_target_rad,
            wheel_radius_fallback_m,
        );
    }
    targets
}

#[allow(clippy::too_many_arguments)]
fn push_visual_wheel_spin_target(
    targets: &mut Vec<JointVelocityTarget>,
    scene_root: Entity,
    path: &str,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    steer_target_rad: f64,
    wheel_radius_fallback_m: f64,
) {
    let Some(pair) = joint_pair(scene_root, path, joints, parents, physics) else {
        return;
    };
    if targets
        .iter()
        .any(|target: &JointVelocityTarget| rigid_body_pair_matches(target.pair, pair.0, pair.1))
    {
        return;
    }

    let radius = visual_wheel_radius(physics, chassis, pair, path, wheel_radius_fallback_m);
    let ground_speed = visual_wheel_ground_speed(physics, chassis, path, steer_target_rad)
        .unwrap_or_else(|| chassis_forward_speed(physics, chassis).unwrap_or(0.0));
    targets.push(visual_wheel_velocity_target(pair, ground_speed / radius));
}

fn visual_wheel_radius(
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    pair: (RigidBodyHandle, RigidBodyHandle),
    path: &str,
    fallback: f64,
) -> f64 {
    raycast_wheel_spec_for_path(path)
        .map(|spec| spec.radius)
        .or_else(|| {
            wheel_body_of(physics, chassis, pair)
                .and_then(|wheel| body_max_collider_radius(physics, wheel))
                .filter(|r| *r > 0.05)
        })
        .unwrap_or(fallback)
}

fn chassis_forward_speed(
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
) -> Option<f64> {
    let body = physics.bodies.get(chassis)?;
    let forward = body_forward_vector(body)?;
    Some(body.linvel().dot(forward))
}

fn visual_wheel_ground_speed(
    physics: &usd_bevy::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    path: &str,
    steer_target_rad: f64,
) -> Option<f64> {
    let spec = raycast_wheel_spec_for_path(path)?;
    let body = physics.bodies.get(chassis)?;
    let world_offset = body.rotation() * spec.chassis_connection;
    let point_velocity = body.linvel() + body.angvel().cross(world_offset);
    let steer = if spec.steered { steer_target_rad } else { 0.0 };
    let rolling_local = Vector::new(steer.sin(), -steer.cos(), 0.0);
    let rolling_world = body.rotation() * rolling_local;
    Some(point_velocity.dot(rolling_world))
}

fn joint_pair(
    scene_root: Entity,
    path: &str,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
) -> Option<(RigidBodyHandle, RigidBodyHandle)> {
    let (_, _, body0, body1) = find_joint_body_pair(scene_root, path, joints, parents, physics)?;
    Some((body0, body1))
}

#[derive(Debug, Clone, Copy, Default)]
struct MotorApplication {
    drive: bool,
    steer: bool,
}

fn apply_articulation_or_impulse_joint_motors(
    physics: &mut usd_bevy::physics::PhysicsWorld,
    wheel_targets: &[JointVelocityTarget],
    steer_targets: &[JointPositionTarget],
) -> MotorApplication {
    let mut applied = MotorApplication::default();
    if wheel_targets.is_empty() && steer_targets.is_empty() {
        return applied;
    }

    for target in wheel_targets {
        if let Some(handle) = multibody_joint_handle(physics, target.pair) {
            if let Some((multibody, link_id)) = physics.multibody_joints.get_mut(handle) {
                if let Some(link) = multibody.link_mut(link_id) {
                    link.joint
                        .data
                        .set_motor_velocity(JointAxis::AngX, target.velocity, target.damping)
                        .set_motor_max_force(JointAxis::AngX, target.max_torque);
                    applied.drive = true;
                }
            }
        }
    }

    for target in steer_targets {
        if let Some(handle) = multibody_joint_handle(physics, target.pair) {
            if let Some((multibody, link_id)) = physics.multibody_joints.get_mut(handle) {
                if let Some(link) = multibody.link_mut(link_id) {
                    link.joint
                        .data
                        .set_motor_position(
                            JointAxis::AngX,
                            target.position,
                            STEER_STIFFNESS,
                            STEER_DAMPING,
                        )
                        .set_motor_max_force(JointAxis::AngX, STEER_MAX_TORQUE);
                    applied.steer = true;
                }
            }
        }
    }

    for (_, joint) in physics.impulse_joints.iter_mut() {
        if let Some(target) = wheel_targets
            .iter()
            .find(|target| rigid_body_pair_matches(target.pair, joint.body1, joint.body2))
        {
            joint
                .data
                .set_motor_velocity(JointAxis::AngX, target.velocity, target.damping)
                .set_motor_max_force(JointAxis::AngX, target.max_torque);
            applied.drive = true;
        }
        if let Some(target) = steer_targets
            .iter()
            .find(|target| rigid_body_pair_matches(target.pair, joint.body1, joint.body2))
        {
            // Rapier's RevoluteJoint motor is always exposed as AngX: the
            // authored USD axis ("Z" for the tractor steering joints) is
            // baked into the joint local axis when usd_rapier builds the
            // revolute joint. Driving AngZ fights a locked axis and can
            // explode/flip the vehicle.
            joint
                .data
                .set_motor_position(
                    JointAxis::AngX,
                    target.position,
                    STEER_STIFFNESS,
                    STEER_DAMPING,
                )
                .set_motor_max_force(JointAxis::AngX, STEER_MAX_TORQUE);
            applied.steer = true;
        }
    }
    applied
}

fn multibody_joint_handle(
    physics: &usd_bevy::physics::PhysicsWorld,
    pair: (RigidBodyHandle, RigidBodyHandle),
) -> Option<MultibodyJointHandle> {
    physics
        .multibody_joints
        .joint_between(pair.0, pair.1)
        .map(|(handle, _, _)| handle)
}

fn rigid_body_pair_matches(
    authored: (RigidBodyHandle, RigidBodyHandle),
    actual_a: RigidBodyHandle,
    actual_b: RigidBodyHandle,
) -> bool {
    (authored.0 == actual_a && authored.1 == actual_b)
        || (authored.0 == actual_b && authored.1 == actual_a)
}

fn find_joint_body_pair(
    scene_root: Entity,
    prim_path: &str,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &usd_bevy::physics::PhysicsWorld,
) -> Option<(Entity, Entity, RigidBodyHandle, RigidBodyHandle)> {
    let (_, _, joint) = joints.iter().find(|(entity, prim, _)| {
        prim.path == prim_path && is_descendant_of(*entity, scene_root, parents)
    })?;
    let body0_entity = joint.body0?;
    let body1_entity = joint.body1?;
    let body0 = physics.entity_to_body.get(&body0_entity).copied()?;
    let body1 = physics.entity_to_body.get(&body1_entity).copied()?;
    Some((body0_entity, body1_entity, body0, body1))
}

fn find_prim_entity(
    scene_root: Entity,
    prim_path: &str,
    prims: &Query<(Entity, &UsdPrimRef)>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    prims
        .iter()
        .find(|(entity, prim)| {
            prim.path == prim_path && is_descendant_of(*entity, scene_root, parents)
        })
        .map(|(entity, _)| entity)
}

fn is_descendant_of(entity: Entity, root: Entity, parents: &Query<&ChildOf>) -> bool {
    let mut current = entity;
    for _ in 0..64 {
        if current == root {
            return true;
        }
        let Ok(parent) = parents.get(current) else {
            return false;
        };
        current = parent.parent();
    }
    false
}

fn discover_controllers(
    stage: &openusd::Stage,
    prim: &SdfPath,
    api_schemas: &[String],
    namespace_default: &str,
    machine_prim: &str,
) -> Vec<ControllerSpec> {
    let mut instances = HashSet::new();
    for api in api_schemas {
        if let Some(instance) = api.strip_prefix("GearboxControllerAPI:") {
            if !instance.is_empty() {
                instances.insert(instance.to_string());
            }
        }
    }

    // Fallback for prototype assets where apiSchemas were stripped but attrs
    // remain. Keep this narrow until we have a full property iterator.
    for known in ["drive", "arm", "implement"] {
        if read_token(stage, prim, &format!("gearbox:controller:{known}:type")).is_some() {
            instances.insert(known.to_string());
        }
    }

    let mut out: Vec<_> = instances
        .into_iter()
        .map(|instance| {
            let prefix = format!("gearbox:controller:{instance}:");
            let namespace_policy = read_token(stage, prim, &(prefix.clone() + "namespacePolicy"))
                .unwrap_or_else(|| "machine_id".to_string());
            let namespace = read_string(stage, prim, &(prefix.clone() + "namespace"))
                .unwrap_or_else(|| namespace_default.to_string());
            ControllerSpec {
                instance,
                enabled: read_bool(stage, prim, &(prefix.clone() + "enabled")).unwrap_or(true),
                controller_type: read_token(stage, prim, &(prefix.clone() + "type"))
                    .unwrap_or_else(|| "builtin:unknown".to_string()),
                namespace,
                namespace_policy,
                update_rate_hz: read_float(stage, prim, &(prefix.clone() + "updateRateHz"))
                    .unwrap_or(60.0),
                command_interface: read_token(stage, prim, &(prefix.clone() + "commandInterface")),
                state_interfaces: read_token_array(
                    stage,
                    prim,
                    &(prefix.clone() + "stateInterfaces"),
                ),
                frame_convention: read_token(stage, prim, &(prefix.clone() + "frameConvention")),
                target: read_rel_first(stage, prim, &(prefix.clone() + "target"))
                    .map(|p| rebase_asset_root_target(machine_prim, &p)),
                body: read_rel_first(stage, prim, &(prefix.clone() + "body"))
                    .map(|p| rebase_asset_root_target(machine_prim, &p)),
                drive_wheels: read_rel_targets_rebased(
                    stage,
                    prim,
                    &(prefix.clone() + "driveWheels"),
                    machine_prim,
                ),
                steer_joints: read_rel_targets_rebased(
                    stage,
                    prim,
                    &(prefix.clone() + "steerJoints"),
                    machine_prim,
                ),
                steer_left_joint: read_rel_first(stage, prim, &(prefix.clone() + "steerLeftJoint"))
                    .map(|p| rebase_asset_root_target(machine_prim, &p)),
                steer_right_joint: read_rel_first(
                    stage,
                    prim,
                    &(prefix.clone() + "steerRightJoint"),
                )
                .map(|p| rebase_asset_root_target(machine_prim, &p)),
                wheel_joints: read_rel_targets_rebased(
                    stage,
                    prim,
                    &(prefix.clone() + "wheelJoints"),
                    machine_prim,
                ),
                drive_wheel_joints: read_rel_targets_rebased(
                    stage,
                    prim,
                    &(prefix.clone() + "driveWheelJoints"),
                    machine_prim,
                ),
                passive_wheel_joints: read_rel_targets_rebased(
                    stage,
                    prim,
                    &(prefix.clone() + "passiveWheelJoints"),
                    machine_prim,
                ),
                front_left_wheel_joint: read_rel_first(
                    stage,
                    prim,
                    &(prefix.clone() + "frontLeftWheelJoint"),
                )
                .map(|p| rebase_asset_root_target(machine_prim, &p)),
                front_right_wheel_joint: read_rel_first(
                    stage,
                    prim,
                    &(prefix.clone() + "frontRightWheelJoint"),
                )
                .map(|p| rebase_asset_root_target(machine_prim, &p)),
                rear_left_wheel_joint: read_rel_first(
                    stage,
                    prim,
                    &(prefix.clone() + "rearLeftWheelJoint"),
                )
                .map(|p| rebase_asset_root_target(machine_prim, &p)),
                rear_right_wheel_joint: read_rel_first(
                    stage,
                    prim,
                    &(prefix.clone() + "rearRightWheelJoint"),
                )
                .map(|p| rebase_asset_root_target(machine_prim, &p)),
                wheel_base: read_float(stage, prim, &(prefix.clone() + "wheelBase")),
                track_width: read_float(stage, prim, &(prefix.clone() + "trackWidth")),
                front_track_width: read_float(stage, prim, &(prefix.clone() + "frontTrackWidth")),
                rear_track_width: read_float(stage, prim, &(prefix.clone() + "rearTrackWidth")),
                wheel_radius: read_float(stage, prim, &(prefix.clone() + "wheelRadius")),
                max_steer_deg: read_float(stage, prim, &(prefix.clone() + "maxSteerDeg")),
                steering_geometry: read_token(stage, prim, &(prefix.clone() + "steeringGeometry")),
                uses_roles: read_token_array(stage, prim, &(prefix.clone() + "usesRoles")),
                executable: read_string(stage, prim, &(prefix.clone() + "executable")),
                args: read_string_array(stage, prim, &(prefix.clone() + "args")),
                transport: read_token(stage, prim, &(prefix.clone() + "transport")),
            }
        })
        .collect();
    out.sort_by(|a, b| a.instance.cmp(&b.instance));
    out
}

fn walk_stage(stage: &openusd::Stage, path: SdfPath, out: &mut Vec<SdfPath>) {
    if path.as_str() != "/" {
        out.push(path.clone());
    }
    for child_name in stage.prim_children(path.clone()).unwrap_or_default() {
        let Ok(child_path) = path.append_path(child_name.as_str()) else {
            continue;
        };
        walk_stage(stage, child_path, out);
    }
}

fn derive_machine_id(prim_path: &str) -> String {
    let mut out = String::new();
    for ch in prim_path.trim_matches('/').chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "machine".to_string()
    } else {
        out
    }
}

fn read_attr(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Option<Value> {
    let attr = prim.append_property(name).ok()?;
    stage.field::<Value>(attr, "default").ok().flatten()
}

fn read_bool(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Option<bool> {
    match read_attr(stage, prim, name)? {
        Value::Bool(v) => Some(v),
        _ => None,
    }
}

fn read_float(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Option<f32> {
    match read_attr(stage, prim, name)? {
        Value::Float(v) => Some(v),
        Value::Double(v) => Some(v as f32),
        Value::Int(v) => Some(v as f32),
        Value::Uint(v) => Some(v as f32),
        _ => None,
    }
}

fn read_string(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Option<String> {
    match read_attr(stage, prim, name)? {
        Value::String(v) | Value::Token(v) | Value::AssetPath(v) => Some(v),
        _ => None,
    }
}

fn read_token(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Option<String> {
    read_string(stage, prim, name)
}

fn read_token_array(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Vec<String> {
    match read_attr(stage, prim, name) {
        Some(Value::TokenVec(v)) | Some(Value::StringVec(v)) => v,
        _ => Vec::new(),
    }
}

fn read_string_array(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Vec<String> {
    read_token_array(stage, prim, name)
}

fn read_rel_targets(stage: &openusd::Stage, prim: &SdfPath, rel_name: &str) -> Vec<String> {
    let Some(raw) = prim
        .append_property(rel_name)
        .ok()
        .and_then(|rel| stage.field::<Value>(rel, "targetPaths").ok().flatten())
    else {
        return Vec::new();
    };
    let paths = match raw {
        Value::PathListOp(op) => op.flatten(),
        Value::PathVec(v) => v,
        _ => return Vec::new(),
    };
    paths.into_iter().map(|p| p.as_str().to_string()).collect()
}

fn read_rel_targets_rebased(
    stage: &openusd::Stage,
    prim: &SdfPath,
    rel_name: &str,
    machine_prim: &str,
) -> Vec<String> {
    read_rel_targets(stage, prim, rel_name)
        .into_iter()
        .map(|target| rebase_asset_root_target(machine_prim, &target))
        .collect()
}

fn read_rel_first(stage: &openusd::Stage, prim: &SdfPath, rel_name: &str) -> Option<String> {
    read_rel_targets(stage, prim, rel_name).into_iter().next()
}

fn rebase_asset_root_target(machine_prim: &str, target: &str) -> String {
    const ASSET_ROOT: &str = "/robot";
    if machine_prim == ASSET_ROOT {
        return target.to_string();
    }
    if target == ASSET_ROOT {
        return machine_prim.to_string();
    }
    if let Some(suffix) = target.strip_prefix("/robot/") {
        return format!("{machine_prim}/{suffix}");
    }
    target.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_tractor_drive_controller_metadata() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/tractor.usd");
        let machines = discover_machines_from_usd(&path).expect("tractor.usd should scan");
        let machine = machines
            .iter()
            .find(|m| m.prim_path == "/robot")
            .expect("tractor root should be a Gearbox machine");
        assert_eq!(machine.id, "robot");
        assert_eq!(machine.kind.as_deref(), Some("tractor"));
        assert_eq!(
            machine.powered_wheel_joints,
            vec![
                "/robot/Joints/rev_back_left".to_string(),
                "/robot/Joints/rev_back_right".to_string(),
            ]
        );
        assert_eq!(
            machine.passive_wheel_joints,
            vec![
                "/robot/Joints/rev_front_left".to_string(),
                "/robot/Joints/rev_front_right".to_string(),
            ]
        );
        assert_eq!(
            machine.steering_joints,
            vec![
                "/robot/Joints/steer_front_left".to_string(),
                "/robot/Joints/steer_front_right".to_string(),
            ]
        );

        let drive = machine
            .controllers
            .iter()
            .find(|c| c.instance == "drive")
            .expect("tractor should author a drive controller");
        assert!(drive.enabled);
        assert_eq!(drive.controller_type, "builtin:ackermann_cmd_vel");
        assert_eq!(drive.namespace, "robot");
        assert_eq!(drive.command_interface.as_deref(), Some("cmd_vel"));
        assert_eq!(drive.steering_geometry.as_deref(), Some("parallel"));
        assert_eq!(
            drive.uses_roles,
            vec![
                "poweredWheelJoints".to_string(),
                "steeringJoints".to_string(),
            ]
        );
        assert!(drive.steer_left_joint.is_none());
        assert!(drive.steer_right_joint.is_none());
        assert!(drive.front_left_wheel_joint.is_none());
        assert!(drive.rear_right_wheel_joint.is_none());
        assert!(drive.drive_wheel_joints.is_empty());
        assert!(drive.passive_wheel_joints.is_empty());
        assert!(drive.steer_joints.is_empty());
        assert!(drive.wheel_joints.is_empty());
    }

    #[test]
    fn discovers_oxbo_drive_controller_metadata() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/oxbo.usd");
        let machines = discover_machines_from_usd(&path).expect("oxbo.usd should scan");
        let machine = machines
            .iter()
            .find(|m| m.prim_path == "/robot")
            .expect("oxbo root should be a Gearbox machine");
        assert_eq!(machine.id, "robot");
        assert_eq!(machine.kind.as_deref(), Some("oxbo"));
        assert_eq!(
            machine.powered_wheel_joints,
            vec![
                "/robot/Joints/rev_rear_left".to_string(),
                "/robot/Joints/rev_rear_right".to_string(),
            ]
        );
        assert_eq!(machine.passive_wheel_joints.len(), 4);
        assert_eq!(
            machine.steering_joints,
            vec![
                "/robot/Joints/steer_front_left".to_string(),
                "/robot/Joints/steer_front_right".to_string(),
                "/robot/Joints/steer_rear_left".to_string(),
                "/robot/Joints/steer_rear_right".to_string(),
            ]
        );

        let drive = machine
            .controllers
            .iter()
            .find(|c| c.instance == "drive")
            .expect("oxbo should author a drive controller");
        assert!(drive.enabled);
        assert_eq!(drive.controller_type, "builtin:ackermann_cmd_vel");
        assert_eq!(drive.command_interface.as_deref(), Some("cmd_vel"));
        assert_eq!(drive.steering_geometry.as_deref(), Some("crab"));
        assert_eq!(
            drive.uses_roles,
            vec![
                "poweredWheelJoints".to_string(),
                "passiveWheelJoints".to_string(),
                "steeringJoints".to_string(),
            ]
        );
    }

    #[test]
    fn world_terrain_usd_composes_without_machine_metadata() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/world/terrain.usd");
        let machines = discover_machines_from_usd(&path).expect("terrain.usd should scan");
        assert!(
            machines.is_empty(),
            "terrain scene should not register as a driveable machine"
        );
    }

    #[test]
    fn discovers_isaac_action_graph_joint_roles_without_gearbox_metadata() {
        let usd = r#"#usda 1.0
(
    defaultPrim = "Leatherback"
)

def Xform "Leatherback" (
    prepend apiSchemas = ["PhysicsArticulationRootAPI"]
)
{
    def Xform "base_link" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI"]
    )
    {
    }

    def Xform "front_left_wheel" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI"]
    )
    {
    }

    def Xform "front_right_wheel" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI"]
    )
    {
    }

    def Xform "rear_left_wheel" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI"]
    )
    {
    }

    def Xform "rear_right_wheel" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI"]
    )
    {
    }

    def Scope "Joints"
    {
        def PhysicsRevoluteJoint "Knuckle__Upright__Front_Left"
        {
            rel physics:body0 = </Leatherback/base_link>
            rel physics:body1 = </Leatherback/front_left_wheel>
        }
        def PhysicsRevoluteJoint "Knuckle__Upright__Front_Right"
        {
            rel physics:body0 = </Leatherback/base_link>
            rel physics:body1 = </Leatherback/front_right_wheel>
        }
        def PhysicsRevoluteJoint "Wheel__Knuckle__Front_Left"
        {
            rel physics:body0 = </Leatherback/base_link>
            rel physics:body1 = </Leatherback/front_left_wheel>
        }
        def PhysicsRevoluteJoint "Wheel__Knuckle__Front_Right"
        {
            rel physics:body0 = </Leatherback/base_link>
            rel physics:body1 = </Leatherback/front_right_wheel>
        }
        def PhysicsRevoluteJoint "Wheel__Upright__Rear_Left"
        {
            rel physics:body0 = </Leatherback/base_link>
            rel physics:body1 = </Leatherback/rear_left_wheel>
        }
        def PhysicsRevoluteJoint "Wheel__Upright__Rear_Right"
        {
            rel physics:body0 = </Leatherback/base_link>
            rel physics:body1 = </Leatherback/rear_right_wheel>
        }
    }

    def Scope "ActionGraph"
    {
        def "SteeringArticulationController"
        {
            custom string inputs:nodeType = "isaacsim.core.nodes.IsaacArticulationController"
            custom token inputs:commandType = "position"
            custom token[] inputs:jointNames = [
                "Knuckle__Upright__Front_Left",
                "Knuckle__Upright__Front_Right"
            ]
        }
        def "WheelArticulationController"
        {
            custom string inputs:nodeType = "isaacsim.core.nodes.IsaacArticulationController"
            custom token inputs:commandType = "velocity"
            custom token[] inputs:jointNames = [
                "Wheel__Upright__Rear_Left",
                "Wheel__Upright__Rear_Right"
            ]
        }
    }
}
"#;
        let path = std::env::temp_dir().join(format!(
            "gearbox_isaac_compat_{}_{}.usda",
            std::process::id(),
            "leatherback"
        ));
        std::fs::write(&path, usd).expect("write synthetic Isaac USD");

        let machines = discover_machines_from_usd(&path).expect("synthetic Isaac USD should scan");
        let _ = std::fs::remove_file(&path);

        let machine = machines
            .iter()
            .find(|machine| machine.prim_path == "/Leatherback")
            .expect("Isaac articulation should become a Gearbox machine");
        assert_eq!(machine.kind.as_deref(), Some("isaac_articulation"));
        assert_eq!(machine.body.as_deref(), Some("/Leatherback/base_link"));
        let drive = machine
            .controllers
            .iter()
            .find(|controller| controller.instance == "drive")
            .expect("compat drive controller");
        assert_eq!(drive.controller_type, "builtin:ackermann_cmd_vel");
        assert_eq!(
            drive.steer_joints,
            vec![
                "/Leatherback/Joints/Knuckle__Upright__Front_Left".to_string(),
                "/Leatherback/Joints/Knuckle__Upright__Front_Right".to_string(),
            ]
        );
        assert_eq!(
            drive.drive_wheel_joints,
            vec![
                "/Leatherback/Joints/Wheel__Upright__Rear_Left".to_string(),
                "/Leatherback/Joints/Wheel__Upright__Rear_Right".to_string(),
            ]
        );
        assert!(
            drive
                .passive_wheel_joints
                .contains(&"/Leatherback/Joints/Wheel__Knuckle__Front_Left".to_string())
        );
        assert!(
            drive
                .passive_wheel_joints
                .contains(&"/Leatherback/Joints/Wheel__Knuckle__Front_Right".to_string())
        );
    }

    #[test]
    fn derives_stable_machine_id_from_prim_path() {
        assert_eq!(derive_machine_id("/World/Tractor_01"), "world_tractor_01");
        assert_eq!(derive_machine_id("/World/Farm/RobotA"), "world_farm_robota");
    }

    #[test]
    fn steering_target_clamps_and_zeros() {
        assert_eq!(steering_target_radians(1.0, 0.0, 2.4, 45.0), 0.0);
        assert!(steering_target_radians(0.0, 1.0, 2.4, 45.0) > 0.0);
        let target = steering_target_radians(2.0, 10.0, 2.4, 30.0);
        assert!((target - 30_f64.to_radians()).abs() < 1e-6);
        let reverse = steering_target_radians(-2.0, 1.0, 2.4, 45.0);
        assert!(reverse > 0.0);
        assert_eq!(reverse, steering_target_radians(2.0, 1.0, 2.4, 45.0));
        assert_eq!(reverse, steering_target_radians(0.0, 1.0, 2.4, 45.0));
    }

    #[test]
    fn ackermann_outputs_parallel_steering_for_tied_front_axle() {
        let center = 0.25;
        let (left, right) = ackermann_steering_angles(center, 2.37, 1.5675, 45.0);
        assert_eq!(left, center);
        assert_eq!(right, center);
    }

    #[test]
    fn command_sanitize_and_slew_are_responsive_but_stable() {
        assert_eq!(
            sanitize_cmd_vel(CmdVel {
                linear_mps: 0.01,
                angular_rps: 0.01
            })
            .linear_mps,
            0.0
        );
        assert_eq!(
            sanitize_cmd_vel(CmdVel {
                linear_mps: 0.01,
                angular_rps: 0.01
            })
            .angular_rps,
            0.0
        );
        assert_eq!(
            sanitize_cmd_vel(CmdVel {
                linear_mps: 10.0,
                angular_rps: 5.0
            })
            .linear_mps,
            4.0
        );
        assert_eq!(
            sanitize_cmd_vel(CmdVel {
                linear_mps: 10.0,
                angular_rps: 5.0
            })
            .angular_rps,
            1.2
        );
        assert!((slew(0.0, 4.0, 0.25) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn tied_rear_axle_uses_same_command_for_both_drive_wheels() {
        let left_velocity = 2.0 / 0.68;
        let right_velocity = 2.0 / 0.68;
        assert_eq!(left_velocity, right_velocity);
    }

    #[test]
    fn impulse_joint_motors_bind_authored_body_pairs() {
        use rapier3d::prelude::{RevoluteJointBuilder, RigidBodyBuilder, Vector};

        let mut physics = usd_bevy::physics::PhysicsWorld::default();
        let chassis = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
        let wheel = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
        let steer = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
        let steer_link = physics.bodies.insert(RigidBodyBuilder::dynamic().build());

        physics.impulse_joints.insert(
            chassis,
            wheel,
            RevoluteJointBuilder::new(Vector::new(1.0, 0.0, 0.0)),
            true,
        );
        physics.impulse_joints.insert(
            steer,
            steer_link,
            RevoluteJointBuilder::new(Vector::new(0.0, 0.0, 1.0)),
            true,
        );

        let applied = apply_articulation_or_impulse_joint_motors(
            &mut physics,
            &[JointVelocityTarget {
                pair: (wheel, chassis),
                velocity: 7.5,
                damping: WHEEL_DRIVE_DAMPING,
                max_torque: WHEEL_DRIVE_MAX_TORQUE,
            }],
            &[JointPositionTarget {
                pair: (steer, steer_link),
                position: 0.25,
            }],
        );
        assert!(applied.drive);
        assert!(applied.steer);

        let mut saw_drive = false;
        let mut saw_steer = false;
        for (_, joint) in physics.impulse_joints.iter() {
            if rigid_body_pair_matches((chassis, wheel), joint.body1, joint.body2) {
                let motor = joint.data.motor(JointAxis::AngX).expect("drive motor");
                assert!((motor.target_vel - 7.5).abs() < 1e-9);
                assert_eq!(motor.max_force, WHEEL_DRIVE_MAX_TORQUE);
                saw_drive = true;
            }
            if rigid_body_pair_matches((steer, steer_link), joint.body1, joint.body2) {
                let motor = joint.data.motor(JointAxis::AngX).expect("steer motor");
                assert!((motor.target_pos - 0.25).abs() < 1e-9);
                assert_eq!(motor.stiffness, STEER_STIFFNESS);
                saw_steer = true;
            }
        }
        assert!(saw_drive);
        assert!(saw_steer);
    }

    #[test]
    fn multibody_joint_motors_bind_authored_body_pairs() {
        use rapier3d::prelude::{RevoluteJointBuilder, RigidBodyBuilder, Vector};

        let mut physics = usd_bevy::physics::PhysicsWorld::default();
        let chassis = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
        let wheel = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
        let steer = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
        let steer_link = physics.bodies.insert(RigidBodyBuilder::dynamic().build());

        physics.multibody_joints.insert(
            chassis,
            wheel,
            RevoluteJointBuilder::new(Vector::new(1.0, 0.0, 0.0)),
            true,
        );
        physics.multibody_joints.insert(
            steer,
            steer_link,
            RevoluteJointBuilder::new(Vector::new(0.0, 0.0, 1.0)),
            true,
        );

        let applied = apply_articulation_or_impulse_joint_motors(
            &mut physics,
            &[JointVelocityTarget {
                pair: (wheel, chassis),
                velocity: 3.5,
                damping: WHEEL_DRIVE_DAMPING,
                max_torque: WHEEL_DRIVE_MAX_TORQUE,
            }],
            &[JointPositionTarget {
                pair: (steer, steer_link),
                position: 0.15,
            }],
        );
        assert!(applied.drive);
        assert!(applied.steer);

        let (drive_handle, _, _) = physics
            .multibody_joints
            .joint_between(chassis, wheel)
            .expect("drive multibody joint");
        let (multibody, link_id) = physics
            .multibody_joints
            .get(drive_handle)
            .expect("drive multibody link");
        let motor = multibody
            .link(link_id)
            .expect("drive link")
            .joint
            .data
            .motor(JointAxis::AngX)
            .expect("drive motor");
        assert!((motor.target_vel - 3.5).abs() < 1e-9);

        let (steer_handle, _, _) = physics
            .multibody_joints
            .joint_between(steer, steer_link)
            .expect("steer multibody joint");
        let (multibody, link_id) = physics
            .multibody_joints
            .get(steer_handle)
            .expect("steer multibody link");
        let motor = multibody
            .link(link_id)
            .expect("steer link")
            .joint
            .data
            .motor(JointAxis::AngX)
            .expect("steer motor");
        assert!((motor.target_pos - 0.15).abs() < 1e-9);
    }

    #[test]
    fn five_referenced_tractor_instances_get_separate_ids_and_targets() {
        let tractor = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/tractor.usd")
            .canonicalize()
            .expect("tractor asset path");
        let world_path = std::env::temp_dir().join(format!(
            "gearbox_controller_instances_{}.usda",
            std::process::id()
        ));
        let tractor_ref = tractor.to_string_lossy();
        let mut defs = String::new();
        for idx in 1..=5 {
            let name = format!("Tractor_{idx:02}");
            let id = format!("tractor_{idx:02}");
            defs.push_str(&format!(
                r#"
    def Xform "{name}" (
        prepend references = @{tractor_ref}@</robot>
    )
    {{
        token gearbox:machine:id = "{id}"
    }}
"#
            ));
        }
        std::fs::write(
            &world_path,
            format!(
                r#"#usda 1.0
(
    defaultPrim = "World"
    upAxis = "Z"
)

def Xform "World"
{{
{defs}
}}
"#
            ),
        )
        .expect("write temp world");

        let mut machines = discover_machines_from_usd(&world_path).expect("world should scan");
        machines.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(machines.len(), 5);
        for idx in 1..=5 {
            let machine = &machines[idx - 1];
            let id = format!("tractor_{idx:02}");
            let root = format!("/World/Tractor_{idx:02}");
            assert_eq!(machine.id, id);
            assert_eq!(machine.controllers[0].namespace, id);
            assert_eq!(machine.controllers[0].instance, "drive");
            assert!(
                machine
                    .powered_wheel_joints
                    .iter()
                    .any(|p| p == &format!("{root}/Joints/rev_back_left"))
            );
            assert!(
                machine
                    .steering_joints
                    .iter()
                    .any(|p| p == &format!("{root}/Joints/steer_front_left"))
            );
        }

        let _ = std::fs::remove_file(world_path);
    }
}
