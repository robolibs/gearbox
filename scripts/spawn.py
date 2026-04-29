#!/usr/bin/env python3
"""Spawn a vehicle in the running gearbox simulator.

Usage:
    python spawn.py [preset] [x] [z] [yaw_deg] [--player]

Defaults: preset=tractor, x=0, z=0, yaw_deg=0, player off.

Available presets: tractor | husky | robotti | drone | oxbo

The simulator publishes a confirmation on `gearbox/sim/spawned`
once the vehicle lands; this script waits up to 2 s for that event
and prints the assigned topic prefix (e.g. `tractor_0`)."""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}

    preset  = args[0]              if len(args) > 0 else "tractor"
    x       = float(args[1])       if len(args) > 1 else 0.0
    z       = float(args[2])       if len(args) > 2 else 0.0
    yaw_deg = float(args[3])       if len(args) > 3 else 0.0
    player  = "--player" in flags

    session = zenoh.open(zenoh.Config())

    # Latch the confirmation in a mutable dict so the callback can
    # write into it without `nonlocal`/`global` gymnastics.
    landed: dict = {}
    def on_spawned(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
        except Exception:
            return
        landed.update(d)

    sub = session.declare_subscriber("gearbox/sim/spawned", on_spawned)
    # Let zenoh propagate the subscriber registration to peers before
    # we publish — the simulator replies on the first Update frame
    # after it sees the spawn, ~16 ms later, which races our
    # subscriber-advertisement otherwise.
    time.sleep(0.5)

    req = {
        "preset": preset,
        "x": x,
        "y": 0.0,            # 0 ⇒ sim picks the preset's natural drop height
        "z": z,
        "yaw_deg": yaw_deg,
        "player": player,
    }
    print(f"spawning `{preset}` at ({x:+.2f}, {z:+.2f}) yaw={yaw_deg:+.1f}° "
          f"player={player}")
    session.put("gearbox/sim/spawn", cbor2.dumps(req))

    # Wait up to 5 s for the confirmation pub.
    t0 = time.time()
    while not landed and time.time() - t0 < 5.0:
        time.sleep(0.05)

    if landed:
        prefix = f"{landed['name']}_{landed['id']}"
        print(f"spawned: id={landed['id']}  topic prefix=`{prefix}`  "
              f"pos=({landed['x']:+.2f},{landed['y']:+.2f},{landed['z']:+.2f})")
    else:
        print("no confirmation received — is the simulator running?")

    del sub
    session.close()


if __name__ == "__main__":
    main()
