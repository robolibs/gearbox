#!/usr/bin/env python3
"""Build `hunter_geom.usdc` from the AgileX Hunter 2.0 STL meshes.

    python scripts/build_hunter_geom.py

Writes `bin/gearbox/assets/hunter_geom.usdc`, which `hunter.usd` references
one mesh per painted part.

The pre-converted `usd_collection/agri/hunter/hunter2_base.usdc` cannot be
used: every file in that collection reads as zero prims on USD 25.8 and 26.8
and fails `primChildren` validation at layer level. The source meshes it was
built from are fine, so the geometry is rebuilt from those instead.

*Frame.* The STLs are authored with the machine's long axis along x and the
wheels' axle along z. `hunter.usd` drives towards -y with wheel axles along x,
so each mesh is turned into that frame and the chassis is recentred on its own
bounding box, which is where `hunter.usd` puts the chassis rigid body.

*Paint.* The URDF gives the whole chassis one colour and the machine reads as a
black brick. It is not one colour — the chassis mesh is 96 separate shells, and
they are sorted here into the dark body, the orange rocker arms and bumpers the
URDF paints on the steer links, and the aluminium rail hardware along the top.
Wheels split into tyre and hub by radius.
"""

from __future__ import annotations

import pathlib
import sys
from collections import defaultdict

import stlgeom as G

MESH_DIR = pathlib.Path(
    "/home/bresilla/data/code/OUSD/urdf_collection/agri/hunter/meshes"
)
OUT = pathlib.Path(__file__).resolve().parents[1] / "bin/gearbox/assets/hunter_geom.usdc"

# Wheels: everything outboard of this radius is tyre, the rest is hub. The
# wheel is 0.1641 m in radius, and the tread wall starts a little inside that.
TYRE_INNER_R = 0.128

WHEELS = {
    "front_left_wheel_link": "wheel_front_left",
    "front_right_wheel_link": "wheel_front_right",
    "left_rear_link": "wheel_back_left",
    "right_rear_link": "wheel_back_right",
}


def chassis_part(faces, members):
    """Which paint one chassis shell takes, from where it sits on the machine.

    +x is left, +y is astern, +z is up, so the machine's nose is -y. The rail
    hardware is everything riding on the top deck; the orange is the rocker
    arms outboard and low over the wheels, plus the bumper under the nose.
    """
    cx, cy, cz = G.centre([faces[i] for i in members])
    if cz > 0.09:
        return "alloy"
    if abs(cx) > 0.15 and cz < -0.03:
        return "accent"
    if cy < -0.30 and cz < -0.02:
        return "accent"
    return "body"


def build_chassis(stage):
    faces = G.read_stl(MESH_DIR / "base_link.STL")
    lo, hi = G.bounds(faces)
    # 90 degrees of yaw, then recentred: the chassis rigid body in hunter.usd
    # sits at the middle of this box, not at the STL's origin.
    shift = ((lo[1] + hi[1]) / 2.0, (lo[0] + hi[0]) / 2.0, (lo[2] + hi[2]) / 2.0)
    faces = G.turn(faces, lambda p: (p[1], -p[0], p[2]), (-shift[0], shift[1], -shift[2]))

    lo, hi = G.bounds(faces)
    ok = G.check("chassis", lo + hi, [-0.322, -0.482, -0.134, 0.322, 0.482, 0.134])

    painted = defaultdict(list)
    for members in G.shells(faces):
        painted[chassis_part(faces, members)].extend(members)
    for paint in ("body", "accent", "alloy"):
        G.emit(stage, f"chassis_{paint}", paint, faces, sorted(painted[paint]))
    return ok


def build_wheel(stage, source, name):
    faces = G.read_stl(MESH_DIR / f"{source}.STL")
    # A quarter turn about y stands the disc up with its axle along x, which
    # is what the tyre colliders in hunter.usd assume.
    faces = G.turn(faces, lambda p: (-p[2], p[1], p[0]))

    lo, hi = G.bounds(faces)
    ok = G.check(name, lo[1:] + hi[1:], [-0.164, -0.164, 0.164, 0.164])

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
    for source, name in WHEELS.items():
        ok = build_wheel(stage, source, name) and ok
    stage.GetRootLayer().Save()
    print(f"\nwrote {OUT}  ({OUT.stat().st_size / 1048576:.1f} MB)")
    if not ok:
        print("geometry did not land where it should — see !! above", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
