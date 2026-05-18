#!/usr/bin/env python3
"""Scatter bales and drive the current USD tractor/machine to them.

Default mode asks Gearbox to spawn ``bin/gearbox/assets/tractor.usd`` and then
targets the USD machine-controller API discovered from that asset:

    python scripts/bale_run.py [machine_namespace] [n_bales] [field_size] [seed]

Defaults: namespace=robot, n_bales=50, field_size=300, seed=42.

The script publishes USD bale assets via ``gearbox/usd/load/<id>`` and drives with:

    gearbox/machines/<namespace>/cmd_vel
    gearbox/machines/<namespace>/state

For the old procedural simulator API, use ``spawn:<preset>`` as arg #1,
e.g. ``python scripts/bale_run.py spawn:tractor``. Existing legacy prefixes
like ``tractor_0`` are also still supported.
"""

from __future__ import annotations

import math
import random
import sys
import time
from dataclasses import dataclass

import cbor2
import zenoh


KNOWN_PRESETS = {"tractor", "husky", "robotti", "drone", "oxbo"}
TRACTOR_USD_PATH = "bin/gearbox/assets/tractor.usd"
BALE_USD_PATH = "markers/bale.usdz"
TARGET_INDICATOR_ID = "bale_target_indicator"


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    session.put(key, cbor2.dumps(payload))


@dataclass
class MachinePose:
    x: float = 0.0
    z: float = 0.0
    heading_rad: float | None = None
    stamp: float = 0.0
    seen: bool = False


def scatter_bales(
    session: zenoh.Session,
    bales: list[tuple[float, float]],
    active: int | None = None,
    hidden: set[int] | None = None,
) -> None:
    """Publish visible USD bales.

    Gearbox does not know these are "bales" or cylinders. It only receives a
    USD asset path plus optional variant selections; the USD asset owns the
    shape/materials.
    """
    hidden = hidden or set()
    for i, (bx, bz) in enumerate(bales):
        if i in hidden:
            remove_bale(session, i)
            continue
        put_cbor(
            session,
            f"gearbox/usd/load/bale_{i}",
            {
                "category": "static_usd",
                "x": float(bx),
                "z": float(bz),
                "usd_path": BALE_USD_PATH,
                "remove": False,
            },
        )


def remove_bale(session: zenoh.Session, bale_id: int) -> None:
    put_cbor(
        session,
        f"gearbox/usd/load/bale_{bale_id}",
        {"x": 0.0, "z": 0.0, "remove": True},
    )


def show_target_indicator(
    session: zenoh.Session,
    active: int | None,
    bales: list[tuple[float, float]],
) -> None:
    if active is None:
        put_cbor(
            session,
            f"gearbox/usd/load/{TARGET_INDICATOR_ID}",
            {"x": 0.0, "z": 0.0, "remove": True},
        )
        return
    bx, bz = bales[active]
    put_cbor(
        session,
        f"gearbox/usd/load/{TARGET_INDICATOR_ID}",
        {
            "category": "static_usd",
            "x": float(bx),
            "y": 2.2,
            "z": float(bz),
            "kind": "box",
            "height": 0.8,
            "radius": 0.35,
            "color": [1.0, 0.0, 0.0],
            "remove": False,
        },
    )


def clear_old_bales(session: zenoh.Session, count: int = 500) -> None:
    show_target_indicator(session, None, [])
    for i in range(count):
        remove_bale(session, i)


def wait_for_pose(pose: MachinePose, timeout_s: float) -> bool:
    t0 = time.time()
    while not pose.seen and time.time() - t0 < timeout_s:
        time.sleep(0.05)
    return pose.seen


def spawn_usd_machine(session: zenoh.Session, usd_path: str = TRACTOR_USD_PATH) -> None:
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
        "gearbox/usd/load/robot",
        {
            "category": "machine",
            "usd_path": usd_path,
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
    bales: list[tuple[float, float]],
    visited: set[int],
) -> tuple[int, float]:
    best_idx = -1
    best_d = math.inf
    for i, (bx, bz) in enumerate(bales):
        if i in visited:
            continue
        d = math.hypot(bx - pose_x, bz - pose_z)
        if d < best_d:
            best_idx = i
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
        spawn_usd_machine(session)
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
    visited: set[int] = set()
    scatter_bales(session, bales, hidden=visited)
    show_target_indicator(session, None, bales)
    time.sleep(0.2)

    visit_order: list[int] = []
    cmd_topic = f"gearbox/machines/{namespace}/cmd_vel"

    def publish_cmd(speed: float, yaw_rate: float) -> None:
        put_cbor(
            session,
            cmd_topic,
            {"linear": [float(speed), 0.0, 0.0], "angular": [0.0, 0.0, float(yaw_rate)]},
        )

    try:
        for step in range(len(bales)):
            best_idx, best_d = nearest_unvisited(pose.x, pose.z, bales, visited)
            if best_idx < 0:
                break
            tx, tz = bales[best_idx]
            scatter_bales(session, bales, active=best_idx, hidden=visited)
            show_target_indicator(session, best_idx, bales)
            print(
                f"\n[{step + 1:>3}/{len(bales)}]  visiting bale_{best_idx}  "
                f"target=({tx:+7.2f},{tz:+7.2f})  "
                f"from=({pose.x:+7.2f},{pose.z:+7.2f})  d={best_d:6.2f} m"
            )

            t_goal = time.time()
            while True:
                dx = tx - pose.x
                dz = tz - pose.z
                d_now = math.hypot(dx, dz)
                if d_now < 2.2:
                    break
                if time.time() - t_goal > 120.0:
                    print("  TIMEOUT — skipping this bale")
                    break

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
                    speed = 0.35
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
            visited.add(best_idx)
            visit_order.append(best_idx)
            remove_bale(session, best_idx)
            show_target_indicator(session, None, bales)
            print("    reached")
    except KeyboardInterrupt:
        print("\ninterrupted — stopping machine.")
        publish_cmd(0.0, 0.0)
        show_target_indicator(session, None, bales)
    finally:
        print(f"\nvisited {len(visited)}/{len(bales)} bales — order: {visit_order}")
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
    visited: set[int] = set()
    scatter_bales(session, bales, hidden=visited)
    show_target_indicator(session, None, bales)
    time.sleep(0.5)

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

    status_state: dict = {"active": False, "was_active": False, "reached_pulse": False}

    def on_status(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        active = bool(d.get("active", False))
        if status_state["was_active"] and not active:
            status_state["reached_pulse"] = True
        status_state["was_active"] = active
        status_state["active"] = active
        if d.get("reached"):
            status_state["reached_pulse"] = True

    session.declare_subscriber(f"{vehicle}/goto_status", on_status)
    time.sleep(0.3)

    visit_order: list[int] = []

    try:
        for step in range(len(bales)):
            best_idx, best_d = nearest_unvisited(pose_state["x"], pose_state["z"], bales, visited)
            if best_idx < 0:
                break
            tx, tz = bales[best_idx]
            scatter_bales(session, bales, active=best_idx, hidden=visited)
            show_target_indicator(session, best_idx, bales)
            print(
                f"\n[{step + 1:>3}/{len(bales)}]  visiting bale_{best_idx}  "
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
            status_state["reached_pulse"] = False
            status_state["was_active"] = False
            status_state["active"] = False
            local_reached = False
            t0 = time.time()
            while True:
                if status_state["reached_pulse"]:
                    break
                d_now = math.hypot(tx - pose_state["x"], tz - pose_state["z"])
                if d_now < float(cmd["tolerance"]):
                    local_reached = True
                    break
                if time.time() - t0 > 120.0:
                    print("  TIMEOUT — skipping this bale")
                    break
                time.sleep(0.2)
            print(f"    reached: {'local' if local_reached else 'status_pulse'}")
            visited.add(best_idx)
            visit_order.append(best_idx)
            remove_bale(session, best_idx)
            show_target_indicator(session, None, bales)
    except KeyboardInterrupt:
        print("\ninterrupted — cancelling current goto.")
        show_target_indicator(session, None, bales)
        put_cbor(
            session,
            f"{vehicle}/goto",
            {"x": 0.0, "z": 0.0, "yaw_deg": 0.0, "tolerance": 0.0, "yaw_tolerance_deg": 0.0, "max_speed": 0.0, "cancel": True},
        )
    finally:
        print(f"\nvisited {len(visited)}/{len(bales)} bales — order: {visit_order}")


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
