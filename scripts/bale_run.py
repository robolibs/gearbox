#!/usr/bin/env python3
"""Scatter bales and drive the current USD tractor/machine to them.

Default mode asks Gearbox to spawn ``bin/gearbox/assets/tractor.usd`` and then
targets the USD machine-controller API discovered from that asset:

    python scripts/bale_run.py [machine_namespace] [n_bales] [field_size] [seed]

Defaults: namespace=robot, n_bales=50, field_size=300, seed=42.

The script publishes USD bale assets via ``gearbox/usd/load/<id>`` and drives with:

    gearbox/machines/<namespace>/cmd_vel
    gearbox/machines/<namespace>/state

A bale's real resting place is decided by the terrain + physics inside Gearbox,
not by this script. Gearbox publishes each bale's settled pose on
``gearbox/usd/pose/**``; the script drives off those authoritative positions so
the red target marker sits exactly on its bale.

For the old procedural simulator API, use ``spawn:<preset>`` as arg #1,
e.g. ``python scripts/bale_run.py spawn:tractor``. Existing legacy prefixes
like ``tractor_0`` are also still supported.
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


KNOWN_PRESETS = {"tractor", "husky", "robotti", "drone", "oxbo"}
TRACTOR_USD_PATH = "bin/gearbox/assets/tractor.usd"
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


def spawn_vehicle(session: zenoh.Session, preset: str, spawned_state: dict) -> str | None:
    spawned_state.clear()
    put_cbor(
        session,
        "gearbox/sim/spawn",
        {"preset": preset, "x": 0.0, "y": 0.0, "z": 0.0, "yaw_deg": 0.0, "player": True},
    )
    t0 = time.time()
    while not spawned_state and time.time() - t0 < 5.0:
        time.sleep(0.05)
    if not spawned_state:
        return None
    return f"{spawned_state['name']}_{spawned_state['id']}"


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

            t_goal = time.time()
            while True:
                for bale_id in harvests.drain():
                    mark_harvested(bale_id, "contact")
                if best_idx in visited:
                    break
                dx = tx - pose.x
                dz = tz - pose.z
                d_now = math.hypot(dx, dz)

                target_heading = math.atan2(dx, dz)
                heading = pose.heading_rad
                if heading is None and last_pose is not None:
                    vx = pose.x - last_pose.x
                    vz = pose.z - last_pose.z
                    if math.hypot(vx, vz) > 0.05:
                        heading = math.atan2(vx, vz)
                if heading is None:
                    heading = target_heading

                err = wrap_pi(target_heading - heading)
                # Fast when lined up, crawl while turning hard. The tractor's
                # Ackermann controller can steer while almost stopped, so this
                # avoids ploughing sideways into the target.
                turn_slowdown = max(0.20, math.cos(abs(err)))
                speed = min(3.5, 0.45 + 0.35 * d_now) * turn_slowdown
                if abs(err) > 1.7:
                    # Ackermann steering changes heading by rolling an arc;
                    # too little speed here makes the tractor look parked.
                    speed = 1.2
                yaw_rate = clamp(1.8 * err, -1.2, 1.2)
                publish_cmd(speed, yaw_rate)

                if int(time.time() - t_goal) % 5 == 0:
                    print(
                        f"    ... pos=({pose.x:+7.2f},{pose.z:+7.2f}) "
                        f"d={d_now:6.2f} heading_err={math.degrees(err):+6.1f}°",
                        end="\r",
                    )
                time.sleep(0.10)

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


def run_legacy_mode(
    session: zenoh.Session,
    arg1: str,
    bales: list[tuple[float, float]],
    field: float,
) -> None:
    spawned_state: dict = {}

    def on_spawned(sample: zenoh.Sample) -> None:
        try:
            spawned_state.update(cbor2.loads(bytes(sample.payload)))
        except Exception:  # noqa: BLE001
            pass

    spawn_sub = session.declare_subscriber("gearbox/sim/spawned", on_spawned)
    put_cbor(session, "gearbox/sim/reset", {"pause_clock": False})
    put_cbor(session, "gearbox/sim/clock/command", {"SetPaused": False})
    time.sleep(0.5)

    preset = arg1.removeprefix("spawn:")
    if preset in KNOWN_PRESETS:
        vehicle = spawn_vehicle(session, preset, spawned_state)
        if vehicle is None:
            raise RuntimeError(f"spawn confirmation for preset `{preset}` timed out")
        print(f"spawned `{preset}` — driving via topic prefix `{vehicle}`")
        time.sleep(0.3)
    else:
        vehicle = arg1
        print(f"using existing legacy vehicle topic prefix `{vehicle}`")
    del spawn_sub

    print(f"scattering {len(bales)} bales across {field:.0f} × {field:.0f} m field")
    clear_old_bales(session, max(500, len(bales) + 50))
    run_nonce = f"bale_run_legacy_{vehicle}_{int(time.time())}"
    bale_runtime_ids = {i: f"bale_{i}_{run_nonce}" for i in range(len(bales))}
    visited: set[int] = set()
    harvests = HarvestTracker(session)
    poses = BalePoseTracker(session)
    scatter_bales(session, bales, bale_runtime_ids, run_nonce)
    bale_pos = wait_for_bale_poses(poses, len(bales))

    pose_state: dict = {"x": 0.0, "z": 0.0}

    def on_odom(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        p = d.get("position", [0, 0, 0])
        pose_state["x"] = p[0]
        pose_state["z"] = p[2]

    session.declare_subscriber(f"{vehicle}/odom", on_odom)
    time.sleep(0.3)

    visit_order: list[int] = []

    def mark_harvested(bale_id: int, reason: str) -> None:
        if bale_id in bale_pos and bale_id not in visited:
            visited.add(bale_id)
            visit_order.append(bale_id)
            remove_bale(session, bale_runtime_ids[bale_id], run_nonce)
            print(f"    harvested bale_{bale_id} ({reason})")

    try:
        for step in range(len(bale_pos)):
            for bale_id in harvests.drain():
                mark_harvested(bale_id, "contact")
            best_idx, best_d = nearest_unvisited(
                pose_state["x"], pose_state["z"], bale_pos, visited
            )
            if best_idx < 0:
                break
            tx, _ty, tz, _top = bale_pos[best_idx]
            set_target_marker(session, bale_pos[best_idx])
            print(
                f"\n[{step + 1:>3}/{len(bale_pos)}]  visiting bale_{best_idx}  "
                f"target=({tx:+7.2f},{tz:+7.2f})  "
                f"from=({pose_state['x']:+7.2f},{pose_state['z']:+7.2f})  d={best_d:6.2f} m"
            )
            cmd = {
                "x": float(tx),
                "z": float(tz),
                "yaw_deg": 0.0,
                "tolerance": 2.0,
                "yaw_tolerance_deg": 0.0,
                "max_speed": 0.0,
                "cancel": False,
            }
            put_cbor(session, f"{vehicle}/goto", cmd)
            while True:
                for bale_id in harvests.drain():
                    mark_harvested(bale_id, "contact")
                if best_idx in visited:
                    break
                time.sleep(0.2)
            print("    harvested by contact")
    except KeyboardInterrupt:
        print("\ninterrupted — cancelling current goto.")
        put_cbor(
            session,
            f"{vehicle}/goto",
            {
                "x": 0.0,
                "z": 0.0,
                "yaw_deg": 0.0,
                "tolerance": 0.0,
                "yaw_tolerance_deg": 0.0,
                "max_speed": 0.0,
                "cancel": True,
            },
        )
    finally:
        set_target_marker(session, None)
        print(f"\nvisited {len(visited)}/{len(bale_pos)} bales — order: {visit_order}")
        harvests.close()
        poses.close()


def main() -> None:
    arg1 = sys.argv[1] if len(sys.argv) > 1 else "robot"
    n_bales = int(sys.argv[2]) if len(sys.argv) > 2 else 50
    field = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 42

    rng = random.Random(seed)
    half = field / 2.0
    bales = [(rng.uniform(-half, half), rng.uniform(-half, half)) for _ in range(n_bales)]

    session = zenoh.open(zenoh.Config())
    try:
        if arg1.startswith("spawn:") or "_" in arg1:
            run_legacy_mode(session, arg1, bales, field)
        else:
            run_machine_mode(session, arg1, bales, field)
    finally:
        session.close()


if __name__ == "__main__":
    main()
