//! Host agent wire types: instance info, clock, scene, USD loads, markers.

use super::common::Props;

pub mod category {
    pub const STATIC: u32 = 0;
    pub const MACHINE: u32 = 1;
    pub const VARIANT: u32 = 2;
    pub const WORLD: u32 = 3;
    pub const TERRAIN: u32 = 4;

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "" | "static" | "static_usd" => Some(STATIC),
            "machine" | "robot" => Some(MACHINE),
            "variant" | "variant_usd" => Some(VARIANT),
            "world" => Some(WORLD),
            "terrain" => Some(TERRAIN),
            _ => None,
        }
    }

    pub fn name(value: u32) -> &'static str {
        match value {
            MACHINE => "machine",
            VARIANT => "variant",
            WORLD => "world",
            TERRAIN => "terrain",
            _ => "static",
        }
    }
}

pub mod clock_op {
    pub const GET: u32 = 0;
    pub const PAUSE: u32 = 1;
    pub const PLAY: u32 = 2;
    pub const TOGGLE: u32 = 3;
    pub const SHUTDOWN: u32 = 4;
}

pub mod clear_scope {
    pub const ALL: u32 = 0;
    pub const MACHINES: u32 = 1;
    pub const PROPS: u32 = 2;
    pub const MARKERS: u32 = 3;
}

pub mod object_kind {
    pub const ANY: u32 = 0;
    pub const MACHINE: u32 = 1;
    pub const PROP: u32 = 2;
    pub const MARKER: u32 = 3;
    pub const TERRAIN: u32 = 4;

    pub fn name(value: u32) -> &'static str {
        match value {
            MACHINE => "machine",
            PROP => "prop",
            MARKER => "marker",
            TERRAIN => "terrain",
            _ => "any",
        }
    }
}

pub mod event_kind {
    pub const LOADED: u32 = 0;
    pub const POSE: u32 = 1;
    pub const HARVESTED: u32 = 2;
    pub const REMOVED: u32 = 3;
    pub const MACHINE_READY: u32 = 4;

    pub fn name(value: u32) -> &'static str {
        match value {
            LOADED => "loaded",
            POSE => "pose",
            HARVESTED => "harvested",
            REMOVED => "removed",
            MACHINE_READY => "machine_ready",
            _ => "unknown",
        }
    }
}

pub mod load_flag {
    pub const REMOVE: u32 = 1;
    pub const DELETE: u32 = 2;
}

#[datapod::datapod(name = "gearbox.host_info.v1")]
#[derive(Default)]
pub struct HostInfo {
    pub uptime_ms: u64,
    pub pid: u32,
    pub paused: u32,
    pub machine_count: u32,
    pub object_count: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl HostInfo {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}

#[datapod::datapod(name = "gearbox.clock_command.v1")]
#[derive(Default)]
pub struct ClockCommand {
    pub op: u32,
    pub steps: u32,
}

#[datapod::datapod(name = "gearbox.clock_state.v1")]
#[derive(Default)]
pub struct ClockState {
    pub step: u64,
    pub paused: u32,
    pub _pad: u32,
}

#[datapod::datapod(name = "gearbox.clear_request.v1")]
#[derive(Default)]
pub struct ClearRequest {
    pub scope: u32,
    pub pause_clock: u32,
}

#[datapod::datapod(name = "gearbox.list_query.v1")]
#[derive(Default)]
pub struct ListQuery {
    pub kind: u32,
}

#[datapod::datapod(name = "gearbox.scene_object.v1")]
#[derive(Default)]
pub struct SceneObject {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw_deg: f32,
    pub kind: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl SceneObject {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}

#[datapod::datapod(name = "gearbox.scene_event.v1")]
#[derive(Default)]
pub struct SceneEvent {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub top_y: f32,
    pub kind: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl SceneEvent {
    pub fn new(kind: u32, id: &str) -> Self {
        Self {
            kind,
            props: Props::from_pairs(&[("id", id)]).into_bytes(),
            ..Default::default()
        }
    }

    pub fn at(mut self, x: f32, y: f32, z: f32) -> Self {
        self.x = x;
        self.y = y;
        self.z = z;
        self
    }

    pub fn with_top(mut self, top_y: f32) -> Self {
        self.top_y = top_y;
        self
    }

    pub fn with_prop(mut self, key: &str, value: &str) -> Self {
        let props = Props::from_bytes(&self.props).with(key, value);
        self.props = props.into_bytes();
        self
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn id(&self) -> String {
        self.props().get("id").unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.usd_load.v1")]
#[derive(Default)]
pub struct UsdLoad {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw_deg: f32,
    pub category: u32,
    pub flags: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl UsdLoad {
    pub fn new(id: &str, path: &str) -> Self {
        Self {
            props: Props::from_pairs(&[("id", id), ("path", path)]).into_bytes(),
            ..Default::default()
        }
    }

    pub fn at(mut self, x: f32, y: f32, z: f32) -> Self {
        self.x = x;
        self.y = y;
        self.z = z;
        self
    }

    pub fn yaw(mut self, yaw_deg: f32) -> Self {
        self.yaw_deg = yaw_deg;
        self
    }

    pub fn category(mut self, category: u32) -> Self {
        self.category = category;
        self
    }

    pub fn with_prop(mut self, key: &str, value: &str) -> Self {
        let props = Props::from_bytes(&self.props).with(key, value);
        self.props = props.into_bytes();
        self
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn id(&self) -> String {
        self.props().get("id").unwrap_or_default()
    }

    pub fn path(&self) -> Option<String> {
        self.props().get("path")
    }

    pub fn namespace(&self) -> Option<String> {
        self.props().get("namespace")
    }

    pub fn nonce(&self) -> String {
        self.props().get("nonce").unwrap_or_default()
    }

    pub fn remove(&self) -> bool {
        self.flags & load_flag::REMOVE != 0
    }

    pub fn delete(&self) -> bool {
        self.flags & load_flag::DELETE != 0
    }

    pub fn is_machine(&self) -> bool {
        self.category == category::MACHINE
    }

    /// Variants as `prim|set|option` entries under `variant.<n>`.
    pub fn variants(&self) -> Vec<(String, String, String)> {
        let props = self.props();
        let mut out = Vec::new();
        for n in 0.. {
            let Some(v) = props.get(&format!("variant.{n}")) else {
                break;
            };
            let parts: Vec<&str> = v.splitn(3, '|').collect();
            if parts.len() == 3 {
                out.push((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                ));
            }
        }
        out
    }
}

#[datapod::datapod(name = "gearbox.usd_ref.v1")]
#[derive(Default)]
pub struct UsdRef {
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl UsdRef {
    pub fn new(id: &str) -> Self {
        Self {
            props: Props::from_pairs(&[("id", id)]).into_bytes(),
        }
    }

    pub fn id(&self) -> String {
        Props::from_bytes(&self.props).get("id").unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.marker_set.v1")]
#[derive(Default)]
pub struct MarkerSet {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub _pad: f32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl MarkerSet {
    pub fn new(id: &str, x: f32, y: f32, z: f32) -> Self {
        Self {
            x,
            y,
            z,
            _pad: 0.0,
            props: Props::from_pairs(&[("id", id)]).into_bytes(),
        }
    }

    pub fn id(&self) -> String {
        Props::from_bytes(&self.props).get("id").unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.marker_ref.v1")]
#[derive(Default)]
pub struct MarkerRef {
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl MarkerRef {
    pub fn new(id: &str) -> Self {
        Self {
            props: Props::from_pairs(&[("id", id)]).into_bytes(),
        }
    }

    pub fn id(&self) -> String {
        Props::from_bytes(&self.props).get("id").unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.selection.v1")]
#[derive(Default)]
pub struct Selection {
    pub kind: u32,
    pub query: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl Selection {
    pub fn get() -> Self {
        Self {
            query: 1,
            ..Default::default()
        }
    }

    pub fn set(kind: u32, id: &str) -> Self {
        Self {
            kind,
            query: 0,
            props: Props::from_pairs(&[("id", id)]).into_bytes(),
        }
    }

    pub fn id(&self) -> String {
        Props::from_bytes(&self.props).get("id").unwrap_or_default()
    }
}

#[datapod::datapod(name = "gearbox.machine_ref.v1")]
#[derive(Default)]
pub struct MachineRef {
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl MachineRef {
    pub fn new(namespace: &str, did: &str, kind: &str, machine_id: &str) -> Self {
        Self {
            props: Props::from_pairs(&[
                ("namespace", namespace),
                ("did", did),
                ("kind", kind),
                ("machine_id", machine_id),
            ])
            .into_bytes(),
        }
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn namespace(&self) -> String {
        self.props().get("namespace").unwrap_or_default()
    }

    pub fn did(&self) -> String {
        self.props().get("did").unwrap_or_default()
    }
}
