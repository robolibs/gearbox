//! Versioned machine messages of the robot contract (`*_v2` leaves under
//! `/machines/<id>/`). SI units: rad or m, rad/s or m/s, N·m or N. DOFs are
//! named by their public names from `description_v2`; a name list is UTF-8
//! with one name per line.

use super::common::Props;

/// The canonical JSON manifest of a machine generation: bodies, joints,
/// DOFs with units and interfaces, frames, owners and capabilities.
#[datapod::datapod(name = "gearbox.machine_description.v2")]
#[derive(Default)]
pub struct MachineDescriptionV2 {
    pub generation: u64,
    #[dp(bytes)]
    pub manifest: Vec<u8>,
}

impl MachineDescriptionV2 {
    pub fn manifest_json(&self) -> String {
        String::from_utf8_lossy(&self.manifest).into_owned()
    }
}

pub fn names_to_bytes<S: AsRef<str>>(names: &[S]) -> Vec<u8> {
    names.iter().map(|n| n.as_ref()).collect::<Vec<_>>().join("\n").into_bytes()
}

pub fn names_from_bytes(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes).lines().filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// Per-DOF mode of a [`JointCommandV2`].
pub mod joint_mode {
    pub const POSITION: u8 = 0;
    pub const VELOCITY: u8 = 1;
    pub const EFFORT: u8 = 2;
    pub const HOLD: u8 = 3;
    pub const FREE: u8 = 4;

    pub fn name(mode: u8) -> &'static str {
        match mode {
            POSITION => "position",
            VELOCITY => "velocity",
            EFFORT => "effort",
            HOLD => "hold",
            FREE => "free",
            _ => "unknown",
        }
    }
}

/// A batch of joint commands, validated whole and applied all or none.
/// `values[i]` is the position, velocity or effort of `names[i]` by
/// `modes[i]`; `feedforward[i]` is a position command's velocity, NaN for
/// none. `validity_ms` 0 asks for the machine's default validity.
#[datapod::datapod(name = "gearbox.joint_command.v2")]
#[derive(Default)]
pub struct JointCommandV2 {
    pub session: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub generation: u64,
    /// Requested operating force or torque cap for every DOF; NaN for none.
    pub max_effort: f64,
    pub validity_ms: u32,
    pub _pad: u32,
    #[dp(bytes, section = "modes")]
    pub modes: Vec<u8>,
    #[dp(bytes, section = "values")]
    pub values: Vec<f64>,
    #[dp(bytes, section = "feedforward")]
    pub feedforward: Vec<f64>,
    #[dp(bytes, section = "names")]
    pub names: Vec<u8>,
}

impl JointCommandV2 {
    pub fn names(&self) -> Vec<String> {
        names_from_bytes(&self.names)
    }
}

/// Where a command is in its life.
pub mod command_status {
    pub const RECEIVED: u32 = 0;
    pub const ACCEPTED: u32 = 1;
    pub const REJECTED: u32 = 2;
    pub const APPLIED: u32 = 3;
}

/// The answer to a [`JointCommandV2`]: accepted for the next physics tick,
/// or rejected with the exact binding or authority error.
#[datapod::datapod(name = "gearbox.joint_command_ack.v2")]
#[derive(Default)]
pub struct JointCommandAckV2 {
    pub sequence: u64,
    pub generation: u64,
    /// Physics tick at acceptance; the command applies from the next one.
    pub tick: u64,
    pub status: u32,
    pub code: u32,
    #[dp(bytes)]
    pub message: Vec<u8>,
}

impl JointCommandAckV2 {
    pub fn message(&self) -> String {
        String::from_utf8_lossy(&self.message).into_owned()
    }
}

pub mod control_op {
    pub const ACQUIRE: u32 = 0;
    pub const RELEASE: u32 = 1;
    /// Clear every command, hold and lease of the machine; each DOF goes
    /// back to its default owner and load-time actuator.
    pub const RESET: u32 = 2;
}

/// Request or release direct authority over a set of DOFs under the
/// caller's validated machine session.
#[datapod::datapod(name = "gearbox.control_request.v2")]
#[derive(Default)]
pub struct ControlRequestV2 {
    pub session: u64,
    pub generation: u64,
    /// The epoch being released.
    pub epoch: u64,
    pub op: u32,
    /// On release: give the DOFs back to their previous owner.
    pub resume: u32,
    #[dp(bytes)]
    pub names: Vec<u8>,
}

/// The server-issued lease: the epoch every command of it carries.
#[datapod::datapod(name = "gearbox.control_response.v2")]
#[derive(Default)]
pub struct ControlResponseV2 {
    pub epoch: u64,
    pub generation: u64,
    /// Physics tick the transfer takes effect at.
    pub tick: u64,
    pub code: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl ControlResponseV2 {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn owner(&self) -> String {
        self.props().get("owner").unwrap_or_default()
    }

    pub fn message(&self) -> String {
        self.props().get("message").unwrap_or_default()
    }
}

/// Measured joint state after a physics step, with what was commanded.
/// `measured_effort[i]` is NaN when the backend cannot measure it; a
/// commanded effort is never reported as measured.
#[datapod::datapod(name = "gearbox.joint_state.v2")]
#[derive(Default)]
pub struct JointStateV2 {
    pub generation: u64,
    pub tick: u64,
    pub sim_time_s: f64,
    /// 1 when the sample is from the current generation's last tick.
    pub valid: u32,
    pub _pad: u32,
    #[dp(bytes, section = "position")]
    pub position: Vec<f64>,
    #[dp(bytes, section = "velocity")]
    pub velocity: Vec<f64>,
    #[dp(bytes, section = "measured_effort")]
    pub measured_effort: Vec<f64>,
    #[dp(bytes, section = "commanded_effort")]
    pub commanded_effort: Vec<f64>,
    #[dp(bytes, section = "target")]
    pub target: Vec<f64>,
    #[dp(bytes, section = "names")]
    pub names: Vec<u8>,
}

impl JointStateV2 {
    pub fn names(&self) -> Vec<String> {
        names_from_bytes(&self.names)
    }
}

/// Parent-relative transforms of a machine's reporting tree in a declared
/// right-handed Z-up world (`props.convention`), with the world root pose of
/// the tree's root. Edge `i` places `children[i]` in `parents[i]`:
/// `translation[3i..3i+3]` m and `rotation[4i..4i+4]` as w, x, y, z.
#[datapod::datapod(name = "gearbox.tf.v2")]
#[derive(Default)]
pub struct TfV2 {
    pub generation: u64,
    pub tick: u64,
    pub sim_time_s: f64,
    #[dp(bytes, section = "translation")]
    pub translation: Vec<f64>,
    #[dp(bytes, section = "rotation")]
    pub rotation: Vec<f64>,
    #[dp(bytes, section = "parents")]
    pub parents: Vec<u8>,
    #[dp(bytes, section = "children")]
    pub children: Vec<u8>,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

impl TfV2 {
    pub fn parents(&self) -> Vec<String> {
        names_from_bytes(&self.parents)
    }

    pub fn children(&self) -> Vec<String> {
        names_from_bytes(&self.children)
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}
