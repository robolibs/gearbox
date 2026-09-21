#!/usr/bin/env python3
"""Build `husky_geom.usdc` from the Clearpath Husky meshes.

    python scripts/build_husky_geom.py

Writes `bin/gearbox/assets/husky_geom.usdc`, which `husky.usd` references
one mesh per painted part. Source is `OUSD/urdf_collection/matlab/husky_description`.

The Husky is a four-wheel skid-steer machine: every wheel is driven, it turns
on the spot. The URDF splits the body into fixed links — base, yellow top
chassis, user rail, top plate, two bumpers — each with its own mesh and
colour, and those are baked into the chassis here at their mounting points.
The `.urdf2usd_rt` conversion that ships beside the URDF does not parse on a
current USD, so the meshes are converted afresh.

*Frame.* The URDF is x forward, y left, z up, origin on the chassis floor.
`husky.usd` drives towards -y with axles along x, so points go through
(x, y, z) -> (y, -x, z); the origin is kept, and `husky.usd` lifts the
chassis body by the 0.132 m between it and the ground.

*Paint.* Taken from the DAE materials the URDF actually displays: the top
chassis is Husky yellow, everything else on the body dark; wheels split into
tyre and hub by radius.
"""

from __future__ import annotations

import pathlib
import sys

import stlgeom as G

MESH_DIR = pathlib.Path(
    "/home/bresilla/data/code/OUSD/urdf_collection/matlab/husky_description/meshes"
)
OUT = pathlib.Path(__file__).resolve().parents[1] / "bin/gearbox/assets/husky_geom.usdc"

# Wheel mesh radius is 0.178 m with the tread; the rim starts about here.
TYRE_INNER_R = 0.105

# Fixed body parts: mesh -> (paint, URDF mount, yawed half a turn, y-up STL).
# bumper.stl alone is exported y-up; the other STLs match their DAE frames.
PARTS = {
    "base_link": ("body", (0.0, 0.0, 0.0), False, False),
    "top_chassis": ("husky_yellow", (0.0, 0.0, 0.0), False, False),
    "user_rail": ("body", (0.272, 0.0, 0.245), False, False),
    "top_plate": ("body", (0.0812, 0.0, 0.245), False, False),
    "bumper": ("body", (0.48, 0.0, 0.091), False, True),
}
REAR_BUMPER = ("bumper", (-0.48, 0.0, 0.091))

WHEELS = {
    "wheel_front_left": None,
    "wheel_front_right": None,
    "wheel_back_left": None,
    "wheel_back_right": None,
}

URDF_TO_MACHINE = lambda p: (p[1], -p[0], p[2])  # noqa: E731


def part_faces(source, mount, yawed, y_up):
    faces = G.read_stl(MESH_DIR / f"{source}.stl")
    if y_up:
        faces = G.turn(faces, lambda p: (p[0], -p[2], p[1]))
    if yawed:
        faces = G.turn(faces, lambda p: (-p[0], -p[1], p[2]))
    faces = G.turn(faces, lambda p: p, mount)
    return G.turn(faces, URDF_TO_MACHINE)


def build_chassis(stage):
    ok = True
    groups = {}
    for source, (paint, mount, yawed, y_up) in PARTS.items():
        groups.setdefault(paint, []).extend(part_faces(source, mount, yawed, y_up))
    source, mount = REAR_BUMPER
    groups["body"].extend(part_faces(source, mount, True, True))

    lo, hi = G.bounds(groups["body"] + groups["husky_yellow"])
    ok = G.check("chassis", lo + hi, [-0.285, -0.493, -0.003, 0.285, 0.493, 0.245], tol=0.01) and ok
    for paint, faces in groups.items():
        G.emit(stage, f"chassis_{paint}", paint, faces, list(range(len(faces))))
    return ok


def build_wheel(stage, name):
    # The disc is authored in the URDF's x-z plane with its axle along y; the
    # same quarter turn as the body puts that axle along x.
    faces = G.turn(G.read_stl(MESH_DIR / "wheel.stl"), URDF_TO_MACHINE)
    lo, hi = G.bounds(faces)
    ok = G.check(name, lo[1:] + hi[1:], [-0.178, -0.178, 0.178, 0.178])
    painted = G.wheel_paint(faces, TYRE_INNER_R)
    G.emit(stage, f"{name}_tyre", "husky_tyre", faces, painted["tyre"])
    G.emit(stage, f"{name}_hub", "hub", faces, painted["hub"])
    return ok


def main() -> int:
    if not MESH_DIR.is_dir():
        print(f"mesh directory not found: {MESH_DIR}", file=sys.stderr)
        return 1
    stage = G.new_stage(OUT)
    ok = build_chassis(stage)
    # One wheel mesh serves all four corners; `husky.usd` places each.
    ok = build_wheel(stage, "wheel") and ok
    stage.GetRootLayer().Save()
    print(f"\nwrote {OUT}  ({OUT.stat().st_size / 1048576:.1f} MB)")
    if not ok:
        print("geometry did not land where it should — see !! above", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
