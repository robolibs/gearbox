#!/usr/bin/env python3
"""Spawn a tractor and a trailer, hitch them, drive a figure eight, unhitch.

Usage:
    python scripts/tractor_trailer.py [seconds] [speed_mps] [yaw_rps]

The trailer is loaded 4 m behind the tractor, attached with teleport onto
the tractor's rear drawbar, towed through a figure eight (alternating turn
direction), detached, and the tractor drives off alone. The composite link
tree and the attachments list are printed along the way.
"""

from __future__ import annotations

import math
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox  # noqa: E402

TRACTOR_USD = "bin/gearbox/assets/tractor.usd"
TRAILER_USD = "bin/gearbox/assets/trailer.usd"
TERRAIN_USD = "world/flatland.usd"


def print_tree(links: list[dict]) -> None:
    by_index = {l["index"]: l for l in links}
    children: dict[int | None, list[dict]] = {}
    for l in links:
        children.setdefault(l["parent_index"], []).append(l)

    def walk(parent: int | None, depth: int) -> None:
        for l in children.get(parent, []):
            coupling = f"  coupling {l['coupling']}" if l.get("coupling") else ""
            print(f"{'  ' * depth}{l['name']}  [{l['role']}]{coupling}")
            walk(l["index"], depth + 1)

    walk(None, 0)
    if not links:
        print("  (no links)")
    _ = by_index


def main() -> None:
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 40.0
    speed = float(sys.argv[2]) if len(sys.argv) > 2 else 2.0
    yaw = float(sys.argv[3]) if len(sys.argv) > 3 else 0.35

    gb = Gearbox()
    gb.wait_ready()
    print("clearing scene, loading terrain")
    gb.clear()
    time.sleep(0.3)
    gb.load("terrain", TERRAIN_USD, category="terrain")
    time.sleep(0.5)

    print("spawning tractor and trailer")
    gb.load_machine("t1", TRACTOR_USD, x=0.0, z=0.0)
    gb.load_machine("tr1", TRAILER_USD, x=0.0, z=-4.0)
    tractor = gb.machine("t1", timeout=30)
    trailer = gb.machine("tr1", timeout=30)
    time.sleep(1.0)

    print("couplings before attach:")
    for l in tractor.links():
        if l.get("coupling"):
            print(f"  t1 {l['name']}: {l['coupling']}")
    for l in trailer.links():
        if l.get("coupling"):
            print(f"  tr1 {l['name']}: {l['coupling']}")

    st = tractor.attach("tr1", teleport=True)
    if not st.ok:
        print(f"attach refused: {st.message}")
        return
    print("attached:", tractor.tools())
    print("composite link tree of t1:")
    print_tree(tractor.links())

    refused = trailer.cmd_vel(1.0, 0.0)
    print(f"driving the trailer directly is refused: code {refused.code} {refused.message}")

    tractor.claim(hold_ms=1000, client="tractor_trailer")
    t0 = time.time()
    half = seconds / 2.0
    print(f"towing for {seconds:.0f}s at {speed} m/s, figure eight with ±{yaw} rad/s")
    try:
        while time.time() - t0 < seconds:
            elapsed = time.time() - t0
            turn = yaw if elapsed < half else -yaw
            tractor.cmd_vel(speed, turn)
            s = tractor.state(wait=0.05)
            ts = trailer.state()
            if s and ts and int(elapsed * 10) % 20 == 0:
                # Odom is REP-103 now (Z up); x/y are the ground plane.
                gap = math.hypot(s.x - ts.x, s.y - ts.y)
                print(f"  t={elapsed:5.1f}s tractor ({s.x:+.1f}, {s.y:+.1f}) trailer ({ts.x:+.1f}, {ts.y:+.1f}) gap {gap:.2f} m")
            time.sleep(0.05)
    finally:
        tractor.stop()

    print("detaching")
    st = tractor.detach("tr1")
    print("detach:", "ok" if st.ok else st.message)
    print("attached now:", tractor.tools())
    for _ in range(40):
        tractor.cmd_vel(speed, 0.0)
        time.sleep(0.05)
    tractor.stop()
    tractor.release()
    s = tractor.state(wait=0.2)
    ts = trailer.state(wait=0.2)
    if s and ts:
        print(f"tractor left the trailer {math.hypot(s.x - ts.x, s.y - ts.y):.1f} m behind")


if __name__ == "__main__":
    main()
