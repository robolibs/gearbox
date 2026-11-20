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
    gb.draw.lines("demo", "path", [(0, 0), (5, 0), (5, 5)], frame="oxbo/odom", color="orange", width=0.1)
"""

from __future__ import annotations

import collections
import json
import math
import os
import struct
import sys
import threading
import time
from dataclasses import dataclass, field
from itertools import islice
from pathlib import Path

# The peerbus, datapod and ondrive wheels live in the repo's
# .python-packages; outside `nix develop` nothing puts them on the path.
_PACKAGES = Path(__file__).resolve().parents[2] / ".python-packages"
if _PACKAGES.is_dir() and str(_PACKAGES) not in sys.path:
    sys.path.insert(0, str(_PACKAGES))

import datapod  # noqa: E402
import peerbus  # noqa: E402

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


@_pod("gearbox.clock_command.v1", "<IId", ("op", "steps", "value"))
class ClockCommand:
    op: int = 0
    steps: int = 0
    value: float = 0.0


@_pod("gearbox.clock_state.v1", "<QIIdddddII", ("step", "paused", "_pad", "world_s", "rate", "day_s", "skip_s", "hour", "day", "_pad2"))
class ClockState:
    step: int = 0
    paused: int = 0
    _pad: int = 0
    world_s: float = 0.0
    rate: float = 1.0
    day_s: float = 0.0
    skip_s: float = 0.0
    hour: float = 0.0
    day: int = 0
    _pad2: int = 0


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


@_pod("gearbox.fields_layout.v1", "<", (), payload="layout")
class FieldsLayout:
    layout: bytes = b""


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
CLOCK_OP = {"get": 0, "pause": 1, "play": 2, "toggle": 3, "shutdown": 4, "step": 5, "speed": 6, "skip": 7}
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


@_pod("gearbox.greenhouse_query.v1", "<II", ("ripe", "_pad"), payload="props")
class GreenhouseQuery:
    ripe: int = 0
    _pad: int = 0
    props: bytes = b""


@_pod("gearbox.plant.v1", "<ffffffII", ("x", "y", "z", "maturity", "health", "height_m", "alive", "ripe"), payload="props")
class PlantRecord:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    maturity: float = 0.0
    health: float = 0.0
    height_m: float = 0.0
    alive: int = 0
    ripe: int = 0
    props: bytes = b""


@_pod("gearbox.greenhouse_room.v1", "<ffffff", ("temperature_c", "humidity", "co2_ppm", "o2_kpa", "pressure_kpa", "lux"), payload="props")
class RoomRecord:
    temperature_c: float = 0.0
    humidity: float = 0.0
    co2_ppm: float = 0.0
    o2_kpa: float = 0.0
    pressure_kpa: float = 0.0
    lux: float = 0.0
    props: bytes = b""


@_pod("gearbox.greenhouse_action.v1", "<II", ("op", "_pad"), payload="props")
class GreenhouseAction:
    op: int = 0
    _pad: int = 0
    props: bytes = b""


GREENHOUSE_OP = {
    "sow": 0, "harvest": 1, "clear": 2, "wash": 3, "amend": 4, "lamps": 5, "clean_tank": 6,
    "fault": 7, "repair": 8, "setpoint": 9, "clean_panels": 10, "clean_glazing": 11,
    "pollinate": 12, "lift": 13, "place": 14,
}
CLEAR_SCOPE = {"all": 0, "machines": 1, "props": 2, "markers": 3}
OBJECT_KIND = {"any": 0, "machine": 1, "prop": 2, "marker": 3, "terrain": 4}
EVENT_KIND = {
    0: "loaded", 1: "pose", 2: "harvested", 3: "removed", 4: "machine_ready",
    5: "attached", 6: "detached", 7: "machine_rejected", 8: "greenhouse", 9: "draw",
}
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
    "draw": "/gearbox/draw",
    "draw_list": "/gearbox/draw/list",
    "select": "/gearbox/select",
    "machines": "/gearbox/machines/list",
    "fields_layout": "/gearbox/fields/layout",
    "plants": "/gearbox/greenhouse/plants",
    "rooms": "/gearbox/greenhouse/rooms",
    "greenhouse": "/gearbox/greenhouse/act",
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
        self.draw = Draw(self)

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

    def set_fields(self, layout: dict) -> Status:
        """Lay the ground at home out anew: a field layout as a
        `GEARBOX_FIELD_LAYOUT` file holds it, `{"default": ..., "fields": [...]}`."""
        return self.call(self.addr, HOST["fields_layout"], FieldsLayout(layout=json.dumps(layout).encode()), Status)

    def home(self) -> tuple[float, float] | None:
        """Latitude and longitude of home, what x, y, z of a load are measured from."""
        text = unpack_map(self.info().props).get("home", "")
        parts = text.split()
        return (float(parts[0]), float(parts[1])) if len(parts) == 2 else None

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
        extra: dict[str, object] | None = None,
    ) -> Status:
        """x, y, z are metres north, up and east of home. `extra` carries more
        props, e.g. `lat`/`lon` to place on Earth or `datum_lat`/`datum_lon`
        for the frame a machine reports its pose in."""
        props = {"id": id, "path": path, **{k: str(v) for k, v in (extra or {}).items()}}
        if namespace:
            props["machine_id"] = namespace
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

    # greenhouse

    def plants(self, ripe: bool = False, tray: str | None = None, species: str | None = None) -> list[dict]:
        """Every plant (or the ripe ones), each a dict of its record and props;
        x, y, z are in the home frame (x north, y up, z east)."""
        keys = {k: v for k, v in (("tray", tray), ("species", species)) if v}
        out = []
        for p in self.query(self.addr, HOST["plants"], GreenhouseQuery(ripe=int(ripe), props=pack_map(keys)), PlantRecord):
            d = unpack_map(p.props)
            d.update({"x": p.x, "y": p.y, "z": p.z, "maturity": p.maturity, "health": p.health,
                      "height_m": p.height_m, "alive": bool(p.alive), "ripe": bool(p.ripe)})
            out.append(d)
        return out

    def rooms(self) -> list[dict]:
        out = []
        for r in self.query(self.addr, HOST["rooms"], Ping(), RoomRecord):
            d = unpack_map(r.props)
            d.update({"temperature_c": r.temperature_c, "humidity": r.humidity, "co2_ppm": r.co2_ppm,
                      "o2_kpa": r.o2_kpa, "pressure_kpa": r.pressure_kpa, "lux": r.lux})
            out.append(d)
        return out

    def greenhouse(self, op: str, **keys) -> Status:
        """A greenhouse action (`sow`, `harvest`, `clean_glazing`, ...), its keys as
        keyword arguments: `gb.greenhouse("harvest", tray="TrayA3", by="base_fleet_archabot")`."""
        req = GreenhouseAction(op=GREENHOUSE_OP[op], props=pack_map({k: v for k, v in keys.items() if v is not None}))
        return self.call(self.addr, HOST["greenhouse"], req, Status)

    def machine(self, namespace: str, timeout: float = 30.0, listing: list[dict[str, str]] | None = None) -> "Machine":
        """The machine `namespace`, looked up in `listing` (a `machines()` read
        another client made) before asking the host."""
        if namespace in self._machines:
            return self._machines[namespace]
        deadline = time.time() + timeout
        while True:
            for m in listing if listing is not None else self.machines():
                if m.get("machine_id") == namespace and m.get("addr"):
                    machine = Machine(self, namespace, m["addr"], m.get("did", ""))
                    self._machines[namespace] = machine
                    return machine
            if time.time() > deadline:
                raise RuntimeError(f"machine `{namespace}` did not appear within {timeout:.0f}s")
            listing = None
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
            master = status.props.get("attached_to")
            if master:
                raise RuntimeError(
                    f"machine `{self.namespace}` is attached to `{master}`; command it through its master"
                )
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

    def set_value(self, link: str, name: str, value: float) -> Status:
        """Set a named value on a link of the tree (`position`, `SetpointWorkState`, ...).

        The controller driving the joint that moves `link` reads the link's
        values, so `set_value("boom", "position", 1.0)` moves the boom.
        """
        req = ControllerCommand(session=self.session, value=float(value),
                                props=pack_map({"link": link, "name": name}))
        return self.gb.call(self.addr, self._topic("cmd"), req, Status)

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


# ─── Debug draws (mirror crates/gearbox-api/src/wire/draw.rs) ────────

DRAW_OP = {"upsert": 0, "append": 1, "delete": 2, "clear": 3, "extend": 4, "update": 5, "patch": 6}
DRAW_KIND = {"lines": 1, "points": 2, "arrows": 3, "shapes": 4, "triangles": 5, "text": 6, "axes": 7,
             "polygon": 8, "grid": 9, "ghost": 10, "panel": 11, "mesh": 12, "interact": 13}
LINE_MODE = {"strip": 0, "loop": 1, "list": 2}
POINT_STYLE = {"sphere": 0, "disc": 1, "square": 2}
ARROW_STYLE = {"arrow": 0, "chevron": 1}
SHAPE = {"box": 0, "cube": 0, "ellipsoid": 1, "sphere": 1, "cylinder": 2, "cone": 3, "capsule": 4,
         "ellipse": 5, "disc": 5, "circle": 5, "rect": 6, "plane": 6, "frustum": 7, "sector": 8, "wedge": 8}
FILL = {"solid": 0, "fill": 0, "filler": 0, "wire": 1, "outline": 1, "both": 2}
GRID_ENCODING = {"u8": 0, "u16": 1, "f32": 2}
PANEL = {"sparkline": 0, "bars": 1, "gauge": 2, "chip": 3, "table": 4, "legend": 5}
INTERACT = {"step": 0, "pick_point": 1, "pick_pose": 2, "handle": 3, "pick_data": 4}
COLORMAP = {name: i for i, name in enumerate((
    "none", "turbo", "viridis", "rdylgn", "costmap", "map", "raw", "okabe", "rainbow", "grey"))}
DRAW_FLAG = {"on_top": 1 << 2, "labels": 1 << 3, "flat_caps": 1 << 4, "dashed": 1 << 5,
             "snapshot": 1 << 6, "shaded": 1 << 7, "flat": 1 << 8, "additive": 1 << 9, "drape": 1 << 10,
             "arrow_tip": 1 << 11, "view_only": 1 << 12}
RING_MARKS = {"hover": 0x50ECE8DC, "selected": 0x8CECE8DC, "following": 0x8CECE8DC}
RING_MIN_RADIUS = 0.5
DRAW_STYLE = {name: i for i, name in enumerate((
    "auto", "plain", "ribbon_flow", "tube_pulse", "hairline", "light_wall", "beacon", "holo", "shield", "alert", "orb",
    "decal", "arrow_glow", "point_glow", "label", "heat", "splat", "sweep", "ghost", "ruler", "field", "ring"))}
TERRAIN_GLYPHS = ("square", "cross", "triangle", "diamond")
_WORLD_MAX_POINTS = 65_536
_WORLD_MAX_STATIONS = 4_096
_F32_MAX = 3.4028234663852886e38
# Role colours (Okabe-Ito): what AUTO styles draw in when no colour is given.
ROLE = {"plan": "#56B4E9", "prediction": "#009E73", "lookahead": "#E69F00", "goal": "#F0E442",
        "obstacle": "#CC79A7", "danger": "#D55E00", "history": "#FFFFFF59", "sensor": "#0072B2"}
DRAW_CHUNK_BYTES = 12 * 1024
NO_RETURN = 0xFFFF
DEFAULT_OPACITY = 0.5
_LINES, _POINTS, _ARROWS, _SHAPES, _TRIANGLES, _TEXT, _AXES = range(1, 8)
_POLYGON, _GRID, _GHOST, _PANEL, _MESH, _INTERACT = range(8, 14)
_SECTIONS = ("points", "colors", "radii", "offsets", "instances", "values", "rings", "qpoints", "ranges", "cells",
             "labels", "props")
_SECTION_FMT = {"points": "f", "colors": "I", "radii": "f", "offsets": "I", "instances": "f", "values": "f",
                "rings": "I", "qpoints": "h", "ranges": "H", "cells": "B"}
_DRAW_FLOATS = ("lifetime_s", "px", "py", "pz", "qw", "qx", "qy", "qz", "sx", "sy", "sz", "width", "glow",
                "pulse_hz", "flow_speed", "dash", "beam_height", "fade", "urgency", "opacity", "vmin", "vmax", "quant")
_DRAW_HEADER = _DRAW_FLOATS + ("op", "kind", "sub", "flags", "style", "seq", "chunk_offset", "last", "color",
                               "color2", "colormap")
_DRAW_FIELDS = _DRAW_HEADER + tuple(f"{s}_{p}" for s in _SECTIONS for p in ("offset", "len"))
# rviz_visual_tools colour names on the Okabe-Ito palette (see `Color`).
_NAMED_COLORS = {
    "black": (0, 0, 0), "brown": (0xA6, 0x76, 0x1D), "blue": (0x00, 0x72, 0xB2), "cyan": (0x56, 0xB4, 0xE9),
    "sky": (0x56, 0xB4, 0xE9), "grey": (0x99, 0x99, 0x99), "gray": (0x99, 0x99, 0x99),
    "dark_grey": (0x55, 0x55, 0x55), "dark_gray": (0x55, 0x55, 0x55), "green": (0x00, 0x9E, 0x73),
    "lime_green": (0x00, 0x9E, 0x73), "magenta": (0xCC, 0x79, 0xA7), "pink": (0xCC, 0x79, 0xA7),
    "purple": (0xCC, 0x79, 0xA7), "orange": (0xE6, 0x9F, 0x00), "red": (0xD5, 0x5E, 0x00),
    "vermillion": (0xD5, 0x5E, 0x00), "white": (255, 255, 255), "yellow": (0xF0, 0xE4, 0x42),
}
_NAMED_ALPHA = {"translucent": 0.25, "translucent_light": 0.1, "translucent_dark": 0.5, "clear": 0.0}


def _draw_entity_class():
    defaults = {"qw": 1.0, "sx": 1.0, "sy": 1.0, "sz": 1.0}
    annotations, body = {}, {}
    for i, name in enumerate(_DRAW_FIELDS):
        kind = float if i < len(_DRAW_FLOATS) else int
        annotations[name] = kind
        body[name] = defaults.get(name, kind(0))
    annotations["payload"] = bytes
    body["payload"] = b""
    body["__annotations__"] = annotations

    def section(self, name: str) -> bytes:
        at = getattr(self, f"{name}_offset")
        return self.payload[at:at + getattr(self, f"{name}_len")]

    body["section"] = section
    body["props"] = property(lambda self: unpack_map(self.section("props")))
    return _pod("gearbox.draw.v1", "<8d15f11I" + "II" * len(_SECTIONS), _DRAW_FIELDS, payload="payload")(
        type("DrawEntity", (), body))


DrawEntity = _draw_entity_class()


def rgba(color, alpha: float | None = None) -> int:
    """A colour as packed RGBA8: "#rgb", "#rrggbb", "#rrggbbaa", a name,
    (r, g, b[, a]) in 0..1 when any is a float, else 0..255, or a packed int.
    `alpha` (0..1) overrides the alpha. None is 0, the entity default."""
    if color is None:
        return 0
    if isinstance(color, int):
        r, g, b, a = color & 255, (color >> 8) & 255, (color >> 16) & 255, (color >> 24) & 255
    elif isinstance(color, str):
        text = color.strip().lower().replace("-", "_").replace(" ", "_")
        if text in ROLE:
            text = ROLE[text]
        if text in _NAMED_ALPHA:
            r, g, b, a = 255, 255, 255, round(_NAMED_ALPHA[text] * 255)
        elif text in _NAMED_COLORS:
            (r, g, b), a = _NAMED_COLORS[text], 255
        else:
            h = text.lstrip("#")
            if len(h) == 3:
                h = "".join(c * 2 for c in h)
            if len(h) not in (6, 8):
                raise ValueError(f"bad colour {color!r}")
            r, g, b = int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)
            a = int(h[6:8], 16) if len(h) == 8 else 255
    else:
        c = list(color)
        unit = any(isinstance(v, float) for v in c)
        q = [round(max(0.0, min(1.0, v)) * 255) if unit else int(v) & 255 for v in c]
        r, g, b = q[0], q[1], q[2]
        a = q[3] if len(q) > 3 else 255
    if alpha is not None:
        a = round(max(0.0, min(1.0, alpha)) * 255)
    return r | (g << 8) | (b << 16) | (a << 24)


def _flat_points(points) -> list[float]:
    out = []
    for p in points or ():
        out += (float(p[0]), float(p[1]), float(p[2]) if len(p) > 2 else 0.0)
    return out


def _faded(color, opacity: float) -> int:
    tint = rgba(color)
    return rgba(tint, (tint >> 24) / 255.0 * max(0.0, min(1.0, opacity))) or 1


range_ = range


def _drapes(frame: str) -> bool:
    """Whether z in `frame` can be height above the terrain."""
    f = frame.strip("/")
    return f in ("", "world", "map", "home") or f.endswith("/odom")


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _normalize(v):
    n = math.sqrt(sum(c * c for c in v))
    return tuple(c / n for c in v) if n > 1e-12 else (0.0, 0.0, 1.0)


def _rotate(q, v):
    """`v` turned by the quaternion (w, x, y, z)."""
    w, u = q[0], q[1:]
    t = tuple(2 * c for c in _cross(u, v))
    ut = _cross(u, t)
    return tuple(v[i] + w * t[i] + ut[i] for i in range(3))


def _apply(origin, q, v):
    r = _rotate(q, v)
    return tuple(float(origin[i]) + r[i] for i in range(3))


def _world_f32(value):
    value = float(value)
    if math.isfinite(value) and abs(value) <= _F32_MAX:
        return struct.unpack("<f", struct.pack("<f", value))[0]
    return math.copysign(math.inf, value) if math.isfinite(value) else value


def _world_point(point):
    if isinstance(point, (str, bytes)):
        raise TypeError("world points must be numeric coordinate sequences")
    point = tuple(point)
    if len(point) not in (2, 3):
        raise ValueError("world points need X,Y or X,Y,Z")
    return tuple(_world_f32(v) for v in point) + (0.0,) * (3 - len(point))


def _world_finite(point):
    return all(math.isfinite(v) and abs(v) <= _F32_MAX for v in point)


def _world_positive(value):
    return math.isfinite(value) and 0.0 < value <= _F32_MAX


def _world_segment(points, a, b):
    a, b = tuple(_world_f32(v) for v in a), tuple(_world_f32(v) for v in b)
    if _world_finite(a) and _world_finite(b) and a != b:
        points.extend((a, b))


def _world_open_outline(points, polygon):
    for a, b in zip(polygon, (*polygon[1:], polygon[0])):
        lerp = lambda t: tuple(a[j] * (1 - t) + b[j] * t for j in range(3))
        _world_segment(points, lerp(0.12), lerp(0.88))


BRACKET_FRACTION = 0.18
STRUCTURE_RULE_PX = 1.0
STRUCTURE_CORNER_GAIN = 1.75


def _world_corner_outline(points, polygon):
    for i, p in enumerate(polygon):
        for q in (polygon[i - 1], polygon[(i + 1) % len(polygon)]):
            _world_segment(points, p, tuple(p[j] + BRACKET_FRACTION * (q[j] - p[j]) for j in range(3)))


def _world_box_brackets(size):
    points = []
    if all(_world_positive(v) for v in size):
        for corner in range(8):
            p = tuple(size[i] * (0.5 if corner & (1 << i) else -0.5) for i in range(3))
            for axis in range(3):
                q = list(p)
                q[axis] -= math.copysign(size[axis] * BRACKET_FRACTION, p[axis])
                _world_segment(points, p, tuple(q))
    return points


def _world_station_mark(points, centre, tangent, size):
    xy = math.hypot(tangent[0], tangent[1])
    side = (-tangent[1] / xy, tangent[0] / xy, 0.0) if xy > 1e-9 else (1.0, 0.0, 0.0)
    at = lambda across, along: tuple(centre[i] + size * (across * side[i] + along * tangent[i]) for i in range(3))
    _world_segment(points, at(-0.5, 0.0), at(0.5, 0.0))
    _world_segment(points, at(-0.3, -0.25), at(0.0, 0.25))
    _world_segment(points, at(0.0, 0.25), at(0.3, -0.25))


def _world_pose(pose):
    if isinstance(pose, (str, bytes)):
        raise TypeError("pose must be a numeric coordinate sequence")
    pose = tuple(float(v) for v in pose)
    if len(pose) not in (2, 3, 4, 7):
        raise ValueError("pose needs X,Y[,Z], X,Y,Z,YAW or X,Y,Z,QW,QX,QY,QZ")
    if not all(math.isfinite(v) for v in pose):
        return None
    position = pose[:3] if len(pose) >= 3 else (*pose, 0.0)
    q = _quat(pose)
    scale = max(abs(v) for v in q)
    q = tuple(v / scale for v in q) if scale else (1.0, 0.0, 0.0, 0.0)
    norm = math.sqrt(sum(v * v for v in q))
    return (*position, *(v / norm for v in q))


def _world_size(size):
    if isinstance(size, (str, bytes)):
        raise TypeError("size must be three numeric full extents")
    size = tuple(_world_f32(v) for v in size)
    if len(size) != 3:
        raise ValueError("size needs three full extents X,Y,Z")
    return size


def _ellipse(c, rx, ry, angle, n):
    s, co = math.sin(angle), math.cos(angle)
    out = []
    for k in range(n):
        t = 2 * math.pi * k / n
        x, y = rx * math.cos(t), ry * math.sin(t)
        out.append((c[0] + x * co - y * s, c[1] + x * s + y * co, c[2]))
    return out


def covariance_ellipse(xx: float, xy: float, yy: float) -> tuple[float, float, float]:
    """One-sigma half axes (major, minor) and the major axis' angle of a 2×2 covariance."""
    mid, half = 0.5 * (xx + yy), math.sqrt(0.25 * (xx - yy) ** 2 + xy * xy)
    return math.sqrt(max(mid + half, 0.0)), math.sqrt(max(mid - half, 0.0)), 0.5 * math.atan2(2 * xy, xx - yy)


def covariance_axes(c) -> tuple[tuple[float, float, float], tuple[float, float, float, float]]:
    """Standard deviations along the principal axes of a 3×3 covariance,
    largest first, and the rotation (w, x, y, z) onto those axes."""
    a = [list(map(float, row)) for row in c]
    v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    for _ in range(50):
        if abs(a[0][1]) + abs(a[0][2]) + abs(a[1][2]) < 1e-15:
            break
        for p, q in ((0, 1), (0, 2), (1, 2)):
            if abs(a[p][q]) < 1e-18:
                continue
            theta = 0.5 * (a[q][q] - a[p][p]) / a[p][q]
            t = (1.0 if theta >= 0 else -1.0) / (abs(theta) + math.sqrt(theta * theta + 1.0))
            cs = 1.0 / math.sqrt(t * t + 1.0)
            sn = t * cs
            for k in range(3):
                akp, akq = a[k][p], a[k][q]
                a[k][p], a[k][q] = cs * akp - sn * akq, sn * akp + cs * akq
            for k in range(3):
                apk, aqk = a[p][k], a[q][k]
                a[p][k], a[q][k] = cs * apk - sn * aqk, sn * apk + cs * aqk
            for row in v:
                vp, vq = row[p], row[q]
                row[p], row[q] = cs * vp - sn * vq, sn * vp + cs * vq
    order = sorted(range(3), key=lambda k: -a[k][k])
    x = tuple(v[r][order[0]] for r in range(3))
    y = tuple(v[r][order[1]] for r in range(3))
    z = _cross(x, y)
    sd = tuple(math.sqrt(max(a[k][k], 0.0)) for k in order)
    return sd, _quat_from_axes(x, y, z)


def _quat_from_axes(x, y, z):
    m00, m11, m22 = x[0], y[1], z[2]
    tr = m00 + m11 + m22
    if tr > 0:
        s = math.sqrt(tr + 1.0) * 2
        q = (0.25 * s, (y[2] - z[1]) / s, (z[0] - x[2]) / s, (x[1] - y[0]) / s)
    elif m00 > m11 and m00 > m22:
        s = math.sqrt(1.0 + m00 - m11 - m22) * 2
        q = ((y[2] - z[1]) / s, 0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s)
    elif m11 > m22:
        s = math.sqrt(1.0 + m11 - m00 - m22) * 2
        q = ((z[0] - x[2]) / s, (y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s)
    else:
        s = math.sqrt(1.0 + m22 - m00 - m11) * 2
        q = ((x[1] - y[0]) / s, (z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s)
    n = math.sqrt(sum(c * c for c in q))
    return tuple(c / n for c in q)


_CLASS_COLORS = {"car": "orange", "vehicle": "orange", "person": "blue", "pedestrian": "blue", "human": "blue",
                 "cyclist": "yellow", "bicycle": "yellow", "bike": "yellow", "motorcycle": "purple",
                 "motorbike": "purple", "truck": "red", "bus": "red", "animal": "green", "other": "grey",
                 "unknown": "grey", "": "grey"}


def class_color(cls: str, overrides: dict | None = None) -> int:
    """vision_msgs_rviz_plugins class colours on the Okabe-Ito palette."""
    if overrides:
        for k, v in overrides.items():
            if k.lower() == cls.lower():
                return rgba(v) or 1
    name = _CLASS_COLORS.get(cls.lower())
    if name:
        return rgba(name)
    h = 7
    for b in cls.encode():
        h = (h * 31 + b) & 0xFFFFFFFF
    return Color.OKABE[h % 7]


def _grid_bytes(cells, encoding: str) -> bytes:
    if isinstance(cells, (bytes, bytearray)):
        return bytes(cells)
    flat = [v for row in cells for v in (row if isinstance(row, (list, tuple)) else [row])]
    fmt = {"u8": "B", "u16": "H", "f32": "f"}[encoding]
    if fmt == "B":
        return bytes(int(v) & 255 for v in flat)
    return struct.pack(f"<{len(flat)}{fmt}", *flat)


def _quat(pose) -> tuple[float, float, float, float]:
    """(w, x, y, z) of a pose: (x, y[, z]) none, (x, y, z, yaw), or (x, y, z, w, qx, qy, qz)."""
    if len(pose) == 4:
        return (math.cos(0.5 * pose[3]), 0.0, 0.0, math.sin(0.5 * pose[3]))
    if len(pose) >= 7:
        return tuple(float(v) for v in pose[3:7])
    return (1.0, 0.0, 0.0, 0.0)


def _instances(poses, sizes, yaws=None) -> list[float]:
    out = []
    for i, pose in enumerate(poses):
        x, y = float(pose[0]), float(pose[1])
        z = float(pose[2]) if len(pose) > 2 else 0.0
        q = _quat((x, y, z, yaws[i])) if yaws is not None else _quat(pose)
        s = sizes[i] if sizes is not None and i < len(sizes) else (0.0, 0.0, 0.0)
        if isinstance(s, (int, float)):
            s = (s, s, s)
        s = tuple(float(v) for v in s) + (0.0,) * (3 - len(s))
        out += (x, y, z, *q, *s[:3])
    return out


class _Spec:
    """One draw entity before it is cut into messages."""

    def __init__(self, kind: int, ns: str, id: str, **header):
        self.kind, self.ns, self.id = kind, ns, id
        self.header = {"kind": kind, **header}
        self.points: list[float] = []
        self.colors: list[int] = []
        self.radii: list[float] = []
        self.offsets: list[int] = []
        self.instances: list[float] = []
        self.values: list[float] = []
        self.rings: list[int] = []
        self.qpoints: list[int] = []
        self.ranges: list[int] = []
        self.cells: bytes = b""
        self.labels: list[str] = []
        self.props: dict[str, object] = {"ns": ns, "id": id}
        self.wait: bool | None = None

    @property
    def sub(self) -> int:
        return self.header.get("sub", 0)

    def instanced(self) -> bool:
        return self.kind in (_SHAPES, _AXES, _GHOST, _MESH) or (self.kind == _ARROWS and bool(self.instances))

    def point_count(self) -> int:
        if self.points:
            return len(self.points) // 3
        if self.qpoints:
            return len(self.qpoints) // 3
        return len(self.ranges)

    def item_count(self) -> int:
        if self.instanced():
            return len(self.instances) // 10
        if self.kind == _GRID:
            return len(self.cells) // (1, 2, 4)[min(self.sub, 2)]
        if self.kind == _PANEL:
            return len(self.values)
        return self.point_count()

    def strip_ranges(self) -> list[tuple[int, int]]:
        n = self.point_count()
        starts = sorted({o for o in self.offsets if o < n} | {0})
        return [(s, e) for s, e in zip(starts, starts[1:] + [n]) if e > s]

    def message(self, op: int = 0, seq: int = 0, offset: int = 0, last: int = 1, labels=None, **sections) -> DrawEntity:
        labels = self.labels if labels is None else labels
        blobs = []
        for name in _SECTIONS[:-2]:
            data = sections.get(name, getattr(self, name))
            blobs.append(bytes(data) if name == "cells" else struct.pack(f"<{len(data)}{_SECTION_FMT[name]}", *data))
        blobs.append("\n".join(str(s).replace("\n", " ") for s in labels).encode())
        blobs.append(pack_map({k: v for k, v in self.props.items() if v is not None}))
        fields = {**self.header, "op": op, "seq": seq, "chunk_offset": offset, "last": last}
        at = 0
        for name, blob in zip(_SECTIONS, blobs):
            fields[f"{name}_offset"], fields[f"{name}_len"] = at, len(blob)
            at += len(blob)
        return DrawEntity(**fields, payload=b"".join(blobs))

    def size(self) -> int:
        size = 4 * (len(self.points) + len(self.colors) + len(self.radii) + len(self.offsets) + len(self.instances)
                    + len(self.values) + len(self.rings))
        return size + 2 * (len(self.qpoints) + len(self.ranges)) + len(self.cells) + sum(len(s) + 1 for s in self.labels)

    def messages(self, op: int = 0, budget: int = DRAW_CHUNK_BYTES) -> list[DrawEntity]:
        """The entity as one message, or as APPEND chunks of at most `budget`
        section bytes; a loop cut into chunks is closed and sent as strips."""
        if self.size() <= budget or op != 0:
            return [self.message(op=op)]
        if self.kind == _GRID:
            return self._grid_chunks(budget)
        if self.kind == _POLYGON:
            return self._polygon_chunks(budget)
        if self.kind == _LINES and self.sub == LINE_MODE["loop"]:
            self._close_loops()
        groups, current, used = [], [], 0
        for atom in self._atoms(budget):
            if current and used + atom["bytes"] > budget:
                groups.append(current)
                current, used = [], 0
            current.append(atom)
            used += atom["bytes"]
        if current:
            groups.append(current)
        seq = _next_seq()
        out, before = [], 0
        for i, group in enumerate(groups):
            msg, items = self._chunk(group, seq, before, int(i + 1 == len(groups)))
            before += items
            out.append(msg)
        return out

    def _grid_chunks(self, budget: int) -> list[DrawEntity]:
        size = (1, 2, 4)[min(self.sub, 2)]
        per = size + 4 * bool(self.values)
        run = max(1, (budget - 64) // per)
        n = max(len(self.cells) // size, len(self.values))
        seq, out = _next_seq(), []
        starts = list(range(0, max(n, 1), run))
        for i, s in enumerate(starts):
            e = min(s + run, n)
            msg = self.message(op=DRAW_OP["append"], seq=seq, offset=s, last=int(i + 1 == len(starts)),
                               points=[], colors=[], radii=[], offsets=[], instances=[], rings=[], qpoints=[],
                               ranges=[], values=self.values[s:e], cells=self.cells[s * size:e * size], labels=[])
            out.append(msg)
        return out

    def _polygon_chunks(self, budget: int) -> list[DrawEntity]:
        """Whole polygons per chunk (a polygon bigger than a chunk goes as
        its own oversized chunk)."""
        strips = self.strip_ranges()
        rings = self.rings or ([len(strips)] if strips else [])
        polys, at = [], 0
        for count in rings:
            polys.append(strips[at:at + count])
            at += count
        per = 12 if self.points else 6
        seq, groups, current, used = _next_seq(), [], [], 0
        for k, poly in enumerate(polys):
            need = sum(e - s for s, e in poly) * per + 24
            if current and used + need > budget - 64:
                groups.append(current)
                current, used = [], 0
            current.append(k)
            used += need
        if current:
            groups.append(current)
        out, before = [], 0
        for i, group in enumerate(groups):
            pts, qps, offs, rng, cols, vals, labels = [], [], [], [], [], [], []
            for k in group:
                rng.append(len(polys[k]))
                for s, e in polys[k]:
                    offs.append((len(pts) + len(qps)) // 3)
                    pts += self.points[3 * s:3 * e]
                    qps += self.qpoints[3 * s:3 * e]
                cols += self.colors[k:k + 1]
                vals += self.values[k:k + 1]
                labels += self.labels[k:k + 1]
            msg = self.message(op=DRAW_OP["append"], seq=seq, offset=before, last=int(i + 1 == len(groups)),
                               points=pts, qpoints=qps, offsets=offs if len(offs) > 1 else [],
                               rings=rng if len(rng) > 1 else [], colors=cols, values=vals, radii=[], labels=labels)
            before += (len(pts) + len(qps)) // 3
            out.append(msg)
        return out

    def _close_loops(self):
        if self.qpoints and not self.points:
            q = self.header.get("quant") or 0.01
            self.points, self.qpoints = [v * q for v in self.qpoints], []
        per_point_values = len(self.values) == self.point_count()
        points, colors, radii, values, offsets = [], [], [], [], []
        for s, e in self.strip_ranges():
            offsets.append(len(points) // 3)
            for i in list(range(s, e)) + [s]:
                points += self.points[3 * i:3 * i + 3]
                if self.colors:
                    colors.append(self.colors[i])
                if self.radii:
                    radii.append(self.radii[i])
                if per_point_values:
                    values.append(self.values[i])
        self.points, self.colors, self.radii = points, colors, radii
        if per_point_values:
            self.values = values
        self.offsets = offsets if len(offsets) > 1 else []
        self.header["sub"] = LINE_MODE["strip"]

    def _atoms(self, budget: int):
        per_point = (12 * bool(self.points) + 6 * bool(self.qpoints) + 2 * bool(self.ranges)
                     + 4 * (bool(self.colors) + bool(self.radii) + bool(self.values)) + bool(self.cells))
        label_bytes = lambda i: len(self.labels[i]) + 1 if i is not None and i < len(self.labels) else 0  # noqa: E731
        if self.kind == _LINES and self.sub != LINE_MODE["list"]:
            room = max(2, (budget - 64) // per_point)
            for s, (a, b) in enumerate(self.strip_ranges()):
                start = a
                while True:
                    end = min(start + room, b)
                    label = s if start == a else None
                    yield {"points": (start, end), "units": (start, end), "label": label, "strip": True, "strip_index": s,
                           "bytes": (end - start) * per_point + 4 + label_bytes(label)}
                    if end >= b:
                        break
                    start = end - 1
        elif self.instanced():
            per = 40 + 4 * (bool(self.colors) + bool(self.radii) + bool(self.values)) + bool(self.cells)
            for i in range(len(self.instances) // 10):
                yield {"points": (0, 0), "units": (i, i + 1), "inst": True, "label": i, "bytes": per + label_bytes(i)}
        else:
            step = 2 if self.kind in (_LINES, _ARROWS) else 3 if self.kind == _TRIANGLES else 1
            n = self.point_count() // step * step
            for u, start in enumerate(range(0, n, step)):
                units = (u, u + 1) if self.kind == _ARROWS else (start, start + step)
                label = None if self.kind == _LINES else u
                yield {"points": (start, start + step), "units": units, "label": label,
                       "bytes": step * per_point + label_bytes(label)}

    def _chunk(self, atoms, seq: int, offset: int, last: int):
        out = {name: [] for name in ("points", "colors", "radii", "offsets", "instances", "values", "qpoints", "ranges")}
        cells, labels = bytearray(), []
        per_strip_values = (self.kind == _LINES and self.values and len(self.values) != self.point_count()
                            and len(self.values) == len(self.strip_ranges()))
        for a in atoms:
            if a.get("strip"):
                out["offsets"].append(len(out["points"]) // 3 + len(out["qpoints"]) // 3 + len(out["ranges"]))
            p0, p1 = a["points"]
            out["points"] += self.points[3 * p0:3 * p1]
            out["qpoints"] += self.qpoints[3 * p0:3 * p1]
            out["ranges"] += self.ranges[p0:p1]
            u0, u1 = a["units"]
            if a.get("inst"):
                out["instances"] += self.instances[10 * u0:10 * u1]
            for name in ("colors", "radii"):
                out[name] += getattr(self, name)[u0:u1]
            if per_strip_values:
                out["values"].append(self.values[a["strip_index"]])
            else:
                out["values"] += self.values[u0:u1]
            cells += self.cells[u0:u1]
            label = a["label"]
            has = label is not None and label < len(self.labels)
            if self.labels and a.get("strip"):
                labels.append(self.labels[label] if has else "")
            elif has:
                labels.append(self.labels[label])
        if len(out["offsets"]) == 1:
            out["offsets"] = []
        items = (len(out["instances"]) // 10 if self.instanced()
                 else len(out["points"]) // 3 + len(out["qpoints"]) // 3 + len(out["ranges"]))
        msg = self.message(op=DRAW_OP["append"], seq=seq, offset=offset, last=last, labels=labels, cells=bytes(cells),
                           rings=[], **out)
        return msg, items


def _seq_counter():
    n = 0
    while True:
        n += 1
        yield n


_DRAW_SEQ = _seq_counter()


def _next_seq() -> int:
    return ((os.getpid() & 0xFFFF) << 16) | (next(_DRAW_SEQ) & 0xFFFF or 1)


class _Sender(threading.Thread):
    """Sends queued draw messages in order on a connection of its own. A newer
    send of an entity takes the place of an older one not yet sent; deletes,
    clears and extends are sent as queued and seal what is queued before."""

    def __init__(self, gb: "Gearbox"):
        super().__init__(name="gearbox-draw", daemon=True)
        self.gb = gb
        self.cv = threading.Condition()
        self.queue: collections.OrderedDict = collections.OrderedDict()
        self.busy = False
        self.errors: list[str] = []
        self.client = None
        self.start()

    def put(self, key, messages: list, coalesce: bool):
        with self.cv:
            if not coalesce:
                self.queue = collections.OrderedDict(((k, object()), v) for k, v in self.queue.items())
                key = (key, object())
            self.queue[key] = messages
            self.cv.notify_all()

    def flush(self, timeout: float) -> bool:
        deadline = time.time() + timeout
        with self.cv:
            while self.queue or self.busy:
                left = deadline - time.time()
                if left <= 0:
                    return False
                self.cv.wait(left)
        return True

    def run(self):
        while True:
            with self.cv:
                while not self.queue:
                    self.busy = False
                    self.cv.notify_all()
                    self.cv.wait()
                self.busy = True
                _, messages = self.queue.popitem(last=False)
            for msg in messages:
                status = self._call(msg)
                if not status.ok:
                    self.errors.append(status.message or f"draw refused (code {status.code})")
                    break

    def _call(self, msg) -> Status:
        last = None
        for _ in range(30):
            try:
                if self.client is None:
                    self.client = self.gb.node.datapod_req_client(self.gb.addr, HOST["draw"])
                return self.client.call_decode(msg, Status)
            except Exception as err:  # noqa: BLE001 - transport errors are retried
                last = err
                self.client = None
                time.sleep(0.1)
        return Status(code=1, detail=pack_map({"message": str(last)}))


class Draw:
    """Retained debug draws in the viewer, kept by the host per (ns, id) until
    deleted, cleared or out of lifetime (sim seconds; 0 keeps them); sending
    an entity again replaces it. Big entities are chunked for you.

    Coordinates are Z-up in `frame`: "world" (home: x east, y north, z up,
    the axes machine states report in), "<machine>/odom" (that machine's own
    datum, the origin its reported pose is measured from: draw at state x,
    y, z), "<machine>/<link>" or "<machine>" (the link's own axes, following
    it). `pose` places the whole entity in its frame: (x, y, z), (x, y, z,
    yaw) or (x, y, z, qw, qx, qy, qz). Widths and radii: > 0 metres, < 0
    pixels, None the default. Colours: see `rgba`; per-item `colors` win
    over `color`, and an entity without a colour gets one from its path.

        gb.draw.lines("lab/robot", "path", [(0, 0), (5, 0), (5, 5)], color="#ffaa00", width=0.08)
        gb.draw.points("lab/robot", "lookahead", [(2, 1, 0.2)], radius=0.15, color="cyan", frame="husky/odom")
        gb.draw.shapes("lab/obs", "pred", "ellipse", [(3, 2, 0.05)], sizes=[(1.2, 0.6)], fill="both",
                       color=(1.0, 0.3, 0.3, 0.25), lifetime=0.5)
        gb.draw.clear("lab")

    Every kind accepts `blur` in 0..1: zero keeps the transmitted background
    clear; one applies full frost. Omission preserves the existing finish.
    Text, geometry, data and glow remain independent of this background filter.
    Every kind takes `opacity` in 0..1 for the whole drawing; None or 0 is
    `DEFAULT_OPACITY` (0.5), 1 fully opaque.
    Every kind takes a `style` and its effect parameters `glow`, `pulse_hz`,
    `flow_speed`, `spacing`, `beam_height`, `fade` and `urgency` (None or 0
    the style's default, < 0 off). "auto" (the default) picks the style by
    kind and by role words in the ns or id: path/plan "ribbon_flow" (a flat
    ribbon with chevrons flowing at `flow_speed`), horizon/predict
    "tube_pulse" (a tube fading to `fade` at its tail, one pulse running
    along it), rollout "hairline" (faint additive lines that build a
    density map), trace/history "light_wall" (a wall of light `beam_height`
    tall fading towards the start), goal "beacon" (ground ring with ripples
    and a beam of light), waypoint "holo" (glowing edges, scanline, short
    beam), obstacle "shield" (fresnel dome, ripples faster and redder with
    `urgency`), lookahead "orb", footprint "plain" (a thin outline), alert/collision "alert"
    (hazard stripes on top); otherwise "point_glow" points, "arrow_glow"
    arrows, "label" text and "plain" for the rest. With no colour, auto
    styles take their role colour from `ROLE`. `flat=True` lays lines and
    points in the frame's XY plane, `additive=True` adds them as light.
    In world and odom frames z is height above the terrain under each point
    and lines bend over the ground (`drape=False` makes z the frame's own).

    Every call waits for the host's answer and returns its `Status`, unless
    `wait=False` (or `gb.draw.wait = False`): then it is queued for a sender
    thread and returns None at once; a newer send of an entity replaces an
    older one still queued, `flush()` waits for the queue and `errors` holds
    what the host refused.
    """

    def __init__(self, gb: "Gearbox", wait: bool = True):
        self.gb = gb
        self.wait = wait
        self._sender: _Sender | None = None
        self._batch: _Batch | None = None

    @property
    def errors(self) -> list[str]:
        return self._sender.errors if self._sender else []

    def flush(self, timeout: float = 5.0) -> bool:
        """Wait until every queued draw is sent; False on timeout."""
        return self._sender.flush(timeout) if self._sender else True

    def backlog(self) -> int:
        """Draws queued and not yet sent."""
        return len(self._sender.queue) if self._sender else 0

    def _deliver(self, key, messages: list, coalesce: bool, wait: bool | None) -> Status | None:
        if wait if wait is not None else self.wait:
            status = Status()
            for msg in messages:
                status = self.gb.call(self.gb.addr, HOST["draw"], msg, Status)
                if not status.ok:
                    break
            return status
        if self._sender is None:
            self._sender = _Sender(self.gb)
        self._sender.put(key, messages, coalesce)
        return None

    def send(self, spec: _Spec, op: int = 0) -> Status | None:
        status = self._deliver((spec.ns, spec.id), spec.messages(op), op == 0, spec.wait)
        self._sent()
        return status

    @staticmethod
    def _spec(kind: int, ns: str, id: str, *, frame: str = "world", pose=None, color=None, alpha=None,
              color2=None, width=None, lifetime: float = 0.0, on_top: bool = False, snapshot: bool = False,
              labels=None, sub: int = 0, flags: int = 0, scale=(1.0, 1.0, 1.0), wait: bool | None = None,
              style: str = "auto", glow: float | None = None, blur: float | None = None, pulse_hz: float | None = None,
              flow_speed: float | None = None, spacing: float | None = None, beam_height: float | None = None,
              fade: float | None = None, urgency: float | None = None, opacity: float | None = None, flat: bool = False,
              additive: bool = False, drape: bool | None = None, colormap=None, vmin: float = 0.0,
              vmax: float = 0.0, arrow_tip: bool = False, props: dict | None = None,
              label_category: str | None = None, label_detail: str | None = None,
              label_icon: str | None = None, label_selected: bool | None = None) -> _Spec:
        pose = tuple(pose) if pose is not None else (0.0, 0.0, 0.0)
        w, qx, qy, qz = _quat(pose)
        flags |= (DRAW_FLAG["on_top"] if on_top else 0) | (DRAW_FLAG["snapshot"] if snapshot else 0)
        flags |= (DRAW_FLAG["flat"] if flat else 0) | (DRAW_FLAG["additive"] if additive else 0)
        flags |= DRAW_FLAG["arrow_tip"] if arrow_tip else 0
        if drape if drape is not None else _drapes(frame):
            flags |= DRAW_FLAG["drape"]
        if labels:
            flags |= DRAW_FLAG["labels"]
        effect = lambda v: float(v) if v is not None else 0.0  # noqa: E731
        cmap = COLORMAP[colormap] if isinstance(colormap, str) else int(colormap or 0)
        spec = _Spec(kind, ns, id, lifetime_s=float(lifetime), px=float(pose[0]), py=float(pose[1]),
                     pz=float(pose[2]) if len(pose) > 2 else 0.0, qw=w, qx=qx, qy=qy, qz=qz,
                     sx=float(scale[0]), sy=float(scale[1]), sz=float(scale[2]), width=float(width or 0.0),
                     glow=effect(glow), pulse_hz=effect(pulse_hz), flow_speed=effect(flow_speed),
                     dash=effect(spacing), beam_height=effect(beam_height), fade=effect(fade), urgency=effect(urgency),
                     opacity=effect(opacity),
                     vmin=float(vmin), vmax=float(vmax), quant=0.0, colormap=cmap,
                     sub=sub, flags=flags, style=DRAW_STYLE[style] if isinstance(style, str) else int(style),
                     color=(rgba(color, alpha) or 1) if color is not None else 0,
                     color2=(rgba(color2) or 1) if color2 is not None else 0)
        spec.props["frame"] = frame
        if props:
            spec.props.update({k: v for k, v in props.items() if v is not None})
        if blur is not None:
            if not math.isfinite(float(blur)) or not 0 <= float(blur) <= 1:
                raise ValueError("blur must be a finite number in 0..1")
            spec.props["blur"] = float(blur)
        for key, value in (("label_category", label_category), ("label_detail", label_detail)):
            if value is not None:
                spec.props[key] = str(value)
        if label_icon in ("robot", "sensor", "destination", "measurement", "obstacle", "frame"):
            spec.props["label_icon"] = label_icon
        if label_selected is not None:
            spec.props["label_selected"] = str(bool(label_selected)).lower()
        spec.labels = [str(s) for s in labels] if labels else []
        spec.wait = wait
        return spec

    @staticmethod
    def _per_item(spec: _Spec, colors=None, radii=None, alpha=None, values=None):
        if colors is not None:
            spec.colors = [(rgba(c, alpha) or 1) if c is not None else 0 for c in colors]
        if radii is not None:
            spec.radii = [float(r) for r in radii]
        if values is not None:
            spec.values = [float(v) for v in values]
            if not spec.header.get("colormap"):
                spec.header["colormap"] = COLORMAP["turbo"]

    def lines(self, ns: str, id: str, points=None, *, strips=None, loop: bool = False, segments: bool = False,
              colors=None, radii=None, values=None, dashed: bool = False, dash: float | None = None,
              flat_caps: bool = False, shaded: bool = False, **common) -> Status:
        """A polyline through `points`, or many in one entity with `strips`
        (a list of point lists). `loop` closes each strip, `segments` pairs
        the points instead. `colors`/`radii` give each point its own;
        `width` is the default. `labels` name each strip. `values` (one per
        point, or one per strip) are coloured through `colormap`;
        `arrow_tip=True` ends each strip in an arrowhead."""
        mode = LINE_MODE["list"] if segments else LINE_MODE["loop"] if loop else LINE_MODE["strip"]
        flags = (DRAW_FLAG["dashed"] if dashed else 0) | (DRAW_FLAG["flat_caps"] if flat_caps else 0)
        flags |= DRAW_FLAG["shaded"] if shaded else 0
        spec = self._spec(_LINES, ns, id, sub=mode, flags=flags, **common)
        if strips is not None:
            for strip in strips:
                spec.offsets.append(len(spec.points) // 3)
                spec.points += _flat_points(strip)
            if len(spec.offsets) <= 1:
                spec.offsets = []
        else:
            spec.points = _flat_points(points)
        if dash:
            spec.header["dash"] = float(dash)
        self._per_item(spec, colors, radii, common.get("alpha"), values)
        return self.send(spec)

    def extend(self, ns: str, id: str, points, *, max_points: int = 0, kind: str = "lines", **common) -> Status:
        """Add `points` to the live entity (the end of a line), making it when
        missing; `max_points` keeps only the newest. For traces."""
        spec = self._spec(DRAW_KIND[kind], ns, id, **common)
        spec.points = _flat_points(points)
        if max_points:
            spec.props["max_points"] = int(max_points)
        per = (DRAW_CHUNK_BYTES // 12) * 3
        parts = [spec.message(op=DRAW_OP["extend"], points=spec.points[i:i + per])
                 for i in range(0, max(len(spec.points), 1), per)]
        return self._deliver((ns, id), parts, False, spec.wait)

    def points(self, ns: str, id: str, points, *, radius: float | None = None, radii=None, colors=None,
               values=None, disc: bool = False, square: bool = False, quant: float = 0.0, **common) -> Status:
        """Spheres (or flat discs with `disc=True`, square splats with
        `square=True`) at `points`; `radius` or per-point `radii`, > 0
        metres, < 0 pixels. `values` are coloured through `colormap`;
        `quant` > 0 sends positions as i16 steps of that many metres."""
        if radius is not None:
            common["width"] = 2.0 * radius
        sub = POINT_STYLE["square" if square else "disc" if disc else "sphere"]
        spec = self._spec(_POINTS, ns, id, sub=sub, **common)
        self._set_points(spec, points, quant)
        self._per_item(spec, colors, radii, common.get("alpha"), values)
        return self.send(spec)

    @staticmethod
    def _set_points(spec: _Spec, points, quant: float = 0.0):
        flat = _flat_points(points)
        if quant > 0:
            spec.qpoints = [max(-32768, min(32767, round(v / quant))) for v in flat]
            spec.header["quant"] = float(quant)
        else:
            spec.points = flat

    def arrows(self, ns: str, id: str, origins=None, vectors=None, *, tips=None, poses=None, length: float = 1.0,
               lengths=None, colors=None, radii=None, values=None, chevron: bool = False, **common) -> Status:
        """Arrows from `origins` along `vectors` (or to `tips`), or along +x of
        each of `poses` (x, y, z[, yaw] or a quaternion) `length` (or
        `lengths`) long; `chevron=True` draws flat chevrons instead."""
        spec = self._spec(_ARROWS, ns, id, scale=(length, length, length), sub=ARROW_STYLE["chevron" if chevron else "arrow"],
                          **common)
        if poses is not None:
            spec.instances = _instances(poses, [(l, l, l) for l in lengths] if lengths is not None else None)
        else:
            tails = _flat_points(origins)
            ends = _flat_points(tips) if tips is not None else [t + v for t, v in zip(tails, _flat_points(vectors))]
            for i in range(0, len(tails), 3):
                spec.points += tails[i:i + 3] + ends[i:i + 3]
        self._per_item(spec, colors, radii, common.get("alpha"), values)
        return self.send(spec)

    def shapes(self, ns: str, id: str, shape: str = "box", centers=None, *, sizes=None, half_sizes=None,
               size=None, yaws=None, poses=None, fill: str = "solid", outline=None, colors=None,
               values=None, shaded: bool = False, **common) -> Status:
        """Instanced `shape`s (box, ellipsoid, cylinder, cone, capsule, ellipse)
        at `centers` (turned by `yaws`) or `poses`. `sizes` are full extents
        (diameters; ellipses in their XY plane), `half_sizes` half of them,
        `size` the default. `fill`: solid, wire, or both (fill in its colour,
        alpha honoured, edged in `outline` or the colour made opaque)."""
        if half_sizes is not None:
            sizes = [tuple(2.0 * v for v in (s if not isinstance(s, (int, float)) else (s, s, s))) for s in half_sizes]
        if size is None:
            size = (1.0, 1.0, 0.0 if SHAPE[shape] == SHAPE["sector"] else 1.0)
        if isinstance(size, (int, float)):
            size = (size, size, size)
        spec = self._spec(_SHAPES, ns, id, sub=SHAPE[shape], flags=FILL[fill] | (DRAW_FLAG["shaded"] if shaded else 0),
                          scale=tuple(size), color2=outline, **common)
        places = poses if poses is not None else centers if centers is not None else [(0.0, 0.0, 0.0)]
        spec.instances = _instances(places, sizes, yaws)
        self._per_item(spec, colors, None, common.get("alpha"), values)
        return self.send(spec)

    def triangles(self, ns: str, id: str, vertices, *, colors=None, shaded: bool = False, **common) -> Status:
        """A triangle list, three vertices each, `colors` per vertex."""
        spec = self._spec(_TRIANGLES, ns, id, flags=DRAW_FLAG["shaded"] if shaded else 0, **common)
        spec.points = _flat_points(vertices)
        self._per_item(spec, colors, None, common.get("alpha"))
        return self.send(spec)

    def text(self, ns: str, id: str, positions, texts=None, *, text: str | None = None, size: float = 16.0,
             size_m: float | None = None, background=None, colors=None, **common) -> Status:
        """Text centred at `positions`: one of `texts` each, or `text` at all.
        `size` in pixels, or `size_m` metres tall; `background` behind it.
        Optional label_category/detail/icon/selected supply annotation metadata."""
        common.setdefault("width", float(size_m) if size_m else -abs(float(size)))
        common.setdefault("color2", background)
        spec = self._spec(_TEXT, ns, id, **common)
        spec.points = _flat_points(positions)
        spec.labels = [str(t) for t in texts] if texts is not None else []
        spec.header["flags"] &= ~DRAW_FLAG["labels"]
        if text is not None:
            spec.props["text"] = text
        self._per_item(spec, colors, None, common.get("alpha"))
        return self.send(spec)

    def axes(self, ns: str, id: str, poses=None, *, length: float = 1.0, **common) -> Status:
        """RGB axis triads `length` long at `poses` (x, y, z[, yaw] or a
        quaternion), or one at the entity pose."""
        spec = self._spec(_AXES, ns, id, scale=(length, length, length), **common)
        if poses is not None:
            spec.instances = _instances(poses, None)
        return self.send(spec)

    def delete(self, ns: str, id: str, wait: bool | None = None) -> Status | None:
        return self._deliver((ns, id), [_Spec(0, ns, id).message(op=DRAW_OP["delete"])], False, wait)

    def clear(self, prefix: str = "", wait: bool | None = None) -> Status | None:
        """Remove the draws at `prefix` and every path below it; "" removes all."""
        return self._deliver((prefix, ""), [_Spec(0, prefix, "").message(op=DRAW_OP["clear"])], False, wait)

    def list(self, prefix: str = "") -> list[dict]:
        """The live entities under `prefix`: ns, id, frame, kind, points, instances."""
        query = _Spec(0, prefix, "").message(op=DRAW_OP["clear"])
        out = []
        for e in self.gb.query(self.gb.addr, HOST["draw_list"], query, DrawEntity):
            d = e.props
            d.update({"kind": next((k for k, v in DRAW_KIND.items() if v == e.kind), str(e.kind)),
                      "lifetime_s": e.lifetime_s})
            out.append(d)
        return out

    # ── batching ──

    def batch(self, every: int = 0, timeout: float = 5.0) -> "_Batch":
        """`with gb.draw.batch():` sends without waiting and flushes on
        leaving; `every=n` also flushes after every n sends (rviz
        triggerEvery)."""
        return _Batch(self, every, timeout)

    def _sent(self):
        b = getattr(self, "_batch", None)
        if b is not None:
            b.tick()

    # ── header changes and patches ──

    def update(self, ns: str, id: str, *, pose=None, scale=None, color=None, alpha=None, color2=None,
               width=None, style=None, glow=None, blur=None, pulse_hz=None, flow_speed=None, spacing=None, beam_height=None,
               fade=None, urgency=None, opacity=None, flags=None, colormap=None, vmin=None, vmax=None, lifetime=None,
               wait: bool | None = None, **props) -> Status | None:
        """Change the live entity's header only, keeping its points: what is
        given here (pose, colour, width, style, effects, colormap,
        lifetime) and props. Unspecified fields are preserved; alpha requires color."""
        if alpha is not None and color is None:
            raise ValueError("update alpha requires color")
        groups = []
        header = {}
        if pose is not None:
            pose = tuple(pose)
            w, qx, qy, qz = _quat(pose)
            header.update(px=float(pose[0]), py=float(pose[1]), pz=float(pose[2]) if len(pose) > 2 else 0.0,
                          qw=w, qx=qx, qy=qy, qz=qz)
            groups.append("pose")
        if scale is not None:
            s = (scale,) * 3 if isinstance(scale, (int, float)) else tuple(scale)
            header.update(sx=float(s[0]), sy=float(s[1]), sz=float(s[2]))
            groups.append("scale")
        if color is not None:
            header["color"] = rgba(color, alpha) or 1
            groups.append("tint")
        if color2 is not None:
            header["color2"] = rgba(color2) or 1
            groups.append("color2")
        if width is not None:
            header["width"] = float(width)
            groups.append("width")
        if style is not None:
            header["style"] = DRAW_STYLE[style] if isinstance(style, str) else int(style)
            groups.append("style")
        effects = dict(glow=glow, pulse_hz=pulse_hz, flow_speed=flow_speed, dash=spacing, beam_height=beam_height,
                       fade=fade, urgency=urgency, opacity=opacity)
        for key, value in effects.items():
            if value is not None:
                header[key] = float(value)
                groups.append(key)
        if flags is not None:
            header["flags"] = int(flags)
            groups.append("flags")
        if colormap is not None:
            header["colormap"] = COLORMAP[colormap] if isinstance(colormap, str) else int(colormap)
            groups.append("palette")
        for key, value in (("vmin", vmin), ("vmax", vmax)):
            if value is not None:
                header[key] = float(value)
                groups.append(key)
        if lifetime is not None:
            header["lifetime_s"] = float(lifetime)
            groups.append("lifetime")
        spec = _Spec(0, ns, id, **header)
        spec.props.update({k: v for k, v in props.items() if v is not None})
        if blur is not None:
            if not math.isfinite(float(blur)) or not 0 <= float(blur) <= 1:
                raise ValueError("blur must be a finite number in 0..1")
            spec.props["blur"] = float(blur)
        spec.props["set"] = ",".join(groups) or "props"
        return self._deliver((ns, id, "update"), [spec.message(op=DRAW_OP["update"])], True, wait)

    def patch(self, ns: str, id: str, offset: int, *, kind: str = "points", points=None, colors=None, radii=None,
              values=None, labels=None, wait: bool | None = None) -> Status | None:
        """Replace items `offset .. offset + n` of the live entity (growing
        it at the end) with the ones given."""
        spec = _Spec(DRAW_KIND[kind], ns, id)
        spec.points = _flat_points(points) if points is not None else []
        self._per_item(spec, colors, radii, None, values)
        spec.header.pop("colormap", None)
        spec.labels = [str(s) for s in labels] if labels else []
        return self._deliver((ns, id, "patch", offset), [spec.message(op=DRAW_OP["patch"], offset=int(offset))], False, wait)

    # ── polygons and grids ──

    def polygon(self, ns: str, id: str, rings=None, *, polygons=None, mode: str = "both", z_offset: float = 0.0,
                colors=None, values=None, color_mode: str = "single", **common) -> Status | None:
        """A polygon with holes (`rings`: outline then holes) or many
        (`polygons`: a list of ring lists), filled as a ground decal and
        outlined (polygon_ros: `mode` outline, fill or both; `z_offset`;
        `color_mode` single, message (`colors` per polygon) or unique)."""
        common.setdefault("style", "decal")
        spec = self._spec(_POLYGON, ns, id, flags=FILL[mode], props={"z_offset": z_offset or None}, **common)
        polys = polygons if polygons is not None else [rings]
        for poly in polys:
            spec.rings.append(len(poly))
            for ring in poly:
                spec.offsets.append(len(spec.points) // 3)
                spec.points += _flat_points(ring)
        if len(spec.rings) <= 1:
            spec.rings = []
        if len(spec.offsets) <= 1:
            spec.offsets = []
        if color_mode == "unique" and colors is None:
            colors = [Color.OKABE[k % len(Color.OKABE)] for k in range(len(polys))]
        self._per_item(spec, colors, None, common.get("alpha"), values)
        return self.send(spec)

    def grid(self, ns: str, id: str, cells, cols: int, rows: int, *, cell: float = 1.0, cell_y: float | None = None,
             origin=(0.0, 0.0, 0.0), encoding: str = "u8", colormap="viridis", vmin: float = 0.0, vmax: float = 0.0,
             unknown: float | None = None, heights=None, contours: float | None = None, **common) -> Status | None:
        """A `cols` × `rows` grid of `cell` metre cells, row by row from
        `origin` (its lower-left corner; x, y, z[, yaw]), as a heat map on
        the ground: `cells` (u8, u16 or f32 per `encoding`) through
        `colormap` (turbo, viridis, rdylgn, costmap, map, raw, okabe,
        rainbow, grey) over `vmin..vmax`; `unknown` cells are left clear,
        `heights` lifts each cell, `contours` draws lines every that much."""
        common.setdefault("style", "heat")
        spec = self._spec(_GRID, ns, id, pose=origin, sub=GRID_ENCODING[encoding], colormap=colormap, vmin=vmin,
                          vmax=vmax, spacing=contours,
                          props={"cols": int(cols), "rows": int(rows), "cell": float(cell), "cell_y": cell_y,
                                 "unknown": unknown}, **common)
        spec.cells = _grid_bytes(cells, encoding)
        if heights is not None:
            spec.values = [float(h) for h in heights]
        return self.send(spec)

    def grid_patch(self, ns: str, id: str, x: int, y: int, w: int, h: int, cells, *, encoding: str = "u8",
                   wait: bool | None = None) -> Status | None:
        """Replace the `w` × `h` cells at column `x`, row `y` of a live grid
        (OccupancyGridUpdate); tiles of 64 × 64 keep each message small."""
        spec = _Spec(_GRID, ns, id, sub=GRID_ENCODING[encoding])
        spec.offsets = [int(x), int(y), int(w), int(h)]
        spec.cells = _grid_bytes(cells, encoding)
        return self._deliver((ns, id, "patch", x, y), [spec.message(op=DRAW_OP["patch"])], True, wait)

    # ── frames and poses ──

    def frame(self, ns: str, name: str, pose=(0.0, 0.0, 0.0), *, parent=None, length: float = 0.5,
              label: str | None = None, frame: str = "world", **common) -> None:
        """A frame triad (x vermillion, y bluish green, z blue), its name,
        and a dashed hairline to the `parent` position."""
        root = f"{ns}/{name}"
        common.setdefault("lifetime", 0.0)
        options = {"frame": frame, **common}
        self.axes(root, "axes", None, length=length, **{"pose": pose, "style": "arrow_glow", "width": 0.05 * length, **options})
        at = tuple(pose[:3]) if len(pose) >= 3 else (pose[0], pose[1], 0.0)
        self.text(root, "name", [at], text=name if label is None else label,
                  **{"color": "white", **options, "props": {"label_icon": "frame", **options.get("props", {})}})
        if parent is not None:
            parent_color = _faded(rgba(common.get("color", "white"), common.get("alpha")), 0.55)
            self.lines(root, "parent", [at, tuple(parent)], **{"width": -1.2,
                       "dash": 0.15, "dashed": True, "style": "plain", **options, "color": parent_color, "alpha": None})
        else:
            self._world_send(self._world_clear(root, ["parent"], options))

    def pose_array(self, ns: str, id: str, poses, *, glyph: str = "chevron", length: float = 0.5, **common) -> Status | None:
        """Poses (x, y, z[, yaw] or a quaternion) as flat chevrons, arrows or
        axes."""
        if glyph == "axes":
            return self.axes(ns, id, poses, length=length, **common)
        common.setdefault("style", "arrow_glow")
        if glyph == "chevron":
            common.setdefault("flat", True)
            common.setdefault("additive", True)
        poses = list(poses)
        return self.arrows(ns, id, poses=poses, length=length, lengths=[length] * len(poses), chevron=glyph == "chevron", **common)

    def waypoints(self, ns: str, id: str, points, *, current: int = 0, color=None, frame: str = "world", **common) -> None:
        """Numbered route stations and selected corner brackets, joined by a ribbon.
        `color` tints future stations; completed/current retain semantic RGB
        and share its opacity. `current` selects one marker."""
        root = f"{ns}/{id}"
        pts = [_world_point(p) for p in points]
        tint = rgba("plan" if color is None else color, common.pop("alpha", None)) or 1
        opacity = (tint >> 24) / 255.0
        selected = _faded("goal", opacity)
        options = {"frame": frame, **common}
        if not pts:
            self._world_send(self._world_clear(root, ("route", "pillars", "stations", "current", "selection"), options))
            return
        labels = [str(i + 1) for i in range(len(pts))]
        colors = [_faded("white", 0.35 * opacity) if i < current else tint for i in range(len(pts))]
        self.lines(root, "route", pts, colors=colors, **{"style": "ribbon_flow", "width": 0.22, **options})
        pillar_colors = [1 if i == current else c for i, c in enumerate(colors)]
        self.points(root, "pillars", pts, disc=True, colors=pillar_colors,
                    **{"style": "plain", "width": -4.0, "labels": ["" if i == current else l for i, l in enumerate(labels)], **options})
        stations, station_colors = [], []
        for i, p in enumerate(pts):
            a, b = (p, pts[i + 1]) if i + 1 < len(pts) else (pts[max(0, i - 1)], p)
            delta = tuple(b[j] - a[j] for j in range(3))
            length = math.sqrt(sum(v * v for v in delta))
            tangent = tuple(v / length for v in delta) if length > 1e-9 else (1.0, 0.0, 0.0)
            start = len(stations)
            _world_station_mark(stations, p, tangent, _world_f32(0.45))
            station_colors.extend([selected if i == current else colors[i]] * (len(stations) - start))
        self.lines(root, "stations", stations, segments=True, colors=station_colors,
                   **{"style": "plain", "width": -1.25, "color": tint, **options})
        if 0 <= current < len(pts):
            self.points(root, "current", [pts[current]], disc=True,
                        **{"style": "plain", "color": selected, "width": -6.0, "labels": [labels[current]], **options,
                           "props": {"label_selected": "true", "label_icon": "destination", **options.get("props", {})}})
            p, corners = pts[current], []
            polygon = [(p[0] + x, p[1] + y, p[2]) for x, y in ((-.32, -.32), (.32, -.32), (.32, .32), (-.32, .32))]
            _world_corner_outline(corners, polygon)
            self._world_send([self._world_lines(root, "selection", corners, selected, options, -STRUCTURE_RULE_PX * STRUCTURE_CORNER_GAIN)])
        else:
            self._world_send(self._world_clear(root, ("current", "selection"), options))

    def covariance(self, ns: str, id: str, mean, cov, *, sigmas=(1.0, 2.0, 3.0), color="cyan", frame: str = "world",
                   **common) -> None:
        """A covariance at `mean` ((x, y, z[, yaw])): 2×2 (4 values or
        nested lists) as nested draped rings fading outward, 3×3 as nested
        shield ellipsoids, 6×6 (x y z roll pitch yaw) as the ellipsoid
        plus a yaw wedge."""
        root = f"{ns}/{id}"
        flat = [float(v) for row in cov for v in (row if isinstance(row, (list, tuple)) else [row])]
        color = rgba(color, common.pop("alpha", None)) or 1
        c = (float(mean[0]), float(mean[1]), float(mean[2]) if len(mean) > 2 else 0.0)
        fades = [1.0, 0.6, 0.35, 0.2]
        if len(flat) in (3, 4):
            xx, xy, yy = (flat[0], flat[1], flat[3]) if len(flat) == 4 else flat
            a, b, ang = covariance_ellipse(xx, xy, yy)
            strips, colors = [], []
            for k, s in enumerate(sigmas):
                ring = _ellipse(c, s * a, s * b, ang, 72)
                strips.append(ring)
                colors += [_faded(color, fades[min(k, 3)])] * len(ring)
            self.lines(root, "rings", strips=strips, loop=True, style="decal", colors=colors, width=0.04, frame=frame,
                       **common)
            self.points(root, "mean", [c], color=color, radius=-2.0, style="point_glow", frame=frame, **common)
            self.delete(root, "ellipsoid")
            self.delete(root, "yaw")
            return
        if len(flat) not in (9, 36):
            raise ValueError(f"a covariance is 2×2, 3×3 or 6×6, not {len(flat)} values")
        n = 3 if len(flat) == 9 else 6
        c3 = [[flat[n * r + k] for k in range(3)] for r in range(3)]
        sd, q = covariance_axes(c3)
        poses = [c + tuple(q)] * len(sigmas)
        sizes = [(2 * s * sd[0], 2 * s * sd[1], 2 * s * sd[2]) for s in sigmas]
        self.shapes(root, "ellipsoid", "ellipsoid", poses=poses, sizes=sizes, style="shield", frame=frame,
                    colors=[_faded(color, fades[min(k, 3)]) for k in range(len(sigmas))], **common)
        if n == 6:
            yaw_sd = math.sqrt(max(flat[35], 0.0))
            qw, qx, qy, qz = _quat(mean)
            yaw = math.atan2(2 * (qw * qz + qx * qy), 1 - 2 * (qy * qy + qz * qz))
            reach = max(2 * sd[0], 0.6) * 1.6
            self.shapes(root, "yaw", "sector", poses=[c + (yaw,)] * len(sigmas),
                        sizes=[(reach, min(2 * s * yaw_sd, 2 * math.pi), 0.0) for s in sigmas], style="sweep",
                        pulse_hz=-1, colors=[_faded("orange", fades[min(k, 3)] * (color >> 24) / 255.0) for k in range(len(sigmas))],
                        frame=frame, **common)
        else:
            self.delete(root, "yaw")
        self.delete(root, "rings")
        self.delete(root, "mean")

    def ruler(self, ns: str, id: str, a, b, *, spacing: float = 0.0, **common) -> Status | None:
        """A ruler from `a` to `b`: ticks every `spacing` (0 picks a round
        step), end caps and its length."""
        common.setdefault("style", "ruler")
        return self.lines(ns, id, [a, b], spacing=spacing or None, **common)

    # ── detections ──

    def detection3d(self, ns: str, id: str, detections, *, class_colors: dict | None = None, threshold: float = 0.0,
                    only_edge: bool = False, line_width: float = 0.05, show_score: bool = True,
                    frame: str = "world", **common) -> None:
        """3-D detections (vision_msgs look): each a dict with `pose` (x, y,
        z[, yaw] or a quaternion), `size`, `class`, `score`, optional
        `track`, `velocity` and `trail` (past centres). Class-coloured
        hologram boxes as bright as their score (`only_edge` for edges),
        identity and score/track labels, velocity arrows, fading track trails; boxes
        under `threshold` are dimmed."""
        root = f"{ns}/{id}"
        poses, sizes, colors, tags, texts, tag_colors = [], [], [], [], [], []
        arrows_from, arrows_to, arrow_colors, trails, trail_colors = [], [], [], [], []
        options = {"frame": frame, **common}
        for d in detections:
            pose = tuple(d["pose"])
            size = tuple(d.get("size", (1.0, 1.0, 1.0)))
            cls, score = str(d.get("class", "")), float(d.get("score", 1.0))
            base = class_color(cls, class_colors)
            strong = score >= threshold
            alpha = 0.35 + 0.65 * min(max(score, 0.0), 1.0) if strong else 0.22
            poses.append(pose)
            sizes.append(size)
            colors.append(_faded(base, alpha))
            q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
            top = _apply(pose[:3], q, (0.0, 0.0, 0.5 * size[2]))
            tags.append(top)
            detail = f"score {score:.2f}" if show_score else ""
            if d.get("track") is not None:
                detail += (" / " if detail else "") + f"track #{d['track']}"
            texts.append(json.dumps([cls or "object", detail], ensure_ascii=False, separators=(',', ':')))
            tag_colors.append(_faded(base, 1.0 if strong else 0.5))
            v = d.get("velocity")
            if v is not None and sum(x * x for x in v) > 1e-6:
                arrows_from.append(pose[:3])
                arrows_to.append(tuple(p + w for p, w in zip(pose[:3], v)))
                arrow_colors.append(base)
            trail = d.get("trail") or []
            if len(trail) >= 2:
                trails.append(trail)
                trail_colors += [rgba(base)] * len(trail)
        if not poses:
            self._world_send(self._world_clear(root, ("boxes", "tags", "brackets", "velocity", "trails"), options))
            return
        box_options = {"fill": "wire", "style": "plain"} if only_edge else {"style": "holo", "beam_height": -1}
        box_props = {"corner_frame": "true", **options.get("props", {})} if only_edge else options.get("props", {})
        self.shapes(root, "boxes", "box", poses=poses, sizes=sizes, colors=colors,
                    **{**box_options, "width": line_width, **options, "props": box_props})
        self.text(root, "tags", tags, texts, text="", colors=tag_colors,
                  **{**options, "props": {"label_icon": "sensor", "label_parts": "identity-detail-json",
                                         "label_neutral_identity": "true", **options.get("props", {})}})
        self._world_send(self._world_clear(root, ["brackets"], options))
        if arrows_from:
            self.arrows(root, "velocity", arrows_from, tips=arrows_to, colors=arrow_colors,
                        **{"style": "arrow_glow", **options})
        else:
            self._world_send(self._world_clear(root, ["velocity"], options))
        if trails:
            self.lines(root, "trails", strips=trails, colors=trail_colors,
                       **{"width": -2.0, "fade": 0.02, "style": "plain", **options})
        else:
            self._world_send(self._world_clear(root, ["trails"], options))

    def detection2d(self, ns: str, id: str, camera_pose, boxes, *, hfov: float, vfov: float, image=(640, 480),
                    classes=None, scores=None, keypoints=None, skeleton=None, depth: float = 2.0,
                    frame: str = "world", **common) -> None:
        """2-D detections on a camera's image plane `depth` metres out along
        +x of `camera_pose`, with a faint wedge from the camera to each box:
        `boxes` are (cx, cy, w, h) pixels of an `image` (w, h) seen with
        `hfov` × `vfov` radians; `keypoints` (u, v) pixels joined by
        `skeleton` index pairs."""
        root = f"{ns}/{id}"
        pose = tuple(camera_pose)
        q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
        hw, hh = depth * math.tan(0.5 * hfov), depth * math.tan(0.5 * vfov)

        def plane(u, v):
            return _apply(pose[:3], q, (depth, hw * (1 - 2 * u / image[0]), hh * (1 - 2 * v / image[1])))

        rects, rect_colors, wedge, wedge_colors, tags, texts, tag_colors = [], [], [], [], [], [], []
        for k, (cx, cy, w, h) in enumerate(boxes):
            cls = classes[k] if classes and k < len(classes) else ""
            base = class_color(cls)
            corners = [plane(cx - w / 2, cy - h / 2), plane(cx + w / 2, cy - h / 2), plane(cx + w / 2, cy + h / 2),
                       plane(cx - w / 2, cy + h / 2)]
            rects.append(corners)
            rect_colors += [rgba(base)] * 4
            for i in range(4):
                wedge += [pose[:3], corners[i], corners[(i + 1) % 4]]
            wedge_colors += [rgba(base, 0.07)] * 12
            tags.append(corners[0])
            detail = f"score {scores[k]:.2f}" if scores and k < len(scores) else ""
            texts.append(json.dumps([cls or "object", detail], ensure_ascii=False, separators=(',', ':')))
            tag_colors.append(rgba(base))
        options = {"frame": frame, **common}
        self.lines(root, "boxes", strips=rects, loop=True, colors=rect_colors,
                   **{"width": -STRUCTURE_RULE_PX, "style": "plain", **options,
                      "props": {"corner_frame": "true", **options.get("props", {})}})
        self.triangles(root, "wedges", wedge, colors=wedge_colors, **options)
        self.text(root, "tags", tags, texts, text="", colors=tag_colors,
                  **{**options, "props": {"label_icon": "sensor", "label_parts": "identity-detail-json",
                                         "label_neutral_identity": "true", **options.get("props", {})}})
        if keypoints:
            pts = [plane(u, v) for u, v in keypoints]
            self.points(root, "keypoints", pts, radius=-3.0,
                        **{"color": "yellow", "style": "point_glow", "frame": frame, **common})
            if skeleton:
                segs = [p for a, b in skeleton for p in (pts[a], pts[b])]
                self.lines(root, "skeleton", segs, segments=True,
                           **{"color": "yellow", "width": -1.5, "frame": frame, **common})

    # ── sensors ──

    def cloud(self, ns: str, id: str, points, *, values=None, colormap="turbo", size: float = -1.5,
              shape: str = "splat", quant: float = 0.0, decay_time: float = 0.0, axis: str | None = None,
              **common) -> Status | None:
        """A point cloud as flat splats (RViz Points / Squares / Spheres /
        Boxes): `values` (intensity; or `axis` "x", "y", "z" for height
        colouring) through `colormap`, `size` > 0 metres or < 0 pixels,
        `quant` > 0 sends i16 positions and u8 values (7 B a point),
        `decay_time` keeps that many seconds of clouds."""
        pts = [tuple(p) + (0.0,) * (3 - len(p)) for p in points]
        if axis is not None:
            values = [p["xyz".index(axis)] for p in pts]
        if decay_time > 0:
            self._decay = getattr(self, "_decay", 0) + 1
            id = f"{id}/{self._decay % 64}"
            common["lifetime"] = decay_time
        if shape == "box":
            return self.shapes(ns, id, "box", pts, size=abs(size) * 2 if size > 0 else 0.05, values=values,
                               colormap=colormap, shaded=True, style="plain", **common)
        common.setdefault("style", "point_glow" if shape == "sphere" else "splat")
        common["width"] = 2.0 * size
        spec = self._spec(_POINTS, ns, id, sub=POINT_STYLE["square" if shape == "square" else "disc"],
                          colormap=colormap if values is not None else None, **common)
        self._set_points(spec, pts, quant)
        if values is not None:
            vals = [float(v) for v in values]
            if quant > 0:
                lo, hi = min(vals), max(vals)
                hi = hi if hi > lo else lo + 1.0
                spec.cells = bytes(round(255 * (v - lo) / (hi - lo)) for v in vals)
                spec.header.update(vmin=lo, vmax=hi)
            else:
                spec.values = vals
        return self.send(spec)

    def scan(self, ns: str, id: str, ranges, *, angle_min: float, angle_increment: float, range_max: float = 0.0,
             pose=(0.0, 0.0, 0.0), frame: str = "world", fan: bool = True, color="sensor", **common) -> None:
        """A planar scan: its hits as splats (sent as u16 millimetres, 6144
        beams a chunk) over a faint sweeping fan."""
        root = f"{ns}/{id}"
        color = rgba(color, common.pop("alpha", None)) or 1
        mm = [NO_RETURN if not (r == r) or r < 0 or r == float("inf") else min(int(round(r * 1000)), NO_RETURN - 1)
              for r in ranges]
        reach = range_max or max((r for r in ranges if r == r and r != float("inf")), default=1.0)
        spec = self._spec(_POINTS, root, "hits", pose=pose, frame=frame,style="splat", color=color,
                          width=-3.0, props={"angle_min": angle_min, "angle_inc": angle_increment, "range_max": reach},
                          **common)
        spec.ranges = mm
        self.send(spec)
        if fan:
            span = min(abs(angle_increment) * max(len(ranges) - 1, 0), 2 * math.pi)
            mid = angle_min + 0.5 * angle_increment * max(len(ranges) - 1, 0)
            self.shapes(root, "fan", "sector", poses=[(0.0, 0.0, 0.0, mid)], sizes=[(reach, span, 0.0)], pose=pose,
                        style="sweep", color=_faded(color, 0.45), spacing=reach / 4, frame=frame,**common)
        else:
            self.delete(root, "fan")

    def sweep(self, ns: str, id: str, pose=(0.0, 0.0, 0.0), *, radius: float = 5.0, angle: float = 1.0,
              inner: float = 0.0, **common) -> Status | None:
        """A sensor's field of view on the ground: a sweep wedge `angle`
        radians wide from `inner` to `radius` metres along +x of `pose`."""
        common.setdefault("style", "sweep")
        return self.shapes(ns, id, "sector", poses=[tuple(pose) if len(pose) > 3 else tuple(pose) + (0.0,) * (4 - len(pose))],
                           sizes=[(radius, angle, inner)], **common)

    def range_cone(self, ns: str, id: str, pose, *, fov: float, min_range: float, max_range: float,
                   range: float | None = None, frame: str = "world", **common) -> None:
        """A range sensor's cone: a sweep wedge with the reading as a bright arc."""
        root = f"{ns}/{id}"
        self.shapes(root, "cone", "sector", poses=[(0.0, 0.0, 0.0)], sizes=[(max_range, fov, min_range)], pose=pose,
                    **{"style": "sweep", "color": "sensor", "frame": frame, **common})
        if range is not None and min_range <= range <= max_range:
            arc = [(range * math.cos(a), range * math.sin(a), 0.0)
                   for a in (-fov / 2 + fov * k / 24 for k in range_(25))]
            self.lines(root, "reading", arc, pose=pose,
                       **{"color": "orange", "style": "tube_pulse", "width": 0.03, "fade": -1, "frame": frame, **common})
        else:
            self.delete(root, "reading")

    def camera_frustum(self, ns: str, id: str, pose, *, hfov: float | None = None, vfov: float | None = None, K=None,
                       image=(640, 480), near: float = 0.3, far: float = 4.0, **common) -> Status | None:
        """A camera's frustum along +x of `pose` out to `far` (fields of view,
        or `K` = (fx, fy, cx, cy) over `image`), glowing edges and a faint
        image plane at `near` (Foxglove CameraCalibration, rerun Pinhole)."""
        if K is not None:
            hfov = 2 * math.atan(0.5 * image[0] / K[0])
            vfov = 2 * math.atan(0.5 * image[1] / K[1])
        common.setdefault("style", "sweep")
        common.setdefault("color", "sensor")
        return self.shapes(ns, id, "frustum", poses=[(0.0, 0.0, 0.0)], sizes=[(hfov or 1.0, vfov or 0.75, far)],
                           pose=pose, props={"near": near}, **common)

    def particles(self, ns: str, id: str, poses, weights=None, *, min_length: float = 0.1, max_length: float = 0.5,
                  glyph: str = "chevron", color="cyan", **common) -> Status | None:
        """A particle cloud (nav2 ParticleCloud): additive flat chevrons (or
        arrows, axes), each between `min_length` and `max_length` long and
        as bright as its weight."""
        n = len(poses)
        weights = list(weights) if weights is not None else [1.0] * n
        top = max(max(weights, default=0.0), 1e-9)
        weights = [min(max(weights[i] / top, 0.0), 1.0) if i < len(weights) else 1.0 for i in range(n)]
        tint = rgba(color, common.pop("alpha", None)) or 1
        lengths = [min_length + (max_length - min_length) * w for w in weights]
        colors = [_faded(tint, 0.25 + 0.75 * w) for w in weights]
        if glyph == "axes":
            spec = self._spec(_AXES, ns, id, **common)
            spec.instances = _instances(poses, [(l, l, l) for l in lengths])
            self._per_item(spec, colors=colors)
            return self.send(spec)
        common.setdefault("style", "arrow_glow")
        if glyph == "chevron":
            common.setdefault("flat", True)
            common.setdefault("additive", True)
            common.setdefault("width", 0.02)
        return self.arrows(ns, id, poses=poses, lengths=lengths, colors=colors, chevron=glyph == "chevron", **common)

    def wrench(self, ns: str, id: str, pose, force, torque=(0.0, 0.0, 0.0), *, scale: float = 0.1,
               frame: str = "world", **common) -> None:
        """A wrench at `pose`: the force an arrow (vermillion), the torque an
        arc with an arrowhead about its axis (yellow); `scale` metres per
        newton."""
        root = f"{ns}/{id}"
        common.pop("color", None)
        self.arrows(root, "force", [(0.0, 0.0, 0.0)], tips=[tuple(f * scale for f in force)], pose=pose, color="red",
                    style="arrow_glow", width=0.03, frame=frame,**common)
        mag = math.sqrt(sum(t * t for t in torque))
        if mag > 1e-6:
            axis = tuple(t / mag for t in torque)
            helper = (0.0, 0.0, 1.0) if abs(axis[2]) < 0.9 else (1.0, 0.0, 0.0)
            u = _normalize(_cross(helper, axis))
            v = _cross(axis, u)
            r = 0.25 + 0.05 * min(mag * scale, 4.0)
            sweep = (0.6 + 0.4 * min(mag * scale, 1.0)) * 1.5 * math.pi
            pts = [tuple(u[i] * math.cos(t) * r + v[i] * math.sin(t) * r for i in range_(3))
                   for t in (sweep * k / 36 for k in range_(37))]
            self.lines(root, "torque", pts, pose=pose, color="yellow", style="arrow_glow", arrow_tip=True, width=0.025,
                       frame=frame,**common)
        else:
            self.delete(root, "torque")

    def contact(self, ns: str, id: str, point, normal, force: float = 1.0, *, scale: float = 0.05,
                frame: str = "world", **common) -> None:
        """A contact: a small orb at `point` and its normal force as an arrow."""
        root = f"{ns}/{id}"
        n = _normalize(tuple(normal))
        length = max(abs(force) * scale, 0.15)
        self.points(root, "point", [point], radius=0.08, **{"style": "orb", "color": "orange", "frame": frame, **common})
        self.arrows(root, "normal", [point], vectors=[tuple(c * length for c in n)],
                    **{"style": "arrow_glow", "color": "orange", "width": 0.02, "frame": frame, **common})

    # ── field robotics ──

    def field(self, ns: str, id: str, boundary, *, holes=(), color="green", fence: float = 0.3,
              frame: str = "world", **common) -> None:
        """A field: its boundary (and `holes`) as a ground decal fenced by a
        wall of light `fence` metres tall."""
        root = f"{ns}/{id}"
        self.polygon(root, "area", [boundary, *holes], color=color, frame=frame, **common)
        ring = [tuple(p) for p in boundary] + [tuple(boundary[0])]
        self.lines(root, "fence", ring, style="light_wall", color=color, beam_height=fence, fade=-1, frame=frame,
                   **common)

    def headland(self, ns: str, id: str, outer, inner, **common) -> Status | None:
        """A headland: the band between a field's boundary and its inner boundary."""
        common.setdefault("color", "orange")
        return self.polygon(ns, id, [outer, inner], **common)

    def swaths(self, ns: str, id: str, swaths, *, width: float = 3.0, done: int = 0, color=None, frame: str = "world",
               **common) -> None:
        """Swaths (point lists) `width` wide (the coverage width), chevrons
        in the driving direction; the first `done` fade to history."""
        root = f"{ns}/{id}"
        tint = rgba("plan" if color is None else color, common.pop("alpha", None)) or 1
        spacing = max(width * 1.5, 0.8)
        todo, old = list(swaths[done:]), list(swaths[:done])
        if todo:
            self.lines(root, "todo", strips=todo, **{"style": "ribbon_flow", "color": tint, "width": width,
                       "flow_speed": 1.2, "spacing": spacing, "frame": frame, **common})
        else:
            self.delete(root, "todo")
        if old:
            self.lines(root, "done", strips=old, **{"style": "ribbon_flow", "color": _faded("history", (tint >> 24) / 255.0),
                       "width": width, "flow_speed": -1, "spacing": spacing, "frame": frame, **common})
        else:
            self.delete(root, "done")

    def turns(self, ns: str, id: str, paths, *, width: float = 3.0, **common) -> Status | None:
        """Headland turns: narrower dashed ribbons."""
        spacing = max(width * 1.5, 0.8)
        return self.lines(ns, id, strips=paths, **{"style": "ribbon_flow", "color": "cyan", "width": 0.5 * max(width, 0.2),
                          "dashed": True, "flow_speed": 1.0, "spacing": spacing, **common})

    def ab_line(self, ns: str, id: str, a, b, *, extend: float = 50.0, frame: str = "world", **common) -> None:
        """An AB line through `a` and `b` run on `extend` metres both ways,
        dashed and long, its ends named A and B."""
        root = f"{ns}/{id}"
        a3, b3 = tuple(a) + (0.0,) * (3 - len(a)), tuple(b) + (0.0,) * (3 - len(b))
        d = _normalize(tuple(q - p for p, q in zip(a3, b3)))
        start = tuple(p - c * extend for p, c in zip(a3, d))
        end = tuple(p + c * extend for p, c in zip(b3, d))
        tint = rgba(common.pop("color", "yellow"), common.pop("alpha", None)) or 1
        self.lines(root, "line", [start, a3, b3, end], **{"style": "plain", "color": _faded(tint, 0.85), "width": -2.0,
                   "dash": 2.0, "dashed": True, "flat_caps": True, "frame": frame, **common})
        self.points(root, "ends", [a3, b3], **{"color": tint, "radius": -3.0, "style": "point_glow", "labels": ["A", "B"],
                    "frame": frame, **common})

    def coverage(self, ns: str, id: str, counts, cols: int, rows: int, *, cell: float = 0.5, origin=(0.0, 0.0, 0.0),
                 max_passes: int = 4, **common) -> Status | None:
        """Coverage: a u8 pass count per cell (0 left clear); update tiles
        with `grid_patch`."""
        return self.grid(ns, id, counts, cols, rows, cell=cell, origin=origin, colormap="viridis", vmin=0,
                         vmax=max_passes, unknown=0, **common)

    def implement(self, ns: str, id: str, width: float, sections_on, *, depth: float = 0.6, frame: str = "world",
                  **common) -> Status | None:
        """An implement `width` wide and `depth` deep behind its hitch (use a
        link frame such as "tractor/hitch"), its sections lit when on
        (AgOpenGPS section control)."""
        n = max(len(sections_on), 1)
        w = width / n
        centers = [(-0.5 * depth, -0.5 * width + w * (k + 0.5), 0.02) for k in range_(n)]
        colors = [rgba("green") if on else rgba("grey", 0.5) for on in sections_on]
        return self.shapes(ns, id, "rect", centers, sizes=[(depth, 0.92 * w, 0.0)] * n, colors=colors, style="decal",
                           width=0.03, frame=frame,**common)

    def zone(self, ns: str, id: str, rings, *, kind: str = "keepout", **common) -> Status | None:
        """A zone on the ground: `kind` keepout (purple decal), slow (orange
        decal) or stop (hazard stripes)."""
        style, color = {"keepout": ("decal", "purple"), "slow": ("decal", "orange"), "stop": ("alert", "danger")}[kind]
        common.setdefault("color", color)
        return self.polygon(ns, id, rings, style=style, **common)

    # ── uncertainty, paths ──

    def odometry(self, ns: str, id: str, pose, *, velocity=(0.0, 0.0, 0.0), cov=None, frame: str = "world",
                 **common) -> None:
        """Odometry: a chevron at the pose, its velocity (body frame) as an
        arrow, the covariance under it. None or empty covariance removes its components."""
        root = f"{ns}/{id}"
        pose = tuple(pose)
        self.pose_array(root, "pose", [pose], length=0.8, **{"color": "plan", "frame": frame, **common})
        q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
        v = _apply((0.0, 0.0, 0.0), q, tuple(velocity))
        if sum(x * x for x in v) > 1e-6:
            self.arrows(root, "twist", [pose[:3]], vectors=[v], **{"style": "arrow_glow", "color": "prediction", "frame": frame,
                        **common})
        else:
            self.delete(root, "twist")
        cov = tuple(cov) if cov is not None else ()
        if cov:
            self.covariance(root, "cov", pose, cov, sigmas=(1.0, 2.0), **{"color": "plan", "frame": frame, **common})
        else:
            for component in ("rings", "mean", "ellipsoid", "yaw"):
                self.delete(f"{root}/cov", component, wait=common.get("wait"))

    def band(self, ns: str, id: str, points, sigma, **common) -> Status | None:
        """A path as a pulsing tube as wide as `sigma` at each point."""
        common.setdefault("style", "tube_pulse")
        common["color"] = _faded(rgba(common.get("color", "prediction"), common.pop("alpha", None)), 0.6)
        return self.lines(ns, id, points, radii=sigma, fade=-1, **common)

    def rollouts(self, ns: str, id: str, strips, costs=None, collisions=None, *, stride: int = 1, **common) -> None:
        """Sampled rollouts as hairlines tinted by cost (low green, high
        vermillion; nav2 MPPI keeps every `stride`-th), colliding ones magenta."""
        root = f"{ns}/{id}"
        common.pop("color", None)
        free, free_cost, hit = [], [], []
        for k in range_(0, len(strips), max(stride, 1)):
            if collisions and k < len(collisions) and collisions[k]:
                hit.append(strips[k])
            else:
                free.append(strips[k])
                free_cost.append(costs[k] if costs else 0.0)
        if free:
            lo, hi = (min(free_cost), max(free_cost)) if free_cost else (0.0, 1.0)
            self.lines(root, "samples", strips=free, style="hairline", values=free_cost, colormap="rdylgn",
                       vmin=hi, vmax=lo if hi > lo else lo - 1.0, color="white", alpha=0.12, **common)
        else:
            self.delete(root, "samples")
        if hit:
            self.lines(root, "collisions", strips=hit, style="hairline", color="magenta", alpha=0.35, **common)
        else:
            self.delete(root, "collisions")

    def path(self, ns: str, id: str, points, *, pose_markers: float = 0.0, marker: str = "axes", **common) -> None:
        """A path as a flowing ribbon with an axes or arrow marker every
        `pose_markers` metres (RViz Path pose style)."""
        root = f"{ns}/{id}"
        pts = [tuple(p) + (0.0,) * (3 - len(p)) for p in points]
        self.lines(root, "path", pts, style="ribbon_flow", **common)
        poses, along, nxt = [], 0.0, 0.0
        for a, b in zip(pts, pts[1:]):
            d = tuple(q - p for p, q in zip(a, b))
            seg = math.sqrt(sum(c * c for c in d))
            while pose_markers > 0 and nxt <= along + seg:
                t = (nxt - along) / seg if seg > 1e-9 else 0.0
                poses.append(tuple(p + c * t for p, c in zip(a, d)) + (math.atan2(d[1], d[0]),))
                nxt += pose_markers
            along += seg
        if poses:
            self.pose_array(root, "poses", poses, glyph="axes" if marker == "axes" else "arrow", length=0.5, **common)
        else:
            self.delete(root, "poses")

    def ee_path(self, ns: str, id: str, points, **common) -> Status | None:
        """An end effector's path (moveit publishTrajectoryLine) as a pulsing tube."""
        common.setdefault("style", "tube_pulse")
        return self.lines(ns, id, points, **common)

    def graph(self, ns: str, id: str, nodes, edges, *, labels=None, color="cyan", frame: str = "world",
              **common) -> None:
        """A graph (RRT, PRM): nodes as glowing points, edges as hairlines."""
        root = f"{ns}/{id}"
        color = rgba(color, common.pop("alpha", None)) or 1
        segs = [p for a, b in edges for p in (nodes[a], nodes[b])]
        self.points(root, "nodes", nodes, style="point_glow", color=color, radius=-2.0, labels=labels, frame=frame,
                    **common)
        self.lines(root, "edges", segs, segments=True, style="hairline", color=_faded(color, 0.35), width=-1.2,
                   frame=frame,**common)

    def voxels(self, ns: str, id: str, centers, *, size: float = 0.1, values=None, colormap="turbo",
               holo: bool = False, **common) -> Status | None:
        """A voxel grid: a box per centre `size` across, coloured by `values`."""
        if holo:
            common.setdefault("beam_height", -1)
        centers = list(centers)
        return self.shapes(ns, id, "box", centers, sizes=[(size, size, size)] * len(centers), values=values, colormap=colormap if values else None,
                           style="holo" if holo else "plain", shaded=not holo, **common)

    def mesh(self, ns: str, id: str, asset: str | None = None, *, vertices=None, faces=None, poses=None,
             scale: float = 1.0, **common) -> Status | None:
        """A mesh: `asset` (an OBJ or STL path the viewer
        opens) at `pose` or each of `poses`, or `vertices` and `faces`."""
        if asset is None:
            tris = [vertices[i] for f in faces for i in f]
            return self.triangles(ns, id, tris, shaded=True, **common)
        spec = self._spec(_MESH, ns, id, scale=(scale, scale, scale), props={"asset": asset}, **common)
        if poses is not None:
            spec.instances = _instances(poses, [(scale,) * 3] * len(poses))
        return self.send(spec)

    def ghost(self, ns: str, id: str, machine: str, poses, *, joints=None, values=None, color="cyan", colors=None,
              fade: float = 0.15, **common) -> Status | None:
        """See-through copies of `machine` at `poses` (MoveIt trail); `joints`
        names with a list of `values` per pose."""
        common.setdefault("style", "ghost")
        spec = self._spec(_GHOST, ns, id, color=color, fade=fade, props={"machine": machine,
                          "joints": ",".join(joints) if joints else None}, **common)
        spec.instances = _instances(poses, [(1.0, 1.0, 1.0)] * len(poses))
        self._per_item(spec, colors=colors, alpha=common.get("alpha"))
        if joints and values:
            spec.values = [float(v) for row in values for v in row]
        return self.send(spec)

    def ghost_trail(self, ns: str, id: str, machine: str, poses, *, step: int = 1, joints=None, values=None, colors=None,
                    **common) -> Status | None:
        """Every `step`-th state of a trajectory as a ghost, fading to the past."""
        idx = list(range_(0, len(poses), max(step, 1)))
        if poses and idx[-1] != len(poses) - 1:
            idx.append(len(poses) - 1)
        return self.ghost(ns, id, machine, [poses[i] for i in idx], joints=joints,
                          values=[values[i] for i in idx] if values else None,
                          colors=[colors[i] if i < len(colors) else None for i in idx] if colors is not None else None, **common)

    # ── charts and annotations ──

    def panel(self, ns: str, id: str, anchor, kind: str = "table", *, values=None, labels=None, title: str = "",
              unit: str = "", lo: float | None = None, hi: float | None = None, text: str | None = None,
              rows=None, color=None, colors=None, frame: str = "world", **common) -> Status | None:
        """An in-world chart at `anchor`: sparkline (`values`, fed with
        `extend(kind="panel")`), bars (`values`, `labels`), gauge (one
        value between `lo` and `hi`), chip (`text`) or table (`rows` as
        (key, value) pairs)."""
        props = {"title": title or None, "unit": unit or None, "min": lo, "max": hi, "text": text,
                 "rows": "\n".join(f"{k}={v}" for k, v in rows) if rows else None}
        props.update(common.pop("props", None) or {})
        spec = self._spec(_PANEL, ns, id, pose=tuple(anchor), sub=PANEL[kind], color=color, frame=frame, props=props,
                          **common)
        spec.values = [float(v) for v in values] if values is not None else []
        spec.labels = [str(s) for s in labels] if labels else []
        self._per_item(spec, colors=colors, alpha=common.get("alpha"))
        spec.header["flags"] &= ~DRAW_FLAG["labels"]
        return self.send(spec)

    def sparkline(self, ns: str, id: str, anchor, value: float, *, max_points: int = 120, title: str = "",
                  unit: str = "", **common) -> Status | None:
        """Adds `value` to a sparkline at `anchor` (rerun Scalars)."""
        props = {"title": title or None, "unit": unit or None, "max_points": int(max_points), **(common.pop("props", None) or {})}
        spec = self._spec(_PANEL, ns, id, pose=tuple(anchor), sub=PANEL["sparkline"], props=props, **common)
        spec.values = [float(value)]
        return self._deliver((ns, id), [spec.message(op=DRAW_OP["extend"])], False, spec.wait)

    def gauge(self, ns: str, id: str, anchor, value: float, *, lo: float = 0.0, hi: float = 100.0, **kw):
        """A gauge (battery, fuel) filled to `value` between `lo` and `hi`."""
        return self.panel(ns, id, anchor, "gauge", values=[value], lo=lo, hi=hi, **kw)

    def chip(self, ns: str, id: str, anchor, text: str, *, color="plan", **kw):
        """A state chip (rerun StateChange) naming `text` in `color`."""
        return self.panel(ns, id, anchor, "chip", text=text, color=color, **kw)

    def badge(self, ns: str, id: str, machine: str, text: str, *, color="plan", height: float = 2.0,
              **common) -> Status | None:
        """An annotation `height` metres over `machine`'s base link."""
        props = {"text": text, "badge": 1, **(common.pop("props", None) or {})}
        spec = self._spec(_PANEL, ns, id, pose=(0.0, 0.0, height), sub=PANEL["chip"], color=color, frame=machine,
                          props=props, **common)
        return self.send(spec)

    def table(self, ns: str, id: str, anchor, rows, *, title: str = "", **kw):
        """A key/value table at `anchor`."""
        return self.panel(ns, id, anchor, "table", rows=rows, title=title, **kw)

    def bars(self, ns: str, id: str, anchor, values, labels=None, *, title: str = "", **kw):
        """A bar chart (rerun BarChart) at `anchor`."""
        return self.panel(ns, id, anchor, "bars", values=values, labels=labels, title=title, **kw)

    def legend(self, ns: str, id: str, colormap: str, vmin: float, vmax: float, *, title: str = "", unit: str = "",
               **common) -> Status | None:
        """A screen legend, or world-anchored with props={"world_anchor": "true"} and pose."""
        props = {"title": title or None, "unit": unit or None, **(common.pop("props", None) or {})}
        spec = self._spec(_PANEL, ns, id, sub=PANEL["legend"], colormap=colormap, vmin=vmin, vmax=vmax,
                          props=props, **{"drape": False, **common})
        return self.send(spec)

    def compass(self, ns: str, id: str, at=(0.0, 0.0, 0.0), *, size: float = 4.0, color="white", frame: str = "world",
                **common) -> None:
        """A compass rose `size` metres across, north in goal yellow."""
        root = f"{ns}/{id}"
        tint = rgba(color, common.pop("alpha", None)) or 1
        north = _faded("goal", (tint >> 24) / 255.0)
        x, y, z = tuple(at) + (0.0,) * (3 - len(at))
        r = 0.5 * size
        self.lines(root, "ring", _ellipse((x, y, z), r, r, 0.0, 72), loop=True, style="decal", color=tint,
                   width=0.03, frame=frame, **common)
        spokes, colors, tips = [], [], []
        for k, (dx, dy) in enumerate(((0, 1), (1, 0), (0, -1), (-1, 0))):
            inner = 0.0 if k == 0 else 0.25 * r
            spokes.append([(x + dx * inner, y + dy * inner, z), (x + dx * r * 1.15, y + dy * r * 1.15, z)])
            colors += [north if k == 0 else tint] * 2
            tips.append((x + dx * r * 1.45, y + dy * r * 1.45, z + 0.05))
        self.lines(root, "spokes", strips=spokes, style="decal", colors=colors, width=0.05, frame=frame, **common)
        self.text(root, "letters", tips, ["N", "E", "S", "W"], colors=[north, tint, tint, tint], size=15,
                  frame=frame, **common)

    def scale_bar(self, ns: str, id: str, at, length: float, **common) -> Status | None:
        """A scale bar `length` metres long from `at` along x."""
        x, y, z = tuple(at) + (0.0,) * (3 - len(at))
        return self.ruler(ns, id, (x, y, z), (x + length, y, z), spacing=length / 5, **common)

    def callout(self, ns: str, id: str, anchor, text: str, *, offset=(0.0, 0.0, 1.5), color="plan",
                frame: str = "world", **common) -> None:
        """A callout: `text` at `anchor + offset` on a leader line."""
        root = f"{ns}/{id}"
        a = tuple(anchor) + (0.0,) * (3 - len(anchor))
        color = rgba(color, common.pop("alpha", None)) or 1
        at = tuple(p + o for p, o in zip(a, offset))
        options = {"frame": frame, **common}
        self.lines(root, "leader", [a, at], **{"color": color, "width": -1.5, "style": "plain", **options})
        marker = []
        _world_open_outline(marker, [(a[0] + x, a[1] + y, a[2]) for x, y in ((0, .12), (.12, 0), (0, -.12), (-.12, 0))])
        self._world_send([self._world_lines(root, "anchor", marker, color, options)])
        spec = self._spec(_TEXT, root, "text", **{"color": _faded("white", (color >> 24) / 255.0), "width": -16.0, **options,
                          "props": {"text": text, "accent": rgba(color), **options.get("props", {})}})
        spec.points = list(at)
        self.send(spec)

    def _world_send(self, children) -> Status | None:
        status = None
        for spec, op in children:
            status = self.send(spec, op)
            if status is not None and not status.ok:
                break
        return status

    def _world_clear(self, root, ids, common):
        options = {k: v for k, v in common.items() if k != "pose"}
        return [(self._spec(0, root, id, **options), DRAW_OP["delete"]) for id in ids]

    def _world_lines(self, root, id, points, color, common, width=-1.25):
        if not points:
            return self._world_clear(root, [id], common)[0]
        tint = rgba(color, common.get("alpha")) or 1
        options = {"style": "plain", "width": width, **common, "color": tint, "sub": LINE_MODE["list"]}
        spec = self._spec(_LINES, root, id, **options)
        spec.points = _flat_points(points)
        return spec, DRAW_OP["upsert"]

    def _world_label(self, root, at, label, color, common):
        if not label or not _world_finite(at):
            return self._world_clear(root, ["label"], common)[0]
        options = {"style": "auto", "width": -16.0, **common, "color": rgba(color, common.get("alpha")) or 1}
        options["props"] = {**common.get("props", {}), "text": str(label), "drape_anchor": "true"}
        spec = self._spec(_TEXT, root, "label", **options)
        spec.points = [_world_f32(v) for v in at]
        return spec, DRAW_OP["upsert"]

    def terrain_marks(self, ns: str, id: str, points, *, glyph: str = "square", size: float = 0.4,
                      color="plan", **common) -> Status | None:
        """Batched world XY glyphs, full span `size` metres; meaning and color are caller-supplied.
        Processes at most 65,536 samples; skips nonfinite points and clears invalid-size marks."""
        if glyph not in TERRAIN_GLYPHS:
            raise ValueError(f"unknown terrain glyph {glyph!r}; choose {', '.join(TERRAIN_GLYPHS)}")
        root, vertices = f"{ns.rstrip('/')}/{id}", []
        size = _world_f32(size)
        if _world_positive(size):
            h = size * 0.5
            polygon = {"square": ((-h, -h), (h, -h), (h, h), (-h, h)),
                       "triangle": ((0.0, h), (-h, -h), (h, -h)),
                       "diamond": ((0.0, h), (-h, 0.0), (0.0, -h), (h, 0.0))}.get(glyph, ())
            for value in islice(points, _WORLD_MAX_POINTS):
                p = _world_point(value)
                if not _world_finite(p):
                    continue
                at = lambda xy: (p[0] + xy[0], p[1] + xy[1], p[2])
                if glyph == "cross":
                    _world_segment(vertices, at((-h, -h)), at((h, h)))
                    _world_segment(vertices, at((-h, h)), at((h, -h)))
                else:
                    for a, b in zip(polygon, (*polygon[1:], polygon[0])):
                        _world_segment(vertices, at(a), at(b))
        return self._world_send([self._world_lines(root, "marks", vertices, color, common)])

    def route_stations(self, ns: str, id: str, points, *, spacing: float = 1.0, size: float = 0.3,
                       color="plan", **common) -> Status | None:
        """A 3D route with equal arc-length transverse bars and forward chevrons.
        Nonfinite points break the route; at most 65,536 inputs and 4,096 stations are used."""
        root, route, marks = f"{ns.rstrip('/')}/{id}", [], []
        spacing, size = _world_f32(spacing), _world_f32(size)
        previous, remaining, count = None, 0.0, 0
        for value in islice(points, _WORLD_MAX_POINTS):
            b = _world_point(value)
            a, previous = previous, b
            if a is None:
                continue
            if not _world_finite(a) or not _world_finite(b):
                remaining = 0.0
                continue
            _world_segment(route, a, b)
            delta = tuple(b[i] - a[i] for i in range(3))
            length = math.sqrt(sum(v * v for v in delta))
            if length <= 1e-9 or not _world_positive(spacing) or not _world_positive(size) or count >= _WORLD_MAX_STATIONS:
                continue
            tangent = tuple(v / length for v in delta)
            while remaining <= length and count < _WORLD_MAX_STATIONS:
                centre = tuple(a[i] + remaining * tangent[i] for i in range(3))
                _world_station_mark(marks, centre, tangent, size)
                remaining += spacing
                count += 1
            remaining -= length
        return self._world_send([self._world_lines(root, "route", route, color, common, -1.0),
                                 self._world_lines(root, "stations", marks, color, common)])

    def destination(self, ns: str, id: str, pose=(0.0, 0.0, 0.0), *, radius: float = 1.0,
                    label: str = "", color="plan", **common) -> Status | None:
        """A broken XY ring and upright open XZ diamond at a world/link pose; radius is metres."""
        root, pose, radius = f"{ns.rstrip('/')}/{id}", _world_pose(pose), _world_f32(radius)
        if pose is None or not _world_positive(radius) or not _world_finite((2.5 * radius,)):
            return self._world_send(self._world_clear(root, ("rings", "marker", "leader", "label"), common))
        options = {**common, "pose": pose, "props": {**common.get("props", {}), "drape_pose_height": "true"}}
        anchored = {**options, "props": {**options["props"], "drape_anchor": "true"}}
        rings, marker = [], []
        for quadrant in range(4):
            for i in range(12):
                def point(k):
                    angle = quadrant * math.pi * 0.5 + 0.14 + k / 12 * (math.pi * 0.5 - 0.28)
                    return (radius * math.cos(angle), radius * math.sin(angle), 0.0)
                _world_segment(rings, point(i), point(i + 1))
        h = radius * 0.5
        diamond = ((0.0, 0.0, 2 * radius + h), (h, 0.0, 2 * radius),
                   (0.0, 0.0, 2 * radius - h), (-h, 0.0, 2 * radius))
        _world_open_outline(marker, diamond)
        leader = [(0.0, 0.0, 0.0), (0.0, 0.0, 1.35 * radius)]
        if label:
            _world_segment(leader, (0.58 * radius, 0.0, 2 * radius), (0.7 * radius, 0.0, 2 * radius))
        return self._world_send([self._world_lines(root, "rings", rings, color, options),
                                 self._world_lines(root, "marker", marker, color, anchored),
                                 self._world_lines(root, "leader", leader, color, anchored),
                                 self._world_label(root, (0.75 * radius, 0.0, 2 * radius), label, color, anchored)])

    def machine_ring(self, ns: str, id: str, centre=(0.0, 0.0, 0.0), *, radius: float = 2.0,
                     mark: str = "selected", color=None, **common) -> Status | None:
        """A machine's ring on the ground, its rim `radius` metres from `centre` (z its height):
        one `ring` style soft off-white band in four arcs with faint gaps, fainter for `hover`,
        closed with a thin outer line when `following`. No color takes the mark's own."""
        if mark not in RING_MARKS:
            raise ValueError(f"unknown ring mark {mark!r}; choose {', '.join(RING_MARKS)}")
        root, centre, radius = f"{ns.rstrip('/')}/{id}", _world_point(centre), _world_f32(radius)
        if not _world_finite(centre) or not math.isfinite(radius) or radius < RING_MIN_RADIUS:
            return self._world_send(self._world_clear(root, ["ring"], common))
        tint = (rgba(color, common.get("alpha")) if color is not None else 0) or RING_MARKS[mark]
        spec = self._spec(_POINTS, root, "ring", **{**common, "style": "ring", "width": 2.0 * radius, "color": tint,
                                                     "props": {**common.get("props", {}), "mark": mark}})
        spec.points = list(centre)
        return self._world_send([(spec, DRAW_OP["upsert"])])

    def clearance_box(self, ns: str, id: str, pose=(0.0, 0.0, 0.0), *, size=(1.0, 1.0, 1.0),
                      color="plan", label: str = "", **common) -> Status | None:
        """Corner brackets at a pose with full XYZ extents; no collision/safety inference."""
        root, pose, size = f"{ns.rstrip('/')}/{id}", _world_pose(pose), _world_size(size)
        if pose is None or not all(_world_positive(v) for v in size):
            return self._world_send(self._world_clear(root, ("brackets", "label"), common))
        options = {**common, "pose": pose, "props": {**common.get("props", {}), "drape_anchor": "true", "drape_pose_height": "true"}}
        points = _world_box_brackets(size)
        return self._world_send([self._world_lines(root, "brackets", points, color, options, -STRUCTURE_RULE_PX * STRUCTURE_CORNER_GAIN),
                                 self._world_label(root, (0.0, 0.0, size[2] * 0.5), label, color, options)])

    def field_volume(self, ns: str, id: str, pose=(0.0, 0.0, 0.0), *, size=(1.0, 1.0, 1.0),
                     spacing: float = 0.5, color="plan", **common) -> Status | None:
        """A normally depth-tested FIELD box with full XYZ extents and major-grid pitch in metres."""
        root, pose, size = f"{ns.rstrip('/')}/{id}", _world_pose(pose), _world_size(size)
        spacing = _world_f32(spacing)
        if pose is None or not all(_world_positive(v) for v in (*size, spacing)):
            return self._world_send(self._world_clear(root, ("volume",), common))
        options = {"style": "field", **common, "pose": pose, "spacing": spacing,
                   "color": rgba(color, common.get("alpha")) or 1, "sub": SHAPE["box"]}
        options["props"] = {**common.get("props", {}), "drape_pose_height": "true"}
        spec = self._spec(_SHAPES, root, "volume", **options)
        spec.header["flags"] |= DRAW_FLAG["dashed"]
        spec.instances = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, *size]
        return self._world_send([(spec, DRAW_OP["upsert"])])

    def angle(self, ns: str, id: str, centre, start: float, end: float, *, radius: float = 1.0,
              frame: str = "world", **common) -> None:
        """An angle at `centre` from heading `start` to `end` (radians): a
        faint wedge, its rays and the angle in degrees."""
        root = f"{ns}/{id}"
        tint = rgba(common.pop("color", "yellow"), common.pop("alpha", None)) or 1
        c = tuple(centre) + (0.0,) * (3 - len(centre))
        span, mid = end - start, start + 0.5 * (end - start)
        self.shapes(root, "wedge", "sector", poses=[c + (mid,)], sizes=[(radius, abs(span), 0.0)], style="sweep",
                    color=_faded(tint, 0.6), pulse_hz=-1, frame=frame, **common)
        arc = [(c[0] + radius * math.cos(a), c[1] + radius * math.sin(a), c[2])
               for a in (start + span * k / 32 for k in range_(33))]
        self.lines(root, "rays", [c] + arc + [c], style="decal", color=tint, width=0.025, frame=frame, **common)
        self.text(root, "degrees", [(c[0] + 1.25 * radius * math.cos(mid), c[1] + 1.25 * radius * math.sin(mid), c[2] + 0.05)],
                  text=f"{math.degrees(span):.1f}°", color=tint, frame=frame, **common)

    def tf_tree(self, machine: str, on: bool = True) -> Status | None:
        """Switch the viewer's TF tree overlay on for `machine` (nothing more
        goes on the wire)."""
        if not on:
            return self.delete("gearbox/tf", machine)
        spec = _Spec(_AXES, "gearbox/tf", machine, sx=0.0, sy=0.0, sz=0.0, qw=1.0)
        spec.props["tf"] = machine
        return self.send(spec)

    # ── interaction (answers on /gearbox/scene/events) ──

    def _interact(self, kind: str, id: str, frame: str, pose=None, props=None) -> tuple[str, str]:
        ns = "gearbox/interact"
        spec = self._spec(_INTERACT, ns, id, sub=INTERACT[kind], frame=frame, pose=pose, drape=False, props=props)
        spec.wait = True
        status = self.send(spec)
        if status is not None and not status.ok:
            raise RuntimeError(status.message or "interaction refused")
        return ns, id

    def _await(self, events: "EventStream", ns: str, id: str, whats, timeout: float | None) -> dict | None:
        deadline = None if timeout is None else time.time() + timeout
        while deadline is None or time.time() < deadline:
            for ev in events.poll():
                if ev.get("kind") == "draw" and ev.get("ns") == ns and ev.get("id") == id and ev.get("what") in whats:
                    return ev
            time.sleep(0.02)
        return None

    def step(self, caption: str = "Next", *, full: bool = False, timeout: float | None = None) -> bool:
        """Wait for the Next button in the viewer (rviz RemoteControl
        waitForNextStep); False on timeout."""
        events = self.gb.events()
        self._step = getattr(self, "_step", 0) + 1
        ns, id = self._interact("step", f"step{self._step}", "world", props={"text": caption, "full": int(full)})
        ev = self._await(events, ns, id, ("step",), timeout)
        if ev is None:
            self.delete(ns, id)
        return ev is not None

    def pick_point(self, prompt: str = "Click a point", *, frame: str = "world",
                   timeout: float | None = None) -> tuple[float, float, float] | None:
        """The next point clicked on the ground, in `frame` (Foxglove point tool)."""
        events = self.gb.events()
        ns, id = self._interact("pick_point", "point", frame, props={"text": prompt})
        ev = self._await(events, ns, id, ("point",), timeout)
        return (ev["x"], ev["y"], ev["z"]) if ev else None

    def pick_pose(self, prompt: str = "Press and drag for the heading", *, frame: str = "world",
                  timeout: float | None = None) -> tuple[float, float, float, float] | None:
        """A pose picked on the ground: press for the place, drag for the
        heading (Foxglove 2D pose tool); (x, y, z, yaw) in `frame`."""
        events = self.gb.events()
        ns, id = self._interact("pick_pose", "pose", frame, props={"text": prompt})
        ev = self._await(events, ns, id, ("pose",), timeout)
        return (ev["x"], ev["y"], ev["z"], float(ev.get("yaw", 0.0))) if ev else None

    def handle(self, ns: str, id: str, pose=(0.0, 0.0, 0.0), *, controls: str = "move,rotate", menu=None,
               button: str | None = None, frame: str = "world", scale: float = 1.0) -> "DrawHandle":
        """A 6-DoF handle at `pose` (InteractiveMarker): drags send
        pose_update, presses mouse_down / mouse_up, `menu` entries
        menu_select, `button` button_click; read them from the returned
        handle's `poll()`."""
        events = self.gb.events()
        spec = self._spec(_INTERACT, ns, id, sub=INTERACT["handle"], frame=frame, pose=pose, drape=False,
                          scale=(scale, scale, scale),
                          props={"controls": controls, "menu": ",".join(menu) if menu else None, "button": button})
        spec.wait = True
        self.send(spec)
        return DrawHandle(self, events, ns, id)

    def tools(self, frame: str = "world", ns: str = "viz", lifetime: float = 0.0, alpha: float | None = None,
              global_scale: float = 10.0, frame_locked: bool = True) -> "Tools":
        """An rviz_visual_tools-style facade: `viz.sphere(p, Color.BLUE,
        Scale.MEDIUM)`, ids numbered per ns."""
        return Tools(self, frame=frame, ns=ns, lifetime=lifetime, alpha=alpha, global_scale=global_scale,
                     frame_locked=frame_locked)


class _Batch:
    """Draws sent without waiting until the block ends (rviz batch publishing)."""

    def __init__(self, draw: Draw, every: int, timeout: float):
        self.draw, self.every, self.timeout, self.count = draw, every, timeout, 0

    def __enter__(self):
        self.saved = self.draw.wait
        self.draw.wait = False
        self.draw._batch = self
        return self

    def tick(self):
        self.count += 1
        if self.every and self.count % self.every == 0:
            self.draw.flush(self.timeout)

    def trigger(self) -> bool:
        return self.draw.flush(self.timeout)

    def __exit__(self, *exc):
        self.draw._batch = None
        self.draw.wait = self.saved
        self.draw.flush(self.timeout)
        return False


class DrawHandle:
    """Answers from a draw handle: `poll()` gives the new events (dicts
    with `what`, x, y, z, qw..qz, `menu`); `pose` the latest dragged pose."""

    def __init__(self, draw: Draw, events: EventStream, ns: str, id: str):
        self.draw, self.events, self.ns, self.id = draw, events, ns, id
        self.pose = None

    def poll(self) -> list[dict]:
        out = []
        for ev in self.events.poll():
            if ev.get("kind") == "draw" and ev.get("ns") == self.ns and ev.get("id") == self.id:
                if ev.get("what") == "pose_update":
                    self.pose = (ev["x"], ev["y"], ev["z"], float(ev.get("qw", 1)), float(ev.get("qx", 0)),
                                 float(ev.get("qy", 0)), float(ev.get("qz", 0)))
                out.append(ev)
        return out

    def remove(self):
        self.draw.delete(self.ns, self.id)


def _pack(r: int, g: int, b: int, a: int = 255) -> int:
    return r | (g << 8) | (b << 16) | (a << 24)


class Color:
    """rviz_visual_tools colours on the Okabe-Ito palette, plus the roles the
    auto styles draw in. `RAND` is a stable colour per id; `DEFAULT` lets
    the style pick."""

    BLACK = _pack(0, 0, 0)
    BROWN = _pack(0xA6, 0x76, 0x1D)
    BLUE = _pack(0x00, 0x72, 0xB2)
    CYAN = _pack(0x56, 0xB4, 0xE9)
    GREY = _pack(0x99, 0x99, 0x99)
    DARK_GREY = _pack(0x55, 0x55, 0x55)
    GREEN = LIME_GREEN = _pack(0x00, 0x9E, 0x73)
    MAGENTA = PINK = PURPLE = _pack(0xCC, 0x79, 0xA7)
    ORANGE = _pack(0xE6, 0x9F, 0x00)
    RED = _pack(0xD5, 0x5E, 0x00)
    WHITE = _pack(255, 255, 255)
    YELLOW = _pack(0xF0, 0xE4, 0x42)
    TRANSLUCENT = _pack(255, 255, 255, 64)
    TRANSLUCENT_LIGHT = _pack(255, 255, 255, 26)
    TRANSLUCENT_DARK = _pack(255, 255, 255, 128)
    CLEAR = _pack(255, 255, 255, 0)
    DEFAULT = None
    RAND = "rand"
    PLAN, PREDICTION, LOOKAHEAD, GOAL = CYAN, GREEN, ORANGE, YELLOW
    OBSTACLE, DANGER, HISTORY, SENSOR = MAGENTA, RED, _pack(255, 255, 255, 89), BLUE
    OKABE = [ORANGE, CYAN, GREEN, YELLOW, BLUE, RED, MAGENTA, GREY]

    @staticmethod
    def scale(value: float) -> int:
        """rviz getColorScale: 0 vermillion, 0.5 yellow, 1 bluish green."""
        v = min(max(float(value), 0.0), 1.0)
        a, b, t = (Color.RED, Color.YELLOW, 2 * v) if v < 0.5 else (Color.YELLOW, Color.GREEN, 2 * v - 1)
        ch = lambda c, k: (c >> (8 * k)) & 255  # noqa: E731
        return _pack(*(round(ch(a, k) + (ch(b, k) - ch(a, k)) * t) for k in range(3)))

    @staticmethod
    def random(key) -> int:
        """A stable bright colour for `key`."""
        h = 0x811C9DC5
        for b in str(key).encode():
            h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
        hue = (h % 360) / 60.0
        x = 1 - abs(hue % 2 - 1)
        r, g, b = [(1, x, 0), (x, 1, 0), (0, 1, x), (0, x, 1), (x, 0, 1), (1, 0, x)][int(hue) % 6]
        lift = lambda v: int(64 + v * 191)  # noqa: E731
        return _pack(lift(r), lift(g), lift(b))

    @staticmethod
    def alpha(color: int, a: float) -> int:
        return (color & 0x00FFFFFF) | (round(min(max(a, 0.0), 1.0) * 255) << 24)


class Scale:
    """rviz_visual_tools sizes in metres, before the facade's global scale
    (10 by default: machines are vehicles, MEDIUM is 0.1 m)."""

    XXXXSMALL, XXXSMALL, XXSMALL, XSMALL, SMALL = 0.001, 0.0025, 0.005, 0.0065, 0.0075
    MEDIUM, LARGE, XLARGE, XXLARGE, XXXLARGE, XXXXLARGE = 0.01, 0.025, 0.05, 0.075, 0.1, 0.5

    @staticmethod
    def px(n: float) -> "_Pixels":
        """A width of `n` pixels."""
        return _Pixels(n)


class _Pixels(float):
    """A width in pixels, sent negative."""


def _qmul(a, b):
    aw, ax, ay, az = a
    bw, bx, by, bz = b
    return (aw * bw - ax * bx - ay * by - az * bz, aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw)


def _q_between(u, v):
    """The shortest rotation (w, x, y, z) taking direction `u` onto `v`."""
    u, v = _normalize(u), _normalize(v)
    d = sum(a * b for a, b in zip(u, v))
    if d < -0.999999:
        axis = _normalize(_cross((1.0, 0.0, 0.0) if abs(u[0]) < 0.9 else (0.0, 1.0, 0.0), u))
        return (0.0, *axis)
    c = _cross(u, v)
    q = (1.0 + d, *c)
    n = math.sqrt(sum(x * x for x in q))
    return tuple(x / n for x in q)


class Tools:
    """rviz_visual_tools on gearbox draws. Every call takes a pose
    ((x, y, z), (x, y, z, yaw) or (x, y, z, qw, qx, qy, qz)) or points, a
    `Color` (or any colour `rgba` takes), a `Scale` (times `global_scale`,
    or `Scale.px(n)`), and an optional `ns` and `id`; ids count up per ns.
    `frame_locked=False` freezes draws at their frame's pose on arrival."""

    def __init__(self, draw: Draw, frame: str = "world", ns: str = "viz", lifetime: float = 0.0,
                 alpha: float | None = None, global_scale: float = 10.0, frame_locked: bool = True):
        self.draw, self.frame, self.ns, self.lifetime = draw, frame, ns, lifetime
        self.alpha_override, self.global_scale, self.frame_locked = alpha, global_scale, frame_locked
        self._ids: dict[str, int] = collections.defaultdict(int)

    # setLifetime, setAlpha, setGlobalScale, setBaseFrame, enableFrameLocking
    def set_lifetime(self, seconds: float):
        self.lifetime = seconds

    def set_alpha(self, alpha: float | None):
        self.alpha_override = alpha

    def set_global_scale(self, scale: float):
        self.global_scale = scale

    def set_base_frame(self, frame: str):
        self.frame = frame

    def enable_frame_locking(self, on: bool = True):
        self.frame_locked = on

    def trigger(self) -> bool:
        return self.draw.flush()

    def trigger_every(self, n: int) -> _Batch:
        return self.draw.batch(every=n)

    def enable_batch_publishing(self, on: bool = True):
        self.draw.wait = not on

    def color_scale(self, value: float) -> int:
        return Color.scale(value)

    def _key(self, ns: str | None, default: str, id) -> tuple[str, str]:
        ns = f"{self.ns}/{ns or default}"
        if id is None:
            self._ids[ns] += 1
            id = self._ids[ns]
        return ns, str(id)

    def _common(self, ns: str, id: str, color, extra=None) -> dict:
        if color == Color.RAND:
            color = Color.random(f"{ns}/{id}")
        out = {"frame": self.frame, "lifetime": self.lifetime, "snapshot": not self.frame_locked}
        if color is not None:
            out["color"] = color
            if self.alpha_override is not None:
                out["alpha"] = self.alpha_override
        out.update(extra or {})
        return out

    def _s(self, scale) -> float:
        if isinstance(scale, _Pixels):
            return -abs(float(scale))
        return float(scale) * self.global_scale

    @staticmethod
    def _xyz(p):
        p = tuple(p)
        return p[:3] if len(p) >= 3 else (p[0], p[1], 0.0)

    # publishSphere(s)
    def sphere(self, pose, color=Color.BLUE, scale=Scale.MEDIUM, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Sphere", id)
        return self.draw.points(ns, id, [self._xyz(pose)], radius=0.5 * self._s(scale), style="point_glow",
                                **self._common(ns, id, color))

    def spheres(self, points, color=Color.BLUE, scale=Scale.MEDIUM, ns: str | None = None, id=None, colors=None):
        ns, id = self._key(ns, "Spheres", id)
        return self.draw.points(ns, id, [self._xyz(p) for p in points], radius=0.5 * self._s(scale),
                                colors=colors, style="point_glow", **self._common(ns, id, color))

    # publishArrow, publishXArrow / YArrow / ZArrow
    def arrow(self, pose_or_start, end=None, color=Color.RED, scale=Scale.MEDIUM, length: float = 0.0,
              axis: str = "x", ns: str | None = None, id=None):
        ns, id = self._key(ns, "Arrow", id)
        common = self._common(ns, id, color, {"width": self._s(scale), "style": "arrow_glow"})
        if end is not None:
            return self.draw.arrows(ns, id, [self._xyz(pose_or_start)], tips=[self._xyz(end)], **common)
        pose = tuple(pose_or_start)
        q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
        turn = {"x": (1.0, 0.0, 0.0, 0.0), "y": (math.cos(math.pi / 4), 0.0, 0.0, math.sin(math.pi / 4)),
                "z": (math.cos(math.pi / 4), 0.0, -math.sin(math.pi / 4), 0.0)}[axis]
        q = _qmul(q, turn)
        return self.draw.arrows(ns, id, poses=[self._xyz(pose) + q], length=length or 8 * abs(self._s(scale)), **common)

    def x_arrow(self, pose, color=Color.RED, scale=Scale.MEDIUM, length: float = 0.0, **kw):
        return self.arrow(pose, color=color, scale=scale, length=length, axis="x", **kw)

    def y_arrow(self, pose, color=Color.GREEN, scale=Scale.MEDIUM, length: float = 0.0, **kw):
        return self.arrow(pose, color=color, scale=scale, length=length, axis="y", **kw)

    def z_arrow(self, pose, color=Color.BLUE, scale=Scale.MEDIUM, length: float = 0.0, **kw):
        return self.arrow(pose, color=color, scale=scale, length=length, axis="z", **kw)

    # publishAxis, publishAxisLabeled, publishAxisPath
    def axis(self, pose, length: float = 0.4, radius: float = 0.0, label: str | None = None,
             ns: str | None = None, id=None):
        ns, id = self._key(ns, "Axis", id)
        width = 2 * radius if radius else 0.0
        return self.draw.axes(ns, id, [tuple(pose)], length=length, width=width, style="arrow_glow",
                              labels=[label] if label else None, **self._common(ns, id, None))

    def axis_labeled(self, pose, label: str, **kw):
        return self.axis(pose, label=label, **kw)

    def axis_path(self, poses, length: float = 0.25, radius: float = 0.0, ns: str | None = None, id=None):
        ns, id = self._key(ns, "AxisPath", id)
        return self.draw.axes(ns, id, [tuple(p) for p in poses], length=length, width=2 * radius if radius else 0.0,
                              **self._common(ns, id, None))

    # publishCuboid, publishWireframeCuboid, publishWireframeRectangle
    def cuboid(self, p1_or_pose, p2=None, size=None, color=Color.BLUE, fill: str = "solid",
               ns: str | None = None, id=None):
        ns, id = self._key(ns, "Cuboid", id)
        if p2 is not None:
            a, b = self._xyz(p1_or_pose), self._xyz(p2)
            centre = tuple(0.5 * (u + v) for u, v in zip(a, b))
            extent = tuple(abs(v - u) for u, v in zip(a, b))
            poses = [centre]
        else:
            poses, extent = [tuple(p1_or_pose)], tuple(size or (1.0, 1.0, 1.0))
        style = "holo" if fill == "wire" else "plain"
        return self.draw.shapes(ns, id, "box", poses=poses, sizes=[extent], fill=fill, style=style,
                                beam_height=-1 if fill == "wire" else None, shaded=fill != "wire",
                                **self._common(ns, id, color))

    def wireframe_cuboid(self, p1_or_pose, p2=None, size=None, color=Color.BLUE, **kw):
        return self.cuboid(p1_or_pose, p2, size, color, fill="wire", **kw)

    def rectangle(self, pose, width: float, height: float, color=Color.BLUE, scale=Scale.XSMALL,
                  ns: str | None = None, id=None):
        ns, id = self._key(ns, "Rectangle", id)
        pose = tuple(pose)
        q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
        o = self._xyz(pose)
        corners = [_apply(o, q, (sx * width / 2, sy * height / 2, 0.0)) for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
        return self.draw.lines(ns, id, corners, loop=True, width=self._s(scale),
                               **self._common(ns, id, color))

    # publishCylinder, publishCone
    def cylinder(self, p1_or_pose, p2=None, color=Color.BLUE, radius: float = 0.1, height: float = 1.0,
                 ns: str | None = None, id=None):
        ns, id = self._key(ns, "Cylinder", id)
        if p2 is not None:
            a, b = self._xyz(p1_or_pose), self._xyz(p2)
            d = tuple(v - u for u, v in zip(a, b))
            height = math.sqrt(sum(c * c for c in d))
            pose = tuple(0.5 * (u + v) for u, v in zip(a, b)) + _q_between((0.0, 0.0, 1.0), d)
        else:
            pose = tuple(p1_or_pose)
        return self.draw.shapes(ns, id, "cylinder", poses=[pose], sizes=[(2 * radius, 2 * radius, height)],
                                style="shield", **self._common(ns, id, color))

    def cone(self, pose, angle: float, color=Color.BLUE, scale=Scale.XXXLARGE, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Cone", id)
        pose = tuple(pose)
        q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
        length = self._s(scale)
        centre = _apply(self._xyz(pose), q, (0.5 * length, 0.0, 0.0))
        qc = _qmul(q, (math.cos(-math.pi / 4), 0.0, math.sin(-math.pi / 4), 0.0))
        d = 2 * length * math.tan(angle)
        return self.draw.shapes(ns, id, "cone", poses=[centre + qc], sizes=[(d, d, length)], style="shield",
                                **self._common(ns, id, color))

    # publishABCDPlane, publishNormalAndDistancePlane, publishXY/XZ/YZPlane
    def plane(self, pose=(0.0, 0.0, 0.0), plane: str = "xy", size: float = 1.0, color=Color.TRANSLUCENT,
              ns: str | None = None, id=None):
        ns, id = self._key(ns, "Plane", id)
        pose = tuple(pose)
        q = _quat(pose) if len(pose) != 3 else (1.0, 0.0, 0.0, 0.0)
        turn = {"xy": (1.0, 0.0, 0.0, 0.0), "xz": (math.cos(math.pi / 4), math.sin(math.pi / 4), 0.0, 0.0),
                "yz": (math.cos(math.pi / 4), 0.0, math.sin(math.pi / 4), 0.0)}[plane]
        return self.draw.shapes(ns, id, "rect", poses=[self._xyz(pose) + _qmul(q, turn)], sizes=[(size, size, 0.0)],
                                fill="both", **self._common(ns, id, color))

    def xy_plane(self, pose=(0.0, 0.0, 0.0), **kw):
        return self.plane(pose, "xy", **kw)

    def xz_plane(self, pose=(0.0, 0.0, 0.0), **kw):
        return self.plane(pose, "xz", **kw)

    def yz_plane(self, pose=(0.0, 0.0, 0.0), **kw):
        return self.plane(pose, "yz", **kw)

    def abcd_plane(self, a: float, b: float, c: float, d: float, size: float = 1.0, **kw):
        n = math.sqrt(a * a + b * b + c * c)
        normal = (a / n, b / n, c / n)
        point = tuple(-d / n * v for v in normal)
        return self.plane(point + _q_between((0.0, 0.0, 1.0), normal), "xy", size=size, **kw)

    def normal_distance_plane(self, normal, distance: float, size: float = 1.0, **kw):
        return self.abcd_plane(normal[0], normal[1], normal[2], -distance, size=size, **kw)

    # publishLine, publishLines, publishLineStrip, publishPath
    def line(self, a, b, color=Color.WHITE, scale=Scale.XSMALL, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Line", id)
        return self.draw.lines(ns, id, [self._xyz(a), self._xyz(b)], width=self._s(scale), style="plain",
                               **self._common(ns, id, color))

    def lines(self, starts, ends=None, colors=None, color=Color.WHITE, scale=Scale.XSMALL, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Lines", id)
        pts = [p for a, b in zip(starts, ends) for p in (self._xyz(a), self._xyz(b))] if ends is not None else \
            [self._xyz(p) for p in starts]
        return self.draw.lines(ns, id, pts, segments=True, colors=colors, width=self._s(scale), style="plain",
                               **self._common(ns, id, color))

    def line_strip(self, points, color=Color.WHITE, scale=Scale.XSMALL, ns: str | None = None, id=None):
        ns, id = self._key(ns, "LineStrip", id)
        return self.draw.lines(ns, id, [self._xyz(p) for p in points], width=self._s(scale), style="plain",
                               **self._common(ns, id, color))

    def path(self, points, color=Color.CYAN, scale=Scale.XXLARGE, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Path", id)
        return self.draw.lines(ns, id, [self._xyz(p) for p in points], width=self._s(scale), style="ribbon_flow",
                               **self._common(ns, id, color))

    # publishPolygon, publishMesh, publishText, publishGraph
    def polygon(self, rings, color=Color.CYAN, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Polygon", id)
        rings = [rings] if rings and not isinstance(rings[0][0], (list, tuple)) else rings
        return self.draw.polygon(ns, id, rings, **self._common(ns, id, color))

    def mesh(self, asset=None, pose=(0.0, 0.0, 0.0), vertices=None, faces=None, color=Color.CLEAR, scale: float = 1.0,
             ns: str | None = None, id=None):
        ns, id = self._key(ns, "Mesh", id)
        color = None if color == Color.CLEAR else color
        return self.draw.mesh(ns, id, asset, vertices=vertices, faces=faces, pose=pose, scale=scale,
                              **self._common(ns, id, color))

    def text(self, pose, text: str, color=Color.WHITE, scale=Scale.LARGE, static_id: bool = True,
             ns: str | None = None, id=None):
        ns = ns or "Text"
        if static_id and id is None:
            id = 0
        ns, id = self._key(ns, "Text", id)
        return self.draw.text(ns, id, [self._xyz(pose)], text=text, size_m=self._s(scale),
                              **self._common(ns, id, color))

    def graph(self, nodes, edges, color=Color.CYAN, ns: str | None = None, id=None):
        ns, id = self._key(ns, "Graph", id)
        return self.draw.graph(ns, id, nodes, edges, color=color or "cyan", frame=self.frame, lifetime=self.lifetime)

    # deleteMarker, deleteAllMarkers
    def delete(self, ns: str, id) -> Status | None:
        return self.draw.delete(f"{self.ns}/{ns}", str(id))

    def delete_all(self, ns: str = "") -> Status | None:
        return self.draw.clear(f"{self.ns}/{ns}".rstrip("/"))
