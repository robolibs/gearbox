#!/usr/bin/env python3
"""Load the flat textured terrain and one Oxbo pea harvester.

Run Gearbox first:
    make run

Then:
    python scripts/oxbo_flatland.py

Nothing drives the machine; use `oxbo_follow_points.py` or `oxbo_joystick.py`.
"""

from __future__ import annotations

import time
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox  # noqa: E402

FLATLAND_USD_PATH = "world/flatland.usd"
OXBO_USD_PATH = "bin/gearbox/assets/oxbo.usd"


def main() -> None:
    gb = Gearbox()
    gb.wait_ready()
    gb.clear()
    time.sleep(0.3)
    # The id has to contain "terrain" for Gearbox's terrain systems to adopt
    # this scene root as the ground.
    gb.load("flatland_terrain", FLATLAND_USD_PATH, category="terrain")
    time.sleep(0.5)
    gb.load_machine("oxbo", OXBO_USD_PATH, label="oxbo.usd")
    print(f"requested flatland terrain: {FLATLAND_USD_PATH}")
    print(f"requested Oxbo pea harvester: {OXBO_USD_PATH}")
    machine = gb.machine("oxbo", timeout=90.0)
    print(f"oxbo is up as {machine.did}")


if __name__ == "__main__":
    main()
