#!/usr/bin/env python3
"""Scatter bales and drive the current USD tractor/machine to them.

Asks Gearbox to spawn ``bin/gearbox/assets/tractor.usd`` and then targets the
USD machine-controller API discovered from that asset:

    python scripts/bale_run.py [machine_namespace] [n_bales] [field_size] [seed]

Defaults: namespace=robot, n_bales=50, field_size=300, seed=42.

The script publishes USD bale assets via ``gearbox/usd/load/<id>`` and drives with:

    gearbox/machines/<namespace>/cmd_vel
    gearbox/machines/<namespace>/state

A bale's real resting place is decided by the terrain + physics inside Gearbox,
not by this script. Gearbox publishes each bale's settled pose on
``gearbox/usd/pose/**``; the script drives off those authoritative positions so
the red target marker sits exactly on its bale.
"""

from __future__ import annotations

import math
import random
import sys
import threading
import time
from dataclasses import dataclass

import cbor2
import zenoh


TRACTOR_USD_PATH = "bin/gearbox/assets/tractor.usd"
TERRAIN_USD_PATH = "world/terrain.usd"
BALE_USD_PATH = "markers/bale.usdz"
TARGET_MARK_ID = "bale_target_marker"
# Red cube floats this far above a bale's reported top.
MARKER_GAP_M = 0.6


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    # BLOCK congestion control: bale scatter + target updates publish in
    # bursts, and zenoh's default would silently DROP messages under load —
    # which leaves the red target marker out of sync with the tractor. BLOCK
    # makes the publisher wait for queue space, so every load/remove arrives.
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


@dataclass
class MachinePose:
    x: float = 0.0
    z: float = 0.0
    heading_rad: float | None = None
    stamp: float = 0.0
    seen: bool = False


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
    its own flat-ground scatter guesses, so the red marker lands exactly on
    its bale with no terrain/height correction.
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


def scatter_bales(
    session: zenoh.Session,
    bales: list[tuple[float, float]],
    bale_runtime_ids: dict[int, str],
    nonce: str,
) -> None:
    """Publish the USD bales at the scattered X/Z.

    Gearbox does not know these are "bales". It receives a USD asset path and
    drops the asset onto the terrain, then reports the settled pose back.
    """
    for i, (bx, bz) in enumerate(bales):
        put_cbor(
            session,
            f"gearbox/usd/load/{bale_runtime_ids[i]}",
            {
                "category": "static_usd",
                "x": float(bx),
                "z": float(bz),
                "usd_path": BALE_USD_PATH,
                "nonce": nonce,
                "remove": False,
            },
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


def remove_bale(session: zenoh.Session, runtime_id: str, nonce: str | None = None) -> None:
    payload = {"remove": True, "delete": True}
    if nonce is not None:
        payload["nonce"] = nonce
    put_cbor(session, f"gearbox/usd/delete/{runtime_id}", payload)


def set_target_marker(
    session: zenoh.Session,
    pose: tuple[float, float, float, float] | None,
) -> None:
    """Place the red target marker through the marker API, or remove it."""
    if pose is None:
        put_cbor(session, f"gearbox/usd/mark/{TARGET_MARK_ID}/delete", {})
        return
    bx, _by, bz, top_y = pose
    put_cbor(
        session,
        f"gearbox/usd/mark/{TARGET_MARK_ID}/delete",
        {},
    )
    put_cbor(
        session,
        f"gearbox/usd/mark/{TARGET_MARK_ID}/{bx}/{top_y + MARKER_GAP_M}/{bz}",
        {},
    )


def clear_old_bales(session: zenoh.Session, count: int = 500) -> None:
    set_target_marker(session, None)
    for i in range(count):
        remove_bale(session, f"bale_{i}")


def wait_for_pose(pose: MachinePose, timeout_s: float) -> bool:
    t0 = time.time()
    while not pose.seen and time.time() - t0 < timeout_s:
        time.sleep(0.05)
    return pose.seen


def wait_for_bale_poses(poses: BalePoseTracker, expected: int, timeout_s: float = 20.0) -> dict:
    """Block until Gearbox has reported every settled bale pose (or timeout)."""
    print("waiting for Gearbox to report settled bale poses ...")
    t0 = time.time()
    while poses.count() < expected and time.time() - t0 < timeout_s:
        time.sleep(0.1)
    bale_pos = poses.snapshot()
    if len(bale_pos) < expected:
        print(f"  only {len(bale_pos)}/{expected} bale poses arrived; running with those")
    else:
        print(f"  got all {len(bale_pos)} bale poses")
    return bale_pos


def spawn_usd_machine(
    session: zenoh.Session,
    namespace: str,
    usd_path: str = TRACTOR_USD_PATH,
) -> None:
    """Ask the running Gearbox app to load/spawn a USD asset.

    This is intentionally generic: the request says only "spawn this USD".
    Gearbox discovers by reading the USD whether it contains a machine and
    which namespace/controller topics it should expose.
    """
    spawned: dict = {}

    def on_spawned(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        if str(d.get("usd_path", "")).endswith(usd_path) or d.get("label") == "tractor.usd":
            spawned.update(d)

    sub = session.declare_subscriber("gearbox/usd/loaded", on_spawned)
    # Same zenoh race as the older spawn API: let the subscriber propagate
    # before publishing the request.
    time.sleep(0.2)
    put_cbor(
        session,
        f"gearbox/usd/load/{namespace}",
        {
            "category": "machine",
            "usd_path": usd_path,
            "namespace": namespace,
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "yaw_deg": 0.0,
            "label": "tractor.usd",
        },
    )
    t0 = time.time()
    while not spawned and time.time() - t0 < 3.0:
        time.sleep(0.05)
    del sub
    if spawned:
        print(f"spawn requested: {spawned.get('label', usd_path)}")
    else:
        print("spawn request sent; no gearbox/usd/loaded ack yet")


def nearest_unvisited(
    pose_x: float,
    pose_z: float,
    bale_pos: dict[int, tuple[float, float, float, float]],
    visited: set[int],
) -> tuple[int, float]:
    best_idx = -1
    best_d = math.inf
    for bid, (bx, _by, bz, _top) in bale_pos.items():
        if bid in visited:
            continue
        d = math.hypot(bx - pose_x, bz - pose_z)
        if d < best_d:
            best_idx = bid
            best_d = d
    return best_idx, best_d


def run_machine_mode(
    session: zenoh.Session,
    namespace: str,
    bales: list[tuple[float, float]],
    field: float,
) -> None:
    try:
        import ondrive  # noqa: PLC0415  (lazy: only needed when actually driving)
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "ondrive Python bindings not found. Install with:\n"
            "  cd ../ondrive && maturin develop --release\n"
            "(adjust the path to wherever the ondrive checkout lives)."
        ) from exc

    pose = MachinePose()
    last_pose: MachinePose | None = None

    def on_state(sample: zenoh.Sample) -> None:
        nonlocal last_pose
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        p = d.get("position", [0.0, 0.0, 0.0])
        last_pose = MachinePose(pose.x, pose.z, pose.heading_rad, pose.stamp, pose.seen)
        # Gearbox/Rapier state is Bevy-style Y-up: drive plane=(X,Z),
        # height=Y. Marker API uses the same (x,z) field plane.
        pose.x = float(p[0])
        pose.z = float(p[2])
        heading = d.get("heading_rad", None)
        pose.heading_rad = float(heading) if heading is not None else pose.heading_rad
        pose.stamp = time.time()
        pose.seen = True

    state_sub = session.declare_subscriber(f"gearbox/machines/{namespace}/state", on_state)
    print("loading USD terrain")
    load_terrain(session)
    # Give Gearbox a short window to instantiate the terrain scene and swap
    # out the default flat ground before bales are scattered.
    time.sleep(1.0)

    print(f"waiting for gearbox/machines/{namespace}/state ...")
    wait_for_pose(pose, 1.0)
    if not pose.seen:
        print(f"no `{namespace}` state yet — spawning {TRACTOR_USD_PATH}")
        spawn_usd_machine(session, namespace)
        wait_for_pose(pose, 12.0)
        if not pose.seen:
            del state_sub
            raise RuntimeError(
                f"no state from machine namespace `{namespace}` after spawning "
                f"{TRACTOR_USD_PATH}. Is the Gearbox app running and built with "
                "the USD spawn API?"
            )

    print(f"using USD machine namespace `{namespace}`")
    print(f"scattering {len(bales)} bales across {field:.0f} × {field:.0f} m field")
    clear_old_bales(session, max(500, len(bales) + 50))
    run_nonce = f"bale_run_{namespace}_{int(time.time())}"
    bale_runtime_ids = {i: f"bale_{i}_{run_nonce}" for i in range(len(bales))}
    visited: set[int] = set()
    harvests = HarvestTracker(session)
    poses = BalePoseTracker(session)
    scatter_bales(session, bales, bale_runtime_ids, run_nonce)
    bale_pos = wait_for_bale_poses(poses, len(bales))

    visit_order: list[int] = []
    cmd_topic = f"gearbox/machines/{namespace}/cmd_vel"

    def publish_cmd(speed: float, yaw_rate: float) -> None:
        put_cbor(
            session,
            cmd_topic,
            {"linear": [float(speed), 0.0, 0.0], "angular": [0.0, 0.0, float(yaw_rate)]},
        )

    def mark_harvested(bale_id: int, reason: str) -> None:
        if bale_id in bale_pos and bale_id not in visited:
            visited.add(bale_id)
            visit_order.append(bale_id)
            remove_bale(session, bale_runtime_ids[bale_id], run_nonce)
            print(f"    harvested bale_{bale_id} ({reason})")

    # ─── ondrive tracker setup ───────────────────────────────────────
    # Pure pursuit (not Stanley) for forward driving. Stanley adds a
    # `k_cte = 1.0` cross-track-error term to the steer command that
    # competes with the heading term — on a straight-line path the two
    # fight each other and a heavy Ackermann tractor wobbles visibly.
    # Pure pursuit aims at a single lookahead point on the path with
    # no competing term; the back-and-turn maneuver when the bale is
    # behind is handled by the state machine below.
    tracker = ondrive.Tracker("pure_pursuit")
    cfg = ondrive.ControllerConfig.default_()
    cfg.goal_tolerance = 2.0
    cfg.angular_tolerance = math.pi  # accept any final yaw
    cfg.lookahead_distance = 5.0
    cfg.output_units = "physical"
    tracker.set_config(cfg)

    cons = ondrive.RobotConstraints.default_()
    cons.steering_type = "ackermann"
    cons.wheelbase = 2.37  # matches the USD tractor's wheel_base
    cons.max_linear_velocity = 3.0
    cons.min_linear_velocity = 0.0  # tracker never commands reverse here
    cons.max_angular_velocity = 1.0  # gentler than Stanley's 1.4
    cons.max_steering_angle = math.radians(40.0)
    tracker.init(cons)

    # ─── Reverse-maneuver state machine ──────────────────────────────
    # When the bale is behind the tractor (`|heading_err| > 90°`), this
    # state machine bypasses Stanley for one or two seconds: it commands
    # a steady reverse with the wheels cranked the OPPOSITE way. Why
    # opposite: `controller.rs::steering_target_radians` derives the
    # wheel steer angle from `angular / |linear|` and does NOT flip
    # the sign when reversing (by design — its comment explains the
    # rationale). So to swing the nose toward a `+heading_err` goal
    # while reversing, we publish NEGATIVE `angular`; the wheels go
    # right and the reverse-motion rotates the body left as we want.
    # Hysteresis (enter 90°, exit 70°) prevents thrash; the 6 s cap is
    # a safety net against pathological geometry.
    REVERSE_ENTER_RAD = math.pi / 2.0
    REVERSE_EXIT_RAD = math.radians(70.0)
    REVERSE_SPEED_MPS = 1.2
    REVERSE_YAW_RPS = 1.2
    MAX_REVERSE_SECS = 6.0
    maneuver = "forward"
    turn_sign = 0.0
    maneuver_t0 = 0.0

    # Gearbox is Y-up: drive plane = (X, Z), height = Y. Ondrive is
    # Z-up planar (x, y). Map planar_x = gearbox_z, planar_y = gearbox_x
    # so a gearbox heading of 0 (facing +Z) becomes ondrive yaw 0
    # (facing +planar_x). Rotation sign is preserved.
    def to_planar(gx: float, gz: float) -> tuple[float, float]:
        return gz, gx

    try:
        for step in range(len(bale_pos)):
            for bale_id in harvests.drain():
                mark_harvested(bale_id, "contact")
            best_idx, best_d = nearest_unvisited(pose.x, pose.z, bale_pos, visited)
            if best_idx < 0:
                break
            tx, _ty, tz, _top = bale_pos[best_idx]
            # Publish the marker once for this bale; only the contact-harvest
            # event below ends the visit and moves the marker to the next bale.
            set_target_marker(session, bale_pos[best_idx])
            print(
                f"\n[{step + 1:>3}/{len(bale_pos)}]  visiting bale_{best_idx}  "
                f"target=({tx:+7.2f},{tz:+7.2f})  "
                f"from=({pose.x:+7.2f},{pose.z:+7.2f})  d={best_d:6.2f} m"
            )

            # Fresh straight-line path from current pose to the bale,
            # densified so Stanley's path-index advancement is smooth
            # over a long approach. Reset the maneuver state — the old
            # `turn_sign` was latched for the previous bale.
            start_px, start_py = to_planar(pose.x, pose.z)
            goal_px, goal_py = to_planar(tx, tz)
            path_yaw = math.atan2(goal_py - start_py, goal_px - start_px)
            path = ondrive.Path()
            path.add_waypoint_xy(start_px, start_py, yaw=path_yaw)
            path.add_waypoint_xy(goal_px, goal_py, yaw=path_yaw)
            path.smoothen(2.0)  # ≤ 2 m segments
            tracker.set_path(path)
            tracker.set_goal(
                ondrive.Goal(
                    target_pose=((goal_px, goal_py, 0.0), path_yaw),
                    tolerance_position=2.0,
                    tolerance_orientation=math.pi,
                )
            )
            maneuver = "forward"
            turn_sign = 0.0

            t_goal = time.time()
            t_last = time.time()
            while True:
                for bale_id in harvests.drain():
                    mark_harvested(bale_id, "contact")
                if best_idx in visited:
                    break

                heading = pose.heading_rad
                if heading is None and last_pose is not None:
                    vx = pose.x - last_pose.x
                    vz = pose.z - last_pose.z
                    if math.hypot(vx, vz) > 0.05:
                        heading = math.atan2(vx, vz)
                if heading is None:
                    heading = path_yaw

                # Bearing-to-goal vs current heading, in the gearbox
                # plane (matches `pose.heading_rad` directly — no
                # ondrive-frame round-trip needed for this).
                dx_g = tx - pose.x
                dz_g = tz - pose.z
                d_now = math.hypot(dx_g, dz_g)
                heading_err = wrap_pi(math.atan2(dx_g, dz_g) - heading)

                # State machine transitions (hysteresis 90° / 70°).
                now = time.time()
                if maneuver == "forward":
                    if abs(heading_err) > REVERSE_ENTER_RAD and d_now > 2.0:
                        maneuver = "reverse"
                        turn_sign = 1.0 if heading_err > 0.0 else -1.0
                        maneuver_t0 = now
                else:
                    if abs(heading_err) < REVERSE_EXIT_RAD or now - maneuver_t0 > MAX_REVERSE_SECS:
                        maneuver = "forward"

                if maneuver == "reverse":
                    publish_cmd(-REVERSE_SPEED_MPS, -turn_sign * REVERSE_YAW_RPS)
                    mode_label = "reverse"
                else:
                    px, py = to_planar(pose.x, pose.z)
                    state = ondrive.RobotState(
                        pose=((px, py, 0.0), heading),
                        allow_move=True,
                        allow_reverse=False,
                    )
                    dt = max(1e-3, now - t_last)
                    t_last = now
                    cmd = tracker.tick(state, dt)
                    if not cmd.valid:
                        publish_cmd(0.0, 0.0)
                    else:
                        publish_cmd(float(cmd.linear_velocity), float(cmd.angular_velocity))
                    mode_label = tracker.get_status().mode

                if int(now - t_goal) % 5 == 0:
                    print(
                        f"    ... pos=({pose.x:+7.2f},{pose.z:+7.2f}) "
                        f"d={d_now:6.2f} mode={mode_label}",
                        end="\r",
                    )
                time.sleep(0.05)

            publish_cmd(0.0, 0.0)
            print("    harvested by contact")
    except KeyboardInterrupt:
        print("\ninterrupted — stopping machine.")
        publish_cmd(0.0, 0.0)
    finally:
        set_target_marker(session, None)
        print(f"\nvisited {len(visited)}/{len(bale_pos)} bales — order: {visit_order}")
        harvests.close()
        poses.close()
        del state_sub


def main() -> None:
    namespace = sys.argv[1] if len(sys.argv) > 1 else "robot"
    n_bales = int(sys.argv[2]) if len(sys.argv) > 2 else 50
    field = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 42

    rng = random.Random(seed)
    half = field / 2.0
    bales = [(rng.uniform(-half, half), rng.uniform(-half, half)) for _ in range(n_bales)]

    session = zenoh.open(zenoh.Config())
    try:
        run_machine_mode(session, namespace, bales, field)
    finally:
        session.close()


if __name__ == "__main__":
    main()
