#!/usr/bin/env python3
"""Load a flat textured terrain scene and one Oxbo pea harvester USD.

Run Gearbox first:

    make run

Then in another shell:

    python scripts/oxbo_flatland.py

This is intentionally not a bale-collection demo: no tractors, no bales, no
driving loop. It only asks the running Gearbox app to load:

* ``world/flatland.usd`` as a terrain/world layer
* ``bin/gearbox/assets/oxbo.usd`` as one USD machine named ``oxbo``
"""

from __future__ import annotations

import time

import cbor2
import zenoh


FLATLAND_USD_PATH = "world/flatland.usd"
OXBO_USD_PATH = "bin/gearbox/assets/oxbo.usd"


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    session.put(
        key,
        cbor2.dumps(payload),
        congestion_control=zenoh.CongestionControl.BLOCK,
    )


def clear_sim(session: zenoh.Session) -> None:
    put_cbor(session, "gearbox/sim/clear", {"pause_clock": False})


def load_flatland(session: zenoh.Session) -> None:
    # Use an id containing "terrain" so Gearbox's terrain systems recognize
    # this as the active terrain scene root.
    put_cbor(
        session,
        "gearbox/usd/load/flatland_terrain",
        {
            "category": "terrain",
            "usd_path": FLATLAND_USD_PATH,
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "remove": False,
        },
    )


def load_oxbo(session: zenoh.Session) -> None:
    put_cbor(
        session,
        "gearbox/usd/load/oxbo",
        {
            "category": "machine",
            "usd_path": OXBO_USD_PATH,
            "namespace": "oxbo",
            "label": "oxbo.usd",
            # Keep it at the exact flatland origin. The terrain height helper
            # is also zero at origin, so this avoids hilly-demo assumptions.
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "yaw_deg": 0.0,
            "remove": False,
        },
    )


def main() -> None:
    opened = zenoh.open(zenoh.Config())
    session = opened.wait() if hasattr(opened, "wait") else opened
    # Let subscriptions in the running app settle before publishing the loads.
    time.sleep(0.2)
    clear_sim(session)
    time.sleep(0.3)
    load_flatland(session)
    time.sleep(0.5)
    load_oxbo(session)
    print(f"requested flatland terrain: {FLATLAND_USD_PATH}")
    print(f"requested Oxbo pea harvester: {OXBO_USD_PATH}")


if __name__ == "__main__":
    main()
