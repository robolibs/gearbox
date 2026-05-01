#!/usr/bin/env python3
"""Spawn a handful of tractors (or whatever preset you pass) in a
small ring around the origin. Pure smoke-test for the spawn API —
no goto, no markers, no input from the simulator side.

Usage:
    python spawn_some.py [n] [preset] [radius]

Defaults: n=4 tractors in a 12 m radius ring around (0, 0).
Preset can be any of: tractor | husky | robotti | drone | oxbo."""

from __future__ import annotations

import math
import sys
import time

import zenoh
import cbor2


def main() -> None:
    n      = int(sys.argv[1])   if len(sys.argv) > 1 else 4
    preset = sys.argv[2]        if len(sys.argv) > 2 else "tractor"
    radius = float(sys.argv[3]) if len(sys.argv) > 3 else 12.0

    session = zenoh.open(zenoh.Config())

    # Latch every confirmation we see so we can print the assigned
    # topic prefixes at the end.
    landed: list[dict] = []

    def on_spawned(sample: zenoh.Sample) -> None:
        try:
            landed.append(cbor2.loads(bytes(sample.payload)))
        except Exception:  # noqa: BLE001
            pass

    sub = session.declare_subscriber("gearbox/sim/spawned", on_spawned)
    # Let zenoh propagate the subscriber registration to peers before
    # we start publishing — otherwise the simulator's reply races our
    # subscriber registration and the first few confirmations get
    # dropped.
    time.sleep(0.5)

    # Wipe whatever's left from a previous run.
    session.put("gearbox/sim/reset", cbor2.dumps({"pause_clock": False}))
    # Unpause so the user immediately sees the vehicles drop and
    # settle on the ground.
    session.put("gearbox/sim/clock/command", cbor2.dumps({"SetPaused": False}))
    time.sleep(0.1)

    print(f"spawning {n}× `{preset}` on a {radius:.1f} m ring")
    for i in range(n):
        angle = (2.0 * math.pi * i) / n
        x = radius * math.cos(angle)
        z = radius * math.sin(angle)
        # Face inward toward the origin.
        # gearbox forward = -Z; angle from +X to (−x, −z)/r:
        #   yaw = atan2(−x, −z) gives the signed Y-axis rotation that
        #   points the vehicle's −Z (forward) along (−x, −z).
        yaw_deg = math.degrees(math.atan2(-x, -z))
        req = {
            "preset": preset,
            "x": float(x),
            "y": 0.0,           # 0 ⇒ sim auto-picks the natural drop height
            "z": float(z),
            "yaw_deg": float(yaw_deg),
            "player": False,
        }
        print(f"  [{i + 1}/{n}]  x={x:+7.2f}  z={z:+7.2f}  yaw={yaw_deg:+7.1f}°")
        session.put("gearbox/sim/spawn", cbor2.dumps(req))
        # Tiny gap so the simulator processes each request on its own
        # Update frame and the assigned ids come out monotonically
        # (purely cosmetic — the API handles bursts fine).
        time.sleep(0.1)

    # Drain confirmations for a couple of seconds, then summarise.
    t0 = time.time()
    while len(landed) < n and time.time() - t0 < 5.0:
        time.sleep(0.05)

    print()
    if not landed:
        print("no confirmations received — is the simulator running and is "
              "`SpawnApiPlugin` active?")
    else:
        print(f"got {len(landed)}/{n} confirmations:")
        for ev in landed:
            print(f"  {ev['name']}_{ev['id']}  at "
                  f"({ev['x']:+.2f}, {ev['y']:+.2f}, {ev['z']:+.2f})  "
                  f"yaw={ev['yaw_deg']:+.1f}°")

    del sub
    session.close()


if __name__ == "__main__":
    main()
