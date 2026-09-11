#!/usr/bin/env python3
"""Drive the machines spawned by `hunter_spawn.py`.

    python scripts/hunter_drive.py [count] [speed_mps] [radius_m]

Defaults: count=3, speed=0.6 m/s, radius=1.8 m. Each machine drives a steady
circle on its own spawn point, alternating direction so neighbours turn away
from each other. Ctrl-C sends a zero twist to every one of them and exits.

The Hunter steers 27 degrees over a 0.651 m wheelbase, so its tightest circle
is 1.28 m; ask for less than that and the controller simply saturates and the
machine drives a wider arc than requested.

A twist is `linear x` forward and `angular z` yaw, ROS convention, and only
the session holder may send it — so each machine is claimed first.
"""

from __future__ import annotations

import math
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox  # noqa: E402

MACHINE = os.environ.get("MACHINE", "husky")
DEFAULT_COUNT = 3
DEFAULT_SPEED_MPS = 0.6
DEFAULT_RADIUS_M = 1.8

RATE_HZ = 20.0
WHEEL_BASE_M = 0.65142
MAX_STEER_RAD = math.radians(27.0)


def main() -> int:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_COUNT
    speed = float(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_SPEED_MPS
    radius = float(sys.argv[3]) if len(sys.argv) > 3 else DEFAULT_RADIUS_M

    tightest = WHEEL_BASE_M / math.tan(MAX_STEER_RAD)
    if radius < tightest:
        print(f"note: {radius:.2f} m is inside the Hunter's {tightest:.2f} m "
              f"minimum; it will drive the widest arc it can")

    gb = Gearbox()
    gb.wait_ready()
    machines = []
    for i in range(count):
        namespace = f"{MACHINE}_{i + 1}"
        machine = gb.machine(namespace, timeout=5.0)
        machine.claim(hold_ms=1000, take=True, client="hunter_drive.py")
        yaw = (speed / radius) * (1.0 if i % 2 == 0 else -1.0)
        machines.append((machine, yaw))
        print(f"{namespace}: {speed:.2f} m/s, {yaw:+.3f} rad/s "
              f"({'left' if yaw > 0 else 'right'} circle, r={radius:.1f} m)")

    print("\nCtrl-C to stop them")
    period = 1.0 / RATE_HZ
    try:
        while True:
            for machine, yaw in machines:
                machine.cmd_vel(speed, yaw)
            time.sleep(period)
    except KeyboardInterrupt:
        for machine, _yaw in machines:
            machine.stop()
            machine.release()
        print("\nstopped")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
