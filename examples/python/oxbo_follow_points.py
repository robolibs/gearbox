#!/usr/bin/env python3
"""Flatland Oxbo point-following demo.

Run Gearbox first:
    make run

Then run:
    python scripts/oxbo_follow_points.py [x z x z ...]

No bales, no tractors. This loads the flat pea field, loads one Oxbo pea
harvester USD, then streams cmd_vel commands so it follows a loop of field
points, marked with red markers.
"""

from __future__ import annotations

import math
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox, Machine  # noqa: E402

FLATLAND_USD_PATH = "world/peafield.usd"
OXBO_USD_PATH = "bin/gearbox/assets/oxbo.usd"
NAMESPACE = "oxbo"
TICK_DT = 0.10
GOAL_TOLERANCE_M = 3.0
WAYPOINT_MARKER_Y = 0.35
WAYPOINT_MARKER_CLEAR_COUNT = 64

DEFAULT_POINTS: list[tuple[float, float]] = [
    (0.0, 42.0),
    (38.0, 42.0),
    (38.0, -28.0),
    (-38.0, -28.0),
    (-38.0, 42.0),
    (0.0, 42.0),
]


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def publish_waypoint_markers(gb: Gearbox, prefix: str, points: list[tuple[float, float]]) -> None:
    for i in range(WAYPOINT_MARKER_CLEAR_COUNT):
        gb.marker_delete(f"{prefix}_{i}")
    for i, (x, z) in enumerate(points[:WAYPOINT_MARKER_CLEAR_COUNT]):
        gb.marker_set(f"{prefix}_{i}", x, WAYPOINT_MARKER_Y, z)


def parse_points(argv: list[str]) -> list[tuple[float, float]]:
    if not argv:
        return DEFAULT_POINTS
    if len(argv) % 2 != 0:
        raise SystemExit("points must be x z pairs, e.g. 0 40 40 40 40 -20")
    vals = [float(v) for v in argv]
    return list(zip(vals[0::2], vals[1::2], strict=True))


def step_toward(machine: Machine, points: list[tuple[float, float]], goal_idx: int) -> int:
    """One control tick. Returns the (possibly advanced) goal index."""
    pose = machine.state()
    if pose is None:
        machine.cmd_vel(0.0, 0.0)
        return goal_idx
    gx, gz = points[goal_idx]
    dx, dz = gx - pose.x, gz - pose.z
    dist = math.hypot(dx, dz)
    if dist < GOAL_TOLERANCE_M:
        print(f"{machine.namespace}: reached point {goal_idx}: x={gx:.1f}, z={gz:.1f}")
        goal_idx = (goal_idx + 1) % len(points)
        gx, gz = points[goal_idx]
        dx, dz = gx - pose.x, gz - pose.z
        dist = math.hypot(dx, dz)
    heading_err = wrap_pi(math.atan2(dx, dz) - pose.heading_rad)
    abs_err = abs(heading_err)
    speed = clamp(dist * 0.18, 0.9, 2.4)
    if abs_err > math.radians(75.0):
        speed = 1.0
    elif abs_err > math.radians(35.0):
        speed = min(speed, 1.35)
    yaw_rate = clamp(1.15 * heading_err, -0.85, 0.85)
    machine.cmd_vel(speed, yaw_rate)
    return goal_idx


def follow_points(machine: Machine, points: list[tuple[float, float]], record_tf: bool = False) -> None:
    goal_idx = 0
    print("following points:")
    for i, (x, z) in enumerate(points):
        print(f"  {i}: x={x:.1f}, z={z:.1f}")
    print("Ctrl-C to stop.")
    if record_tf:
        machine.tf(True)
        print("recording wheel link poses (tf) once a second")
    last_tf_print = time.time()
    try:
        while True:
            goal_idx = step_toward(machine, points, goal_idx)
            if record_tf:
                # Drain the tf stream; keep the newest pose per wheel link.
                wheels: dict[str, dict] = {}
                while (pose := machine.tf_next(0.0)) is not None:
                    if "wheel" in pose.get("name", ""):
                        wheels[pose["name"]] = pose
                if wheels and time.time() - last_tf_print >= 1.0:
                    last_tf_print = time.time()
                    for name in sorted(wheels):
                        p = wheels[name]
                        print(f"  tf {name}: ({p['x']:+.2f}, {p['y']:+.2f}, {p['z']:+.2f}) q({p['qw']:+.2f}, {p['qx']:+.2f}, {p['qy']:+.2f}, {p['qz']:+.2f})")
            time.sleep(TICK_DT)
    except KeyboardInterrupt:
        pass
    finally:
        if record_tf:
            machine.tf(False)
        machine.stop()
        machine.release()
        print("stopped Oxbo")


def main() -> None:
    argv = [a for a in sys.argv[1:] if a != "--tf"]
    record_tf = len(argv) != len(sys.argv) - 1
    points = parse_points(argv)
    gb = Gearbox()
    gb.wait_ready()
    gb.clear()
    time.sleep(0.3)
    gb.load("flatland_terrain", FLATLAND_USD_PATH, category="terrain")
    publish_waypoint_markers(gb, "oxbo_waypoint", points)
    print(f"spawning fresh {OXBO_USD_PATH}")
    gb.load_machine(NAMESPACE, OXBO_USD_PATH, label="oxbo.usd")
    machine = gb.machine(NAMESPACE, timeout=90.0)
    machine.claim(hold_ms=1000, take=True, client="oxbo_follow_points.py")
    if machine.state(wait=30.0) is None:
        raise SystemExit(f"no state from machine `{NAMESPACE}`. Is Gearbox running?")
    follow_points(machine, points, record_tf)


if __name__ == "__main__":
    main()
