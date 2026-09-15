#!/usr/bin/env python3
"""Load the de Marke cow house and put some machines in its centre alley.

Run Gearbox first, then:

    python scripts/barn_spawn.py [count]

The barn is `bin/gearbox/assets/demarke/barn.usdc`, built from the Isaac Sim
export by `scripts/build_demarke_barn.py` (run that first if it is missing).
It is loaded as a `world` layer, so Gearbox neither terrain-snaps it nor
treats its floor as the terrain — the flat ground underneath stays the
ground, and the barn's floor is authored at z = 0 so it sits on that ground.

The barn keeps its own frame — the one `syncbot`'s `barn_layout.py` measured
its lanes in: x across the building (21.5..44.8 m), y along it (0..67.5 m),
z up. Gearbox is Y-up, and the loader turns Z-up USD into it as
(x, y, z) -> (x, z, -y), so a barn point (x, y) is Gearbox (x, -y). The
machines are placed on the centre alley, x = 29.25..32.5, spread along it.
"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox  # noqa: E402

FLATLAND_USD_PATH = "world/flatland.usd"
BARN_USD_PATH = "demarke/barn.usdc"
MACHINE = os.environ.get("MACHINE", "husky")
MACHINE_USD_PATH = f"bin/gearbox/assets/{MACHINE}.usd"

DEFAULT_COUNT = 3
ALLEY_X = (29.25 + 32.50) / 2.0
ALLEY_Y = (8.0, 60.0)
SPAWN_TIMEOUT_S = 10.0
# The barn is 1.4 GB of textures and 1.4 M triangles; give it a moment before
# dropping machines into it.
BARN_SETTLE_S = 6.0


def main() -> int:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_COUNT

    gb = Gearbox()
    gb.wait_ready()
    gb.clear()
    time.sleep(0.5)
    gb.load("flatland_terrain", FLATLAND_USD_PATH, category="terrain")
    time.sleep(0.5)
    gb.load("demarke_barn", BARN_USD_PATH, category="world")
    print(f"barn requested: {BARN_USD_PATH}")
    time.sleep(BARN_SETTLE_S)

    namespaces = [f"{MACHINE}_{i + 1}" for i in range(count)]
    step = (ALLEY_Y[1] - ALLEY_Y[0]) / max(count - 1, 1)
    for i, namespace in enumerate(namespaces):
        barn_y = ALLEY_Y[0] + i * step
        gb.load_machine(namespace, MACHINE_USD_PATH, x=ALLEY_X, z=-barn_y, label=f"{MACHINE} ({namespace})")
        try:
            gb.machine(namespace, timeout=SPAWN_TIMEOUT_S)
            live = ""
        except RuntimeError:
            live = "  (no agent within timeout)"
        print(f"{namespace} at barn ({ALLEY_X:.2f}, {barn_y:.1f}){live}")

    print(f"\ndrive them with:  python scripts/hunter_drive.py {count} (MACHINE={MACHINE})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
