#!/usr/bin/env python3
"""Flatland Oxbo point-following demo.

Run Gearbox first:

    make run

Then run:

    python scripts/oxbo_follow_points.py

No bales, no tractors. This loads the flat textured terrain, loads one Oxbo
pea harvester USD, then streams cmd_vel commands so it follows a small loop of
field points.
"""

from __future__ import annotations

import math
import sys
import threading
import time
from dataclasses import dataclass

import cbor2
import zenoh


FLATLAND_USD_PATH = "world/flatland.usd"
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


def open_session() -> zenoh.Session:
    opened = zenoh.open(zenoh.Config())
    return opened.wait() if hasattr(opened, "wait") else opened


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    session.put(
        key,
        cbor2.dumps(payload),
        congestion_control=zenoh.CongestionControl.BLOCK,
    )


def clear_sim(session: zenoh.Session) -> None:
    put_cbor(session, "gearbox/sim/clear", {"pause_clock": False})


def delete_waypoint_markers(session: zenoh.Session) -> None:
    for i in range(WAYPOINT_MARKER_CLEAR_COUNT):
        put_cbor(session, f"gearbox/usd/mark/oxbo_waypoint_{i}/delete", {})


def publish_waypoint_markers(
    session: zenoh.Session,
    points: list[tuple[float, float]],
) -> None:
    delete_waypoint_markers(session)
    for i, (x, z) in enumerate(points[:WAYPOINT_MARKER_CLEAR_COUNT]):
        put_cbor(
            session,
            f"gearbox/usd/mark/oxbo_waypoint_{i}/{x:.3f}/{WAYPOINT_MARKER_Y:.3f}/{z:.3f}",
            {},
        )


@dataclass
class MachinePose:
    x: float = 0.0
    z: float = 0.0
    heading_rad: float = 0.0
    seen: bool = False


class PoseTracker:
    def __init__(self, session: zenoh.Session, namespace: str):
        self.pose = MachinePose()
        self._lock = threading.Lock()

        def on_state(sample: zenoh.Sample) -> None:
            try:
                d = cbor2.loads(bytes(sample.payload))
            except Exception:  # noqa: BLE001
                return
            p = d.get("position", [0.0, 0.0, 0.0])
            with self._lock:
                self.pose.x = float(p[0])
                self.pose.z = float(p[2])
                self.pose.heading_rad = float(d.get("heading_rad", 0.0))
                self.pose.seen = True

        self._sub = session.declare_subscriber(
            f"gearbox/machines/{namespace}/state",
            on_state,
        )

    def snapshot(self) -> MachinePose:
        with self._lock:
            return MachinePose(
                x=self.pose.x,
                z=self.pose.z,
                heading_rad=self.pose.heading_rad,
                seen=self.pose.seen,
            )

    def clear_seen(self) -> None:
        with self._lock:
            self.pose.seen = False

    def close(self) -> None:
        del self._sub


def load_flatland(session: zenoh.Session) -> None:
    put_cbor(
        session,
        "gearbox/usd/load/flatland_terrain",
        {
            "category": "terrain",
            "usd_path": FLATLAND_USD_PATH,
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "remove": False,
        },
    )


def load_oxbo(session: zenoh.Session) -> None:
    put_cbor(
        session,
        f"gearbox/usd/load/{NAMESPACE}",
        {
            "category": "machine",
            "usd_path": OXBO_USD_PATH,
            "namespace": NAMESPACE,
            "label": "oxbo.usd",
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "yaw_deg": 0.0,
            "remove": False,
        },
    )


def publish_cmd(session: zenoh.Session, speed: float, yaw_rate: float) -> None:
    put_cbor(
        session,
        f"gearbox/machines/{NAMESPACE}/cmd_vel",
        {
            "linear": [float(speed), 0.0, 0.0],
            "angular": [0.0, 0.0, float(yaw_rate)],
        },
    )


def wait_for_pose(tracker: PoseTracker, timeout_s: float) -> bool:
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        if tracker.snapshot().seen:
            return True
        time.sleep(0.05)
    return False


def parse_points(argv: list[str]) -> list[tuple[float, float]]:
    if not argv:
        return DEFAULT_POINTS
    if len(argv) % 2 != 0:
        raise SystemExit("points must be x z pairs, e.g. 0 40 40 40 40 -20")
    vals = [float(v) for v in argv]
    return list(zip(vals[0::2], vals[1::2], strict=True))


def follow_points(session: zenoh.Session, tracker: PoseTracker, points: list[tuple[float, float]]) -> None:
    goal_idx = 0
    print("following points:")
    for i, (x, z) in enumerate(points):
        print(f"  {i}: x={x:.1f}, z={z:.1f}")
    print("Ctrl-C to stop.")

    try:
        while True:
            pose = tracker.snapshot()
            if not pose.seen:
                publish_cmd(session, 0.0, 0.0)
                time.sleep(TICK_DT)
                continue

            gx, gz = points[goal_idx]
            dx = gx - pose.x
            dz = gz - pose.z
            dist = math.hypot(dx, dz)
            if dist < GOAL_TOLERANCE_M:
                print(f"reached point {goal_idx}: x={gx:.1f}, z={gz:.1f}")
                goal_idx = (goal_idx + 1) % len(points)
                gx, gz = points[goal_idx]
                dx = gx - pose.x
                dz = gz - pose.z
                dist = math.hypot(dx, dz)

            target_heading = math.atan2(dx, dz)
            heading_err = wrap_pi(target_heading - pose.heading_rad)
            abs_err = abs(heading_err)

            # Keep rolling while turning. Ackermann/raycast vehicles need
            # forward motion; crawling at 0.45 m/s makes point turns look stuck.
            speed = clamp(dist * 0.18, 0.9, 2.4)
            if abs_err > math.radians(75.0):
                speed = 1.0
            elif abs_err > math.radians(35.0):
                speed = min(speed, 1.35)

            yaw_rate = clamp(1.15 * heading_err, -0.85, 0.85)
            publish_cmd(session, speed, yaw_rate)
            time.sleep(TICK_DT)
    except KeyboardInterrupt:
        pass
    finally:
        publish_cmd(session, 0.0, 0.0)
        print("stopped Oxbo")


def main() -> None:
    points = parse_points(sys.argv[1:])
    session = open_session()
    tracker = PoseTracker(session, NAMESPACE)
    time.sleep(0.2)

    clear_sim(session)
    tracker.clear_seen()
    time.sleep(0.3)
    load_flatland(session)
    publish_waypoint_markers(session, points)
    print(f"spawning fresh {OXBO_USD_PATH}")
    load_oxbo(session)
    tracker.clear_seen()
    if not wait_for_pose(tracker, 8.0):
        raise SystemExit(
            f"no state from machine namespace `{NAMESPACE}`. Is Gearbox running?"
        )

    follow_points(session, tracker, points)
    tracker.close()


if __name__ == "__main__":
    main()
