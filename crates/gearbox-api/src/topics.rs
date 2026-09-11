//! Topic names. Exact-match on peerbus, so ids ride in payloads.

pub const HOST_INFO: &str = "/gearbox/info";
pub const SCENE_CLOCK: &str = "/gearbox/scene/clock";
pub const SCENE_CLOCK_STATE: &str = "/gearbox/scene/clock/state";
pub const SCENE_CLEAR: &str = "/gearbox/scene/clear";
pub const SCENE_LIST: &str = "/gearbox/scene/list";
pub const SCENE_EVENTS: &str = "/gearbox/scene/events";
pub const USD_LOAD: &str = "/gearbox/usd/load";
pub const USD_DELETE: &str = "/gearbox/usd/delete";
pub const MARKER_SET: &str = "/gearbox/marker/set";
pub const MARKER_DELETE: &str = "/gearbox/marker/delete";
pub const SELECT: &str = "/gearbox/select";
pub const MACHINES_LIST: &str = "/gearbox/machines/list";

pub const MACHINE_INFO: &str = "info";
pub const MACHINE_CLAIM: &str = "claim";
pub const MACHINE_RELEASE: &str = "release";
pub const MACHINE_SESSION: &str = "session";
pub const MACHINE_CMD_VEL: &str = "cmd_vel";
pub const MACHINE_CMD: &str = "cmd";
pub const MACHINE_STATE: &str = "state";
pub const MACHINE_ODOM: &str = "odom";
pub const MACHINE_LINKS: &str = "links";

/// Topics hosted by a machine's own agent, unique per namespace so the
/// host directory holds one owner per topic.
pub fn machine_topic(namespace: &str, leaf: &str) -> String {
    format!("/machines/{namespace}/{leaf}")
}
