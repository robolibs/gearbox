#!/usr/bin/env python3
"""Several Oxbo harvesters, each following its own loop of field points.

Run Gearbox first:
    make run

Then:
    python scripts/oxbo_follow_points_multi.py [--route "x z x z ..."]...

One machine per `--route`; two default routes when none is given. Each
machine is spawned at its route's first point, facing the second, under its
own namespace `oxbo_<n>` with its own waypoint markers.
"""

from __future__ import annotations

import argparse
import math
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox, Machine  # noqa: E402
from oxbo_follow_points import publish_waypoint_markers, step_toward  # noqa: E402

FLATLAND_USD_PATH = "world/peafield.usd"
OXBO_USD_PATH = "bin/gearbox/assets/oxbo.usd"
TICK_DT = 0.10
POSE_TIMEOUT_S = 90.0

DEFAULT_ROUTES: list[list[tuple[float, float]]] = [
    [(-20.0, 42.0), (-4.0, 42.0), (-4.0, -28.0), (-36.0, -28.0), (-36.0, 42.0)],
    [(20.0, -28.0), (36.0, -28.0), (36.0, 42.0), (4.0, 42.0), (4.0, -28.0)],
]


@dataclass
class Harvester:
    idx: int
    route: list[tuple[float, float]]
    namespace: str = ""
    goal_idx: int = 0
    machine: Machine | None = field(default=None)

    def __post_init__(self) -> None:
        self.namespace = f"oxbo_{self.idx}"

    def spawn_pose(self) -> tuple[float, float, float]:
        (x0, z0), (x1, z1) = self.route[0], self.route[1 % len(self.route)]
        return x0, z0, math.degrees(math.atan2(x1 - x0, z1 - z0))

    def load(self, gb: Gearbox) -> None:
        x, z, yaw_deg = self.spawn_pose()
        gb.load_machine(self.namespace, OXBO_USD_PATH, x=x, z=z, yaw_deg=yaw_deg, label=f"{self.namespace}.usd")

    def attach(self, gb: Gearbox) -> bool:
        try:
            self.machine = gb.machine(self.namespace, timeout=POSE_TIMEOUT_S)
        except RuntimeError:
            return False
        self.machine.claim(hold_ms=1000, take=True, client="oxbo_follow_points_multi.py")
        return self.machine.state(wait=30.0) is not None

    def step(self) -> None:
        if self.machine is not None:
            self.goal_idx = step_toward(self.machine, self.route, self.goal_idx)

    def stop(self) -> None:
        if self.machine is not None:
            self.machine.stop()
            self.machine.release()


def parse_route(text: str) -> list[tuple[float, float]]:
    vals = [float(v) for v in text.replace(",", " ").split()]
    if len(vals) < 4 or len(vals) % 2 != 0:
        raise SystemExit(f"route needs at least two x z pairs: {text!r}")
    return list(zip(vals[0::2], vals[1::2], strict=True))


def parse_args() -> list[list[tuple[float, float]]]:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--route", action="append", metavar='"x z x z ..."',
                        help="waypoints for one machine; repeat the flag for more machines")
    args = parser.parse_args()
    return [parse_route(r) for r in args.route] if args.route else DEFAULT_ROUTES


def follow(machines: list[Harvester]) -> None:
    for m in machines:
        print(f"{m.namespace} route:")
        for i, (x, z) in enumerate(m.route):
            print(f"  {i}: x={x:.1f}, z={z:.1f}")
    print("Ctrl-C to stop.")
    try:
        while True:
            for m in machines:
                m.step()
            time.sleep(TICK_DT)
    except KeyboardInterrupt:
        pass
    finally:
        for m in machines:
            m.stop()
        print("stopped all machines")


def main() -> None:
    routes = parse_args()
    machines = [Harvester(i, route) for i, route in enumerate(routes)]
    gb = Gearbox()
    gb.wait_ready()
    gb.clear()
    time.sleep(0.3)
    gb.load("flatland_terrain", FLATLAND_USD_PATH, category="terrain")
    for m in machines:
        publish_waypoint_markers(gb, f"{m.namespace}_wp", m.route)
        m.load(gb)
    missing = [m.namespace for m in machines if not m.attach(gb)]
    if missing:
        raise SystemExit("no state from machines: " + ", ".join(missing) + ". Is Gearbox running?")
    follow(machines)


if __name__ == "__main__":
    main()
