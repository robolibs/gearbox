#!/usr/bin/env python3
"""Send a constant cmd_vel to one vehicle at 10 Hz until interrupted.

Usage:
    python drive.py [vehicle] [linear_x] [angular_z]

Defaults: vehicle=tractor_0, linear_x=2.0 m/s, angular_z=0.0 rad/s.
Examples:
    python drive.py                    # tractor crawls forward
    python drive.py tractor_0 -2.0     # reverse
    python drive.py tractor_0 2.0 0.5  # forward + steer
    python drive.py husky_1 0.5 1.0    # husky differential turn"""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    vehicle = sys.argv[1] if len(sys.argv) > 1 else "tractor_0"
    lx = float(sys.argv[2]) if len(sys.argv) > 2 else 2.0
    az = float(sys.argv[3]) if len(sys.argv) > 3 else 0.0

    session = zenoh.open(zenoh.Config())
    key = f"{vehicle}/cmd_vel"
    print(f"driving {vehicle}: linear=({lx:.2f}, 0, 0)  angular=(0, 0, {az:.2f})")
    print("press the Play button in the Transport ribbon if the sim is paused.")
    print("Ctrl-C to stop.")

    twist = {
        "linear": [lx, 0.0, 0.0],
        "angular": [0.0, 0.0, az],
    }
    payload = cbor2.dumps(twist)

    try:
        while True:
            session.put(key, payload)
            time.sleep(0.1)
    except KeyboardInterrupt:
        # Hand-off zero so the wheels stop on exit.
        zero = cbor2.dumps({"linear": [0, 0, 0], "angular": [0, 0, 0]})
        session.put(key, zero)
        print("\nstopped — sent zero twist on exit.")
    finally:
        session.close()


if __name__ == "__main__":
    main()
