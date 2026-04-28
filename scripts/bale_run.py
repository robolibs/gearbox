#!/usr/bin/env python3
"""Scatter N "bales" (yellow cones) randomly across a square field
and have the tractor visit them one by one in nearest-neighbour
order, removing each cone as it's reached.

The user reference is BaleUAVision (georkara/BaleUAVision on GitHub)
— their dataset is for aerial detection of bales, no ground-truth
coordinate dump we can grab, so we just generate a random spread
that has the same overall flavour.

Usage:
    python bale_run.py [vehicle] [n_bales] [field_size] [seed]

Defaults: vehicle=tractor_0, n_bales=50, field_size=200 (so points
fall in [-100, +100]² metres around the origin), seed=42."""

from __future__ import annotations

import math
import random
import sys
import time

import zenoh
import cbor2


def main() -> None:
    vehicle = sys.argv[1] if len(sys.argv) > 1 else "tractor_0"
    n_bales = int(sys.argv[2]) if len(sys.argv) > 2 else 50
    field = float(sys.argv[3]) if len(sys.argv) > 3 else 200.0
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 42

    rng = random.Random(seed)
    half = field / 2.0
    bales = [
        (rng.uniform(-half, half), rng.uniform(-half, half))
        for _ in range(n_bales)
    ]

    session = zenoh.open(zenoh.Config())

    # Unpause first so the tractor can move.
    session.put("gearbox/sim/clock/command", cbor2.dumps({"SetPaused": False}))
    time.sleep(0.05)

    # Spawn every bale as a yellow cone before we start visiting,
    # so the user sees the whole field upfront.
    print(f"scattering {n_bales} bales across {field:.0f} × {field:.0f} m field")
    for i, (bx, bz) in enumerate(bales):
        marker = {
            "x": float(bx),
            "z": float(bz),
            "height": 1.4,
            "radius": 0.45,
            "kind": "cone",
            "color": [0.95, 0.85, 0.15],
            "remove": False,
        }
        session.put(f"gearbox/markers/bale_{i}", cbor2.dumps(marker))
    time.sleep(0.5)  # let Bevy spawn them all

    # Subscribe to vehicle pose so we know when each goal is reached
    # (and so we can pick the nearest unvisited bale at each step).
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

    # We can't rely on `reached: true` from the broker — it's only
    # set for the single frame before the goal is cleared, and the
    # next published status has `active: false, reached: false`.
    # Track `was_active` and treat the active→inactive transition
    # as an arrival.
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

    time.sleep(0.3)  # let first odom arrive

    visited: set[int] = set()
    visit_order: list[int] = []

    try:
        for step in range(n_bales):
            # Nearest-neighbour pick — keeps the path short.
            cx, cz = pose_state["x"], pose_state["z"]
            best_idx = -1
            best_d = math.inf
            for i, (bx, bz) in enumerate(bales):
                if i in visited:
                    continue
                d = math.hypot(bx - cx, bz - cz)
                if d < best_d:
                    best_d = d
                    best_idx = i
            if best_idx < 0:
                break
            tx, tz = bales[best_idx]
            print(
                f"\n[{step + 1:>3}/{n_bales}]  visiting bale_{best_idx}  "
                f"target=({tx:+7.2f},{tz:+7.2f})  "
                f"from=({cx:+7.2f},{cz:+7.2f})  d={best_d:6.2f} m"
            )
            cmd = {
                "x": float(tx),
                "z": float(tz),
                "yaw_deg": 0.0,
                "tolerance": 2.0,           # generous radius — the tractor is big
                "yaw_tolerance_deg": 0.0,   # 0 ⇒ broker uses 2π default
                "max_speed": 0.0,
                "cancel": False,
            }
            session.put(f"{vehicle}/goto", cbor2.dumps(cmd))
            status_state["reached_pulse"] = False
            status_state["was_active"] = False
            status_state["active"] = False
            local_reached = False

            # Three independent ways to detect arrival — whichever
            # fires first wins. The status pulse is fragile (single
            # frame) so we also derive arrival from odom directly.
            t0 = time.time()
            while True:
                if status_state["reached_pulse"]:
                    break
                cx, cz = pose_state["x"], pose_state["z"]
                d_now = math.hypot(tx - cx, tz - cz)
                if d_now < float(cmd["tolerance"]):
                    local_reached = True
                    break
                if time.time() - t0 > 120.0:
                    print("  TIMEOUT — skipping this bale")
                    break
                if int(time.time() - t0) % 5 == 0:
                    print(
                        f"    ...  pos=({cx:+7.2f},{cz:+7.2f})  d_remaining={d_now:6.2f} m",
                        end="\r",
                    )
                time.sleep(0.2)
            print(f"    reached: {'local' if local_reached else 'status_pulse'}")

            visited.add(best_idx)
            visit_order.append(best_idx)
            # Remove the bale's marker now that it's been visited.
            session.put(
                f"gearbox/markers/bale_{best_idx}",
                cbor2.dumps({
                    "x": 0.0, "z": 0.0, "height": 0.0, "radius": 0.0,
                    "kind": "", "color": [0.0, 0.0, 0.0], "remove": True,
                }),
            )
    except KeyboardInterrupt:
        print("\ninterrupted — cancelling current goto.")
        session.put(
            f"{vehicle}/goto",
            cbor2.dumps({
                "x": 0.0, "z": 0.0, "yaw_deg": 0.0,
                "tolerance": 0.0, "yaw_tolerance_deg": 0.0,
                "max_speed": 0.0, "cancel": True,
            }),
        )
    finally:
        print(f"\nvisited {len(visited)}/{n_bales} bales — order: {visit_order}")
        session.close()


if __name__ == "__main__":
    main()
