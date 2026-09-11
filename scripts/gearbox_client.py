"""Python client for a running Gearbox host and its machines, over peerbus.

The host writes a registry entry under ``$XDG_RUNTIME_DIR/gearbox/`` with its
endpoint address. Machines are found through the host's ``/gearbox/machines/list``
answer, which carries each machine agent's address. Every topic carries a
type-erased datapod envelope, so the classes below mirror the Rust wire types
byte for byte and share their canonical names.

    from gearbox_client import Gearbox
    gb = Gearbox()                       # first live instance in the registry
    gb.load("flatland_terrain", "world/flatland.usd", category="terrain")
    gb.load_machine("oxbo", "bin/gearbox/assets/oxbo.usd", x=0, z=0)
    m = gb.machine("oxbo")               # waits for the agent to appear
    session = m.claim()
    m.cmd_vel(session, 1.0, 0.2)
    print(m.state())
"""

from __future__ import annotations

import json
import os
import struct
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import datapod
import peerbus

# ─── Registry ────────────────────────────────────────────────────────


def registry_dir() -> Path:
    override = os.environ.get("GEARBOX_REGISTRY_DIR")
    if override:
        return Path(override)
    base = os.environ.get("XDG_RUNTIME_DIR")
    if base:
        return Path(base) / "gearbox"
    return Path("/tmp") / f"gearbox-{os.environ.get('USER', 'user')}" / "gearbox"


@dataclass
class Instance:
    name: str
    did: str
    addr: str
    pid: int
    version: str = ""

    @staticmethod
    def list() -> list["Instance"]:
        out = []
        d = registry_dir()
        if not d.is_dir():
            return out
        for path in sorted(d.glob("*.json")):
            try:
                raw = json.loads(path.read_text())
                inst = Instance(raw["name"], raw["did"], raw["addr"], int(raw["pid"]), raw.get("version", ""))
            except (OSError, ValueError, KeyError):
                continue
            if not Path("/proc").joinpath(str(inst.pid)).exists():
                continue
            out.append(inst)
        return out

    @staticmethod
    def find(name: str | None = None) -> "Instance":
        wanted = name or os.environ.get("GEARBOX_INSTANCE")
        entries = Instance.list()
        for e in entries:
            if wanted is None or e.name == wanted or e.did == wanted:
                return e
        raise RuntimeError(
            f"no running gearbox instance{' named ' + wanted if wanted else ''} in {registry_dir()}"
        )


# ─── Map payload (datapod::Map wire layout) ──────────────────────────


def pack_map(pairs: dict[str, object]) -> bytes:
    items = sorted((str(k).encode(), str(v).encode()) for k, v in pairs.items())
    entries = bytearray()
    blob = bytearray()
    for k, v in items:
        entries += struct.pack("<IIII", len(blob), len(k), len(blob) + len(k), len(v))
        blob += k + v
    return struct.pack("<I", len(items)) + bytes(entries) + bytes(blob)


def unpack_map(data: bytes) -> dict[str, str]:
    if len(data) < 4:
        return {}
    (count,) = struct.unpack_from("<I", data, 0)
    blob_off = 4 + 16 * count
    out = {}
    for i in range(count):
        ko, kl, vo, vl = struct.unpack_from("<IIII", data, 4 + 16 * i)
        k = data[blob_off + ko : blob_off + ko + kl]
        v = data[blob_off + vo : blob_off + vo + vl]
        out[k.decode(errors="replace")] = v.decode(errors="replace")
    return out


# ─── Wire types (mirror crates/gearbox-api/src/wire) ─────────────────


def _pod(name: str, fmt: str, fields: tuple[str, ...], payload: str | None = None):
    def decorate(cls):
        kwargs = {"dataclass": True}
        if payload:
            kwargs["payload_field"] = payload
        return datapod.datapod_type(name, fmt, fields, **kwargs)(cls)

    return decorate


@_pod("gearbox.ping.v1", "<Q", ("nonce",))
class Ping:
    nonce: int = 0


@_pod("gearbox.status.v1", "<I", ("code",), payload="detail")
class Status:
    code: int = 0
    detail: bytes = b""

    @property
    def ok(self) -> bool:
        return self.code == 0

    @property
    def props(self) -> dict[str, str]:
        return unpack_map(self.detail)

    @property
    def message(self) -> str:
        return self.props.get("message", "")


@_pod("gearbox.host_info.v1", "<QIIII", ("uptime_ms", "pid", "paused", "machine_count", "object_count"), payload="props")
class HostInfo:
    uptime_ms: int = 0
    pid: int = 0
    paused: int = 0
    machine_count: int = 0
    object_count: int = 0
    props: bytes = b""


@_pod("gearbox.clock_command.v1", "<II", ("op", "steps"))
class ClockCommand:
    op: int = 0
    steps: int = 0


@_pod("gearbox.clock_state.v1", "<QII", ("step", "paused", "_pad"))
class ClockState:
    step: int = 0
    paused: int = 0
    _pad: int = 0


@_pod("gearbox.clear_request.v1", "<II", ("scope", "pause_clock"))
class ClearRequest:
    scope: int = 0
    pause_clock: int = 0


@_pod("gearbox.list_query.v1", "<I", ("kind",))
class ListQuery:
    kind: int = 0


@_pod("gearbox.scene_object.v1", "<ffffI", ("x", "y", "z", "yaw_deg", "kind"), payload="props")
class SceneObject:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    yaw_deg: float = 0.0
    kind: int = 0
    props: bytes = b""


@_pod("gearbox.scene_event.v1", "<ffffI", ("x", "y", "z", "top_y", "kind"), payload="props")
class SceneEvent:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    top_y: float = 0.0
    kind: int = 0
    props: bytes = b""


@_pod("gearbox.usd_load.v1", "<ffffII", ("x", "y", "z", "yaw_deg", "category", "flags"), payload="props")
class UsdLoad:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    yaw_deg: float = 0.0
    category: int = 0
    flags: int = 0
    props: bytes = b""


@_pod("gearbox.usd_ref.v1", "<", (), payload="props")
class UsdRef:
    props: bytes = b""


@_pod("gearbox.marker_set.v1", "<ffff", ("x", "y", "z", "_pad"), payload="props")
class MarkerSet:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    _pad: float = 0.0
    props: bytes = b""


@_pod("gearbox.marker_ref.v1", "<", (), payload="props")
class MarkerRef:
    props: bytes = b""


@_pod("gearbox.selection.v1", "<II", ("kind", "query"), payload="props")
class Selection:
    kind: int = 0
    query: int = 0
    props: bytes = b""


@_pod("gearbox.machine_ref.v1", "<", (), payload="props")
class MachineRef:
    props: bytes = b""


@_pod("gearbox.machine_info.v1", "<II", ("controller_count", "held"), payload="props")
class MachineInfo:
    controller_count: int = 0
    held: int = 0
    props: bytes = b""


@_pod("gearbox.claim_request.v1", "<II", ("take", "hold_ms"), payload="props")
class ClaimRequest:
    take: int = 0
    hold_ms: int = 500
    props: bytes = b""


@_pod("gearbox.claim_response.v1", "<QII", ("session", "code", "_pad"), payload="props")
class ClaimResponse:
    session: int = 0
    code: int = 0
    _pad: int = 0
    props: bytes = b""


@_pod("gearbox.session_ref.v1", "<Q", ("session",))
class SessionRef:
    session: int = 0


@_pod("gearbox.session_info.v1", "<QQQII", ("session", "age_ms", "idle_ms", "held", "_pad"), payload="props")
class SessionInfo:
    session: int = 0
    age_ms: int = 0
    idle_ms: int = 0
    held: int = 0
    _pad: int = 0
    props: bytes = b""


@_pod("gearbox.twist_cmd.v1", "<Qdddddd", ("session", "vx", "vy", "vz", "wx", "wy", "wz"))
class TwistCmd:
    session: int = 0
    vx: float = 0.0
    vy: float = 0.0
    vz: float = 0.0
    wx: float = 0.0
    wy: float = 0.0
    wz: float = 0.0


@_pod("gearbox.controller_command.v1", "<QdII", ("session", "value", "element", "_pad"), payload="props")
class ControllerCommand:
    session: int = 0
    value: float = 0.0
    element: int = 0
    _pad: int = 0
    props: bytes = b""


_STATE_FIELDS = (
    "x", "y", "z", "qw", "qx", "qy", "qz",
    "vx", "vy", "vz", "wx", "wy", "wz",
    "heading_rad", "roll_rad", "pitch_rad", "session",
)


@_pod("gearbox.machine_state.v1", "<" + "d" * 16 + "Q", _STATE_FIELDS, payload="props")
class MachineState:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    qw: float = 1.0
    qx: float = 0.0
    qy: float = 0.0
    qz: float = 0.0
    vx: float = 0.0
    vy: float = 0.0
    vz: float = 0.0
    wx: float = 0.0
    wy: float = 0.0
    wz: float = 0.0
    heading_rad: float = 0.0
    roll_rad: float = 0.0
    pitch_rad: float = 0.0
    session: int = 0
    props: bytes = b""

    @property
    def position(self) -> tuple[float, float, float]:
        return (self.x, self.y, self.z)

    @property
    def linear_speed(self) -> float:
        return (self.vx * self.vx + self.vy * self.vy + self.vz * self.vz) ** 0.5

    @property
    def yaw_rate(self) -> float:
        return self.wz


CATEGORY = {"static": 0, "machine": 1, "robot": 1, "variant": 2, "world": 3, "terrain": 4}
CLOCK_OP = {"get": 0, "pause": 1, "play": 2, "toggle": 3, "shutdown": 4}
@_pod("gearbox.link_record.v1", "<" + "d" * 7 + "II", ("x", "y", "z", "qw", "qx", "qy", "qz", "index", "parent_index"), payload="props")
class LinkRecord:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    qw: float = 1.0
    qx: float = 0.0
    qy: float = 0.0
    qz: float = 0.0
    index: int = 0
    parent_index: int = 0xFFFFFFFF
    props: bytes = b""


@_pod("gearbox.link_pose.v1", "<" + "d" * 7 + "II", ("x", "y", "z", "qw", "qx", "qy", "qz", "index", "stamp_ms"), payload="props")
class LinkPose:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    qw: float = 1.0
    qx: float = 0.0
    qy: float = 0.0
    qz: float = 0.0
    index: int = 0
    stamp_ms: int = 0
    props: bytes = b""


@_pod("gearbox.attach_request.v1", "<QII", ("session", "teleport", "_pad"), payload="props")
class AttachRequest:
    session: int = 0
    teleport: int = 0
    _pad: int = 0
    props: bytes = b""


@_pod("gearbox.detach_request.v1", "<Q", ("session",), payload="props")
class DetachRequest:
    session: int = 0
    props: bytes = b""


@_pod("gearbox.attachment.v1", "<II", ("controlled", "depth"), payload="props")
class AttachmentRecord:
    controlled: int = 0
    depth: int = 0
    props: bytes = b""


CLEAR_SCOPE = {"all": 0, "machines": 1, "props": 2, "markers": 3}
OBJECT_KIND = {"any": 0, "machine": 1, "prop": 2, "marker": 3, "terrain": 4}
EVENT_KIND = {0: "loaded", 1: "pose", 2: "harvested", 3: "removed", 4: "machine_ready"}
LOAD_REMOVE = 1
LOAD_DELETE = 2
CODE_REFUSED = 4
CODE_BUSY = 6

HOST = {
    "info": "/gearbox/info",
    "clock": "/gearbox/scene/clock",
    "clock_state": "/gearbox/scene/clock/state",
    "clear": "/gearbox/scene/clear",
    "list": "/gearbox/scene/list",
    "events": "/gearbox/scene/events",
    "usd_load": "/gearbox/usd/load",
    "usd_delete": "/gearbox/usd/delete",
    "marker_set": "/gearbox/marker/set",
    "marker_delete": "/gearbox/marker/delete",
    "select": "/gearbox/select",
    "machines": "/gearbox/machines/list",
}


def machine_topic(namespace: str, leaf: str) -> str:
    return f"/machines/{namespace}/{leaf}"


# ─── Client ──────────────────────────────────────────────────────────


class Gearbox:
    """One peerbus node talking to one host and any of its machines."""

    def __init__(self, instance: str | None = None, addr: str | None = None, node: peerbus.Node | None = None):
        if addr is None:
            self.instance = Instance.find(instance)
            addr = self.instance.addr
        else:
            self.instance = None
        self.addr = addr
        self.node = node or peerbus.Node(no_relay=True)
        self._req: dict[tuple[str, str], object] = {}
        self._machines: dict[str, Machine] = {}

    def _client(self, peer: str, topic: str):
        key = (peer, topic)
        if key not in self._req:
            self._req[key] = self.node.datapod_req_client(peer, topic)
        return self._req[key]

    def call(self, peer: str, topic: str, request, response_type, retries: int = 30):
        last = None
        for _ in range(retries):
            try:
                return self._client(peer, topic).call_decode(request, response_type)
            except Exception as err:  # noqa: BLE001 - transport errors are retried
                last = err
                self._req.pop((peer, topic), None)
                time.sleep(0.1)
        raise RuntimeError(f"{topic}: {last}")

    def query(self, peer: str, topic: str, request, answer_type) -> list:
        return self.node.datapod_que_client(peer, topic).send_decode(request, answer_type)

    def subscribe(self, peer: str, topic: str, latest: bool = False):
        """Latest-wins delivery for telemetry: a slow reader gets the newest
        sample instead of a lag error."""
        if latest:
            try:
                return self.node.datapod_subscriber(peer, topic, qos=peerbus.TopicQos.latest())
            except TypeError:
                pass
        return self.node.datapod_subscriber(peer, topic)

    # host

    def info(self) -> HostInfo:
        return self.call(self.addr, HOST["info"], Ping(), HostInfo)

    def wait_ready(self, timeout: float = 10.0) -> HostInfo:
        deadline = time.time() + timeout
        while True:
            try:
                return self.info()
            except RuntimeError:
                if time.time() > deadline:
                    raise
                time.sleep(0.2)

    def clock(self, op: str = "get") -> ClockState:
        return self.call(self.addr, HOST["clock"], ClockCommand(op=CLOCK_OP[op]), ClockState)

    def play(self) -> ClockState:
        return self.clock("play")

    def pause(self) -> ClockState:
        return self.clock("pause")

    def clear(self, scope: str = "all", pause_clock: bool = False) -> Status:
        req = ClearRequest(scope=CLEAR_SCOPE[scope], pause_clock=int(pause_clock))
        status = self.call(self.addr, HOST["clear"], req, Status)
        self._machines.clear()
        return status

    def list(self, kind: str = "any") -> list[dict]:
        out = []
        for o in self.query(self.addr, HOST["list"], ListQuery(kind=OBJECT_KIND[kind]), SceneObject):
            d = unpack_map(o.props)
            d.update({"x": o.x, "y": o.y, "z": o.z, "yaw_deg": o.yaw_deg, "kind": o.kind})
            out.append(d)
        return out

    def load(
        self,
        id: str,
        path: str,
        *,
        x: float = 0.0,
        y: float = 0.0,
        z: float = 0.0,
        yaw_deg: float = 0.0,
        category: str = "static",
        namespace: str | None = None,
        label: str | None = None,
        nonce: str | None = None,
        variants: list[tuple[str, str, str]] | None = None,
    ) -> Status:
        props = {"id": id, "path": path}
        if namespace:
            props["namespace"] = namespace
        if label:
            props["label"] = label
        if nonce:
            props["nonce"] = nonce
        for i, (prim, vset, opt) in enumerate(variants or []):
            props[f"variant.{i}"] = f"{prim}|{vset}|{opt}"
        req = UsdLoad(x=x, y=y, z=z, yaw_deg=yaw_deg, category=CATEGORY[category], flags=0, props=pack_map(props))
        return self.call(self.addr, HOST["usd_load"], req, Status)

    def load_machine(self, namespace: str, path: str, **kw) -> Status:
        return self.load(namespace, path, category="machine", namespace=namespace, **kw)

    def remove(self, id: str, nonce: str | None = None) -> Status:
        props = {"id": id}
        if nonce:
            props["nonce"] = nonce
        req = UsdLoad(flags=LOAD_REMOVE, props=pack_map(props))
        return self.call(self.addr, HOST["usd_load"], req, Status)

    def delete(self, id: str) -> Status:
        return self.call(self.addr, HOST["usd_delete"], UsdRef(props=pack_map({"id": id})), Status)

    def marker_set(self, id: str, x: float, y: float, z: float) -> Status:
        req = MarkerSet(x=x, y=y, z=z, props=pack_map({"id": id}))
        return self.call(self.addr, HOST["marker_set"], req, Status)

    def marker_delete(self, id: str) -> Status:
        return self.call(self.addr, HOST["marker_delete"], MarkerRef(props=pack_map({"id": id})), Status)

    def select(self, kind: str, id: str) -> dict:
        req = Selection(kind=OBJECT_KIND[kind], query=0, props=pack_map({"id": id}))
        res = self.call(self.addr, HOST["select"], req, Selection)
        return unpack_map(res.props)

    def machines(self) -> list[dict[str, str]]:
        return [unpack_map(m.props) for m in self.query(self.addr, HOST["machines"], Ping(), MachineRef)]

    def events(self) -> "EventStream":
        return EventStream(self.subscribe(self.addr, HOST["events"]))

    def machine(self, namespace: str, timeout: float = 30.0) -> "Machine":
        if namespace in self._machines:
            return self._machines[namespace]
        deadline = time.time() + timeout
        while True:
            for m in self.machines():
                if m.get("namespace") == namespace and m.get("addr"):
                    machine = Machine(self, namespace, m["addr"], m.get("did", ""))
                    self._machines[namespace] = machine
                    return machine
            if time.time() > deadline:
                raise RuntimeError(f"machine `{namespace}` did not appear within {timeout:.0f}s")
            time.sleep(0.2)


class EventStream:
    """Non-blocking reader over `/gearbox/scene/events`."""

    def __init__(self, sub):
        self._sub = sub

    def poll(self) -> list[dict]:
        out = []
        while True:
            try:
                ev = self._sub.take(SceneEvent)
            except RuntimeError as err:
                # The ring wrapped past us; the reader has already skipped
                # ahead, so report it and keep draining.
                if "lagged" not in str(err):
                    raise
                print(f"WARN: scene events {err}", file=sys.stderr)
                continue
            if ev is None:
                return out
            d = unpack_map(ev.props)
            d.update({"kind": EVENT_KIND.get(ev.kind, str(ev.kind)), "x": ev.x, "y": ev.y, "z": ev.z, "top_y": ev.top_y})
            out.append(d)


class Machine:
    """A machine agent: claim it, stream twists, read its state."""

    def __init__(self, gb: Gearbox, namespace: str, addr: str, did: str):
        self.gb = gb
        self.namespace = namespace
        self.addr = addr
        self.did = did
        self.session = 0
        self._hold_ms = 500
        self._client = ""
        self._state_sub = None
        self._last_state: MachineState | None = None

    def _topic(self, leaf: str) -> str:
        return machine_topic(self.namespace, leaf)

    def info(self) -> dict[str, str]:
        info = self.gb.call(self.addr, self._topic("info"), Ping(), MachineInfo)
        d = unpack_map(info.props)
        d["controller_count"] = str(info.controller_count)
        d["held"] = str(info.held)
        return d

    def implements(self, interface: str = "cmd_vel") -> bool:
        info = self.info()
        n = int(info.get("controller_count", "0"))
        return any(info.get(f"controller.{i}.command_interface") == interface for i in range(n))

    def claim(self, hold_ms: int = 500, take: bool = False, client: str = "") -> int:
        req = ClaimRequest(take=int(take), hold_ms=hold_ms, props=pack_map({"client": client or self.gb.node.endpoint_addr()[:16]}))
        res = self.gb.call(self.addr, self._topic("claim"), req, ClaimResponse)
        if res.code != 0:
            holder = unpack_map(res.props).get("holder", "?")
            raise RuntimeError(f"machine `{self.namespace}` is held by {holder}")
        self.session = res.session
        self._hold_ms = hold_ms
        self._client = client
        return self.session

    def release(self) -> Status:
        status = self.gb.call(self.addr, self._topic("release"), SessionRef(session=self.session), Status)
        self.session = 0
        return status

    def session_info(self) -> dict[str, str]:
        info = self.gb.call(self.addr, self._topic("session"), Ping(), SessionInfo)
        d = unpack_map(info.props)
        d.update({"session": str(info.session), "held": str(info.held), "idle_ms": str(info.idle_ms)})
        return d

    def cmd_vel(self, forward_mps: float, yaw_rps: float, session: int | None = None) -> Status:
        sid = self.session if session is None else session
        req = TwistCmd(session=sid, vx=forward_mps, wz=yaw_rps)
        status = self.gb.call(self.addr, self._topic("cmd_vel"), req, Status)
        if status.ok or session is not None or self.session == 0:
            return status
        # Our session lapsed (agent restarted, or we were silent too long):
        # claim again and resend. Someone else holding the machine raises.
        if status.code in (CODE_REFUSED, CODE_BUSY):
            self.claim(hold_ms=self._hold_ms, client=self._client)
            req = TwistCmd(session=self.session, vx=forward_mps, wz=yaw_rps)
            status = self.gb.call(self.addr, self._topic("cmd_vel"), req, Status)
        return status

    def stop(self) -> Status:
        return self.cmd_vel(0.0, 0.0)

    def attach(self, slave: str, hitch: str | None = None, coupler: str | None = None,
               teleport: bool = False) -> Status:
        """Hang `slave` on one of this machine's hitches (uses our session if held)."""
        props = {"slave": slave}
        if hitch:
            props["hitch"] = hitch
        if coupler:
            props["coupler"] = coupler
        req = AttachRequest(session=self.session, teleport=int(teleport), props=pack_map(props))
        return self.gb.call(self.addr, self._topic("tools/attach"), req, Status)

    def detach(self, slave: str) -> Status:
        req = DetachRequest(session=self.session, props=pack_map({"slave": slave}))
        return self.gb.call(self.addr, self._topic("tools/detach"), req, Status)

    def tools(self) -> list[dict]:
        """Attachments below this machine, depth-first."""
        out = []
        for r in self.gb.query(self.addr, self._topic("tools"), Ping(), AttachmentRecord):
            d = unpack_map(r.props)
            d.update({"controlled": bool(r.controlled), "depth": r.depth})
            out.append(d)
        return out

    def tf(self, on: bool = True) -> Status:
        """Switch the machine's link pose stream on or off (no session needed)."""
        req = ControllerCommand(props=pack_map({"tf": "on" if on else "off"}))
        return self.gb.call(self.addr, self._topic("cmd"), req, Status)

    def tf_next(self, timeout: float = 1.0) -> dict | None:
        """Next link pose as a dict, or None when nothing arrived in time."""
        if getattr(self, "_tf_sub", None) is None:
            self._tf_sub = self.gb.subscribe(self.addr, self._topic("tf"))
        deadline = time.time() + timeout
        while True:
            try:
                p = self._tf_sub.take(LinkPose)
            except RuntimeError as err:
                if "lagged" in str(err):
                    continue
                raise
            if p is not None:
                d = unpack_map(p.props)
                d.update({"index": p.index, "stamp_ms": p.stamp_ms, "x": p.x, "y": p.y, "z": p.z,
                          "qw": p.qw, "qx": p.qx, "qy": p.qy, "qz": p.qz})
                return d
            if time.time() >= deadline:
                return None
            time.sleep(0.002)

    def links(self) -> list[dict]:
        """The link tree, base_link first: name, parent, role, prim, offset."""
        out = []
        for r in self.gb.query(self.addr, self._topic("links"), Ping(), LinkRecord):
            d = unpack_map(r.props)
            d.update({
                "index": r.index,
                "parent_index": None if r.parent_index == 0xFFFFFFFF else r.parent_index,
                "offset": {"x": r.x, "y": r.y, "z": r.z, "qw": r.qw, "qx": r.qx, "qy": r.qy, "qz": r.qz},
            })
            out.append(d)
        return out

    def state(self, wait: float = 0.0) -> MachineState | None:
        """Latest state sample, or the last one seen when none is pending."""
        if self._state_sub is None:
            self._state_sub = self.gb.subscribe(self.addr, self._topic("state"), latest=True)
        deadline = time.time() + wait
        while True:
            sample = self._take_state()
            if sample is not None:
                self._last_state = sample
                while True:
                    newer = self._take_state()
                    if newer is None:
                        break
                    self._last_state = newer
                return self._last_state
            if time.time() >= deadline:
                return self._last_state
            time.sleep(0.005)

    def _take_state(self) -> MachineState | None:
        try:
            return self._state_sub.take(MachineState)
        except RuntimeError as err:
            # The sim publishes every frame; falling behind is not an error
            # for a reader that only wants the newest pose.
            if "lagged" in str(err):
                return None
            raise
