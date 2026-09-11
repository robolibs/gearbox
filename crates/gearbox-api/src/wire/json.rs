//! JSON views of every wire type, keyed by canonical datapod name, so a
//! CLI or a script can print any envelope and build any request without
//! knowing the Rust types.

use datapod::robot::{Odom, Twist};
use datapod::schema::SchemaDescriptor;
use datapod::{Point, Pose, Quaternion};
use serde_json::{Map, Value, json};

use super::common::{Env, Ping, Props, Status, unpack};
use super::host::*;
use super::machine::*;

pub struct TypeDesc {
    pub name: &'static str,
    pub type_hash: u64,
    pub schema: fn() -> SchemaDescriptor,
    pub to_json: fn(&[u8]) -> Result<Value, String>,
    pub from_json: fn(&Value) -> Result<Env, String>,
}

macro_rules! desc {
    ($ty:ty, $to:expr, $from:expr) => {
        TypeDesc {
            name: <$ty>::CANONICAL_NAME,
            type_hash: <$ty>::TYPE_HASH,
            schema: || datapod::schema::describe::<$ty>(<$ty>::CANONICAL_NAME),
            to_json: |wire| {
                let v: $ty = unpack(<$ty>::TYPE_HASH, wire).map_err(|e| e.to_string())?;
                Ok($to(&v))
            },
            from_json: |v| Ok(super::common::pack(&$from(v)?)),
        }
    };
}

pub fn types() -> Vec<TypeDesc> {
    vec![
        desc!(Ping, ping_json, ping_from),
        desc!(Status, status_json, status_from),
        desc!(HostInfo, host_info_json, host_info_from),
        desc!(ClockCommand, clock_command_json, clock_command_from),
        desc!(ClockState, clock_state_json, clock_state_from),
        desc!(ClearRequest, clear_request_json, clear_request_from),
        desc!(ListQuery, list_query_json, list_query_from),
        desc!(SceneObject, scene_object_json, scene_object_from),
        desc!(SceneEvent, scene_event_json, scene_event_from),
        desc!(UsdLoad, usd_load_json, usd_load_from),
        desc!(UsdRef, usd_ref_json, usd_ref_from),
        desc!(MarkerSet, marker_set_json, marker_set_from),
        desc!(MarkerRef, marker_ref_json, marker_ref_from),
        desc!(Selection, selection_json, selection_from),
        desc!(MachineRef, machine_ref_json, machine_ref_from),
        desc!(MachineInfo, machine_info_json, machine_info_from),
        desc!(ClaimRequest, claim_request_json, claim_request_from),
        desc!(ClaimResponse, claim_response_json, claim_response_from),
        desc!(SessionRef, session_ref_json, session_ref_from),
        desc!(SessionInfo, session_info_json, session_info_from),
        desc!(TwistCmd, twist_cmd_json, twist_cmd_from),
        desc!(
            ControllerCommand,
            controller_command_json,
            controller_command_from
        ),
        desc!(MachineState, machine_state_json, machine_state_from),
    ]
}

pub fn find_by_name(name: &str) -> Option<TypeDesc> {
    let name = if name.contains('.') {
        name.to_string()
    } else {
        format!("gearbox.{name}.v1")
    };
    types().into_iter().find(|t| t.name == name)
}

pub fn find_by_hash(type_hash: u64) -> Option<TypeDesc> {
    types().into_iter().find(|t| t.type_hash == type_hash)
}

/// JSON for any envelope carrying a gearbox or datapod type.
pub fn env_to_json(env: &Env) -> Result<Value, String> {
    if let Some(t) = find_by_hash(env.type_hash()) {
        return (t.to_json)(env.wire());
    }
    if env.type_hash() == datapod::bind::type_hash::<Odom>() {
        let odom: Odom = unpack(env.type_hash(), env.wire()).map_err(|e| e.to_string())?;
        return Ok(odom_json(&odom));
    }
    let view = env.dynamic().map_err(|e| e.to_string())?;
    Ok(json!({
        "type": view.schema().canonical_name,
        "type_hash": env.type_hash(),
        "bytes": env.wire().len(),
    }))
}

pub fn env_from_json(name: &str, value: &Value) -> Result<Env, String> {
    let t = find_by_name(name).ok_or_else(|| format!("unknown wire type `{name}`"))?;
    (t.from_json)(value)
}

// ── helpers ─────────────────────────────────────────────────────────

fn props_json(bytes: &[u8]) -> Value {
    let mut map = Map::new();
    for (k, v) in Props::from_bytes(bytes).iter() {
        map.insert(k, Value::String(v));
    }
    Value::Object(map)
}

/// Every key that is not a fixed field becomes a string prop, and an
/// explicit `props` object merges in on top.
fn props_from(value: &Value, fixed: &[&str]) -> Vec<u8> {
    let mut props = Props::new();
    if let Some(obj) = value.as_object() {
        for (k, v) in obj {
            if k == "props" || fixed.contains(&k.as_str()) {
                continue;
            }
            if let Some(s) = scalar_string(v) {
                props.set(k, &s);
            }
        }
        if let Some(extra) = obj.get("props").and_then(Value::as_object) {
            for (k, v) in extra {
                if let Some(s) = scalar_string(v) {
                    props.set(k, &s);
                }
            }
        }
    }
    props.into_bytes()
}

fn scalar_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn uint(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::Bool(b)) => *b as u64,
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn velocity_json(v: &datapod::Velocity) -> Value {
    json!({ "vx": v.vx, "vy": v.vy, "vz": v.vz })
}

fn twist_json(t: &Twist) -> Value {
    json!({ "linear": velocity_json(&t.linear), "angular": velocity_json(&t.angular) })
}

fn twist_from(v: &Value) -> Twist {
    let lin = v.get("linear").cloned().unwrap_or(Value::Null);
    let ang = v.get("angular").cloned().unwrap_or(Value::Null);
    Twist::from_components(
        num(&lin, "vx"),
        num(&lin, "vy"),
        num(&lin, "vz"),
        num(&ang, "vx"),
        num(&ang, "vy"),
        num(&ang, "vz"),
    )
}

fn odom_json(o: &Odom) -> Value {
    let p = o.pose.point;
    let r = o.pose.rotation;
    json!({
        "pose": {
            "point": { "x": p.x, "y": p.y, "z": p.z },
            "rotation": { "w": r.w, "x": r.x, "y": r.y, "z": r.z },
        },
        "twist": twist_json(&o.twist),
    })
}

fn odom_from(v: &Value) -> Odom {
    let pose = v.get("pose").cloned().unwrap_or(Value::Null);
    let point = pose.get("point").cloned().unwrap_or(Value::Null);
    let rot = pose.get("rotation").cloned().unwrap_or(Value::Null);
    let w = rot.get("w").and_then(Value::as_f64).unwrap_or(1.0);
    Odom::new(
        Pose {
            point: Point::new(num(&point, "x"), num(&point, "y"), num(&point, "z")),
            rotation: Quaternion::new(w, num(&rot, "x"), num(&rot, "y"), num(&rot, "z")),
        },
        twist_from(&v.get("twist").cloned().unwrap_or(Value::Null)),
    )
}

// ── per type ────────────────────────────────────────────────────────

fn ping_json(p: &Ping) -> Value {
    json!({ "nonce": p.nonce })
}

fn ping_from(v: &Value) -> Result<Ping, String> {
    Ok(Ping {
        nonce: uint(v, "nonce"),
    })
}

fn status_json(s: &Status) -> Value {
    json!({ "code": s.code, "ok": s.is_ok(), "message": s.message(), "detail": props_json(&s.detail) })
}

fn status_from(v: &Value) -> Result<Status, String> {
    Ok(Status {
        code: uint(v, "code") as u32,
        detail: props_from(v, &["code", "ok", "message", "detail"]),
    })
}

fn host_info_json(h: &HostInfo) -> Value {
    json!({
        "uptime_ms": h.uptime_ms, "pid": h.pid, "paused": h.paused != 0,
        "machine_count": h.machine_count, "object_count": h.object_count,
        "props": props_json(&h.props),
    })
}

fn host_info_from(v: &Value) -> Result<HostInfo, String> {
    Ok(HostInfo {
        uptime_ms: uint(v, "uptime_ms"),
        pid: uint(v, "pid") as u32,
        paused: uint(v, "paused") as u32,
        machine_count: uint(v, "machine_count") as u32,
        object_count: uint(v, "object_count") as u32,
        props: props_from(
            v,
            &[
                "uptime_ms",
                "pid",
                "paused",
                "machine_count",
                "object_count",
            ],
        ),
    })
}

fn clock_command_json(c: &ClockCommand) -> Value {
    json!({ "op": c.op, "op_name": clock_op::name(c.op), "steps": c.steps })
}

fn clock_command_from(v: &Value) -> Result<ClockCommand, String> {
    let op = match v.get("op") {
        Some(Value::String(s)) => {
            clock_op::parse(s).ok_or_else(|| format!("unknown clock op `{s}`"))?
        }
        _ => uint(v, "op") as u32,
    };
    Ok(ClockCommand {
        op,
        steps: uint(v, "steps") as u32,
    })
}

fn clock_state_json(c: &ClockState) -> Value {
    json!({ "step": c.step, "paused": c.paused != 0 })
}

fn clock_state_from(v: &Value) -> Result<ClockState, String> {
    Ok(ClockState {
        step: uint(v, "step"),
        paused: uint(v, "paused") as u32,
        _pad: 0,
    })
}

fn clear_request_json(c: &ClearRequest) -> Value {
    json!({ "scope": c.scope, "scope_name": clear_scope::name(c.scope), "pause_clock": c.pause_clock != 0 })
}

fn clear_request_from(v: &Value) -> Result<ClearRequest, String> {
    let scope = match v.get("scope") {
        Some(Value::String(s)) => {
            clear_scope::parse(s).ok_or_else(|| format!("unknown clear scope `{s}`"))?
        }
        _ => uint(v, "scope") as u32,
    };
    Ok(ClearRequest {
        scope,
        pause_clock: uint(v, "pause_clock") as u32,
    })
}

fn list_query_json(q: &ListQuery) -> Value {
    json!({ "kind": q.kind, "kind_name": object_kind::name(q.kind) })
}

fn list_query_from(v: &Value) -> Result<ListQuery, String> {
    let kind = match v.get("kind") {
        Some(Value::String(s)) => {
            object_kind::parse(s).ok_or_else(|| format!("unknown object kind `{s}`"))?
        }
        _ => uint(v, "kind") as u32,
    };
    Ok(ListQuery { kind })
}

fn scene_object_json(o: &SceneObject) -> Value {
    json!({
        "x": o.x, "y": o.y, "z": o.z, "yaw_deg": o.yaw_deg,
        "kind": o.kind, "kind_name": object_kind::name(o.kind),
        "props": props_json(&o.props),
    })
}

fn scene_object_from(v: &Value) -> Result<SceneObject, String> {
    Ok(SceneObject {
        x: num(v, "x") as f32,
        y: num(v, "y") as f32,
        z: num(v, "z") as f32,
        yaw_deg: num(v, "yaw_deg") as f32,
        kind: uint(v, "kind") as u32,
        props: props_from(v, &["x", "y", "z", "yaw_deg", "kind", "kind_name"]),
    })
}

fn scene_event_json(e: &SceneEvent) -> Value {
    json!({
        "x": e.x, "y": e.y, "z": e.z, "top_y": e.top_y,
        "kind": e.kind, "kind_name": event_kind::name(e.kind),
        "props": props_json(&e.props),
    })
}

fn scene_event_from(v: &Value) -> Result<SceneEvent, String> {
    Ok(SceneEvent {
        x: num(v, "x") as f32,
        y: num(v, "y") as f32,
        z: num(v, "z") as f32,
        top_y: num(v, "top_y") as f32,
        kind: uint(v, "kind") as u32,
        props: props_from(v, &["x", "y", "z", "top_y", "kind", "kind_name"]),
    })
}

fn usd_load_json(u: &UsdLoad) -> Value {
    json!({
        "x": u.x, "y": u.y, "z": u.z, "yaw_deg": u.yaw_deg,
        "category": u.category, "category_name": category::name(u.category),
        "flags": u.flags, "props": props_json(&u.props),
    })
}

fn usd_load_from(v: &Value) -> Result<UsdLoad, String> {
    let cat = match v.get("category") {
        Some(Value::String(s)) => {
            category::parse(s).ok_or_else(|| format!("unknown category `{s}`"))?
        }
        _ => uint(v, "category") as u32,
    };
    Ok(UsdLoad {
        x: num(v, "x") as f32,
        y: num(v, "y") as f32,
        z: num(v, "z") as f32,
        yaw_deg: num(v, "yaw_deg") as f32,
        category: cat,
        flags: uint(v, "flags") as u32,
        props: props_from(
            v,
            &[
                "x",
                "y",
                "z",
                "yaw_deg",
                "category",
                "category_name",
                "flags",
            ],
        ),
    })
}

fn usd_ref_json(r: &UsdRef) -> Value {
    json!({ "props": props_json(&r.props) })
}

fn usd_ref_from(v: &Value) -> Result<UsdRef, String> {
    Ok(UsdRef {
        props: props_from(v, &[]),
    })
}

fn marker_set_json(m: &MarkerSet) -> Value {
    json!({ "x": m.x, "y": m.y, "z": m.z, "props": props_json(&m.props) })
}

fn marker_set_from(v: &Value) -> Result<MarkerSet, String> {
    Ok(MarkerSet {
        x: num(v, "x") as f32,
        y: num(v, "y") as f32,
        z: num(v, "z") as f32,
        _pad: 0.0,
        props: props_from(v, &["x", "y", "z"]),
    })
}

fn marker_ref_json(r: &MarkerRef) -> Value {
    json!({ "props": props_json(&r.props) })
}

fn marker_ref_from(v: &Value) -> Result<MarkerRef, String> {
    Ok(MarkerRef {
        props: props_from(v, &[]),
    })
}

fn selection_json(s: &Selection) -> Value {
    json!({
        "kind": s.kind, "kind_name": object_kind::name(s.kind),
        "query": s.query != 0, "props": props_json(&s.props),
    })
}

fn selection_from(v: &Value) -> Result<Selection, String> {
    let kind = match v.get("kind") {
        Some(Value::String(s)) => {
            object_kind::parse(s).ok_or_else(|| format!("unknown object kind `{s}`"))?
        }
        _ => uint(v, "kind") as u32,
    };
    Ok(Selection {
        kind,
        query: uint(v, "query") as u32,
        props: props_from(v, &["kind", "kind_name", "query"]),
    })
}

fn machine_ref_json(m: &MachineRef) -> Value {
    json!({ "props": props_json(&m.props) })
}

fn machine_ref_from(v: &Value) -> Result<MachineRef, String> {
    Ok(MachineRef {
        props: props_from(v, &[]),
    })
}

fn machine_info_json(m: &MachineInfo) -> Value {
    json!({ "controller_count": m.controller_count, "held": m.held != 0, "props": props_json(&m.props) })
}

fn machine_info_from(v: &Value) -> Result<MachineInfo, String> {
    Ok(MachineInfo {
        controller_count: uint(v, "controller_count") as u32,
        held: uint(v, "held") as u32,
        props: props_from(v, &["controller_count", "held"]),
    })
}

fn claim_request_json(c: &ClaimRequest) -> Value {
    json!({ "take": c.take != 0, "hold_ms": c.hold_ms, "props": props_json(&c.props) })
}

fn claim_request_from(v: &Value) -> Result<ClaimRequest, String> {
    Ok(ClaimRequest {
        take: uint(v, "take") as u32,
        hold_ms: uint(v, "hold_ms") as u32,
        props: props_from(v, &["take", "hold_ms"]),
    })
}

fn claim_response_json(c: &ClaimResponse) -> Value {
    json!({ "session": c.session, "code": c.code, "props": props_json(&c.props) })
}

fn claim_response_from(v: &Value) -> Result<ClaimResponse, String> {
    Ok(ClaimResponse {
        session: uint(v, "session"),
        code: uint(v, "code") as u32,
        _pad: 0,
        props: props_from(v, &["session", "code"]),
    })
}

fn session_ref_json(s: &SessionRef) -> Value {
    json!({ "session": s.session })
}

fn session_ref_from(v: &Value) -> Result<SessionRef, String> {
    Ok(SessionRef {
        session: uint(v, "session"),
    })
}

fn session_info_json(s: &SessionInfo) -> Value {
    json!({
        "session": s.session, "age_ms": s.age_ms, "idle_ms": s.idle_ms,
        "held": s.held != 0, "props": props_json(&s.props),
    })
}

fn session_info_from(v: &Value) -> Result<SessionInfo, String> {
    Ok(SessionInfo {
        session: uint(v, "session"),
        age_ms: uint(v, "age_ms"),
        idle_ms: uint(v, "idle_ms"),
        held: uint(v, "held") as u32,
        _pad: 0,
        props: props_from(v, &["session", "age_ms", "idle_ms", "held"]),
    })
}

fn twist_cmd_json(t: &TwistCmd) -> Value {
    json!({ "session": t.session, "twist": twist_json(&t.twist) })
}

fn twist_cmd_from(v: &Value) -> Result<TwistCmd, String> {
    // `forward` and `turn` are the short form; a full `twist` object wins.
    let twist = match v.get("twist") {
        Some(t) => twist_from(t),
        None => Twist::from_components(num(v, "forward"), 0.0, 0.0, 0.0, 0.0, num(v, "turn")),
    };
    Ok(TwistCmd {
        session: uint(v, "session"),
        twist,
    })
}

fn controller_command_json(c: &ControllerCommand) -> Value {
    json!({ "session": c.session, "value": c.value, "element": c.element, "props": props_json(&c.props) })
}

fn controller_command_from(v: &Value) -> Result<ControllerCommand, String> {
    Ok(ControllerCommand {
        session: uint(v, "session"),
        value: num(v, "value"),
        element: uint(v, "element") as u32,
        _pad: 0,
        props: props_from(v, &["session", "value", "element"]),
    })
}

fn machine_state_json(s: &MachineState) -> Value {
    json!({
        "odom": odom_json(&s.odom),
        "heading_rad": s.heading_rad, "roll_rad": s.roll_rad, "pitch_rad": s.pitch_rad,
        "session": s.session, "props": props_json(&s.props),
    })
}

fn machine_state_from(v: &Value) -> Result<MachineState, String> {
    Ok(MachineState {
        odom: odom_from(&v.get("odom").cloned().unwrap_or(Value::Null)),
        heading_rad: num(v, "heading_rad"),
        roll_rad: num(v, "roll_rad"),
        pitch_rad: num(v, "pitch_rad"),
        session: uint(v, "session"),
        props: props_from(
            v,
            &["odom", "heading_rad", "roll_rad", "pitch_rad", "session"],
        ),
    })
}
