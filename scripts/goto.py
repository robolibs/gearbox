#!/usr/bin/env python3
"""Send a one-shot `goto` command to a vehicle and watch its progress.

Usage:
    python goto.py <vehicle> <x> <z> [yaw_deg] [tolerance]
    python goto.py <vehicle> --cancel

Coordinates are gearbox world (X = lateral, Z = longitudinal — same
as the Inspector). `yaw_deg` follows gearbox heading: 0 = facing +Z,
90 = facing +X. `tolerance` is the position tolerance in metres
(default 0.6).

While the goto is active, the script prints both:
  * `goto_status`  — the controller's view (distance, heading error)
  * `odom`         — the actual chassis pose / speed
so you can sanity-check that the vehicle is actually moving toward
the target."""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    vehicle = sys.argv[1]

    if sys.argv[2] == "--cancel":
        cmd = {"x": 0.0, "z": 0.0, "yaw_deg": 0.0,
               "tolerance": 0.0, "yaw_tolerance_deg": 0.0,
               "max_speed": 0.0, "cancel": True}
    else:
        cmd = {
            "x": float(sys.argv[2]),
            "z": float(sys.argv[3]),
            "yaw_deg": float(sys.argv[4]) if len(sys.argv) > 4 else 0.0,
            "tolerance": float(sys.argv[5]) if len(sys.argv) > 5 else 0.0,
            "yaw_tolerance_deg": 0.0,
            "max_speed": 0.0,
            "cancel": False,
        }

    session = zenoh.open(zenoh.Config())

    # Auto-unpause so the user doesn't have to remember the Play
    # button. `ClockCommand::SetPaused(false)` is encoded as a
    # CBOR enum: { "SetPaused": false }.
    session.put(
        "gearbox/sim/clock/command",
        cbor2.dumps({"SetPaused": False}),
    )
    time.sleep(0.05)

    print(f"sending goto to {vehicle}: {cmd}")
    session.put(f"{vehicle}/goto", cbor2.dumps(cmd))

    if cmd["cancel"]:
        print("cancel sent.")
        session.close()
        return

    target_x, target_z = cmd["x"], cmd["z"]
    print(
        f"target = ({target_x:+.2f}, {target_z:+.2f})  "
        f"tol = {cmd['tolerance'] or 0.6:.2f} m"
    )
    print("watching — Ctrl-C to stop watching (the navigation keeps running).")

    state = {"pose": None, "speed": None, "status": None}

    def on_odom(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        p = d.get("position", [0, 0, 0])
        v = d.get("linear_velocity", [0, 0, 0])
        speed = (v[0] ** 2 + v[2] ** 2) ** 0.5
        # Recompute the actual world distance to target so we can
        # cross-check the controller's reported `distance_to_goal`.
        dx = target_x - p[0]
        dz = target_z - p[2]
        dist_actual = (dx * dx + dz * dz) ** 0.5
        state["pose"] = (p[0], p[2], dist_actual)
        state["speed"] = speed
        emit(state)

    def on_status(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:  # noqa: BLE001
            return
        if not d.get("active"):
            return
        state["status"] = (
            d.get("mode", "?"),
            d.get("distance_to_goal", 0.0),
            d.get("heading_error", 0.0),
            d.get("reached", False),
        )
        emit(state)

    def emit(state: dict) -> None:
        if state["pose"] is None or state["status"] is None:
            return
        x, z, dist_actual = state["pose"]
        speed = state["speed"]
        mode, dist_ctl, hdg, reached = state["status"]
        print(
            f"  pos=({x:+7.2f},{z:+7.2f})  "
            f"speed={speed:5.2f} m/s  "
            f"dist_real={dist_actual:6.2f}  "
            f"dist_ctl={dist_ctl:6.2f}  "
            f"hdg_err={hdg*57.2958:+7.2f}°  "
            f"mode={mode}{' REACHED' if reached else ''}"
        )

    session.declare_subscriber(f"{vehicle}/odom", on_odom)
    session.declare_subscriber(f"{vehicle}/goto_status", on_status)

    try:
        while True:
            time.sleep(0.5)
    except KeyboardInterrupt:
        pass
    finally:
        session.close()


if __name__ == "__main__":
    main()
