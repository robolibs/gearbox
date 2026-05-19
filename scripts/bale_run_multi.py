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

Positions are *not* guessed by this script. It scatters bales at chosen X/Z,
but a bale's real resting place is decided by the terrain + physics inside
Gearbox. Gearbox publishes each bale's settled pose on ``gearbox/usd/pose/**``
and the script drives off those authoritative positions — so a red marker sits
exactly on its bale and never drifts.
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
# Red cube floats this far above a bale's reported top so it reads as a
# "target above the bale", not a decal on it.
MARKER_GAP_M = 0.6


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    # BLOCK congestion control: bale scatter + target updates publish in
    # bursts, and zenoh's default would silently DROP messages under load.
    # A dropped marker create leaves a tractor with no red marker; a dropped
    # remove leaves a stale marker on a harvested bale. BLOCK makes the
    # publisher wait for queue space instead, so every load/remove is
    # delivered, in order.
    session.put(
        key,
        cbor2.dumps(payload),
        congestion_control=zenoh.CongestionControl.BLOCK,
    )


def bale_id_from(text: object) -> int | None:
    """Parse the integer bale index out of ``bale_<n>...`` runtime ids."""
    text = str(text or "")
    if not text.startswith("bale_"):
        return None
    digits = []
    for ch in text.removeprefix("bale_"):
        if not ch.isdigit():
            break
        digits.append(ch)
    if not digits:
        return None
    try:
        return int("".join(digits))
    except ValueError:
        return None


class RobotProxy:
    def __init__(self, idx: int, namespace: str):
        self.idx = idx
        self.namespace = namespace
        self.pose = (0.0, 0.0)
        self.last_pose = (0.0, 0.0)
        self.heading_rad: float | None = None
        self.seen = False
        self.target_bale: int | None = None
        # Which bale the published red marker currently sits on. Kept separate
        # from `target_bale` so the marker is (re)published only when the
        # target actually changes — never every tick.
        self.marker_bale: int | None = None
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
            bale_id = bale_id_from(d.get("id"))
            if bale_id is None:
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


class BalePoseTracker:
    """Receives Gearbox's authoritative settled poses for loaded USD bales.

    Gearbox publishes ``gearbox/usd/pose/bale_<id>`` once a bale has come to
    rest on the terrain. The script drives off these real positions instead of
    its own flat-ground scatter guesses, so red markers land exactly on bales
    and never need terrain/height correction.
    """

    def __init__(self, session: zenoh.Session):
        self._lock = threading.Lock()
        # bale_id -> (x, y, z, top_y)
        self._poses: dict[int, tuple[float, float, float, float]] = {}
        self._sub = session.declare_subscriber("gearbox/usd/pose/**", self._on_event)

    def _on_event(self, sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        bale_id = bale_id_from(d.get("id"))
        if bale_id is None:
            return
        with self._lock:
            self._poses[bale_id] = (
                float(d.get("x", 0.0)),
                float(d.get("y", 0.0)),
                float(d.get("z", 0.0)),
                float(d.get("top_y", 0.0)),
            )

    def count(self) -> int:
        with self._lock:
            return len(self._poses)

    def snapshot(self) -> dict[int, tuple[float, float, float, float]]:
        with self._lock:
            return dict(self._poses)

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


def load_bale(session: zenoh.Session, runtime_id: str, x: float, z: float, nonce: str) -> None:
    # The bale is dropped onto the terrain by Gearbox, which then reports the
    # settled pose back on gearbox/usd/pose/**.
    put_cbor(
        session,
        f"gearbox/usd/load/{runtime_id}",
        {
            "category": "static_usd",
            "x": float(x),
            "z": float(z),
            "usd_path": BALE_USD_PATH,
            "nonce": nonce,
            "remove": False,
        },
    )


def remove_bale(session: zenoh.Session, runtime_id: str, nonce: str | None = None) -> None:
    payload = {"remove": True, "delete": True}
    if nonce is not None:
        payload["nonce"] = nonce
    put_cbor(session, f"gearbox/usd/delete/{runtime_id}", payload)


def set_target_marker(
    session: zenoh.Session,
    robot_idx: int,
    pose: tuple[float, float, float, float] | None,
) -> None:
    """Place tractor ``robot_idx``'s red marker through the marker API."""
    mark_id = f"target_marker_{robot_idx}"
    if pose is None:
        put_cbor(session, f"gearbox/usd/mark/{mark_id}/delete", {})
        return
    bx, _by, bz, top_y = pose
    put_cbor(session, f"gearbox/usd/mark/{mark_id}/delete", {})
    put_cbor(session, f"gearbox/usd/mark/{mark_id}/{bx}/{top_y + MARKER_GAP_M}/{bz}", {})


def clear_old_bales(session: zenoh.Session, count: int = 500) -> None:
    for i in range(32):
        set_target_marker(session, i, None)
    for i in range(count):
        remove_bale(session, f"bale_{i}")


def pick_nearest_bale(
    robot: RobotProxy,
    bale_pos: dict[int, tuple[float, float, float, float]],
    visited: set[int],
    claimed: set[int],
) -> int | None:
    cx, cz = robot.pose
    best_idx: int | None = None
    best_d = math.inf
    for bid, (bx, _by, bz, _top) in bale_pos.items():
        if bid in visited or bid in claimed:
            continue
        d = math.hypot(bx - cx, bz - cz)
        if d < best_d:
            best_idx = bid
            best_d = d
    return best_idx


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
    scatter = [(rng.uniform(-half, half), rng.uniform(-half, half)) for _ in range(n_bales)]

    run_id = int(time.time())
    run_nonce = f"bale_run_multi_{run_id}"
    bale_runtime_ids = {
        i: f"bale_{i}_{run_nonce}" for i in range(n_bales)
    }
    robots = [RobotProxy(i, f"robot_{run_id}_{i}") for i in range(n_tractors)]

    session = zenoh.open(zenoh.Config())
    harvests: HarvestTracker | None = None
    poses: BalePoseTracker | None = None
    try:
        for robot in robots:
            robot.declare(session)

        print(f"spawning {n_tractors} USD tractors")
        spawn_usd_tractors(session, robots)

        print(f"scattering {n_bales} bales across {field:.0f} × {field:.0f} m field")
        clear_old_bales(session, max(500, n_bales + 50))
        harvests = HarvestTracker(session)
        poses = BalePoseTracker(session)
        for i, (bx, bz) in enumerate(scatter):
            load_bale(session, bale_runtime_ids[i], bx, bz, run_nonce)

        # Wait for Gearbox to report each bale's settled pose. The world
        # publishes gearbox/usd/pose/bale_<id> once a bale freezes on the
        # terrain — those are the authoritative positions the run drives off.
        print("waiting for Gearbox to report settled bale poses ...")
        t0 = time.time()
        while poses.count() < n_bales and time.time() - t0 < 20.0:
            time.sleep(0.1)
        bale_pos = poses.snapshot()
        n_targets = len(bale_pos)
        if n_targets < n_bales:
            print(f"  only {n_targets}/{n_bales} bale poses arrived; running with those")
        else:
            print(f"  got all {n_targets} bale poses")

        visited: set[int] = set()
        print(f"\n── DRIVING  R={n_tractors}  B={n_targets}  field={field:.0f} m ──\n")

        def mark_harvested(bale_id: int) -> None:
            if bale_id not in bale_pos or bale_id in visited:
                return
            visited.add(bale_id)
            remove_bale(session, bale_runtime_ids[bale_id], run_nonce)
            for robot in robots:
                if robot.target_bale == bale_id:
                    robot.stop(session)
                    robot.collected.append(bale_id)
                    robot.target_bale = None
                    # The marker is not published here. The reconcile pass at
                    # the end of the tick moves it once — after this tractor
                    # has (maybe) been assigned its next bale — so the red
                    # marker slides straight from the harvested bale to the
                    # new one with no despawn flash.
                    print(
                        f"  R{robot.idx} touched bale_{bale_id}"
                        f"  collected={len(robot.collected)}"
                        f"  total={len(visited)}/{n_targets}"
                    )
                    return
            print(f"  contact harvested bale_{bale_id}  total={len(visited)}/{n_targets}")

        while len(visited) < n_targets:
            for bid in harvests.drain():
                mark_harvested(bid)
            claimed = {r.target_bale for r in robots if r.target_bale is not None}

            for robot in robots:
                if robot.target_bale in visited:
                    robot.stop(session)
                    robot.target_bale = None

                if robot.target_bale is not None:
                    bx, _by, bz, _top = bale_pos[robot.target_bale]
                    # Keep the target locked. Do not switch on distance or
                    # timeout: only the Gearbox contact-harvest event may clear
                    # this target and move the red marker to a new bale.
                    drive_toward(session, robot, (bx, bz))

                if robot.is_idle:
                    pick = pick_nearest_bale(robot, bale_pos, visited, claimed)
                    if pick is None:
                        continue
                    bx, _by, bz, _top = bale_pos[pick]
                    robot.target_bale = pick
                    claimed.add(pick)
                    cx, cz = robot.pose
                    d = math.hypot(bx - cx, bz - cz)
                    print(
                        f"  R{robot.idx} → bale_{pick}"
                        f"  target=({bx:+7.2f},{bz:+7.2f})"
                        f"  d={d:6.2f} m"
                    )

            # One red marker per tractor. Publish only when a tractor's target
            # actually changed; the loader then moves the existing marker in
            # place (no despawn/respawn), so it never flashes and never lands
            # on a stale bale.
            for robot in robots:
                if robot.marker_bale != robot.target_bale:
                    if robot.target_bale is None:
                        set_target_marker(session, robot.idx, None)
                    else:
                        set_target_marker(
                            session, robot.idx, bale_pos[robot.target_bale]
                        )
                    robot.marker_bale = robot.target_bale

            time.sleep(TICK_DT)

    except KeyboardInterrupt:
        print("\ninterrupted — stopping all tractors")
        for robot in robots:
            robot.stop(session)
            set_target_marker(session, robot.idx, None)
    finally:
        for robot in robots:
            robot.stop(session)
            set_target_marker(session, robot.idx, None)
        if harvests is not None:
            harvests.close()
        if poses is not None:
            poses.close()
        print(f"\nfinal: collected {len({b for r in robots for b in r.collected})}/{n_bales} bales")
        for robot in robots:
            print(f"  R{robot.idx} ({robot.namespace}): {len(robot.collected)} bales")
        session.close()


if __name__ == "__main__":
    main()
