#!/usr/bin/env python3
"""Spawn several USD tractors, scatter USD bales, and collect them greedily.

Usage:
    python scripts/bale_run_multi.py [n_tractors] [n_bales] [field_size] [seed]

Defaults: n_tractors=3, n_bales=50, field_size=300, seed=42.

This uses the same Gearbox USD-machine API as ``bale_run.py``. Each tractor is
the same USD asset, but it is spawned with a unique runtime namespace so the
controllers are independent:

    gearbox/machines/<namespace>/state
    gearbox/machines/<namespace>/cmd_vel
"""

from __future__ import annotations

import math
import random
import sys
import threading
import time

import cbor2
import zenoh


TRACTOR_USD_PATH = "bin/gearbox/assets/tractor.usd"
BALE_USD_PATH = "markers/bale.usdz"
RING_RADIUS = 15.0
TICK_DT = 0.10


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    session.put(key, cbor2.dumps(payload))


class RobotProxy:
    def __init__(self, idx: int, namespace: str):
        self.idx = idx
        self.namespace = namespace
        self.pose = (0.0, 0.0)
        self.last_pose = (0.0, 0.0)
        self.heading_rad: float | None = None
        self.seen = False
        self.target_bale: int | None = None
        self.target_since = 0.0
        self.collected: list[int] = []

    @property
    def is_idle(self) -> bool:
        return self.target_bale is None

    def declare(self, session: zenoh.Session) -> None:
        def on_state(sample: zenoh.Sample) -> None:
            try:
                d = cbor2.loads(bytes(sample.payload))
            except Exception:  # noqa: BLE001
                return
            p = d.get("position", [0.0, 0.0, 0.0])
            self.last_pose = self.pose
            # Gearbox/Rapier state is Y-up: field plane is X/Z.
            self.pose = (float(p[0]), float(p[2]))
            heading = d.get("heading_rad", None)
            if heading is not None:
                self.heading_rad = float(heading)
            self.seen = True

        self._sub_state = session.declare_subscriber(
            f"gearbox/machines/{self.namespace}/state",
            on_state,
        )

    def publish_cmd(self, session: zenoh.Session, speed: float, yaw_rate: float) -> None:
        put_cbor(
            session,
            f"gearbox/machines/{self.namespace}/cmd_vel",
            {
                "linear": [float(speed), 0.0, 0.0],
                "angular": [0.0, 0.0, float(yaw_rate)],
            },
        )

    def stop(self, session: zenoh.Session) -> None:
        self.publish_cmd(session, 0.0, 0.0)

    def heading_or_motion(self, target_heading: float) -> float:
        if self.heading_rad is not None:
            return self.heading_rad
        vx = self.pose[0] - self.last_pose[0]
        vz = self.pose[1] - self.last_pose[1]
        if math.hypot(vx, vz) > 0.05:
            return math.atan2(vx, vz)
        return target_heading


class HarvestTracker:
    """Receives Gearbox contact-delete events so scripts never respawn bales."""

    def __init__(self, session: zenoh.Session):
        self._lock = threading.Lock()
        self._harvested: set[int] = set()
        self._sub = session.declare_subscriber("gearbox/usd/harvested/**", self._on_event)

    def _on_event(self, sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        bale_id = d.get("bale_id")
        if not isinstance(bale_id, int):
            text = str(d.get("id", ""))
            if text.startswith("bale_"):
                try:
                    bale_id = int(text.removeprefix("bale_"))
                except ValueError:
                    return
            else:
                return
        with self._lock:
            self._harvested.add(int(bale_id))

    def drain(self) -> set[int]:
        with self._lock:
            out = set(self._harvested)
            self._harvested.clear()
            return out

    def close(self) -> None:
        del self._sub


def spawn_usd_tractors(session: zenoh.Session, robots: list[RobotProxy]) -> None:
    landed: dict[str, dict] = {}

    def on_spawned(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        namespace = d.get("namespace")
        if namespace:
            landed[str(namespace)] = d

    sub = session.declare_subscriber("gearbox/usd/loaded", on_spawned)
    time.sleep(0.3)

    n = len(robots)
    for robot in robots:
        angle = (2.0 * math.pi * robot.idx) / max(1, n)
        x = RING_RADIUS * math.cos(angle)
        z = RING_RADIUS * math.sin(angle)
        desired_heading = math.atan2(-x, -z)
        # Runtime yaw follows the same field heading convention as state:
        # 0° drives +Z, 90° drives +X. Aim each tractor at the field center.
        yaw_deg = math.degrees(desired_heading)
        put_cbor(
            session,
            f"gearbox/usd/load/{robot.namespace}",
            {
                "category": "machine",
                "usd_path": TRACTOR_USD_PATH,
                "x": float(x),
                "y": 0.0,
                "z": float(z),
                "yaw_deg": float(yaw_deg),
                "label": f"tractor_{robot.idx}.usd",
                "namespace": robot.namespace,
            },
        )
        time.sleep(0.1)

    t0 = time.time()
    while time.time() - t0 < 12.0:
        if all(robot.seen for robot in robots):
            break
        time.sleep(0.05)
    del sub

    missing = [r.namespace for r in robots if not r.seen]
    if missing:
        raise RuntimeError(
            "no state for spawned USD tractor namespaces: "
            + ", ".join(missing)
            + ". Restart make run so it includes the USD spawn API."
        )
    print(
        "spawned USD tractors: "
        + ", ".join(f"R{r.idx}={r.namespace}" for r in robots)
    )


def recolor_bale(session: zenoh.Session, bale_id: int, x: float, z: float, variant: str) -> None:
    put_cbor(
        session,
        f"gearbox/usd/load/bale_{bale_id}",
        {
            "category": "static_usd",
            "x": float(x),
            "z": float(z),
            "usd_path": BALE_USD_PATH,
            "remove": False,
        },
    )


def remove_bale(session: zenoh.Session, bale_id: int) -> None:
    put_cbor(session, f"gearbox/usd/load/bale_{bale_id}", {"x": 0.0, "z": 0.0, "remove": True})


def show_target_indicator(
    session: zenoh.Session,
    robot_idx: int,
    target: tuple[float, float] | None,
) -> None:
    load_id = f"target_indicator_{robot_idx}"
    if target is None:
        put_cbor(session, f"gearbox/usd/load/{load_id}", {"x": 0.0, "z": 0.0, "remove": True})
        return
    bx, bz = target
    put_cbor(
        session,
        f"gearbox/usd/load/{load_id}",
        {
            "category": "static_usd",
            "x": float(bx),
            "y": 0.0,
            "z": float(bz),
            "kind": "box",
            "height": 0.45,
            "radius": 0.32,
            "color": [1.0, 0.0, 0.0],
            "remove": False,
        },
    )


def clear_old_bales(session: zenoh.Session, count: int = 500) -> None:
    for i in range(32):
        show_target_indicator(session, i, None)
    for i in range(count):
        remove_bale(session, i)


def pick_nearest_bale(
    robot: RobotProxy,
    bales: list[tuple[float, float]],
    visited: set[int],
    claimed: set[int],
) -> int | None:
    cx, cz = robot.pose
    best_idx = -1
    best_d = math.inf
    for i, (bx, bz) in enumerate(bales):
        if i in visited or i in claimed:
            continue
        d = math.hypot(bx - cx, bz - cz)
        if d < best_d:
            best_idx = i
            best_d = d
    return best_idx if best_idx >= 0 else None


def drive_toward(session: zenoh.Session, robot: RobotProxy, target: tuple[float, float]) -> float:
    tx, tz = target
    cx, cz = robot.pose
    dx = tx - cx
    dz = tz - cz
    d_now = math.hypot(dx, dz)
    target_heading = math.atan2(dx, dz)
    heading = robot.heading_or_motion(target_heading)
    err = wrap_pi(target_heading - heading)

    turn_slowdown = max(0.20, math.cos(abs(err)))
    speed = min(3.5, 0.45 + 0.35 * d_now) * turn_slowdown
    if abs(err) > 1.7:
        # Ackermann steering changes heading by rolling an arc; crawling at
        # 0.35 m/s looked like "stopping" and never lined up in practice.
        speed = 1.2
    yaw_rate = clamp(1.8 * err, -1.2, 1.2)
    robot.publish_cmd(session, speed, yaw_rate)
    return d_now


def main() -> None:
    n_tractors = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    n_bales = int(sys.argv[2]) if len(sys.argv) > 2 else 50
    field = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 42

    rng = random.Random(seed)
    half = field / 2.0
    bales = [(rng.uniform(-half, half), rng.uniform(-half, half)) for _ in range(n_bales)]

    run_id = int(time.time())
    robots = [RobotProxy(i, f"robot_{run_id}_{i}") for i in range(n_tractors)]

    session = zenoh.open(zenoh.Config())
    try:
        for robot in robots:
            robot.declare(session)

        print(f"spawning {n_tractors} USD tractors")
        spawn_usd_tractors(session, robots)

        print(f"scattering {n_bales} bales across {field:.0f} × {field:.0f} m field")
        clear_old_bales(session, max(500, n_bales + 50))
        harvests = HarvestTracker(session)
        for i, (bx, bz) in enumerate(bales):
            recolor_bale(session, i, bx, bz, "default")
        # Let startup cleanup/removal messages drain before assigning target
        # indicators. The loader replaces entities per message, so target
        # markers must be published only on target changes, not every tick.
        time.sleep(0.8)

        visited: set[int] = set()
        print(f"\n── DRIVING  R={n_tractors}  B={n_bales}  field={field:.0f} m ──\n")

        def mark_harvested(bale_id: int) -> None:
            if not (0 <= bale_id < n_bales) or bale_id in visited:
                return
            visited.add(bale_id)
            remove_bale(session, bale_id)
            for robot in robots:
                if robot.target_bale == bale_id:
                    robot.stop(session)
                    robot.collected.append(bale_id)
                    show_target_indicator(session, robot.idx, None)
                    robot.target_bale = None
                    print(
                        f"  R{robot.idx} touched bale_{bale_id}"
                        f"  collected={len(robot.collected)}"
                        f"  total={len(visited)}/{n_bales}"
                    )
                    return
            print(f"  contact harvested bale_{bale_id}  total={len(visited)}/{n_bales}")

        while len(visited) < n_bales:
            now = time.time()
            for bid in harvests.drain():
                mark_harvested(bid)
            for bid in visited:
                remove_bale(session, bid)
            claimed = {r.target_bale for r in robots if r.target_bale is not None}

            for robot in robots:
                if robot.target_bale in visited:
                    robot.stop(session)
                    show_target_indicator(session, robot.idx, None)
                    robot.target_bale = None

                if robot.target_bale is not None:
                    bid = robot.target_bale
                    # Keep the target locked. Do not switch on distance or
                    # timeout: only the Gearbox contact-harvest event may clear
                    # this target and move the red marker to a new bale.
                    drive_toward(session, robot, bales[bid])

                if robot.is_idle:
                    pick = pick_nearest_bale(robot, bales, visited, claimed)
                    if pick is None:
                        continue
                    bx, bz = bales[pick]
                    robot.target_bale = pick
                    robot.target_since = now
                    claimed.add(pick)
                    cx, cz = robot.pose
                    d = math.hypot(bx - cx, bz - cz)
                    show_target_indicator(session, robot.idx, (bx, bz))
                    print(
                        f"  R{robot.idx} → bale_{pick}"
                        f"  target=({bx:+7.2f},{bz:+7.2f})"
                        f"  d={d:6.2f} m"
                    )

            time.sleep(TICK_DT)

    except KeyboardInterrupt:
        print("\ninterrupted — stopping all tractors")
        for robot in robots:
            robot.stop(session)
            show_target_indicator(session, robot.idx, None)
    finally:
        for robot in robots:
            robot.stop(session)
            show_target_indicator(session, robot.idx, None)
        if "harvests" in locals():
            harvests.close()
        print(f"\nfinal: collected {len({b for r in robots for b in r.collected})}/{n_bales} bales")
        for robot in robots:
            print(f"  R{robot.idx} ({robot.namespace}): {len(robot.collected)} bales")
        session.close()


if __name__ == "__main__":
    main()
