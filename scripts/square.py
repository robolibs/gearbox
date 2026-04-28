#!/usr/bin/env python3
"""Drive a vehicle in a square — 4 sides, ~4 seconds each.

Usage:
    python square.py [vehicle]

A simple demo of streaming cmd_vel commands. Forward 4 s, turn 2 s,
repeat. Sends zero twist on Ctrl-C so the vehicle stops cleanly."""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    vehicle = sys.argv[1] if len(sys.argv) > 1 else "tractor_0"
    session = zenoh.open(zenoh.Config())
    key = f"{vehicle}/cmd_vel"

    def send(lx: float, az: float) -> None:
        twist = {"linear": [lx, 0, 0], "angular": [0, 0, az]}
        session.put(key, cbor2.dumps(twist))

    print(f"driving {vehicle} in a square — Ctrl-C to stop.")
    try:
        while True:
            for _ in range(4):
                # Straight leg.
                t0 = time.time()
                while time.time() - t0 < 4.0:
                    send(2.0, 0.0)
                    time.sleep(0.1)
                # 90° turn.
                t0 = time.time()
                while time.time() - t0 < 2.0:
                    send(0.5, 0.8)
                    time.sleep(0.1)
    except KeyboardInterrupt:
        send(0.0, 0.0)
        print("\nstopped.")
    finally:
        session.close()


if __name__ == "__main__":
    main()
