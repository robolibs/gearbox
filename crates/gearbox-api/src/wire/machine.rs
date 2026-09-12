//! Machine agent wire types: info, sessions, commands, state.

use datapod::robot::{Odom, Twist};

use super::common::Props;

#[datapod::datapod(name = "gearbox.machine_info.v1")]
#[derive(Default)]
pub struct MachineInfo {
    pub controller_count: u32,
    pub held: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl MachineInfo {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn namespace(&self) -> String {
        self.props().get("namespace").unwrap_or_default()
    }

    /// Controller instances with the given command interface.
    pub fn controllers_with_command(&self, interface: &str) -> Vec<String> {
        let props = self.props();
        (0..self.controller_count)
            .filter_map(|n| {
                let name = props.get(&format!("controller.{n}.instance"))?;
                let cmd = props.get(&format!("controller.{n}.command_interface"))?;
                (cmd == interface).then_some(name)
            })
            .collect()
    }
}

#[datapod::datapod(name = "gearbox.claim_request.v1")]
#[derive(Default)]
pub struct ClaimRequest {
    pub take: u32,
    pub hold_ms: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl ClaimRequest {
    pub fn new(client: &str, hold_ms: u32) -> Self {
        Self {
            take: 0,
            hold_ms,
            props: Props::from_pairs(&[("client", client)]).into_bytes(),
        }
    }

    pub fn taking(mut self) -> Self {
        self.take = 1;
        self
    }

    pub fn client(&self) -> String {
        Props::from_bytes(&self.props)
            .get("client")
            .unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.claim_response.v1")]
#[derive(Default)]
pub struct ClaimResponse {
    pub session: u64,
    pub code: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl ClaimResponse {
    pub fn granted(session: u64) -> Self {
        Self {
            session,
            ..Default::default()
        }
    }

    pub fn busy(holder: &str) -> Self {
        Self {
            code: super::common::code::BUSY,
            props: Props::from_pairs(&[("holder", holder), ("message", "machine is held")])
                .into_bytes(),
            ..Default::default()
        }
    }

    pub fn holder(&self) -> String {
        Props::from_bytes(&self.props)
            .get("holder")
            .unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.session_ref.v1")]
#[derive(Default)]
pub struct SessionRef {
    pub session: u64,
}

#[datapod::datapod(name = "gearbox.session_info.v1")]
#[derive(Default)]
pub struct SessionInfo {
    pub session: u64,
    pub age_ms: u64,
    pub idle_ms: u64,
    pub held: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl SessionInfo {
    pub fn holder(&self) -> String {
        Props::from_bytes(&self.props)
            .get("holder")
            .unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.twist_cmd.v1")]
#[derive(Default)]
pub struct TwistCmd {
    pub session: u64,
    pub twist: Twist,
}

impl TwistCmd {
    pub fn new(session: u64, forward_mps: f64, yaw_rps: f64) -> Self {
        Self {
            session,
            twist: Twist::from_components(forward_mps, 0.0, 0.0, 0.0, 0.0, yaw_rps),
        }
    }
}

#[datapod::datapod(name = "gearbox.controller_command.v1")]
#[derive(Default)]
pub struct ControllerCommand {
    pub session: u64,
    pub value: f64,
    pub element: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl ControllerCommand {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}

#[datapod::datapod(name = "gearbox.machine_state.v1")]
#[derive(Default)]
pub struct MachineState {
    pub odom: Odom,
    pub heading_rad: f64,
    pub roll_rad: f64,
    pub pitch_rad: f64,
    pub session: u64,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl MachineState {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn position(&self) -> [f64; 3] {
        let p = self.odom.pose.point;
        [p.x, p.y, p.z]
    }

    pub fn linear_speed(&self) -> f64 {
        let v = self.odom.twist.linear;
        (v.vx * v.vx + v.vy * v.vy + v.vz * v.vz).sqrt()
    }

    pub fn yaw_rate(&self) -> f64 {
        self.odom.twist.angular.vz
    }
}

/// One link of a machine's tree, answered on `/machines/<ns>/links`
/// base_link first. The offset is the static transform to the parent link
/// in the asset's Z-up frame; joint-connected links report zero.
#[datapod::datapod(name = "gearbox.link_record.v1")]
#[derive(Default)]
pub struct LinkRecord {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub qw: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    pub index: u32,
    pub parent_index: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl LinkRecord {
    pub const NO_PARENT: u32 = u32::MAX;

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn name(&self) -> String {
        self.props().get("name").unwrap_or_default()
    }

    pub fn parent(&self) -> Option<String> {
        self.props().get("parent").filter(|p| !p.is_empty())
    }

    pub fn role(&self) -> String {
        self.props()
            .get("role")
            .unwrap_or_else(|| "link".to_string())
    }

    /// Element kind when the link is a working part (function, bin, ...).
    pub fn element(&self) -> Option<String> {
        self.props().get("element").filter(|e| !e.is_empty())
    }

    /// `(name, value)` pairs from the `value.*` props.
    pub fn values(&self) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = self
            .props()
            .iter()
            .into_iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("value.")
                    .map(|n| (n.to_string(), v.parse().unwrap_or(0.0)))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

/// World pose of one link, streamed on `/machines/<ns>/tf` while a client
/// has switched it on with `/cmd` `tf = on`. Sim world frame, Y up.
#[datapod::datapod(name = "gearbox.link_pose.v1")]
#[derive(Default)]
pub struct LinkPose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub qw: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    pub index: u32,
    pub stamp_ms: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl LinkPose {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn name(&self) -> String {
        self.props().get("name").unwrap_or_default()
    }
}

/// Attach a slave to one of this machine's hitches
/// (`/machines/<master>/tools/attach`). Props: `slave`, `hitch`, `coupler`.
#[datapod::datapod(name = "gearbox.attach_request.v1")]
#[derive(Default)]
pub struct AttachRequest {
    pub session: u64,
    pub teleport: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl AttachRequest {
    pub fn new(session: u64, slave: &str) -> Self {
        Self {
            session,
            props: Props::from_pairs(&[("slave", slave)]).into_bytes(),
            ..Default::default()
        }
    }

    pub fn with_hitch(mut self, hitch: &str) -> Self {
        self.props = Props::from_bytes(&self.props)
            .with("hitch", hitch)
            .into_bytes();
        self
    }

    pub fn with_coupler(mut self, coupler: &str) -> Self {
        self.props = Props::from_bytes(&self.props)
            .with("coupler", coupler)
            .into_bytes();
        self
    }

    pub fn teleporting(mut self) -> Self {
        self.teleport = 1;
        self
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn slave(&self) -> String {
        self.props().get("slave").unwrap_or_default()
    }

    pub fn hitch(&self) -> Option<String> {
        self.props().get("hitch").filter(|h| !h.is_empty())
    }

    pub fn coupler(&self) -> Option<String> {
        self.props().get("coupler").filter(|c| !c.is_empty())
    }
}

/// Detach a slave (`/machines/<master>/tools/detach`). Props: `slave`.
#[datapod::datapod(name = "gearbox.detach_request.v1")]
#[derive(Default)]
pub struct DetachRequest {
    pub session: u64,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl DetachRequest {
    pub fn new(session: u64, slave: &str) -> Self {
        Self {
            session,
            props: Props::from_pairs(&[("slave", slave)]).into_bytes(),
        }
    }

    pub fn slave(&self) -> String {
        Props::from_bytes(&self.props)
            .get("slave")
            .unwrap_or_default()
    }
}

/// One attachment of a composite, answered on `/machines/<master>/tools`
/// depth-first. Props: `master`, `slave`, `hitch`, `coupler`, `type`.
#[datapod::datapod(name = "gearbox.attachment.v1")]
#[derive(Default)]
pub struct AttachmentRecord {
    pub controlled: u32,
    pub depth: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl AttachmentRecord {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn slave(&self) -> String {
        self.props().get("slave").unwrap_or_default()
    }
}
