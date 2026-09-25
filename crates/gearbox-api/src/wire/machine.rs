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

    pub fn machine_id(&self) -> String {
        self.props().get("machine_id").unwrap_or_default()
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

/// One link of a machine's tree, answered on `/machines/<machine_id>/links`
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

/// World pose of one link, streamed on `/machines/<machine_id>/tf` while a client
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

/// One chunk of a LiDAR sweep of an authored `role = "sensor"` link, streamed
/// on `/machines/<machine_id>/sensors/<link>`. A sweep is `rows` zenith rows
/// of `cols` azimuth columns in row-major pulse order, zenith measured from
/// the link's +Z and azimuth from its +X towards +Y; this chunk carries the
/// `pulse_count` pulses from `pulse_offset`, and every chunk of one sweep
/// shares `sample`. Each pulse has one f32 range and one f32 xyz point in the
/// link's own frame; a miss has an infinite range and NaN coordinates.
#[datapod::datapod(name = "gearbox.lidar_scan.v1")]
#[derive(Default)]
pub struct LidarScan {
    pub sim_time_s: f64,
    pub theta_min: f64,
    pub theta_max: f64,
    pub phi_min: f64,
    pub phi_max: f64,
    pub max_range_m: f64,
    pub link_index: u32,
    pub rows: u32,
    pub cols: u32,
    pub pulse_offset: u32,
    pub pulse_count: u32,
    pub hits: u32,
    pub stamp_ms: u32,
    pub sample: u32,
    #[dp(bytes, section = "ranges")]
    pub ranges: Vec<f32>,
    #[dp(bytes, section = "points")]
    pub points: Vec<f32>,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

/// Pulses per chunk so that ranges, points and the header stay under
/// [`crate::host::MAX_PAYLOAD_BYTES`].
pub const LIDAR_CHUNK_PULSES: usize = 900;

/// One row band of one channel of a camera frame from an authored
/// `role = "sensor"` link, streamed on `/machines/<machine_id>/sensors/<link>`.
/// The image is `width` × `height` pixels, row-major from the top-left; this
/// chunk carries `rows` rows from `row_offset`, and every chunk of one frame
/// shares `sample`. `channel` is [`CAMERA_COLOR`] (packed RGBA8 per pixel) or
/// [`CAMERA_DEPTH`] (little-endian f32 metres along the pixel ray, infinite
/// for a miss). The camera looks along the link's +X with +Z up.
#[datapod::datapod(name = "gearbox.camera_frame.v1")]
#[derive(Default)]
pub struct CameraFrame {
    pub sim_time_s: f64,
    pub fov_y_rad: f64,
    pub max_range_m: f64,
    pub link_index: u32,
    pub width: u32,
    pub height: u32,
    pub row_offset: u32,
    pub rows: u32,
    pub channel: u32,
    pub stamp_ms: u32,
    pub sample: u32,
    #[dp(bytes, section = "data")]
    pub data: Vec<u8>,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

pub const CAMERA_COLOR: u32 = 0;
pub const CAMERA_DEPTH: u32 = 1;

/// Bytes of pixel data per chunk, under [`crate::host::MAX_PAYLOAD_BYTES`]
/// with the header and props.
pub const CAMERA_CHUNK_BYTES: usize = 12 * 1024;

impl CameraFrame {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn name(&self) -> String {
        self.props().get("name").unwrap_or_default()
    }

    pub fn is_last(&self) -> bool {
        self.row_offset + self.rows >= self.height
    }

    /// Rows per chunk for an image of `width` pixels at four bytes each.
    pub fn chunk_rows(width: u32) -> u32 {
        (CAMERA_CHUNK_BYTES / (width.max(1) as usize * 4)).max(1) as u32
    }

    /// Depth chunk pixels in metres; empty for a color chunk.
    pub fn depths(&self) -> Vec<f32> {
        if self.channel != CAMERA_DEPTH {
            return Vec::new();
        }
        self.data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    /// Color chunk pixels as packed RGBA8; empty for a depth chunk.
    pub fn colors(&self) -> Vec<u32> {
        if self.channel != CAMERA_COLOR {
            return Vec::new();
        }
        self.data
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }
}

/// A reading of a generic sensor link (the Webots sensor set beyond IMU,
/// LiDAR and camera), streamed on `/machines/<machine_id>/sensors/<link>`.
/// `values` holds `count` records laid out per [`measurement_kind`]; `data`
/// carries a receiver's packet; `props` names the link (`name`) and, per
/// kind, the emitter (`from`) or recognised objects (`names`, one per line).
#[datapod::datapod(name = "gearbox.measurement.v1")]
#[derive(Default)]
pub struct Measurement {
    pub sim_time_s: f64,
    pub link_index: u32,
    pub kind: u32,
    pub stamp_ms: u32,
    pub sample: u32,
    pub count: u32,
    pub _pad: u32,
    #[dp(bytes, section = "values")]
    pub values: Vec<f64>,
    #[dp(bytes, section = "data")]
    pub data: Vec<u8>,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

/// [`Measurement`] kinds and the layout of one record of `values`. Vectors
/// are in the link's axes (+X forward, +Z up) unless noted.
pub mod measurement_kind {
    /// `[ax, ay, az]` specific force (m/s²).
    pub const ACCELEROMETER: u32 = 1;
    /// `[wx, wy, wz]` angular velocity (rad/s).
    pub const GYRO: u32 = 2;
    /// `[roll, pitch, yaw, qx, qy, qz, qw]`: attitude in east-north-up (rad;
    /// yaw zero facing east, counter-clockwise positive).
    pub const INERTIAL_UNIT: u32 = 3;
    /// `[nx, ny, nz, heading]`: north in link axes, and the clockwise angle
    /// from north to the link's +X (rad).
    pub const COMPASS: u32 = 4;
    /// `[latitude°, longitude°, altitude m, east, north, up, speed,
    /// v_east, v_north, v_up]` (m, m/s; east-north-up from the site origin).
    pub const GPS: u32 = 5;
    /// `[distance m (infinite for none), value]`.
    pub const DISTANCE: u32 = 6;
    /// `[irradiance, direct, sky (W/m²), visible sources, value]`.
    pub const LIGHT: u32 = 7;
    /// `[position (rad or m), velocity]` of the link's joint.
    pub const POSITION: u32 = 8;
    /// Per target `[distance m, azimuth, elevation (rad), radial speed m/s,
    /// received power dBm]`, nearest first.
    pub const RADAR: u32 = 9;
    /// `[touching 0|1, contacts, force N, fx, fy, fz]`.
    pub const TOUCH: u32 = 10;
    /// One packet `[channel, signal strength, dx, dy, dz]` (direction to the
    /// emitter); payload in `data`, emitter link in props `from`.
    pub const RECEIVER: u32 = 11;
    /// Per object `[id, pixels, left, top, right, bottom, x, y, z, sx, sy,
    /// sz]` in camera axes (-Z forward, +Y up).
    pub const RECOGNITION: u32 = 12;
    /// Actuator: `[position, velocity, commanded velocity, force, brake
    /// damping, brake force, mode, target]` of a motor or brake (rad or m;
    /// force N·m or N; mode 0 position, 1 velocity, 2 force).
    pub const MOTOR: u32 = 13;
    /// Actuator: `[rotor speed rad/s, thrust N, torque N·m, advance m/s]`.
    pub const PROPELLER: u32 = 14;
    /// Actuator: `[travel m, speed m/s]` of a belt.
    pub const BELT: u32 = 15;
    /// Actuator: `[presence 0|1, locked 0|1, linked 0|1, tensile N, shear
    /// N]`; the present or linked peer in props `peer`.
    pub const CONNECTOR: u32 = 16;

    /// Values per record.
    pub fn width(kind: u32) -> usize {
        match kind {
            ACCELEROMETER | GYRO => 3,
            INERTIAL_UNIT => 7,
            COMPASS | PROPELLER => 4,
            GPS => 10,
            DISTANCE | POSITION | BELT => 2,
            LIGHT | RADAR | RECEIVER | CONNECTOR => 5,
            TOUCH => 6,
            RECOGNITION => 12,
            MOTOR => 8,
            _ => 0,
        }
    }

    pub fn name(kind: u32) -> &'static str {
        match kind {
            ACCELEROMETER => "accelerometer",
            GYRO => "gyro",
            INERTIAL_UNIT => "inertial_unit",
            COMPASS => "compass",
            GPS => "gps",
            DISTANCE => "distance",
            LIGHT => "light",
            POSITION => "position",
            RADAR => "radar",
            TOUCH => "touch",
            RECEIVER => "receiver",
            RECOGNITION => "recognition",
            MOTOR => "motor",
            PROPELLER => "propeller",
            BELT => "belt",
            CONNECTOR => "connector",
            _ => "unknown",
        }
    }
}

impl Measurement {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn name(&self) -> String {
        self.props().get("name").unwrap_or_default()
    }

    /// The `count` records of `values`.
    pub fn records(&self) -> impl Iterator<Item = &[f64]> {
        let width = measurement_kind::width(self.kind).max(1);
        self.values.chunks_exact(width).take(self.count as usize)
    }
}

/// A packet for an emitter link of a machine, sent to
/// `/machines/<machine_id>/emit`. Props: `link` names the emitter link.
#[datapod::datapod(name = "gearbox.emit_request.v1")]
#[derive(Default)]
pub struct EmitRequest {
    pub stamp_ms: u32,
    pub _pad: u32,
    #[dp(bytes, section = "data")]
    pub data: Vec<u8>,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

impl EmitRequest {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn link(&self) -> String {
        self.props().get("link").unwrap_or_default()
    }
}

/// A command for an actuator device of a machine, sent to
/// `/machines/<machine_id>/actuate`. Props: `device` names it; the other
/// props are its settings (`position`, `velocity`, `force`, `speed`,
/// `acceleration`, `max_force`, `pid`, `brake`, `lock`).
#[datapod::datapod(name = "gearbox.actuator_command.v1")]
#[derive(Default)]
pub struct ActuatorCommand {
    pub stamp_ms: u32,
    pub _pad: u32,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

impl ActuatorCommand {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn device(&self) -> String {
        self.props().get("device").unwrap_or_default()
    }
}

impl LidarScan {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn name(&self) -> String {
        self.props().get("name").unwrap_or_default()
    }

    /// Pulses of the whole sweep.
    pub fn sweep_pulses(&self) -> usize {
        self.rows as usize * self.cols as usize
    }

    /// Whether this chunk ends its sweep.
    pub fn is_last(&self) -> bool {
        self.pulse_offset as usize + self.pulse_count as usize >= self.sweep_pulses()
    }

    /// Row and column of a pulse index of the sweep.
    pub fn row_col(&self, pulse: usize) -> (u32, u32) {
        let cols = self.cols.max(1) as usize;
        ((pulse / cols) as u32, (pulse % cols) as u32)
    }

    /// Point of every pulse in the link frame, row-major; NaN for a miss.
    pub fn points(&self) -> Vec<[f32; 3]> {
        self.points
            .chunks_exact(3)
            .map(|p| [p[0], p[1], p[2]])
            .collect()
    }

    pub fn set_points(&mut self, points: &[[f32; 3]]) {
        self.points = points.iter().flatten().copied().collect();
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
