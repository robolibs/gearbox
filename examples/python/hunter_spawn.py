#!/usr/bin/env python3
"""Load a flat terrain and some AgileX machines.

Run Gearbox first:

    make run

Then in another shell:

    python scripts/hunter_spawn.py [count] [spacing_m]

Defaults: count=3, spacing=3.0 m. Each machine gets its own namespace —
``husky_1``, ``husky_2``, … — and therefore its own machine agent with
``/machines/<ns>/claim``, ``/machines/<ns>/cmd_vel`` and ``/machines/<ns>/state``.

Nothing drives them; `hunter_drive.py` does. `stop.py <namespace>` takes one
over and sends zero velocity if a driver leaves it moving.

`MACHINE=hunter` spawns the Ackermann Hunter 2.0, `MACHINE=husky` (default)
the skid-steer Husky. The Hunter asset is authored from
`OUSD/urdf_collection/agri/hunter`.
"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox  # noqa: E402

FLATLAND_USD_PATH = "world/flatland.usd"
MACHINE = os.environ.get("MACHINE", "husky")
MACHINE_USD_PATH = f"bin/gearbox/assets/{MACHINE}.usd"

DEFAULT_COUNT = 3
DEFAULT_SPACING_M = 3.0
# Loading the next machine while the last one is still being wired into the
# physics world takes Gearbox down inside Rapier's broad-phase, so each spawn
# waits for its agent to answer before the next request goes out.
SPAWN_TIMEOUT_S = 10.0


def main() -> int:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_COUNT
    spacing = float(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_SPACING_M

    gb = Gearbox()
    gb.wait_ready()
    gb.clear()
    time.sleep(0.5)
    gb.load("flatland_terrain", FLATLAND_USD_PATH, category="terrain")
    time.sleep(0.5)

    namespaces = [f"{MACHINE}_{i + 1}" for i in range(count)]
    first = -spacing * (count - 1) / 2.0
    for i, namespace in enumerate(namespaces):
        x = first + i * spacing
        gb.load_machine(namespace, MACHINE_USD_PATH, x=x, label=f"{MACHINE} ({namespace})")
        try:
            gb.machine(namespace, timeout=SPAWN_TIMEOUT_S)
            live = ""
        except RuntimeError:
            live = "  (no agent within timeout)"
        print(f"requested {namespace} at x={x:+.1f}{live}")

    print(f"\nterrain: {FLATLAND_USD_PATH}")
    print(f"machine: {MACHINE_USD_PATH}")
    print(f"\ndrive them with:  python scripts/hunter_drive.py {count}")
    print(f"stop one with:    python scripts/stop.py {namespaces[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
