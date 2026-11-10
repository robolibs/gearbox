#!/usr/bin/env python3
"""Drive gearbox machines with ondrive controllers through scenarios and score them.

    ondrive_lab.py characterize husky
    ondrive_lab.py run husky pure_pursuit s_curve
    ondrive_lab.py run husky all all --out target/lab/husky.json

ondrive works in the machine state frame: x east, y north, z up, heading
counter-clockwise from x (east). Loads take (north, up, east); draws go
in the robot's odom frame, `robot/odom`.
Every run gets fresh ground: each starts on its own patch north of home.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gearbox_client import Gearbox  # noqa: E402

import ondrive  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
TICK = 0.05
# Seconds a part of one control-loop pass may take before the run reports it.
SLOW_CALL_S = 1.0
# Draws queued past which the scene skips a frame; ONDRIVE_LAB_DRAW=0 draws none.
SCENE_BACKLOG = 32
AREA_FILE = REPO / "target" / "lab" / "area.txt"
# Areas the runs cycle through: twelve along each of the flat rows, north
# metres (the row at 240 is on a slope).
AREA_ROWS = (60.0, 150.0, 330.0)
AREAS = 12 * len(AREA_ROWS)
# Share of the nominal speed a run is allowed to average, for soil slip.
SLIP_ALLOWANCE = 0.25
# Areas tried before a run starts anyway, and the heading, radians, a
# freshly spawned machine must settle within.
SPAWN_TRIES = 3
SPAWN_YAW_TOLERANCE = 0.05
# Wall-clock seconds a loaded machine is waited on before its spawn fails.
LOAD_WAIT_S = 240.0
# Sim seconds after which a robot that never got STUCK_DISTANCE metres from its
# spawn counts as stuck in the terrain there.
STUCK_S = 20.0
STUCK_DISTANCE = 0.5
# Runs between viewer restarts when ONDRIVE_LAB_KITTY or ONDRIVE_LAB_DETACH is set.
RESTART_EVERY = 6
# Gearbox coasts on a command below COAST_MPS and COAST_RPS while the machine
# rolls faster than PARKED_MPS; a stop is sent as BRAKE_MPS in the direction
# of travel, which the drive holds, until the machine is slow enough to park.
COAST_MPS = 0.05
COAST_RPS = 0.02
PARKED_MPS = 0.25
BRAKE_MPS = 0.06
# Clearance, metres, below which the robot touches an obstacle and the run
# stops.
CONTACT_M = 0.0


@dataclass
class Machine:
    usd: str
    steering: str
    vmax: float
    wmax: float
    accel: float
    width: float
    length: float
    wheelbase: float = 0.0
    max_steer: float = 0.0
    track: float = 0.0
    steer_rate: float = 0.0
    rear_offset: float = 0.0
    # Metres the middle of the body lies ahead of the reported pose.
    body_offset: float = 0.0
    scale: float = 1.0


MACHINES: dict[str, Machine] = {
    "husky": Machine("assets/machines/usd/clearpath_husky.usdz", "skid_steer", vmax=1.0, wmax=1.5, accel=1.0,
                     width=0.67, length=0.99, track=0.55),
    "hunter": Machine("assets/machines/usd/agilex_hunter.usdz", "ackermann", vmax=1.5, wmax=1.5, accel=1.0,
                      width=0.75, length=1.06, wheelbase=0.65, max_steer=0.46, steer_rate=0.3, scale=1.5,
                      rear_offset=0.28, body_offset=0.045),
    "kubota": Machine("assets/machines/usd/kubota_tractor.usdz", "ackermann", vmax=2.0, wmax=0.7, accel=0.6,
                      width=2.3, length=5.1, wheelbase=2.7, max_steer=0.6, steer_rate=0.4, scale=3.0,
                      rear_offset=1.365, body_offset=-0.015),
    "agxeed": Machine("assets/machines/usd/agxeed_agbot.usdz", "skid_steer", vmax=1.5, wmax=1.0, accel=0.6,
                      width=3.0, length=4.5, track=2.5, scale=3.0),
}

CONTROLLERS = [
    "pid", "carrot", "pose_regulator", "apf", "dwa", "pose_reach",
    "pure_pursuit", "regulated_pursuit", "vector_pursuit", "stanley", "lqr", "kanayama", "flc", "ilc",
    "mpc", "ilqr", "mppi", "mca", "soc", "teb", "slip_adaptive", "velocity_obstacle",
]
OBSTACLE_AWARE = ["mca", "soc", "dwa", "teb", "regulated_pursuit", "apf", "mppi", "velocity_obstacle"]


# ─── scenarios ───────────────────────────────────────────────────────


@dataclass
class Scenario:
    name: str
    points: list[tuple[float, float]]
    speeds: list[float] = field(default_factory=list)
    closed: bool = False
    reverse: bool = False
    goal: tuple[float, float, float] | None = None
    static: list[tuple[float, float, float]] = field(default_factory=list)
    # Robots driving their own course: "start" (x, y, yaw), "speed" (m/s),
    # "meet" (x, y): held until timed to reach it as the robot does, or
    # None to set off at once.
    movers: list[dict] = field(default_factory=list)
    laps: float = 0.0
    time_limit: float = 0.0


def _arc(cx, cy, r, a0, a1, n):
    return [(cx + r * math.cos(a0 + (a1 - a0) * i / n), cy + r * math.sin(a0 + (a1 - a0) * i / n)) for i in range(n + 1)]


def scenario(name: str, s: float) -> Scenario:
    """Scenario `name` scaled by `s` (1 for a ~1 m robot)."""
    if name == "line":
        return Scenario(name, [(0, 0), (20 * s, 0)])
    if name == "s_curve":
        pts = [(x * s, 2.0 * s * math.sin(2 * math.pi * x / 20.0)) for x in [i * 0.5 for i in range(61)]]
        return Scenario(name, pts)
    if name == "corners":
        return Scenario(name, [(0, 0), (8 * s, 0), (8 * s, 6 * s), (2 * s, 6 * s), (2 * s, 12 * s)])
    if name == "u_turn":
        pts = [(0, 0), (8 * s, 0)] + _arc(8 * s, 3 * s, 3 * s, -math.pi / 2, math.pi / 2, 24)[1:] + [(0, 6 * s)]
        return Scenario(name, pts)
    if name == "cusp":
        fwd = [(x * s, 0.0) for x in [i * 0.5 for i in range(17)]]
        back = [(8 * s - 3 * s * math.sin(a), 3 * s * (1 - math.cos(a))) for a in [i * math.pi / 2 / 16 for i in range(1, 17)]]
        return Scenario(name, fwd + back, speeds=[1.0] * len(fwd) + [-1.0] * len(back), reverse=True)
    if name == "loop":
        pts = _arc(0, 5 * s, 5 * s, -math.pi / 2, 1.5 * math.pi, 48)[:-1]
        return Scenario(name, pts, closed=True, laps=2.0)
    if name == "park":
        return Scenario(name, [(0, 0), (6 * s, 0)], goal=(6 * s, 3 * s, math.pi / 2), reverse=True)
    if name == "static_obstacle":
        return Scenario(name, [(0, 0), (25 * s, 0)], static=[(12 * s, 0.1 * s, 0.6 * s)])
    if name == "crossing":
        return Scenario(name, [(0, 0), (30 * s, 0)], movers=[_mover((15 * s, -9 * s, math.pi / 2), 0.7, (15 * s, 0.0))])
    if name == "head_on":
        return Scenario(name, [(0, 0), (30 * s, 0)], movers=[_mover((30 * s, 0.2 * s, math.pi), 0.6)])
    if name == "slalom":
        return Scenario(name, [(0, 0), (32 * s, 0)],
                        static=[(8 * s, 0.4 * s, 0.5 * s), (15 * s, -0.4 * s, 0.5 * s), (22 * s, 0.4 * s, 0.5 * s)])
    if name == "overtake":
        return Scenario(name, [(0, 0), (40 * s, 0)], movers=[_mover((6 * s, 0.1 * s, 0.0), 0.3)])
    if name == "crossing_both":
        return Scenario(name, [(0, 0), (36 * s, 0)],
                        movers=[_mover((12 * s, -9 * s, math.pi / 2), 0.7, (12 * s, 0.0)),
                                _mover((24 * s, 9 * s, -math.pi / 2), 0.6, (24 * s, 0.0))])
    if name == "mixed":
        return Scenario(name, [(0, 0), (52 * s, 0)], static=[(8 * s, 0.1 * s, 0.6 * s)],
                        movers=[_mover((14 * s, -0.1 * s, 0.0), 0.3),
                                _mover((30 * s, 9 * s, -math.pi / 2), 0.7, (30 * s, 0.0)),
                                _mover((58 * s, 0.2 * s, math.pi), 0.5, (42 * s, 0.0))])
    raise SystemExit(f"unknown scenario {name}")


def _mover(start, speed, meet=None) -> dict:
    return {"start": start, "speed": speed, "meet": meet}


SCENARIOS = ["line", "s_curve", "corners", "u_turn", "cusp", "loop", "park", "static_obstacle", "crossing", "head_on",
             "slalom", "overtake", "crossing_both", "mixed"]
OBSTACLE_SCENARIOS = ["static_obstacle", "crossing", "head_on", "slalom", "overtake", "crossing_both", "mixed"]


# ─── geometry ────────────────────────────────────────────────────────


def wrap(a: float) -> float:
    return (a + math.pi) % (2 * math.pi) - math.pi


def densify(points, step):
    out = [points[0]]
    for (ax, ay), (bx, by) in zip(points, points[1:]):
        n = max(1, math.ceil(math.hypot(bx - ax, by - ay) / step))
        out += [(ax + (bx - ax) * k / n, ay + (by - ay) * k / n) for k in range(1, n + 1)]
    return out


def box_clearance(px, py, yaw, length, width, ox, oy, r, offset=0.0):
    """Distance from a circle (ox, oy, r) to a robot box centred `offset`
    ahead of (px, py)."""
    c, s = math.cos(yaw), math.sin(yaw)
    dx, dy = ox - (px + offset * c), oy - (py + offset * s)
    bx, by = c * dx + s * dy, -s * dx + c * dy
    qx, qy = max(abs(bx) - length / 2, 0.0), max(abs(by) - width / 2, 0.0)
    return math.hypot(qx, qy) - r


class Reference:
    """The scenario path, for scoring: cross-track error with forward progress."""

    def __init__(self, pts, closed):
        self.pts = pts + ([pts[0]] if closed else [])
        self.seg = 0
        self.lap_len = sum(math.dist(a, b) for a, b in zip(self.pts, self.pts[1:]))
        self.closed = closed
        self.travel = 0.0

    def error(self, x, y):
        """Distance to the nearest segment around the progress mark, which
        only moves forward: reversing out of a cusp scores against the leg
        leaving it, not the one it came in on."""
        n = len(self.pts) - 1
        best = (float("inf"), 0)
        for k in range(-3, 30):
            i = self.seg + k
            if self.closed:
                i %= n
            elif i < 0 or i >= n:
                continue
            (ax, ay), (bx, by) = self.pts[i], self.pts[i + 1]
            vx, vy = bx - ax, by - ay
            ll = vx * vx + vy * vy
            t = 0.0 if ll < 1e-12 else max(0.0, min(1.0, ((x - ax) * vx + (y - ay) * vy) / ll))
            d = math.hypot(ax + t * vx - x, ay + t * vy - y)
            if d < best[0] - 1e-9:
                best = (d, k)
        self.seg += max(best[1], 0)
        if self.closed:
            self.seg %= n
        return best[0]


# ─── the lab ─────────────────────────────────────────────────────────


def next_area() -> tuple[float, float]:
    AREA_FILE.parent.mkdir(parents=True, exist_ok=True)
    k = (int(AREA_FILE.read_text()) if AREA_FILE.exists() else 0) % AREAS
    AREA_FILE.write_text(str(k + 1))
    return 60.0 + 70.0 * (k % 12), AREA_ROWS[k // 12]


class SpawnFailed(RuntimeError):
    """A machine loaded into the viewer that never came up."""


class Lab:
    def __init__(self, instance: str):
        self.gb = Gearbox(instance)
        self.instance = instance
        self.origin = (0.0, 0.0)
        self.markers: list[str] = []
        self.runs = 0

    def fresh(self, force: bool = False):
        """Every `RESTART_EVERY` runs, or now when `force`, restart the viewer
        in the kitty window at `ONDRIVE_LAB_KITTY` with `ONDRIVE_LAB_LAUNCH`,
        or, with `ONDRIVE_LAB_DETACH`, stop it and run that shell command,
        and reconnect. Nothing when neither is set."""
        if not force:
            self.runs += 1
        kitty = os.environ.get("ONDRIVE_LAB_KITTY")
        detach = os.environ.get("ONDRIVE_LAB_DETACH")
        if (not kitty and not detach) or (not force and self.runs % RESTART_EVERY):
            return
        if not kitty:
            self.relaunch(detach)
            return
        launch = os.environ.get("ONDRIVE_LAB_LAUNCH", f"target/debug/gearbox run --name {self.instance} --ephemeral")
        send = lambda text: subprocess.run(["kitty", "@", "--to", kitty, "send-text", "--match", "id:1", text],  # noqa: E731
                                           capture_output=True, timeout=20)
        print(f"  restarting {self.instance} after {self.runs - 1} runs", flush=True)
        send("\x03")
        listed = lambda: self.instance in subprocess.run(  # noqa: E731
            [str(REPO / "target" / "debug" / "gearbox"), "instance", "list"], capture_output=True, text=True, timeout=20).stdout
        deadline = time.time() + 60
        while listed() and time.time() < deadline:
            time.sleep(1.0)
        send(launch + "\n")
        deadline = time.time() + 180
        while not listed():
            if time.time() > deadline:
                raise RuntimeError(f"{self.instance} did not come back")
            time.sleep(2.0)
        time.sleep(10.0)
        self.gb = Gearbox(self.instance)

    def relaunch(self, launch: str):
        """Stop the viewer through `gearbox instance stop`, start it again with
        the shell command `launch` (one that detaches), and reconnect."""
        gearbox = str(REPO / "target" / "debug" / "gearbox")
        print(f"  restarting {self.instance} after {self.runs - 1} runs", flush=True)
        subprocess.run([gearbox, "instance", "stop", self.instance, "--force"], capture_output=True, timeout=60)
        subprocess.run(launch, shell=True, cwd=REPO, capture_output=True, timeout=240)
        listed = lambda: self.instance in subprocess.run(  # noqa: E731
            [gearbox, "instance", "list"], capture_output=True, text=True, timeout=20).stdout
        deadline = time.time() + 180
        while not listed():
            if time.time() > deadline:
                raise RuntimeError(f"{self.instance} did not come back")
            time.sleep(2.0)
        time.sleep(10.0)
        self.gb = Gearbox(self.instance)

    def spawn(self, ns: str, m: Machine, x: float, y: float, yaw: float):
        """Spawn at (x, y) of the robot frame. A machine reports its pose from
        its own spawn point, axes x east and y north."""
        north, east = self.origin[0] + y, self.origin[1] + x
        self.gb.load_machine(ns, str(REPO / m.usd), x=north, z=east, yaw_deg=math.degrees(yaw))
        deadline = time.time() + LOAD_WAIT_S
        while True:
            try:
                mach = self.gb.machine(ns, timeout=30)
                break
            except RuntimeError:
                if time.time() > deadline:
                    raise SpawnFailed(f"machine `{ns}` did not appear within {LOAD_WAIT_S:.0f}s") from None
                time.sleep(2.0)
        mach.claim(hold_ms=800, client="ondrive_lab")
        t0, s = time.time(), None
        while time.time() - t0 < 30:
            drive(mach, 0.0, 0.0, s.vx if s is not None else 0.0)
            s = mach.state(wait=0.5)
            if s is not None and abs(s.vx) < 0.02 and time.time() - t0 > 2.0:
                break
            time.sleep(0.1)
        return mach

    def camera(self, ns: str, distance: float):
        """Keep the viewer on machine `ns`, from behind and above."""
        subprocess.run([str(REPO / "target" / "debug" / "gearbox"), "-i", self.instance, "instance", "camera", "look", ns,
                        "--bearing", "20", "--pitch", "45", "--distance", f"{distance:.1f}", "--over", "1.0"],
                       capture_output=True, timeout=20)

    def marker(self, key: str, x: float, y: float, z: float):
        self.gb.marker_set(key, self.origin[0] + y, z, self.origin[1] + x)
        if key not in self.markers:
            self.markers.append(key)

    def clear(self):
        self.gb.clear("machines")
        self.gb.clear("markers")
        self.markers.clear()


class Scene:
    """One run drawn in the viewer, in the robot's odom frame: the path as a
    flowing ribbon, the goal as a beacon, the driven trace, the controller's
    plan and target, the trajectories it sampled or solved tinted by cost,
    its detours, and every obstacle with its prediction."""

    NS = "lab"
    FRAME = "robot/odom"
    BODY = "robot/base_link"

    def __init__(self, gb: Gearbox, m: Machine, label: str):
        self.d = gb.draw
        self.d.wait = False
        self.enabled = os.environ.get("ONDRIVE_LAB_DRAW", "1") != "0"
        if self.enabled:
            self.d.clear(self.NS)
        self.m = m
        self.label = label
        self.ticks = 0

    def setup(self, path_pts, goal, statics, v_nom):
        if not self.enabled:
            return
        d, f, m = self.d, self.FRAME, self.m
        d.lines("lab/robot", "path", [(x, y, 0.05) for x, y in path_pts], frame=f, flow_speed=v_nom,
                width=0.25 * max(1.0, m.scale ** 0.5))
        gx, gy, gyaw = goal
        d.arrows("lab/robot", "goal", poses=[(gx, gy, 0.1, gyaw)], length=max(1.0, m.length), frame=f)
        for i, (x, y, r) in enumerate(statics):
            d.shapes("lab/obstacle", f"static_{i}", "ellipsoid", [(x, y, 0.0)], sizes=[(2 * r, 2 * r, 1.2 * r)], frame=f)

    def tick(self, st, tracker, cmd, moving, clearance):
        self.ticks += 1
        if not self.enabled or self.ticks % 4 or self.d.backlog() > SCENE_BACKLOG:
            return
        d, f, m = self.d, self.FRAME, self.m
        d.extend("lab/robot", "trace", [(st.x, st.y, 0.02)], max_points=4000, frame=f)
        plan = tracker.predicted_trajectory()
        if len(plan) > 1:
            d.lines("lab/robot", "horizon", [(p[0], p[1], 0.15) for p in plan], frame=f, lifetime=1.0)
        frame = tracker.debug_frame()
        if frame["rollouts"]:
            lo, hi = min(frame["costs"]), max(frame["costs"])
            cheap = [1.0 - (c - lo) / (hi - lo + 1e-9) for c in frame["costs"]]
            d.lines("lab/robot/rollouts", "samples", strips=[[(x, y, 0.08) for x, y in r] for r in frame["rollouts"]],
                    values=cheap, colormap="rdylgn", color="#ffffff", alpha=0.45, frame=f, lifetime=1.0)
        if frame["guides"]:
            d.lines("lab/robot", "detours", strips=[[(x, y, 0.06) for x, y in g] for g in frame["guides"]],
                    dashed=True, frame=f, lifetime=1.0)
        target = tracker.current_target()
        if target is not None:
            d.points("lab/robot", "lookahead", [(target[0], target[1], 0.3)], radius=0.1 * max(1.0, m.scale ** 0.5),
                     frame=f, lifetime=1.0)
        urgency = min(1.0, max(0.0, 1.0 - clearance / (1.5 * m.scale)))
        for i, (x, y, vx, vy, r) in enumerate(moving):
            d.shapes("lab/obstacle", f"moving_{i}", "ellipsoid", [(x, y, 0.0)], sizes=[(2 * r, 2 * r, r)],
                     frame=f, lifetime=1.0, urgency=urgency)
            times = [0.5 * k for k in range(1, 8)]
            centers = [(x + vx * t, y + vy * t, 0.03) for t in times]
            sizes = [(2 * (r + 2 * (0.05 + 0.1 * t)),) * 2 for t in times]
            d.shapes("lab/obstacle", f"moving_{i}_prediction", "ellipse", centers, sizes=sizes, fill="both",
                     frame=f, lifetime=1.0, urgency=urgency)
        if self.ticks % 12 == 0:
            d.text("lab/robot", "status", [(0.0, 0.0, m.length + 1.0)],
                   text=f"{self.label}   v {cmd.linear_velocity:+.2f}  ω {cmd.angular_velocity:+.2f}", frame=self.BODY,
                   lifetime=2.0)

    def finish(self, st, status: str):
        if not self.enabled:
            return
        self.d.text("lab/robot", "status", [(0.0, 0.0, self.m.length + 1.0)], text=f"{self.label}: {status}",
                    frame=self.BODY)
        self.d.flush(2.0)


def constraints(m: Machine, reverse: bool):
    c = ondrive.RobotConstraints.default_()
    c.steering_type = m.steering
    c.wheelbase = m.wheelbase or 1.0
    c.max_linear_velocity = m.vmax
    c.min_linear_velocity = -0.6 * m.vmax if reverse else 0.0
    c.max_angular_velocity = m.wmax
    c.max_linear_acceleration = m.accel
    c.max_angular_acceleration = 2.0 * m.wmax
    c.max_steering_angle = m.max_steer if m.steering == "ackermann" else 0.0
    c.min_turning_radius = 0.0
    c.track_width = m.track
    c.max_steering_rate = m.steer_rate
    c.rear_wheelbase = m.rear_offset
    c.robot_width = m.width
    c.robot_length = m.length
    front, back = m.body_offset + m.length / 2, m.body_offset - m.length / 2
    c.set_footprint(points=[(front, m.width / 2), (back, m.width / 2), (back, -m.width / 2), (front, -m.width / 2)])
    return c


def tracker_for(kind: str, m: Machine, sc: Scenario, params: dict):
    t = ondrive.Tracker(kind)
    cfg = ondrive.ControllerConfig.default_()
    cfg.output_units = "physical"
    cfg.allow_reverse = sc.reverse
    cfg.lookahead_distance = max(0.8, 1.2 * m.length)
    cfg.goal_tolerance = 0.25 * max(1.0, m.scale)
    cfg.angular_tolerance = 0.25 if sc.goal else math.pi
    t.set_config(cfg)
    t.init(constraints(m, sc.reverse))
    t.set_seed(7)
    t.set_debug("full")
    if os.environ.get("ONDRIVE_SAFETY") and hasattr(t, "set_safety"):
        t.set_safety(json.loads(os.environ["ONDRIVE_SAFETY"]))
    if kind == "flc":
        params = {"base_velocity": 0.8 * m.vmax, **params}
    if params:
        t.set_params(params)
    return t


PRED_DT = 0.2
PRED_STEPS = 30


def predicted(obs):
    """Constant-velocity Gaussian prediction of an observed obstacle, every
    PRED_DT seconds, its spread growing with time when it moves."""
    x, y, vx, vy, r = obs
    mx = [x + vx * k * PRED_DT for k in range(PRED_STEPS + 1)]
    my = [y + vy * k * PRED_DT for k in range(PRED_STEPS + 1)]
    moving = math.hypot(vx, vy) > 0.05
    sd = [0.05 + (0.1 if moving else 0.0) * k * PRED_DT for k in range(PRED_STEPS + 1)]
    return (1.0, mx, my, sd, sd)


def send(machine, v: float, w: float):
    """`cmd_vel` that survives one dropped call: the next tick sends again."""
    try:
        machine.cmd_vel(v, w)
    except RuntimeError as e:
        print(f"  command dropped: {e}", flush=True)


def drive(machine, v: float, w: float, vx: float):
    """`send` with a stop braking the machine instead of coasting it."""
    if abs(v) < COAST_MPS and abs(w) < COAST_RPS and abs(vx) > PARKED_MPS:
        v = math.copysign(BRAKE_MPS, vx)
    send(machine, v, w)


class SimClock:
    """Seconds of simulated world time, the last reading held when a call fails."""

    def __init__(self, gb):
        self.gb, self.last = gb, gb.clock().world_s

    def __call__(self) -> float:
        try:
            self.last = self.gb.clock().world_s
        except RuntimeError:
            pass
        return self.last


def run_one(lab: Lab, mname: str, kind: str, sname: str, params: dict, verbose: bool) -> dict:
    m = MACHINES[mname]
    sc = scenario(sname, m.scale)
    lab.fresh()
    for _ in range(SPAWN_TRIES):
        lab.origin = next_area()
        lab.clear()
        robot = lab.spawn("robot", m, 0.0, 0.0, 0.0)
        s0 = robot.state(wait=1.0)
        if s0 is not None and abs(wrap(s0.heading_rad)) < SPAWN_YAW_TOLERANCE:
            break
        print(f"  spawn at {lab.origin} settled at yaw {s0.heading_rad if s0 else float('nan'):+.2f}, next area", flush=True)
    lab.camera("robot", 14.0 * m.scale)
    ox, oy = s0.x, s0.y
    scene = Scene(lab.gb, m, f"{kind} · {sname}")

    pts = [(ox + x, oy + y) for x, y in sc.points]
    path = ondrive.Path(is_closed=sc.closed)
    for i, (x, y) in enumerate(pts):
        nx, ny = pts[min(i + 1, len(pts) - 1)]
        px, py = pts[max(i - 1, 0)]
        yaw = math.atan2(ny - py, nx - px)
        sp = sc.speeds[i] if sc.speeds else None
        if sp is not None and sp < 0:
            yaw = wrap(yaw + math.pi)
        path.add_waypoint_xy(x, y, yaw=yaw, speed=sp)
    path.smoothen(0.2 * max(1.0, m.scale ** 0.5))
    ref = Reference(densify(pts, 0.1), sc.closed)

    t = tracker_for(kind, m, sc, params)
    if sc.goal:
        gx, gy, gyaw = sc.goal
        t.set_goal(ondrive.Goal(target_pose=((ox + gx, oy + gy, 0.0), gyaw), tolerance_position=0.25 * m.scale, tolerance_orientation=0.25))
    else:
        t.set_path(path)

    statics = [(ox + x, oy + y, r) for x, y, r in sc.static]

    movers = []
    for k, spec in enumerate(sc.movers):
        ix, iy, iyaw = spec["start"]
        movers.append({"spec": spec, "machine": lab.spawn(f"mover_{k}", MACHINES["husky"], ox + ix, oy + iy, iyaw),
                       "origin": (ox + ix, oy + iy), "going": spec.get("meet") is None})

    v_nom = 0.8 * m.vmax
    end_pt = pts[-1] if not sc.closed else pts[0]
    goal_pose = (ox + sc.goal[0], oy + sc.goal[1], sc.goal[2]) if sc.goal else (end_pt[0], end_pt[1], math.atan2(pts[-1][1] - pts[-2][1], pts[-1][0] - pts[-2][0]))
    scene.setup(densify(pts, 0.2), goal_pose, statics, v_nom)
    length = ref.lap_len * (sc.laps or 1.0)
    limit = sc.time_limit or (length / (SLIP_ALLOWANCE * v_nom) + 15.0)
    lost_after, lost_since = max(3.0, 2.0 * m.length), None
    log = []
    try:
        clock = SimClock(lab.gb)
    except RuntimeError:
        clock = time.time
    t0 = clock()
    last = t0
    min_clear = float("inf")
    farthest, driven, last_xy = 0.0, 0.0, (ox, oy)
    flips, last_w = 0, 0.0
    status = "timeout"
    while True:
        wall = time.time()
        now = clock()
        if now - t0 > limit:
            break
        marks = [("clock", time.time())]
        st = robot.state()
        if st is None:
            time.sleep(TICK)
            continue
        world = ondrive.World()
        obs_now = [(x, y, 0.0, 0.0, r) for x, y, r in statics]
        for mv in movers:
            ist = mv["machine"].state()
            if ist is None:
                continue
            spec, (mx0, my0) = mv["spec"], mv["origin"]
            if not mv["going"]:
                mx, my = spec["meet"]
                d_robot = math.hypot(ox + mx - st.x, oy + my - st.y)
                d_mover = math.hypot(ox + mx - mx0 - ist.x, oy + my - my0 - ist.y)
                v_robot = max(abs(st.vx), 0.3 * v_nom)
                mv["going"] = d_mover / spec["speed"] + 1.2 >= d_robot / v_robot
            herr = wrap(spec["start"][2] - ist.heading_rad)
            send(mv["machine"], spec["speed"] if mv["going"] else 0.0, 1.5 * herr if mv["going"] else 0.0)
            ivx = ist.vx * math.cos(ist.heading_rad)
            ivy = ist.vx * math.sin(ist.heading_rad)
            obs_now.append((mx0 + ist.x, my0 + ist.y, ivx, ivy, 0.6))
        marks.append(("movers", time.time()))
        for i, o in enumerate(obs_now):
            world.add_obstacle(i, o[4], [predicted(o)], dt=PRED_DT)
        clears = [box_clearance(st.x, st.y, st.heading_rad, m.length, m.width, o[0], o[1], o[4], m.body_offset) for o in obs_now]
        now_clear = min(clears, default=float("inf"))
        near = obs_now[clears.index(now_clear)] if clears else None
        min_clear = min(min_clear, now_clear)

        state = ondrive.RobotState(pose=((st.x, st.y, st.z), st.heading_rad), velocity=(st.vx, st.wz, st.vy),
                                   allow_move=True, allow_reverse=sc.reverse)
        dt = max(1e-3, now - last)
        last = now
        cmd = t.tick(state, dt, world)
        marks.append(("tick", time.time()))
        v, w = (cmd.linear_velocity, cmd.angular_velocity) if cmd.valid else (0.0, 0.0)
        drive(robot, v, w, st.vx)
        marks.append(("drive", time.time()))
        if abs(w) > 0.2 * m.wmax and last_w * w < 0:
            flips += 1
        if abs(w) > 0.2 * m.wmax:
            last_w = w
        err = ref.error(st.x, st.y)
        log.append([round(now - t0, 3), round(st.x - ox, 3), round(st.y - oy, 3), round(st.heading_rad, 3),
                    round(st.vx, 3), round(st.wz, 3), round(v, 3), round(w, 3), round(err, 3), cmd.valid,
                    round(now_clear, 3) if near else None,
                    [round(near[0] - ox, 2), round(near[1] - oy, 2)] if near else None])
        if verbose and len(log) % 20 == 0:
            print(f"  t={now - t0:5.1f} err={err:5.2f} v={v:+.2f} vx={st.vx:+.2f} w={w:+.2f} "
                  f"near={now_clear:5.2f} min={min_clear:5.2f} {cmd.status_message}", flush=True)
        scene.tick(st, t, cmd, [o for o in obs_now if o[2] or o[3]], now_clear)
        marks.append(("scene", time.time()))
        slow = [(name, at - before) for (_, before), (name, at) in zip([("start", wall)] + marks, marks) if at - before > SLOW_CALL_S]
        if slow:
            print(f"  slow loop at t={now - t0:.1f}: " + ", ".join(f"{name} {s:.1f} s" for name, s in slow), flush=True)
        if not cmd.valid and "completed" not in cmd.status_message.lower():
            status = f"invalid: {cmd.status_message}"
        if now_clear < CONTACT_M:
            status = f"contact: {now_clear:.2f} m at t={now - t0:.1f}"
            print(f"  CONTACT {now_clear:.2f} m at t={now - t0:.1f} with obstacle at "
                  f"({near[0] - ox:.1f}, {near[1] - oy:.1f})", flush=True)
            break
        if t.is_path_completed() or (sc.goal and t.is_goal_reached()):
            status = "completed"
            break
        lost_since = (lost_since or now) if err > lost_after and not sc.goal else None
        if lost_since is not None and now - lost_since > 3.0:
            status = f"lost: {err:.1f} m off the path"
            break
        farthest = max(farthest, math.hypot(st.x - ox, st.y - oy))
        driven += math.hypot(st.x - last_xy[0], st.y - last_xy[1])
        last_xy = (st.x, st.y)
        if sc.closed and driven >= length:
            status = "completed"
            break
        if now - t0 > STUCK_S and farthest < STUCK_DISTANCE:
            status = "stuck: terrain at the spawn"
            break
        time.sleep(max(0.0, TICK - (time.time() - wall)))
    for _ in range(60):
        st = robot.state() or st
        drive(robot, 0.0, 0.0, st.vx)
        for mv in movers:
            send(mv["machine"], 0.0, 0.0)
        if abs(st.vx) <= PARKED_MPS:
            break
        time.sleep(0.05)
    for _ in range(5):
        send(robot, 0.0, 0.0)
        time.sleep(0.05)
    st = robot.state() or st
    scene.finish(st, status)
    errs = [r[8] for r in log]
    travelled = sum(math.hypot(a[1] - b[1], a[2] - b[2]) for a, b in zip(log, log[1:]))
    end = sc.goal or (sc.points[-1][0], sc.points[-1][1], None)
    final_pos = math.hypot(st.x - (ox + end[0]), st.y - (oy + end[1]))
    final_yaw = abs(wrap(st.heading_rad - end[2])) if end[2] is not None else None
    if sc.closed:
        status = "completed" if travelled >= ref.lap_len * sc.laps * 0.95 else status
    result = {
        "machine": mname, "controller": kind, "scenario": sname, "status": status, "area": list(lab.origin),
        "time": round(log[-1][0] if log else 0.0, 2), "travelled": round(travelled, 2),
        "cte_rms": round(math.sqrt(sum(e * e for e in errs) / max(1, len(errs))), 3),
        "cte_max": round(max(errs) if errs else 0.0, 3),
        "final_pos": round(final_pos, 3), "final_yaw": round(final_yaw, 3) if final_yaw is not None else None,
        "min_clearance": round(min_clear, 3) if min_clear != float("inf") else None,
        "flips_per_min": round(flips / max(1e-3, (log[-1][0] if log else 1.0) / 60.0), 1),
        "params": params, "log": log,
    }
    return result


def characterize(lab: Lab, mname: str, steps_out: str = ""):
    m = MACHINES[mname]
    lab.fresh()
    lab.origin = next_area()
    lab.clear()
    mach = lab.spawn("robot", m, 0.0, 0.0, 0.0)

    def hold(v, w, secs):
        out = []
        t0 = time.time()
        while time.time() - t0 < secs:
            mach.cmd_vel(v, w)
            s = mach.state()
            out.append((time.time() - t0, s.vx, s.wz, s.heading_rad))
            time.sleep(TICK)
        return out

    hold(0, 0, 1.0)
    run = hold(3.0 * m.vmax, 0.0, 6.0)
    vmax = max(r[1] for r in run)
    t63 = next((r[0] for r in run if r[1] >= 0.63 * vmax), None)
    hold(0, 0, 3.0)
    spin_v = 0.0 if m.steering != "ackermann" else 0.6 * m.vmax
    turn = hold(spin_v, 3.0, 5.0)
    wmax = max(abs(r[2]) for r in turn)
    t90 = next((r[0] for r in turn if abs(r[2]) >= 0.9 * wmax), None)
    out = {"machine": mname, "vmax": round(vmax, 3), "t63_speed": t63, "wmax": round(wmax, 3), "t90_turn": t90}
    if m.steering == "ackermann":
        v_turn = sum(r[1] for r in turn[-20:]) / 20
        k = wmax / max(v_turn, 1e-3)
        out.update(curvature_max=round(k, 3), turn_radius=round(1 / k, 3), steer_at_wheelbase=round(math.atan(k * m.wheelbase), 3))
        hold(0, 0, 3.0)
        v = 0.6 * m.vmax
        steps = [(0.0, 2.0), (1.2, 3.0), (-1.2, 4.0), (0.6, 3.0), (-0.3, 3.0), (1.2, 3.0), (0.0, 3.0)]
        log, t0 = [], time.time()
        for steer, secs in steps:
            w = v * math.tan(steer) / m.wheelbase
            start = time.time() - t0
            log += [[round(start + tr, 3), 0.0, 0.0, hd, vx, wz, v, w, 0.0, True] for tr, vx, wz, hd in hold(v, w, secs)]
        if steps_out:
            Path(steps_out).parent.mkdir(parents=True, exist_ok=True)
            Path(steps_out).write_text(json.dumps([{"machine": mname, "log": log}]))
    hold(0, 0, 2.0)
    print(json.dumps(out))
    return out


def fit_steering(results_file: str, wheelbase: float):
    """Fit an Ackermann machine's steering from a `run --out` log: pure delay,
    steering rate and the largest steering angle it reaches, by grid search
    of a rate-limited, clamped, delayed model against the measured yaw rate."""
    r = json.loads(Path(results_file).read_text())[0]
    t, vx, wz, vc, wc = ([row[k] for row in r["log"]] for k in (0, 4, 5, 6, 7))
    moving = [i for i in range(len(t)) if vx[i] > 0.3 and vc[i] > 0.3]
    cmd = [math.atan(wc[i] * wheelbase / max(vc[i], 0.1)) for i in range(len(t))]
    seen = [math.atan(wz[i] * wheelbase / max(vx[i], 0.1)) for i in range(len(t))]
    best = None
    for delay in range(0, 16, 2):
        for rate in (0.15, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.5, 2.0, 3.0, 5.0):
            for limit in (0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.8):
                s, err, steer = 0.0, 0.0, []
                for i in range(len(t)):
                    step = rate * (t[i] - t[i - 1] if i else TICK)
                    s += max(-step, min(step, max(-limit, min(limit, cmd[max(0, i - delay)])) - s))
                    steer.append(s)
                err = math.sqrt(sum((steer[i] - seen[i]) ** 2 for i in moving) / max(1, len(moving)))
                if best is None or err < best[0]:
                    best = (err, delay * TICK, rate, limit)
    out = {"rms": round(best[0], 3), "delay_s": round(best[1], 2), "steer_rate": best[2], "max_steer": best[3],
           "max_seen": round(max(abs(seen[i]) for i in moving), 3)}
    print(json.dumps(out))
    return out


def stop_test(lab: Lab, mname: str):
    """Distance a machine rolls after a bare zero command and after `drive`'s stop."""
    m = MACHINES[mname]
    lab.fresh()
    lab.origin = next_area()
    lab.clear()
    mach = lab.spawn("robot", m, 0.0, 0.0, 0.0)
    clock = SimClock(lab.gb)

    def hold(v, secs, braking=False):
        t0, s = clock(), mach.state()
        x0, y0 = s.x, s.y
        while clock() - t0 < secs:
            s = mach.state() or s
            drive(mach, v, 0.0, s.vx) if braking else send(mach, v, 0.0)
            time.sleep(TICK)
        return math.hypot(s.x - x0, s.y - y0), s.vx

    out = {"machine": mname}
    for mode in ("zero", "brake"):
        hold(0.0, 2.0, braking=True)
        _, v = hold(0.8 * m.vmax, 8.0)
        dist, v_end = hold(0.0, 8.0, braking=mode == "brake")
        out[mode] = {"from_mps": round(v, 2), "rolled_m": round(dist, 2), "end_mps": round(v_end, 2)}
    print(json.dumps(out), flush=True)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--instance", default="lab")
    sub = ap.add_subparsers(dest="cmd", required=True)
    ch = sub.add_parser("characterize")
    ch.add_argument("machine")
    ch.add_argument("--out", default="", help="save the steering step log, for `fit`")
    ft = sub.add_parser("fit", help="fit an Ackermann machine's steering from a `run --out` file")
    ft.add_argument("machine")
    ft.add_argument("results")
    sp = sub.add_parser("stop", help="distance rolled after a zero command and after a braking stop")
    sp.add_argument("machine")
    rn = sub.add_parser("run")
    rn.add_argument("machine")
    rn.add_argument("controller")
    rn.add_argument("scenario")
    rn.add_argument("--params", default="{}", help="controller parameters as JSON")
    rn.add_argument("--out", default="")
    rn.add_argument("-v", "--verbose", action="store_true")
    a = ap.parse_args()
    if a.cmd == "fit":
        fit_steering(a.results, MACHINES[a.machine].wheelbase)
        return
    lab = Lab(a.instance)
    if a.cmd == "characterize":
        characterize(lab, a.machine, a.out)
        return
    if a.cmd == "stop":
        stop_test(lab, a.machine)
        return
    kinds = CONTROLLERS if a.controller == "all" else a.controller.split(",")
    names = SCENARIOS if a.scenario == "all" else a.scenario.split(",")
    results = []
    print(f"{'machine':8} {'controller':18} {'scenario':16} {'status':12} {'time':>6} {'rms':>6} {'max':>6} {'final':>6} {'yaw':>5} {'clear':>6} {'flips':>5}")
    for sname in names:
        for kind in kinds:
            if sname in OBSTACLE_SCENARIOS and kind not in OBSTACLE_AWARE and a.controller == "all":
                continue
            try:
                r = run_one(lab, a.machine, kind, sname, json.loads(a.params), a.verbose)
            except SpawnFailed as e:
                print(f"  {e}; restarting {lab.instance} and running it again", flush=True)
                lab.fresh(force=True)
                r = run_one(lab, a.machine, kind, sname, json.loads(a.params), a.verbose)
            results.append(r)
            yaw = "" if r["final_yaw"] is None else f"{r['final_yaw']:.2f}"
            clear = "" if r["min_clearance"] is None else f"{r['min_clearance']:.2f}"
            print(f"{r['machine']:8} {kind:18} {sname:16} {r['status'][:12]:12} {r['time']:6.1f} {r['cte_rms']:6.2f} "
                  f"{r['cte_max']:6.2f} {r['final_pos']:6.2f} {yaw:>5} {clear:>6} {r['flips_per_min']:5.1f}", flush=True)
            if a.out:
                Path(a.out).parent.mkdir(parents=True, exist_ok=True)
                Path(a.out).write_text(json.dumps(results))


if __name__ == "__main__":
    main()
