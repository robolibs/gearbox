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
