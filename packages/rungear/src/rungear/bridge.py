"""Peerbus shared-memory -> Rerun bridge for Gearbox simulators.

Subscribes directly to Gearbox host and machine topics over native `peerbus`
shared memory (via `gearbox_client`) and logs everything onto one synchronized
`sim_time` timeline:
  - Live machine root poses + trajectories (`world/<machine>`)
  - Full static + dynamic `/links` & `/tf` (`gearbox.tf.v1`) transform hierarchy
  - LiDAR sweeps (`Points3D` colored by height or range in sensor TF frame)
  - Cameras (`Pinhole` frustum + `Image` RGB + 2D Turbo `DepthImage` + `Boxes2D`)
  - Radar (`Points3D` targets + `Arrows3D` radial velocity vectors)
  - IMU, GPS, Distance (`Arrows3D` + scalars), Light, Climate, Touch, Position,
    Compass, InertialUnit, Accelerometer, Gyro, and Receiver telemetry.
"""

from __future__ import annotations

import collections
import math
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import rerun as rr
import rerun.blueprint as rrb


def _ensure_gearbox_paths() -> None:
    """Make repo `scripts/control` and `.python-packages` importable."""
    here = Path(__file__).resolve()
    for ancestor in here.parents:
        ctrl = ancestor / "scripts" / "control"
        pkgs = ancestor / ".python-packages"
        if (ctrl / "gearbox_client.py").is_file():
            for p in (str(ctrl), str(pkgs)):
                if p not in sys.path:
                    sys.path.insert(0, p)
            return


_ensure_gearbox_paths()

from gearbox_client import (  # noqa: E402
    CAMERA_COLOR,
    CAMERA_DEPTH,
    CameraImage,
    Gearbox,
    Imu,
    Instance,
    LidarSweep,
    Machine,
    Measurement,
    SensorStream,
)


def _build_turbo_lut() -> np.ndarray:
    t = np.linspace(0.0, 1.0, 256, dtype=np.float32)
    r = np.clip(0.1357 + t * (4.6154 + t * (-42.6603 + t * (132.1311 + t * (-152.9424 + t * 59.2864)))), 0.0, 1.0)
    g = np.clip(0.0914 + t * (2.1942 + t * (4.8430 + t * (-14.1850 + t * (4.2773 + t * 2.8296)))), 0.0, 1.0)
    b = np.clip(0.1067 + t * (12.6419 + t * (-60.5820 + t * (110.3628 + t * (-89.9031 + t * 27.3482)))), 0.0, 1.0)
    return (np.stack([r, g, b], axis=-1) * 255.0).astype(np.uint8)


_TURBO_LUT = _build_turbo_lut()


@dataclass
class RungearConfig:
    instance: str | None = None
    machines: set[str] = field(default_factory=set)
    sensors: set[str] = field(default_factory=set)
    enable_tf: bool = True
    enable_state: bool = True
    enable_cameras: bool = True
    enable_lidar: bool = True
    lidar_color_by: str = "height"  # "height" or "range"
    lidar_point_radius: float = 0.035
    discovery_interval_s: float = 2.0
    trajectory_max_points: int = 600


class _MachineBinding:
    """Tracks one machine's peerbus subscriptions and TF path hierarchy."""

    def __init__(self, machine: Machine, cfg: RungearConfig):
        self.machine = machine
        self.id = machine.namespace
        self.cfg = cfg
        self.root = f"world/{self.id}"
        self.link_paths: dict[str, str] = {}
        self.sensor_streams: dict[str, SensorStream] = {}
        self.sensor_kinds: dict[str, str] = {}
        self.pinhole_logged: set[tuple[str, int, int]] = set()
        self.series_styled: set[str] = set()
        self.trajectory: collections.deque[list[float]] = collections.deque(maxlen=cfg.trajectory_max_points)
        self.last_sim_time_s: float | None = None
        self.last_state_wall: float = 0.0
        self.last_tf_wall: float = 0.0
        self.tf_enabled = False
        self._init_links_and_sensors()

    def _init_links_and_sensors(self) -> None:
        links = self.machine.links()
        by_name = {l["name"]: l for l in links if l.get("name")}

        def resolve_path(name: str, seen: set[str] | None = None) -> str:
            if name in self.link_paths:
                return self.link_paths[name]
            seen = seen or set()
            if name in seen:
                return f"{self.root}/{name}"
            seen.add(name)
            rec = by_name.get(name)
            parent = rec.get("parent") if rec else None
            if parent and parent in by_name and parent != name:
                path = f"{resolve_path(parent, seen)}/{name}"
            else:
                path = f"{self.root}/{name}"
            self.link_paths[name] = path
            return path

        # Log static link tree offsets so every sensor starts in its true mount frame.
        for rec in links:
            name = rec.get("name")
            if not name:
                continue
            path = resolve_path(name)
            off = rec.get("offset") or {}
            rr.log(
                path,
                rr.Transform3D(
                    translation=[off.get("x", 0.0), off.get("y", 0.0), off.get("z", 0.0)],
                    quaternion=rr.Quaternion(
                        xyzw=[off.get("qx", 0.0), off.get("qy", 0.0), off.get("qz", 0.0), off.get("qw", 1.0)]
                    ),
                ),
                static=True,
            )

        if self.cfg.enable_tf:
            try:
                self.machine.tf(True)
                self.tf_enabled = True
            except Exception as err:  # noqa: BLE001
                print(f"[rungear] {self.id}: failed to enable /tf: {err}", file=sys.stderr)

        for rec in links:
            if rec.get("role") != "sensor":
                continue
            name = rec.get("name", "")
            if not name:
                continue
            if self.cfg.sensors and name not in self.cfg.sensors:
                continue
            try:
                self.sensor_streams[name] = self.machine.subscribe_sensor(name)
                print(f"[rungear] subscribed {self.id}/sensors/{name} (peerbus SHM)", flush=True)
            except Exception as err:  # noqa: BLE001
                print(f"[rungear] failed to subscribe {self.id}/sensors/{name}: {err}", file=sys.stderr)

    def entity_for_link(self, link: str) -> str:
        return self.link_paths.get(link) or f"{self.root}/{link}"

    def style_series(self, path: str, names: list[str], colors: list[list[int]] | None = None) -> None:
        if path in self.series_styled:
            return
        self.series_styled.add(path)
        kwargs = {"names": names}
        if colors:
            kwargs["colors"] = colors
        rr.log(path, rr.SeriesLines(**kwargs), static=True)

    def close(self) -> None:
        for stream in self.sensor_streams.values():
            stream.close()
        self.sensor_streams.clear()


class RungearBridge:
    """Streams live Gearbox peerbus shared-memory data into Rerun."""

    def __init__(self, cfg: RungearConfig):
        self.cfg = cfg
        self.gb: Gearbox | None = None
        self.instance_pid: int | None = None
        self.bindings: dict[str, _MachineBinding] = {}
        self._blueprint_sig: tuple[tuple[str, tuple[str, ...]], ...] | None = None
        self._stop = False

    def stop(self) -> None:
        self._stop = True

    def _connect(self) -> bool:
        try:
            inst = Instance.find(self.cfg.instance)
        except RuntimeError:
            return False
        if self.gb is not None and self.instance_pid == inst.pid:
            return True
        self._close_all()
        self.gb = Gearbox(instance=inst.name)
        self.instance_pid = inst.pid
        print(f"[rungear] connected to Gearbox instance `{inst.name}` (pid {inst.pid}) via peerbus SHM", flush=True)
        rr.log("world", rr.ViewCoordinates.RIGHT_HAND_Z_UP, static=True)
        return True

    def _close_all(self) -> None:
        for b in self.bindings.values():
            b.close()
        self.bindings.clear()
        self.gb = None
        self.instance_pid = None

    def _discover_machines(self) -> None:
        if not self._connect() or self.gb is None:
            return
        try:
            refs = self.gb.machines()
        except Exception as err:  # noqa: BLE001
            print(f"[rungear] lost host connection ({err}); waiting to reconnect…", file=sys.stderr)
            self._close_all()
            return

        live_ids: set[str] = set()
        changed = False
        for ref in refs:
            mid = ref.get("machine_id") or ref.get("namespace") or ""
            addr = ref.get("addr") or ""
            did = ref.get("did") or ""
            if not mid or not addr:
                continue
            if self.cfg.machines and mid not in self.cfg.machines:
                continue
            live_ids.add(mid)
            existing = self.bindings.get(mid)
            if existing is None or existing.machine.addr != addr:
                if existing is not None:
                    existing.close()
                m = Machine(self.gb, mid, addr, did)
                try:
                    self.bindings[mid] = _MachineBinding(m, self.cfg)
                    changed = True
                except Exception as err:  # noqa: BLE001
                    print(f"[rungear] failed to bind machine `{mid}`: {err}", file=sys.stderr)

        for stale in list(self.bindings):
            if stale not in live_ids:
                self.bindings.pop(stale).close()
                changed = True

        if changed:
            self._maybe_send_blueprint()

    def _maybe_send_blueprint(self) -> None:
        cam_entries: list[tuple[str, str, str]] = []
        plot_tabs: list[rrb.TimeSeriesView] = []
        sig_items: list[tuple[str, tuple[str, ...]]] = []

        for mid in sorted(self.bindings):
            b = self.bindings[mid]
            kinds = tuple(sorted(f"{k}:{v}" for k, v in b.sensor_kinds.items()))
            sig_items.append((mid, kinds))
            for link, kind in sorted(b.sensor_kinds.items()):
                if kind == "camera":
                    cam_entries.append((f"{mid}/{link}", b.entity_for_link(link), f"cameras/{mid}/{link}/depth"))
                elif kind == "imu":
                    plot_tabs.append(rrb.TimeSeriesView(name=f"{mid} IMU Accel", origin=f"telemetry/{mid}/{link}/accel"))
                    plot_tabs.append(rrb.TimeSeriesView(name=f"{mid} IMU Gyro", origin=f"telemetry/{mid}/{link}/gyro"))
                elif kind == "lidar":
                    plot_tabs.append(rrb.TimeSeriesView(name=f"{mid} LiDAR Hits", origin=f"telemetry/{mid}/{link}"))
                elif kind == "gps":
                    plot_tabs.append(rrb.TimeSeriesView(name=f"{mid} GPS", origin=f"telemetry/{mid}/{link}"))
                else:
                    plot_tabs.append(rrb.TimeSeriesView(name=f"{mid} {link}", origin=f"telemetry/{mid}/{link}"))
            plot_tabs.append(rrb.TimeSeriesView(name=f"{mid} Speed", origin=f"telemetry/{mid}/state"))

        sig = tuple(sig_items)
        if sig == self._blueprint_sig or not self.bindings:
            return
        self._blueprint_sig = sig

        right_views = []
        for label, color_origin, depth_origin in cam_entries[:4]:
            right_views.append(
                rrb.Horizontal(
                    rrb.Spatial2DView(name=f"{label} RGB", origin=f"{color_origin}/color"),
                    rrb.Spatial2DView(name=f"{label} Depth", origin=depth_origin),
                )
            )
        if plot_tabs:
            right_views.append(rrb.Tabs(*plot_tabs) if len(plot_tabs) > 1 else plot_tabs[0])

        layout = rrb.Horizontal(
            rrb.Spatial3DView(name="3D World", origin="world"),
            rrb.Vertical(*right_views) if len(right_views) > 1 else right_views[0],
            column_shares=[3, 2],
        )
        rr.send_blueprint(rrb.Blueprint(layout, collapse_panels=True))

    def run(self) -> None:
        last_discover = 0.0
        waiting_logged = False
        try:
            while not self._stop:
                now = time.monotonic()
                if now - last_discover >= self.cfg.discovery_interval_s:
                    last_discover = now
                    self._discover_machines()
                    if self.gb is None and not waiting_logged:
                        print("[rungear] waiting for a running Gearbox instance…", flush=True)
                        waiting_logged = True
                    elif self.gb is not None:
                        waiting_logged = False

                if not self.bindings:
                    time.sleep(0.1)
                    continue

                did_work = False
                for b in list(self.bindings.values()):
                    did_work |= self._poll_machine(b, now)

                if not did_work:
                    time.sleep(0.005)
        finally:
            self._close_all()

    def _set_sim_time(self, b: _MachineBinding, sim_time_s: float) -> None:
        b.last_sim_time_s = sim_time_s
        rr.set_time("sim_time", duration=sim_time_s)

    def _poll_machine(self, b: _MachineBinding, now: float) -> bool:
        did_work = False

        # 1. Live /tf dynamic link poses (gearbox.tf.v1 carries exact sim_time_s)
        if self.cfg.enable_tf and b.tf_enabled and now - b.last_tf_wall >= 0.033:
            try:
                tf = b.machine.tf_latest()
            except Exception as err:  # noqa: BLE001
                print(f"[rungear] {b.id}/tf error: {err}", file=sys.stderr)
                tf = None
            if tf is not None:
                b.last_tf_wall = now
                did_work = True
                self._set_sim_time(b, tf.sim_time_s)
                for edge in tf.edges():
                    child = edge["child"]
                    # Skip the synthetic inverse `chassis -> base_link` edge;
                    # `world/<machine>/base_link/chassis` is already rooted by MachineState.
                    if child == "base_link" or child not in b.link_paths:
                        continue
                    rr.log(
                        b.link_paths[child],
                        rr.Transform3D(
                            translation=[edge["x"], edge["y"], edge["z"]],
                            quaternion=rr.Quaternion(xyzw=[edge["qx"], edge["qy"], edge["qz"], edge["qw"]]),
                        ),
                    )

        # 2. Authored sensor streams (all share sim_time)
        blueprint_dirty = False
        for link, stream in list(b.sensor_streams.items()):
            try:
                events = stream.drain(limit=128)
            except Exception as err:  # noqa: BLE001
                print(f"[rungear] {b.id}/{link} drain error: {err}", file=sys.stderr)
                continue
            if not events:
                continue
            did_work = True
            for kind, payload in events:
                prev_kind = b.sensor_kinds.get(link)
                logged_kind = self._log_sensor(b, link, kind, payload)
                if logged_kind and logged_kind != prev_kind:
                    b.sensor_kinds[link] = logged_kind
                    blueprint_dirty = True

        # 3. Machine root pose + trajectory (aligned to the same sim_time timeline)
        if self.cfg.enable_state and b.last_sim_time_s is not None and now - b.last_state_wall >= 0.033:
            try:
                st = b.machine.state(wait=0.0)
            except Exception:  # noqa: BLE001
                st = None
            if st is not None:
                b.last_state_wall = now
                did_work = True
                rr.set_time("sim_time", duration=b.last_sim_time_s)
                rr.log(
                    b.root,
                    rr.Transform3D(
                        translation=[st.x, st.y, st.z],
                        quaternion=rr.Quaternion(xyzw=[st.qx, st.qy, st.qz, st.qw]),
                    ),
                )
                pos = [st.x, st.y, st.z]
                if not b.trajectory or math.dist(b.trajectory[-1], pos) >= 0.05:
                    b.trajectory.append(pos)
                    if len(b.trajectory) >= 2:
                        rr.log(
                            f"world/{b.id}_trajectory",
                            rr.LineStrips3D([list(b.trajectory)], colors=[[86, 180, 233]], radii=[0.04]),
                        )
                rr.log(f"telemetry/{b.id}/state/speed_mps", rr.Scalars(st.linear_speed))

        if blueprint_dirty:
            self._maybe_send_blueprint()

        return did_work

    def _log_sensor(self, b: _MachineBinding, link: str, kind: str, payload: object) -> str | None:
        entity = b.entity_for_link(link)
        tbase = f"telemetry/{b.id}/{link}"

        if kind == "imu":
            assert isinstance(payload, Imu)
            if b.last_sim_time_s is None:
                return "imu"
            rr.set_time("sim_time", duration=b.last_sim_time_s)
            b.style_series(f"{tbase}/accel", ["ax", "ay", "az"], [[213, 94, 0], [0, 158, 115], [86, 180, 233]])
            b.style_series(f"{tbase}/gyro", ["wx", "wy", "wz"], [[213, 94, 0], [0, 158, 115], [86, 180, 233]])
            rr.log(f"{tbase}/accel", rr.Scalars([payload.ax, payload.ay, payload.az]))
            rr.log(f"{tbase}/gyro", rr.Scalars([payload.wx, payload.wy, payload.wz]))
            return "imu"

        if kind == "lidar":
            if not self.cfg.enable_lidar:
                return "lidar"
            assert isinstance(payload, LidarSweep)
            self._set_sim_time(b, payload.sim_time_s)
            total = payload.rows * payload.cols
            ranges = np.frombuffer(payload.ranges_bytes, dtype="<f4", count=total)
            points = np.frombuffer(payload.points_bytes, dtype="<f4", count=total * 3).reshape((total, 3))
            valid = np.isfinite(ranges) & (ranges > 0.0) & np.all(np.isfinite(points), axis=1)
            pts = points[valid]
            rng = ranges[valid]
            if len(pts) > 0:
                if self.cfg.lidar_color_by == "range":
                    max_r = max(float(payload.max_range_m), 1.0)
                    norm = np.clip(rng / max_r, 0.0, 1.0)
                else:
                    norm = np.clip((pts[:, 2] + 2.0) / 5.0, 0.0, 1.0)
                colors = _TURBO_LUT[(norm * 255.0).astype(np.uint8)]
                rr.log(
                    f"{entity}/points",
                    rr.Points3D(pts, colors=colors, radii=self.cfg.lidar_point_radius),
                )
            rr.log(f"{tbase}/hits", rr.Scalars(float(len(pts))))
            return "lidar"

        if kind == "camera":
            if not self.cfg.enable_cameras:
                return "camera"
            assert isinstance(payload, CameraImage)
            self._set_sim_time(b, payload.sim_time_s)
            pkey = (link, payload.width, payload.height)
            if pkey not in b.pinhole_logged and payload.fov_y_rad > 0.0:
                b.pinhole_logged.add(pkey)
                fy = float(payload.height) / (2.0 * math.tan(0.5 * payload.fov_y_rad))
                rr.log(
                    entity,
                    rr.Pinhole(
                        resolution=[payload.width, payload.height],
                        focal_length=fy,
                        camera_xyz=rr.ViewCoordinates.FLU,
                        image_plane_distance=1.2,
                    ),
                    static=True,
                )
            if payload.channel == CAMERA_COLOR:
                rgba = np.frombuffer(payload.data, dtype=np.uint8).reshape((payload.height, payload.width, 4))
                rr.log(f"{entity}/color", rr.Image(rgba[:, :, :3]))
            elif payload.channel == CAMERA_DEPTH:
                depths = np.frombuffer(payload.data, dtype="<f4").reshape((payload.height, payload.width)).copy()
                valid = np.isfinite(depths) & (depths > 0.0)
                depths[~valid] = 0.0
                d_lo = float(np.percentile(depths[valid], 2)) if np.any(valid) else 0.5
                d_hi = float(min(np.percentile(depths[valid], 98), 35.0)) if np.any(valid) else 25.0
                if d_hi <= d_lo + 0.1:
                    d_hi = d_lo + 5.0
                # Log depth under `cameras/...` (outside the 3D Pinhole) so 2D depth
                # displays cleanly in Turbo without projecting a 100 m sheet over the LiDAR.
                rr.log(
                    f"cameras/{b.id}/{link}/depth",
                    rr.DepthImage(depths, meter=1.0, colormap="turbo", depth_range=[d_lo, d_hi]),
                )
            return "camera"

        if kind == "measurement":
            assert isinstance(payload, Measurement)
            self._set_sim_time(b, payload.sim_time_s)
            kname = payload.kind_name
            recs = payload.records()
            props = payload.props

            if kname == "recognition":
                names = props.get("names", "").splitlines()
                mins, sizes, labels = [], [], []
                for i, r in enumerate(recs):
                    if len(r) < 12:
                        continue
                    left, top, right, bottom = r[2], r[3], r[4], r[5]
                    mins.append([left, top])
                    sizes.append([max(0.0, right - left), max(0.0, bottom - top)])
                    labels.append(names[i] if i < len(names) else f"obj_{int(r[0])}")
                if mins:
                    rr.log(
                        f"{entity}/color/recognition",
                        rr.Boxes2D(mins=mins, sizes=sizes, labels=labels, show_labels=True, colors=[[86, 180, 233]]),
                    )
                else:
                    rr.log(f"{entity}/color/recognition", rr.Clear(recursive=False))
                return "camera"

            if kname == "radar":
                pts, origins, vecs, colors, labels = [], [], [], [], []
                for r in recs:
                    if len(r) < 5:
                        continue
                    dist, az, el, vrad, pwr = r[:5]
                    if not (math.isfinite(dist) and math.isfinite(az) and math.isfinite(el)):
                        continue
                    ca, sa = math.cos(az), math.sin(az)
                    ce, se = math.cos(el), math.sin(el)
                    ux, uy, uz = ce * ca, ce * sa, se
                    p = [dist * ux, dist * uy, dist * uz]
                    pts.append(p)
                    origins.append(p)
                    vecs.append([vrad * ux, vrad * uy, vrad * uz])
                    colors.append([213, 94, 0] if vrad < -0.2 else ([0, 158, 115] if vrad > 0.2 else [86, 180, 233]))
                    labels.append(f"{dist:.1f}m {vrad:+.1f}m/s {pwr:.0f}dBm")
                if pts:
                    rr.log(f"{entity}/targets", rr.Points3D(pts, colors=colors, radii=0.22, labels=labels, show_labels=True))
                    rr.log(f"{entity}/velocity", rr.Arrows3D(origins=origins, vectors=vecs, colors=colors))
                else:
                    rr.log(f"{entity}/targets", rr.Clear(recursive=False))
                    rr.log(f"{entity}/velocity", rr.Clear(recursive=False))
                rr.log(f"{tbase}/targets", rr.Scalars(float(len(pts))))
                return "radar"

            if kname == "gps" and recs and len(recs[0]) >= 10:
                lat, lon, alt, east, north, up, speed = recs[0][:7]
                if math.isfinite(east) and math.isfinite(north) and math.isfinite(up):
                    b.style_series(f"{tbase}/enu_m", ["east", "north", "up"], [[213, 94, 0], [0, 158, 115], [86, 180, 233]])
                    rr.log(f"{tbase}/enu_m", rr.Scalars([east, north, up]))
                if math.isfinite(speed):
                    rr.log(f"{tbase}/speed_mps", rr.Scalars(speed))
                return "gps"

            if kname == "distance" and recs and len(recs[0]) >= 2:
                dist, val = recs[0][0], recs[0][1]
                if math.isfinite(dist):
                    rr.log(f"{entity}/ray", rr.Arrows3D(origins=[[0, 0, 0]], vectors=[[dist, 0, 0]], colors=[[230, 159, 0]]))
                    rr.log(f"{tbase}/distance_m", rr.Scalars(dist))
                if math.isfinite(val):
                    rr.log(f"{tbase}/value", rr.Scalars(val))
                return "distance"

            if kname == "light" and recs and len(recs[0]) >= 5:
                irr, direct, sky, sources, val = recs[0][:5]
                b.style_series(f"{tbase}/irradiance", ["total", "direct", "sky"])
                rr.log(f"{tbase}/irradiance", rr.Scalars([irr, direct, sky]))
                return "light"

            if kname == "climate" and recs and len(recs[0]) >= 5:
                temp, rh, co2, o2, press = recs[0][:5]
                rr.log(f"{tbase}/temp_c", rr.Scalars(temp))
                rr.log(f"{tbase}/rh", rr.Scalars(rh))
                rr.log(f"{tbase}/co2_ppm", rr.Scalars(co2))
                return "climate"

            if kname == "touch" and recs and len(recs[0]) >= 6:
                touching, contacts, force = recs[0][:3]
                rr.log(f"{tbase}/touching", rr.Scalars(touching))
                rr.log(f"{tbase}/force_n", rr.Scalars(force))
                return "touch"

            if kname == "position" and recs and len(recs[0]) >= 2:
                rr.log(f"{tbase}/position", rr.Scalars(recs[0][0]))
                rr.log(f"{tbase}/velocity", rr.Scalars(recs[0][1]))
                return "position"

            if kname == "receiver" and recs:
                sender = props.get("from", "?")
                raw = payload.section("data")
                text = raw.decode("utf-8", errors="replace") if raw else ""
                rr.log(f"{tbase}/packets", rr.TextLog(f"from={sender} ch={recs[0][0]:.0f} sig={recs[0][1]:.2f} {text}"))
                return "receiver"

            if recs:
                vals = [v for v in recs[0] if math.isfinite(v)]
                if vals:
                    rr.log(f"{tbase}/{kname}", rr.Scalars(vals))
            return kname

        return None
