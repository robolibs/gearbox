#!/usr/bin/env python3
"""Build `tracer_geom.usdc` from the AgileX Tracer STL meshes.

    python scripts/build_tracer_geom.py

Writes `bin/gearbox/assets/tracer_geom.usdc`, which `tracer.usd` references
one mesh per painted part. Source is `OUSD/urdf_collection/agri/tracer`.

The Tracer is a differential-drive machine: two driven wheels on the middle
axle and four swivel casters. The URDF has the casters as fixed links, so
they are baked into the chassis here at their mounting points and never
move; the two wheels are their own meshes so `tracer.usd` can spin them.

*Frame.* The URDF is x forward, y left, z up with its origin on the deck.
`tracer.usd` drives towards -y with axles along x, so points go through
(x, y, z) -> (y, -x, z); the origin is kept, and `tracer.usd` lifts the
chassis body by the 0.153 m between deck origin and ground.

*Paint.* Body charcoal, the two side rails orange, the T-slot rails on the
deck aluminium; wheels split into tyre and hub, casters black.
"""

from __future__ import annotations

import pathlib
import sys
from collections import defaultdict

import stlgeom as G

MESH_DIR = pathlib.Path(
    "/home/bresilla/data/code/OUSD/urdf_collection/agri/tracer/meshes"
)
OUT = pathlib.Path(__file__).resolve().parents[1] / "bin/gearbox/assets/tracer_geom.usdc"

# Wheel radius is 0.062 m; the rubber starts about here.
TYRE_INNER_R = 0.046

# Caster mounts from the URDF, in its own frame. The right-hand pair is
# authored rolled half a turn so the same mesh hangs down on both sides.
CASTERS = {
    "left_front_link": ((0.23918, 0.1977, -0.058), False),
    "left_rear_link": ((-0.27077, 0.18305, -0.058), False),
    "right_front_link": ((0.23918, -0.1977, -0.058), True),
    "right_rear_link": ((-0.27082, -0.183, -0.058), True),
}

# The URDF rolls each wheel a quarter turn so the STL's +z points inboard.
# `outward` is therefore -x for the left wheel and +x for the right.
WHEELS = {
    "left_Link": ("wheel_left", lambda p: (-p[2], p[0], -p[1])),
    "right_link": ("wheel_right", lambda p: (p[2], p[0], p[1])),
}

URDF_TO_MACHINE = lambda p: (p[1], -p[0], p[2])  # noqa: E731


def chassis_part(faces, members):
    """Paint by where a shell sits: deck hardware, side rails, or body."""
    cx, cy, cz = G.centre([faces[i] for i in members])
    if cz > -0.003:
        return "alloy"
    if abs(cx) > 0.28 and -0.05 < cz < -0.01:
        return "accent"
    return "body"


def build_chassis(stage):
    faces = G.turn(G.read_stl(MESH_DIR / "base_link.STL"), URDF_TO_MACHINE)
    lo, hi = G.bounds(faces)
    ok = G.check("chassis", lo + hi, [-0.305, -0.341, -0.125, 0.305, 0.363, 0.018])

    painted = defaultdict(list)
    for members in G.shells(faces):
        painted[chassis_part(faces, members)].extend(members)
    for paint in ("body", "accent", "alloy"):
        G.emit(stage, f"chassis_{paint}", paint, faces, sorted(painted[paint]))

    # Casters: placed in the URDF frame first, then turned with the body.
    casters = []
    for source, (origin, rolled) in CASTERS.items():
        part = G.read_stl(MESH_DIR / f"{source}.STL")
        if rolled:
            part = G.turn(part, lambda p: (p[0], -p[1], -p[2]))
        part = G.turn(part, lambda p: p, origin)
        casters.extend(G.turn(part, URDF_TO_MACHINE))
    lo, hi = G.bounds(casters)
    ok = G.check("casters", [lo[2], hi[2]], [-0.153, -0.056], tol=0.004) and ok
    G.emit(stage, "casters", "tyre", casters, list(range(len(casters))))
    return ok


def build_wheel(stage, source, name, rotate):
    faces = G.turn(G.read_stl(MESH_DIR / f"{source}.STL"), rotate)
    lo, hi = G.bounds(faces)
    ok = G.check(name, lo[1:] + hi[1:], [-0.062, -0.062, 0.062, 0.062])
    painted = G.wheel_paint(faces, TYRE_INNER_R)
    for paint in ("tyre", "hub"):
        G.emit(stage, f"{name}_{paint}", paint, faces, painted[paint])
    return ok


def main() -> int:
    if not MESH_DIR.is_dir():
        print(f"mesh directory not found: {MESH_DIR}", file=sys.stderr)
        return 1
    stage = G.new_stage(OUT)
    ok = build_chassis(stage)
    for source, (name, rotate) in WHEELS.items():
        ok = build_wheel(stage, source, name, rotate) and ok
    stage.GetRootLayer().Save()
    print(f"\nwrote {OUT}  ({OUT.stat().st_size / 1048576:.1f} MB)")
    if not ok:
        print("geometry did not land where it should — see !! above", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
