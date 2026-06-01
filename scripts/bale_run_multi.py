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

# The per-robot control loop uses ondrive's Stanley tracker for the
# back-and-turn maneuver when the next bale is behind the tractor. Imported
# softly so the rest of the script (USD loading, marker logic, harvest
# tracking) stays importable on a host where ondrive isn't on `sys.path`.
try:
    import ondrive  # type: ignore
except ModuleNotFoundError:
    ondrive = None  # type: ignore[assignment]


TRACTOR_USD_PATH = "bin/gearbox/assets/tractor.usd"
TERRAIN_USD_PATH = "world/terrain.usd"
BALE_USD_PATH = "markers/bale.usdz"
RING_RADIUS = 15.0
TICK_DT = 0.10
# Red cube floats this far above a bale's reported top so it reads as a
# "target above the bale", not a decal on it.
MARKER_GAP_M = 0.6


def _planar(gx: float, gz: float) -> tuple[float, float]:
    """Gearbox (X, Z) → ondrive planar (x, y).

    Gearbox is Y-up: drive plane = (X, Z). Ondrive is Z-up planar (x, y) with
    yaw 0 = +x. Mapping planar_x = gearbox_z, planar_y = gearbox_x means a
    gearbox heading of 0 (facing +Z) is ondrive yaw 0 (facing +planar_x); the
    rotation sign is preserved.
    """
    return gz, gx


def _build_tracker():
    """Per-robot ondrive Pure-Pursuit tracker for forward driving only.

    We use pure pursuit instead of Stanley because Stanley's steering
    sums a heading-error term and a cross-track-error term (`k_cte = 1.0`
    hardcoded inside the controller). The two compete on a straight-line
    path: a tiny drift off the line yields a large CTE kick, the heading
    overshoots, CTE flips, and a heavy Ackermann tractor wobbles
    visibly. Pure pursuit aims at a single lookahead point on the path —
    no competing term, much smoother. The actual back-and-turn maneuver
    when the bale is behind lives in `_reverse_maneuver_cmd`; this
    tracker owns the forward-driving half only.
    """
    tracker = ondrive.Tracker("pure_pursuit")
    cfg = ondrive.ControllerConfig.default_()
    cfg.goal_tolerance = 2.0
    cfg.angular_tolerance = math.pi
    # Lookahead big enough to swallow the natural pose-update jitter; a
    # 2 m densified path means this looks ~2 waypoints ahead.
    cfg.lookahead_distance = 5.0
    cfg.output_units = "physical"
    tracker.set_config(cfg)
    cons = ondrive.RobotConstraints.default_()
    cons.steering_type = "ackermann"
    cons.wheelbase = 2.37
    cons.max_linear_velocity = 3.0
    cons.min_linear_velocity = 0.0  # tracker never commands reverse
    cons.max_angular_velocity = 1.0  # gentler than Stanley's 1.4
    cons.max_steering_angle = math.radians(40.0)
    tracker.init(cons)
    return tracker


# ─── Reverse-maneuver state machine ────────────────────────────────────
#
# Stanley drives the tractor when the goal is in front. When the goal is
# behind (`|heading_err| > 90°`), this state machine takes over: it
# commands a steady reverse with the wheels cranked the OPPOSITE way of
# where they'd point for a forward turn — because gearbox's cmd_vel
# handler (`controller.rs::steering_target_radians`) derives the wheel
# steer angle from `angular / |linear|` and does NOT flip the sign when
# reversing (deliberately, per its comment). So to swing the nose toward
# a `+heading_err` goal while reversing, we publish a NEGATIVE
# `angular.z`; the wheels crank right and reverse-motion rotates the
# body left as we want.
#
# Hysteresis (enter 90°, exit 70°) means the forward arc from 70° won't
# climb back past 90° and re-trigger reverse. The hard time cap is just
# a safety against geometry going pathological (stuck wheel, pinned
# chassis) — the maneuver is geometrically convergent.
REVERSE_ENTER_RAD = math.pi / 2.0          # 90°
REVERSE_EXIT_RAD = math.radians(70.0)      # 70°
REVERSE_SPEED_MPS = 1.2                    # back up at this magnitude
REVERSE_YAW_RPS = 1.2                      # full-lock-equivalent yaw cmd
MAX_REVERSE_SECS = 6.0


def _reverse_maneuver_cmd(turn_sign: float) -> tuple[float, float]:
    """`(linear, angular)` cmd_vel for one tick of the reverse maneuver.

    `turn_sign` is `sign(heading_err)` latched at maneuver entry — we
    negate it before publishing for the cmd_vel sign-quirk reason
    described in the module-level comment above.
    """
    return -REVERSE_SPEED_MPS, -turn_sign * REVERSE_YAW_RPS


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


def clear_sim(session: zenoh.Session) -> None:
    put_cbor(session, "gearbox/sim/clear", {"pause_clock": False})


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
        # ondrive Stanley tracker state, lazily allocated in `drive_toward`
        # so a host without the `ondrive` Python module can still load this
        # script for static checks. `_tracker_target` doubles as the cache
        # key: a new (x, z) means rebuild path + goal.
        self._tracker = None
        self._tracker_target: tuple[float, float] | None = None
        self._tracker_path_yaw: float = 0.0
        self._tracker_last_tick: float | None = None
        # Reverse-maneuver state machine on top of Stanley (see
        # `_reverse_maneuver_cmd` / `REVERSE_*` constants). `_maneuver` is
        # "forward" or "reverse"; `_turn_sign` (±1) is latched at the
        # moment we enter reverse so jitter near `err ≈ ±π` can't flip
        # the steer mid-maneuver.
        self._maneuver: str = "forward"
        self._turn_sign: float = 0.0
        self._maneuver_t0: float = 0.0

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


def load_terrain(session: zenoh.Session) -> None:
    put_cbor(
        session,
        "gearbox/usd/load/terrain",
        {
            "category": "terrain",
            "usd_path": TERRAIN_USD_PATH,
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "remove": False,
        },
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
    if ondrive is None:
        raise RuntimeError(
            "ondrive Python bindings not found. Install with:\n"
            "  cd ../ondrive && maturin develop --release\n"
            "(adjust the path to wherever the ondrive checkout lives)."
        )

    tx, tz = target
    cx, cz = robot.pose
    d_now = math.hypot(tx - cx, tz - cz)

    # First time, or target changed → fresh straight-line path + goal,
    # and reset the maneuver state machine (the old turn_sign was
    # latched for the previous bale).
    target_changed = robot._tracker is None or robot._tracker_target != target
    if target_changed:
        robot._tracker = _build_tracker()
        start_px, start_py = _planar(cx, cz)
        goal_px, goal_py = _planar(tx, tz)
        path_yaw = math.atan2(goal_py - start_py, goal_px - start_px)
        path = ondrive.Path()
        path.add_waypoint_xy(start_px, start_py, yaw=path_yaw)
        path.add_waypoint_xy(goal_px, goal_py, yaw=path_yaw)
        path.smoothen(2.0)  # ≤ 2 m segments
        robot._tracker.set_path(path)
        robot._tracker.set_goal(
            ondrive.Goal(
                target_pose=((goal_px, goal_py, 0.0), path_yaw),
                tolerance_position=2.0,
                tolerance_orientation=math.pi,
            )
        )
        robot._tracker_target = target
        robot._tracker_path_yaw = path_yaw
        robot._tracker_last_tick = None
        robot._maneuver = "forward"
        robot._turn_sign = 0.0

    # Bearing-to-goal vs current heading in the gearbox plane. We do
    # this in the gearbox (X, Z) frame so the sign convention matches
    # `pose.heading_rad` directly — no ondrive-frame round-trip.
    heading = robot.heading_or_motion(robot._tracker_path_yaw)
    heading_err = wrap_pi(math.atan2(tx - cx, tz - cz) - heading)

    # State machine transitions (hysteresis: 90° enter / 70° exit).
    now = time.time()
    if robot._maneuver == "forward":
        if abs(heading_err) > REVERSE_ENTER_RAD and d_now > 2.0:
            robot._maneuver = "reverse"
            robot._turn_sign = 1.0 if heading_err > 0.0 else -1.0
            robot._maneuver_t0 = now
    else:  # reverse
        timed_out = now - robot._maneuver_t0 > MAX_REVERSE_SECS
        if abs(heading_err) < REVERSE_EXIT_RAD or timed_out:
            robot._maneuver = "forward"

    # Reverse branch: hand-rolled cmd, bypasses Stanley entirely.
    if robot._maneuver == "reverse":
        lin, ang = _reverse_maneuver_cmd(robot._turn_sign)
        robot.publish_cmd(session, lin, ang)
        return d_now

    # Forward branch: Stanley tick. With `allow_reverse=False` it can
    # only ever command non-negative `linear_velocity`, so the
    # cmd_vel-handler sign-flip we used to do here isn't needed.
    px, py = _planar(cx, cz)
    state = ondrive.RobotState(
        pose=((px, py, 0.0), heading),
        allow_move=True,
        allow_reverse=False,
    )
    dt = max(1e-3, now - robot._tracker_last_tick) if robot._tracker_last_tick else TICK_DT
    robot._tracker_last_tick = now
    cmd = robot._tracker.tick(state, dt)
    if not cmd.valid:
        robot.publish_cmd(session, 0.0, 0.0)
        return d_now
    robot.publish_cmd(session, float(cmd.linear_velocity), float(cmd.angular_velocity))
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

        print("clearing simulator")
        clear_sim(session)
        time.sleep(0.3)
        print("loading USD terrain")
        load_terrain(session)
        # Give Gearbox one frame window to instantiate the terrain scene and
        # swap out the default flat ground before machines/bales are placed.
        time.sleep(1.0)

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
