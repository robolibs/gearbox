"""The vertical farm's robot: a twin-mast stacker crane that tends the racks
in place, after the miniload cranes of automated warehouses and plant
factories (a bottom carriage on one floor rail, masts held upright by a
top guide rail, a lifting carriage hoisted by toothed belts).

    rail      held to the world: the floor rail, the top guide rail hung
              from the aisle ties, the enclosed conductor rail, the end
              stops with their buffers, the travel laser's reflector and
              the optical data link at the parking end
    crane     `travel` along the aisle: the bottom carriage with its drive
              and idler wheels and guide rollers, buffers, the two masts,
              their belts and pulleys, the top beam with its guide rollers
              and beacon, the hoist drive, the control cabinet, the lasers
    carriage  `lift` between the masts on rollers riding their guide fins,
              the slewing drive under its cross beam
    turret    `turn` 0..180 degrees from the +Y rack to the -Y one: the
              telescopic arm's housing and its drive
    stage     `reach`: the arm's middle stage, at half the stroke
    arm       `reach_top`: the top stage and the tending head at its tip,
              with a recognition camera looking down, a light sensor, a
              climate probe, a ring light, spray nozzles and a gripper

The arm telescopes like a load handler's three-stage fork: one controller,
`reach`, moves both stages, each over half the stroke. Retracted, the arm
turns clear of the masts and the racks; retract it before turning or
travelling. Power reaches the crane by the conductor rail and its
commands by the optical data link, as on long aisles; the carriage is fed
by an energy chain running in its channel up the -X mast.

Frame: x along the aisle, y across it, z up from the hall's floor, the
crane's middle at travel 0 at the origin. Runs inside Blender.
"""
import math
import os
from dataclasses import dataclass, field

import bmesh
import bpy
from mathutils import Matrix

import kit
import mechanics as mk
import patterns
import vertical
from kit import box, prism, rod, tube
from mechanics import ALONG, UP, Part, at, bar, cyl, turned
from robots import body, drum, head_sensors, hull, joint
from usda import Usda, end, schemas
from vehicles import bindings, export_geom, visual


@dataclass
class Crane:
    """A stacker crane's sizes, metres, in its frame at rest."""
    travel: float = vertical.CRANE_TRAVEL
    ties: list = field(default_factory=lambda: [x - vertical.CRANE_X0 for x in vertical.ties()])
    tie_z: float = vertical.TIE[0] - vertical.FLOOR
    rail_top: float = 0.052
    wheel_r: float = 0.125
    wheel_x: float = 0.88
    base: tuple = (1.05, 0.17, 0.24, 0.49)
    mast_x: float = 0.80
    mast: tuple = (0.16, 0.24)
    beam: tuple = (5.18, 5.34)
    carriage_z: float = 0.97
    lift: float = 3.91
    reach: float = 0.72
    cond_y: float = 0.38
    cond_z: float = 0.30


CRANE = Crane()


def anchor(c=CRANE):
    """Where the crane's anchor holds its rail, in the machine's frame."""
    return (c.travel / 2, 0.0, 0.05)


def frame(c):
    """The heights and offsets the looks and bodies share."""
    g = {}
    g["fin_x"] = c.mast_x - c.mast[0] / 2
    g["xi"] = g["fin_x"] - 0.05
    g["web_z"] = c.tie_z - 0.012 - 0.14
    g["roll_z"] = c.tie_z - 0.09
    g["rail_x"] = (-1.31, c.travel + 1.31)
    g["top_x"] = (-1.5, c.travel + 1.5)
    g["turret"] = c.carriage_z - 0.15
    g["stage"] = (0.0, 0.215, c.carriage_z - 0.2775)
    g["head"] = (0.0, 0.44, c.carriage_z - 0.3875)
    return g


# Materials


def materials(looks):
    """The machine parts' finishes and the crane's own paint, signals and markings."""
    m = mk.materials(looks)
    g = looks.get
    for key, name, colour, rough, seed in (("yellow", "crane_yellow", (0.93, 0.68, 0.08), 0.45, 81), ("blue", "crane_blue", (0.08, 0.24, 0.45), 0.5, 82)):
        m[key] = g(name, colour, rough, 0.0, maps=mk.finish(colour, rough, seed, grime=0.08))
    m.update({
        "red": g("crane_red", (0.78, 0.06, 0.04), 0.35, 0.0),
        "amber": g("crane_amber", (1.0, 0.5, 0.05), 0.2, 0.0, emit=((1.0, 0.5, 0.05), 4.0)),
        "screen": g("crane_screen", (0.12, 0.32, 0.45), 0.1, 0.0, emit=((0.25, 0.55, 0.75), 1.5)),
        "reflector": g("crane_reflector", (0.85, 0.08, 0.05), 0.15, 0.0),
        "hazard": g("crane_hazard", (0.9, 0.6, 0.05), 0.5, 0.0, maps=lambda: patterns.stripes((0.9, 0.6, 0.05), (0.03, 0.03, 0.03), 4)),
    })
    return m


def _gusset(dy, h, t=0.012):
    """A triangular gusset in the YZ plane reaching dy along Y and h up, t thick across X."""
    bm = prism([(0.0, 0.0), (dy, 0.0), (0.0, h)], -t / 2, t / 2, 0.3)
    bm.transform(Matrix.Rotation(math.pi / 2, 4, "Z") @ Matrix.Rotation(math.pi / 2, 4, "X"))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


# Looks


def rail_part(c, g):
    """The rail: the floor rail on its sole plate with clips, the top guide
    rail clamped under every aisle tie, the conductor rail on its floor
    stands, and the end stops with their buffers, reflector and data link."""
    p = Part("rail", anchor(c))
    x0, x1 = g["rail_x"]
    length = x1 - x0
    xm = (x0 + x1) / 2
    p.add("steel", box((length, 0.16, 0.012), (xm, 0, 0.006), tile=1.0))
    p.add("steel", box((length, 0.05, 0.04), (xm, 0, 0.032), tile=1.0, bevel=0.004))
    n = int(length / 0.6)
    for k in range(n + 1):
        x = x0 + 0.1 + (length - 0.2) * k / n
        for s in (-1, 1):
            p.add("black", box((0.08, 0.04, 0.022), (x, s * 0.047, 0.023), tile=0.2, bevel=0.003))
            p.add("steel", mk.hexagon(0.012, 0.034, 0.044), Matrix.Translation((x, s * 0.06, 0)))
    # Top guide rail: a T of flange and web, clamped to the ties, spliced every 6 m.
    t0, t1 = g["top_x"]
    tm = (t0 + t1) / 2
    p.add("steel", box((t1 - t0, 0.12, 0.012), (tm, 0, c.tie_z - 0.006), tile=1.0))
    p.add("steel", box((t1 - t0, 0.03, 0.14), (tm, 0, g["web_z"] + 0.07), tile=1.0))
    for x in c.ties:
        x = min(max(x, t0 + 0.06), t1 - 0.06)
        for s in (-1, 1):
            p.add("black", box((0.08, 0.04, 0.03), (x, s * 0.08, c.tie_z - 0.015), tile=0.2))
            p.add("steel", mk.hexagon(0.01, c.tie_z - 0.04, c.tie_z - 0.03), Matrix.Translation((x, s * 0.085, 0)))
    for k in range(1, int((t1 - t0) / 6.0) + 1):
        xs = t0 + k * (t1 - t0) / (int((t1 - t0) / 6.0) + 1)
        for s in (-1, 1):
            p.add("black", box((0.22, 0.008, 0.1), (xs, s * 0.019, g["web_z"] + 0.07), tile=0.2))
            mk.bolts(p, at((xs, s * 0.023, g["web_z"] + 0.07), (0, s, 0)), [(dx, dz) for dx in (-0.07, 0.07) for dz in (-0.025, 0.025)], 0.006)
    for x, d in ((t0 + 0.02, -1), (t1 - 0.02, 1)):
        p.add("black", box((0.04, 0.12, 0.16), (x, 0, g["web_z"] + 0.06), tile=0.2))
    # Conductor rail on floor stands, its collector slot toward the crane.
    stands = [x0 + 0.4 + 1.5 * k for k in range(int((length - 0.8) / 1.5) + 1)]
    mk.conductor(p, x0 + 0.3, x1 - 0.3, c.cond_y, c.cond_z, 0.03, 0.05, -1, [], c.cond_z)
    for x in stands:
        p.add("grey", box((0.03, 0.036, 0.06), (x, c.cond_y, c.cond_z), tile=0.2))
        p.add("steel", box((0.03, 0.03, c.cond_z - 0.025), (x, c.cond_y + 0.03, (c.cond_z - 0.025) / 2), tile=0.2))
        p.add("steel", box((0.1, 0.1, 0.008), (x, c.cond_y + 0.03, 0.004), tile=0.2))
    p.add("plastic", tube([(x0 + 0.36, c.cond_y + 0.06, c.cond_z), (x0 + 0.36, c.cond_y + 0.06, 0.04), (x0 - 0.1, c.cond_y + 0.06, 0.02)], 0.012, 8))
    # End stops: welded stands straddling the rail ends, a polyurethane buffer pair facing the crane.
    for xe, d in ((x0, -1), (x1, 1)):
        xc = xe + d * 0.125
        p.add("yellow", box((0.25, 0.5, 0.5), (xc, 0, 0.25), tile=0.3, bevel=0.01))
        p.add("hazard", box((0.004, 0.46, 0.2), (xe - d * 0.002, 0, 0.38), tile=0.4))
        p.add("steel", box((0.4, 0.62, 0.012), (xc, 0, 0.006), tile=0.3))
        for sx in (-1, 1):
            for sy in (-1, 1):
                p.add("steel", mk.hexagon(0.014, 0.012, 0.026), Matrix.Translation((xc + sx * 0.15, sy * 0.27, 0)))
        for s in (-1, 1):
            mk.bumper(p, at((xe, s * 0.1, 0.38), (-d, 0, 0)), 0.045, 0.12)
    p.add("reflector", box((0.006, 0.16, 0.16), (x1 - 0.003, 0.0, 0.2), tile=0.2))
    p.add("black", box((0.12, 0.1, 0.12), (x0 - 0.06, 0.3, 0.6), tile=0.2, bevel=0.006))
    p.add("lens", box((0.004, 0.06, 0.06), (x0 + 0.002, 0.3, 0.6), tile=0.2))
    p.add("steel", box((0.04, 0.04, 0.54), (x0 - 0.1, 0.3, 0.27), tile=0.2))
    return p


def crane_part(c, g):
    """The crane: the bottom carriage's box girder with its wheel blocks,
    drive and idler wheels, guide rollers and buffers; both masts with
    their foot gussets, guide fins, belts, pulleys and splices; the top beam
    with guide rollers and beacon; the hoist drive, the control cabinet,
    the maintenance ladder, the chain channel, the collector, the lasers
    and the data link."""
    p = Part("crane", (0, 0, 0.38))
    bx, by, bz0, bz1 = c.base
    bzc = (bz0 + bz1) / 2
    wz = c.rail_top + c.wheel_r
    p.add("yellow", bar((-0.7, 0, bz0), (0.7, 0, bz0), mk.rect(2 * by, bz1 - bz0, 0.0), UP, 0.6))
    for k in range(-2, 3):
        for s in (-1, 1):
            p.add("yellow", box((0.012, 0.012, bz1 - bz0 - 0.04), (k * 0.28, s * (by + 0.006), bzc), tile=0.2))
    # Wheel blocks: cheek plates either side of each wheel, a top plate, guide rollers on the rail head.
    for xw, drive in ((c.wheel_x, True), (-c.wheel_x, False)):
        d = 1 if xw > 0 else -1
        xa, xb = 0.7 * d, bx * d
        xc, xl = (xa + xb) / 2, abs(xb - xa)
        for s in (-1, 1):
            p.add("yellow", box((xl, 0.03, bz1 - 0.07), (xc, s * 0.06, (bz1 + 0.07) / 2), tile=0.3))
            p.add("yellow", box((xl, 0.03, bz1 - bz0), (xc, s * (by - 0.015), bzc), tile=0.3))
            mk.bolts(p, at((xw, s * 0.075, wz), (0, s, 0)), [(dx, dz) for dx in (-0.05, 0.05) for dz in (-0.05, 0.05)], 0.007)
        p.add("yellow", box((xl, 2 * by, 0.02), (xc, 0, bz1 - 0.01), tile=0.3))
        mk.wheel(p, at((xw, 0, wz), (0, 1, 0)), c.wheel_r, 0.06, "steel")
        p.add("steel", rod((xw, -0.09, wz), (xw, 0.09, wz), 0.025, 12))
        for xr in (xw - 0.13, xw + 0.13):
            for s in (-1, 1):
                p.add("plastic", cyl(0.035, 0.016, 0.046, 14), Matrix.Translation((xr, s * 0.06, 0)))
                p.add("steel", rod((xr, s * 0.06, 0.046), (xr, s * 0.06, 0.09), 0.008, 8))
            p.add("black", box((0.06, 0.16, 0.03), (xr, 0, 0.1), tile=0.2))
        if drive:
            p.add("black", box((0.14, 0.06, 0.14), (xw, -0.1, wz), tile=0.2, bevel=0.006))
            mk.motor(p, at((xw, -0.13, wz), (0, 1, 0)), 0.12, 0.16, "orange", 0.06)
        else:
            p.add("black", cyl(0.05, 0.0, 0.05, 16), at((xw, -0.09, wz), (0, -1, 0)))
            p.add("plastic", tube([(xw, -0.14, wz), (xw, -0.16, wz + 0.1), (xw - 0.1, -0.16, bz1 + 0.05)], 0.006, 6))
        for s in (-1, 1):
            mk.bumper(p, at((xb, s * 0.1, 0.38), (d, 0, 0)), 0.045, 0.12)
        p.add("hazard", box((0.004, 2 * by, bz1 - bz0), (xb + d * 0.002, 0, bzc), tile=0.4))
    # Masts on gusseted feet, guide fins and belts on their inner faces, spliced halfway.
    mw, md = c.mast
    top = c.beam[0]
    for s in (-1, 1):
        x = s * c.mast_x
        p.add("yellow", bar((x, 0, bz1 + 0.02), (x, 0, top), mk.rect(md, mw), ALONG, 1.0))
        p.add("black", box((mw + 0.1, md + 0.1, 0.02), (x, 0, bz1 + 0.01), tile=0.3))
        for sy in (-1, 1):
            for dx in (-0.05, 0.05):
                p.add("yellow", _gusset(sy * 0.05, 0.16), Matrix.Translation((x + dx, sy * md / 2, bz1 + 0.02)))
        mk.bolts(p, at((x, 0, bz1 + 0.02)), [(dx, dy) for dx in (-0.1, 0.1) for dy in (-0.15, 0.15)], 0.012)
        xf = s * g["fin_x"]
        for fy in (-0.09, 0.09):
            p.add("steel", box((0.04, 0.02, top - bz1 - 0.1), (xf - s * 0.02, fy, (top + bz1) / 2), tile=0.5))
        p.add("plastic", box((0.006, 0.07, top - 0.06 - (bz1 - 0.06)), (xf - s * 0.003, 0, (top - 0.06 + bz1 - 0.06) / 2), tile=0.3))
        for z, r in ((top - 0.06, 0.06), (bz1 + 0.0, 0.06)):
            p.add("black", cyl(r, -0.04, 0.04, 20), at((xf - s * r, 0, z), (0, 1, 0)))
            p.add("steel", rod((xf - s * r, -0.05, z), (xf - s * r, 0.05, z), 0.012, 8))
        zs = (bz1 + top) / 2
        for sy in (-1, 1):
            p.add("black", box((mw - 0.02, 0.01, 0.3), (x, sy * (md / 2 + 0.005), zs), tile=0.2))
            mk.bolts(p, at((x, sy * (md / 2 + 0.01), zs), (0, sy, 0)), [(dx, dz) for dx in (-0.04, 0.04) for dz in (-0.1, 0.0, 0.1)], 0.008)
    # Top beam, its guide rollers on the rail's web, the beacon.
    b0, b1 = c.beam
    p.add("yellow", bar((-c.mast_x - mw / 2, 0, b0), (c.mast_x + mw / 2, 0, b0), mk.rect(md, b1 - b0, 0.0), UP, 0.6))
    for xr in (-0.55, 0.55):
        p.add("black", box((0.22, 0.2, 0.02), (xr, 0, b1 + 0.01), tile=0.2))
        for dx in (-0.06, 0.06):
            for s in (-1, 1):
                yr = s * (0.015 + 0.03)
                p.add("steel", rod((xr + dx, yr, b1 + 0.02), (xr + dx, yr, g["roll_z"] + 0.04), 0.008, 8))
                p.add("plastic", cyl(0.03, g["roll_z"] - 0.03, g["roll_z"] + 0.03, 16), Matrix.Translation((xr + dx, yr, 0)))
                p.add("black", box((0.04, 0.03, g["roll_z"] - 0.03 - b1 - 0.02), (xr + dx, s * 0.085, (g["roll_z"] - 0.03 + b1 + 0.02) / 2), tile=0.2))
    beacon = Matrix.Translation((0.3, 0.1, b1))
    p.add("black", cyl(0.035, 0.0, 0.02, 16), beacon)
    p.add("amber", cyl(0.03, 0.02, 0.09, 16), beacon)
    p.add("black", cyl(0.033, 0.09, 0.105, 16), beacon)
    # Hoist drive beyond the +X mast: motor and gearbox over the girder, its shaft down to the belt pulleys inside.
    hx = c.mast_x + mw / 2 + 0.09
    p.add("black", box((0.16, 0.22, 0.18), (hx + 0.02, 0, bz1 + 0.09), tile=0.2, bevel=0.008))
    mk.motor(p, at((hx + 0.02, -0.11, bz1 + 0.09), (0, 1, 0)), 0.13, 0.12, "orange")
    p.add("black", cyl(0.045, 0.0, 0.03, 16), at((hx + 0.02, 0.11, bz1 + 0.09), (0, 1, 0)))
    # The control cabinet at the -X mast's foot: doors, screen, e-stop, switches, vents, glands.
    cx0, cx1 = -bx - 0.11, -c.mast_x - mw / 2 - 0.02
    cxm, cw, ch = (cx0 + cx1) / 2, 0.5, 1.3
    cz = bz1 + ch / 2
    p.add("paint", box((cx1 - cx0, cw, ch), (cxm, 0, cz), tile=0.4, bevel=0.01))
    p.add("paint", box((0.012, cw - 0.04, ch - 0.06), (cx0 - 0.006, 0, cz), tile=0.4))
    p.add("plastic", box((0.004, cw - 0.02, ch - 0.04), (cx0 - 0.001, 0, cz), tile=0.4))
    p.add("black", box((0.02, 0.24, 0.18), (cx0 - 0.02, 0.05, cz + 0.32), tile=0.2))
    p.add("screen", box((0.004, 0.2, 0.14), (cx0 - 0.031, 0.05, cz + 0.32), tile=0.2))
    p.add("yellow", box((0.04, 0.1, 0.1), (cx0 - 0.03, -0.15, cz + 0.32), tile=0.2, bevel=0.005))
    p.add("red", turned([(0.0, 0.0), (0.03, 0.0), (0.03, 0.02), (0.026, 0.035), (0.0, 0.035)], 16), at((cx0 - 0.05, -0.15, cz + 0.32), (-1, 0, 0)))
    for k, key in enumerate(("led", "amber", "screen")):
        p.add(key, cyl(0.012, 0.0, 0.01, 12), at((cx0 - 0.012, 0.0 + 0.05 * (k - 1) + 0.05, cz + 0.14), (-1, 0, 0)))
    p.add("red", box((0.03, 0.05, 0.12), (cx0 - 0.025, 0.17, cz - 0.05), tile=0.2))
    p.add("steel", box((0.03, 0.03, 0.16), (cx0 - 0.02, -0.2, cz - 0.1), tile=0.2))
    for k in range(5):
        for s in (-1, 1):
            p.add("plastic", box((0.12, 0.004, 0.012), (cxm, s * (cw / 2 + 0.002), cz - 0.4 + 0.03 * k), tile=0.2))
    p.add("hazard", box((0.003, 0.12, 0.1), (cx0 - 0.013, -0.15, cz + 0.52), tile=0.2))
    for y in (-0.15, -0.05, 0.05, 0.15):
        p.add("black", cyl(0.016, -0.02, 0.0, 10), Matrix.Translation((cxm, y, bz1)))
    p.add("plastic", tube([(cxm, 0.15, bz1 - 0.02), (cxm, 0.15, bz0 + 0.03), (0.2, 0.15, bz0 + 0.03)], 0.012, 8))
    # The data link beside the cabinet, facing the stop at the parking end.
    p.add("black", box((0.1, 0.1, 0.12), (-bx - 0.06, 0.3, 0.6), tile=0.2, bevel=0.006))
    p.add("lens", box((0.004, 0.06, 0.06), (-bx - 0.112, 0.3, 0.6), tile=0.2))
    p.add("steel", box((0.06, 0.16, 0.02), (-bx - 0.06, 0.22, 0.53), tile=0.2))
    # The travel laser at the +X end, aimed at the far stop's reflector; the lift laser up at the carriage.
    p.add("black", box((0.1, 0.09, 0.09), (bx + 0.06, 0, 0.2), tile=0.2, bevel=0.005))
    p.add("lens", box((0.004, 0.04, 0.04), (bx + 0.112, 0, 0.2), tile=0.2))
    lx, ly = -(g["fin_x"] - 0.1), 0.2
    p.add("black", box((0.08, 0.08, 0.1), (lx, ly, bz1 + 0.05), tile=0.2, bevel=0.005))
    p.add("lens", box((0.04, 0.04, 0.004), (lx, ly, bz1 + 0.102), tile=0.2))
    p.add("steel", box((0.06, ly - by + 0.04, 0.01), (lx, (ly + by) / 2, bz1 + 0.005), tile=0.2))
    # The conductor rail's collector on its arm, its cable to the cabinet.
    p.add("black", box((0.05, c.cond_y - 0.04 - by, 0.03), (0.2, (c.cond_y - 0.04 + by) / 2, c.cond_z), tile=0.2))
    p.add("plastic", box((0.12, 0.02, 0.03), (0.2, c.cond_y - 0.03, c.cond_z), tile=0.2, bevel=0.003))
    p.add("black", box((0.04, 0.02, c.cond_z - bz0 + 0.04), (0.2, by + 0.01, (c.cond_z + bz0) / 2), tile=0.2))
    # The energy chain's channel up the -X mast's +Y face, closed but for the slot the carriage's bracket runs in.
    chx = -c.mast_x
    z0, z1 = c.carriage_z - 0.3, c.carriage_z + c.lift + 0.1
    zm = (z0 + z1) / 2
    p.add("grey", box((0.1, 0.006, z1 - z0), (chx, md / 2 + 0.121, zm), tile=0.5))
    p.add("grey", box((0.006, 0.124, z1 - z0), (chx - 0.047, md / 2 + 0.062, zm), tile=0.5))
    p.add("grey", box((0.006, 0.095, z1 - z0), (chx + 0.047, md / 2 + 0.0475, zm), tile=0.5))
    for z in (z0, z1):
        p.add("grey", box((0.1, 0.124, 0.006), (chx, md / 2 + 0.062, z), tile=0.2))
    for k in range(int((z1 - z0) / 0.6)):
        zz = z0 + 0.3 + 0.6 * k
        p.add("black", box((0.11, 0.004, 0.03), (chx, md / 2 + 0.126, zz), tile=0.2))
    p.add("plastic", tube([(cxm, cw / 2 + 0.012, bz1 + 0.25), (chx - 0.02, cw / 2 + 0.012, bz1 + 0.25), (chx - 0.02, md / 2 + 0.06, z0 - 0.03),
                           (chx - 0.02, md / 2 + 0.06, z0 + 0.01)], 0.012, 8))
    # The maintenance ladder up the +X mast's outer face.
    lx0 = c.mast_x + mw / 2
    for sy in (-0.2, 0.2):
        p.add("steel", box((0.05, 0.012, top - 0.9), (lx0 + 0.06, sy, (top + 0.9) / 2), tile=0.4))
    for k in range(int((top - 1.0) / 0.3) + 1):
        z = 1.0 + k * 0.3
        p.add("steel", rod((lx0 + 0.07, -0.2, z), (lx0 + 0.07, 0.2, z), 0.011, 8))
    for z in (1.2, top - 0.3):
        for sy in (-0.2, 0.2):
            p.add("steel", box((0.06, 0.02, 0.02), (lx0 + 0.03, sy, z), tile=0.2))
    return p


def carriage_part(c, g):
    """The carriage: its cross beam between the masts, side plates with
    rollers on both faces of each guide fin above and below, the belt
    clamps, the slewing drive and its motor, the lift laser's reflector and
    the drag chain's last links into the channel."""
    z = c.carriage_z
    p = Part("carriage", (0, 0, z))
    xi = g["xi"]
    p.add("grey", bar((-xi + 0.02, 0, z - 0.06), (xi - 0.02, 0, z - 0.06), mk.rect(0.2, 0.12, 0.0), UP, 0.4))
    for s in (-1, 1):
        xs = s * (xi - 0.02)
        p.add("grey", box((0.04, 0.3, 0.42), (xs, 0, z - 0.09), tile=0.3, bevel=0.004))
        mk.bolts(p, at((xs - s * 0.02, 0.0, z), (-s, 0, 0)), [(dy, dz) for dy in (-0.07, 0.07) for dz in (-0.04, 0.04)], 0.008)
        for zr in (z + 0.08, z - 0.26):
            for fy in (-0.09, 0.09):
                for side in (-1, 1):
                    yr = fy + side * 0.035
                    p.add("steel", rod((s * (xi), yr, zr), (s * (g["fin_x"] - 0.035), yr, zr), 0.006, 8))
                    p.add("plastic", cyl(0.025, -0.0125, 0.0125, 14), at((s * (g["fin_x"] - 0.02), yr, zr), (1, 0, 0)))
        p.add("black", box((g["fin_x"] - 0.006 - xi, 0.08, 0.1), (s * (xi + g["fin_x"] - 0.006) / 2, 0, z), tile=0.2))
        mk.bolts(p, at((s * (g["fin_x"] - 0.04), 0.0, z + 0.05), UP), [(0.0, -0.025), (0.0, 0.025)], 0.006)
    # Slewing drive under the beam: housing, worm and its motor; the turret's ring turns below it.
    p.add("orange", cyl(0.15, -0.14, -0.06, 32), Matrix.Translation((0, 0, z)))
    p.add("black", cyl(0.155, -0.145, -0.14, 32), Matrix.Translation((0, 0, z)))
    p.add("orange", box((0.12, 0.1, 0.08), (0.17, 0.0, z - 0.1), tile=0.2, bevel=0.006))
    mk.motor(p, at((0.23, 0.0, z - 0.1), (-1, 0, 0)), 0.06, 0.08, "orange")
    mk.bolts(p, at((0, 0, z - 0.06)), [(0.12 * math.cos(a), 0.12 * math.sin(a)) for a in [math.pi * k / 4 for k in range(8)]], 0.007)
    # The lift laser's reflector, and the drag chain's moving end into its channel's slot.
    lx, ly = -(g["fin_x"] - 0.1), 0.2
    p.add("steel", box((0.04, ly - 0.15 + 0.04, 0.01), (-(xi - 0.02), (ly + 0.15) / 2, z - 0.3), tile=0.2))
    p.add("reflector", box((0.1, 0.1, 0.006), (lx, ly, z - 0.305), tile=0.2))
    chx, slot = -c.mast_x, c.mast[1] / 2 + 0.108
    xp = -(xi - 0.02)
    p.add("black", box((0.06, 0.03, 0.12), (xp, slot - 0.01, z - 0.02), tile=0.2))
    p.add("steel", box((xp - chx + 0.02, 0.006, 0.1), ((xp + chx) / 2, slot, z - 0.02), tile=0.2))
    mk.bolts(p, at((xp - 0.04, slot + 0.003, z - 0.02), (0, 1, 0)), [(0.0, -0.03), (0.0, 0.03)], 0.006)
    p.add("plastic", tube([(xp, slot - 0.02, z - 0.06), (xp + 0.1, 0.13, z - 0.08), (0.12, 0.12, z - 0.08), (0.12, 0.06, z - 0.08)], 0.009, 8))
    return p


def turret_part(c, g):
    """The turret: the slewing ring's flange, the hangers holding the arm's
    housing under it, the housing's guide strips and end stops, and the
    reach drive's motor at its back."""
    zt = g["turret"]
    p = Part("turret", (0, 0, zt))
    p.add("black", cyl(0.17, -0.02, 0.0, 32), Matrix.Translation((0, 0, zt)))
    mk.bolts(p, at((0, 0, zt - 0.02), (0, 0, -1)), [(0.15 * math.cos(a), 0.15 * math.sin(a)) for a in [math.pi * k / 6 for k in range(12)]], 0.006)
    h0, h1 = zt - 0.1, zt - 0.03
    for y in (-0.06, 0.06):
        p.add("grey", box((0.15, 0.04, 0.012), (0, y, zt - 0.025), tile=0.2))
    p.add("alu", bar((0, -0.12, h0), (0, 0.5, h0), mk.rect(0.16, h1 - h0, 0.0), UP, 0.3))
    for s in (-1, 1):
        p.add("black", box((0.012, 0.6, 0.012), (s * 0.074, 0.19, h0 - 0.006), tile=0.3))
        mk.bolts(p, at((s * 0.08, 0.19, (h0 + h1) / 2), (s, 0, 0)), [(dy, 0.0) for dy in (-0.25, -0.1, 0.05, 0.2)], 0.005)
    p.add("plastic", box((0.16, 0.006, h1 - h0), (0, 0.503, (h0 + h1) / 2), tile=0.2))
    p.add("black", box((0.08, 0.05, 0.06), (0, -0.145, (h0 + h1) / 2), tile=0.2, bevel=0.004))
    mk.motor(p, at((0, -0.17, (h0 + h1) / 2), (0, 1, 0)), 0.05, 0.05, "orange")
    mk.switch(p, at((0.085, 0.42, (h0 + h1) / 2), (1, 0, 0)), 0.8)
    p.add("plastic", tube([(0.0, 0.0, zt - 0.025), (0.06, -0.1, h1 + 0.01), (0.06, 0.4, h1 + 0.01)], 0.007, 6))
    return p


def stage_part(c, g):
    """The middle stage: an aluminium slide with its cam followers riding
    the housing above, the rack the reach drive's pinion turns, its chain
    sprockets at both ends and the guide strips the top stage runs in."""
    sx, sy, sz = g["stage"]
    p = Part("stage", (sx, sy, sz))
    y0, y1 = sy - 0.285, sy + 0.285
    p.add("alu", bar((0, y0, sz - 0.0225), (0, y1, sz - 0.0225), mk.rect(0.13, 0.045, 0.0), UP, 0.3))
    for y in (y0 + 0.04, y1 - 0.04):
        for s in (-1, 1):
            p.add("steel", cyl(0.012, 0.0, 0.01, 12), Matrix.Translation((s * 0.05, y, sz + 0.0225)))
        p.add("black", mk.gear(0.016, 0.02, -0.008, 0.008, 14), at((0.07, y, sz), (1, 0, 0)))
    p.add("steel", mk.rack(y1 - y0 - 0.12, 0.02, 0.01, 0.008), at((-0.065, y0 + 0.06, sz), (-1, 0, 0), (0, 1, 0)))
    for s in (-1, 1):
        p.add("black", box((0.01, y1 - y0, 0.01), (s * 0.045, sy, sz - 0.0275), tile=0.3))
    p.add("plastic", box((0.13, 0.006, 0.045), (0, y1 + 0.003, sz), tile=0.2))
    return p


def arm_part(c, g):
    """The top stage and the tending head at its tip: the head's cover,
    the recognition camera in its ring light, spray nozzles, a gripper with
    cutting jaws, a time-of-flight range sensor, the light sensor's dome,
    the climate probe in its radiation shield and the status lights."""
    hx, hy, hz = g["head"]
    p = Part("arm", (hx, hy, hz))
    sy = g["stage"][1]
    y0, y1 = sy - 0.285, sy + 0.285
    zs = c.carriage_z - 0.325
    p.add("alu", bar((0, y0, zs - 0.02), (0, y1, zs - 0.02), mk.rect(0.1, 0.04, 0.0), UP, 0.3))
    p.add("plastic", box((0.1, 0.006, 0.04), (0, y0 - 0.003, zs), tile=0.2))
    for y in (y0 + 0.04, y1 - 0.12):
        for s in (-1, 1):
            p.add("steel", cyl(0.01, 0.0, 0.008, 12), Matrix.Translation((s * 0.038, y, zs + 0.02)))
    p.add("plastic", tube([(0.0, y0 + 0.02, zs + 0.025), (0.0, y1 - 0.1, zs + 0.025), (0.03, hy, hz + 0.045)], 0.006, 6))
    # The head: a dark frame round a white cover over the camera, its lit lens ring underneath, the tools round it.
    p.add("grey", box((0.14, 0.15, 0.03), (hx, hy, hz - 0.0225), tile=0.2, bevel=0.004))
    mk.cover(p, Matrix.Translation((hx, hy, hz + 0.015)), (0.132, 0.142, 0.045), "paint", bevel=0.008)
    p.add("black", box((0.12, 0.13, 0.008), (hx, hy, hz + 0.0415), tile=0.2))
    mk.bolts(p, at((hx, hy - 0.0715, hz - 0.0225), (0, -1, 0), (1, 0, 0)), [(dx, 0.0) for dx in (-0.05, 0.05)], 0.004)
    bottom = hz - 0.0375
    mk.lens_ring(p, Matrix.Translation((hx, hy, bottom)), 0.011, 0.032)
    for sx in (-1, 1):
        mk.nozzle(p, at((hx + sx * 0.052, hy + 0.058, bottom), (-sx * 0.2, -0.3, 1.0)), 2.2)
        p.add("plastic", tube([(hx + sx * 0.052, hy + 0.058, bottom + 0.002), (hx + sx * 0.06, hy + 0.06, hz), (hx + sx * 0.071, hy + 0.04, hz + 0.01)], 0.003, 5))
    mk.gripper(p, at((hx, hy + 0.06, bottom + 0.01), (0, 0.5, -1.0)), 1.6)
    p.add("orange", box((0.05, 0.02, 0.012), (hx, hy + 0.077, bottom + 0.012), tile=0.2))
    p.add("black", box((0.03, 0.02, 0.012), (hx - 0.045, hy - 0.05, bottom - 0.006), tile=0.2))
    p.add("lens", cyl(0.006, -0.002, 0.0, 10), Matrix.Translation((hx - 0.045, hy - 0.05, bottom - 0.012)))
    dome = Matrix.Translation((hx - 0.06, hy + 0.04, hz + 0.0375))
    p.add("black", cyl(0.009, 0.0, 0.003, 14), dome)
    p.add("paint", turned([(0.0, 0.003), (0.008, 0.003), (0.006, 0.008), (0.0, 0.01)], 14), dome)
    px = hx + 0.07 + 0.012
    p.add("black", box((0.016, 0.012, 0.008), (px - 0.006, hy - 0.02, hz + 0.02), tile=0.2))
    p.add("paint", mk.louvred(0.01, hz - 0.025, hz + 0.016), Matrix.Translation((px, hy - 0.02, 0)))
    for sx in (-1, 1):
        p.add("led", box((0.003, 0.11, 0.008), (hx + sx * 0.0715, hy, hz + 0.025), tile=0.2))
    return p


def crane_looks(c):
    g = frame(c)
    return [rail_part(c, g), crane_part(c, g), carriage_part(c, g), turret_part(c, g), stage_part(c, g), arm_part(c, g)]


# Machine


def open_machine(u, kind, controllers, base, note, name):
    """The machine's header: its prim, the joint position controllers (a
    controller naming several joints moves them together) and its looks."""
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "scripts/mars_base/crane.py: {note}"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/crane.py and build again.")
    u.emit(0, "# Each joint takes a position 0..1 of its range: `gearbox machine set-value <joint> position 0.5 --machine <id>`.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'PhysicsArticulationRootAPI', *(f'GearboxControllerAPI:{n}' for n in controllers)])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, f'token gearbox:machine:kind = "{kind}"')
    u.emit(1, f"rel gearbox:machine:body = </robot/{base}>")
    for name_, targets in controllers.items():
        u.emit(1, f'token gearbox:controller:{name_}:type = "builtin:joint_position"')
        rels = ", ".join(f"</robot/Joints/{t}>" for t in targets)
        u.emit(1, f"rel gearbox:controller:{name_}:target = {rels if len(targets) == 1 else f'[{rels}]'}")
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{name}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")


def box_inertia(mass, size):
    x, y, z = size
    return (mass * (y * y + z * z) / 12, mass * (x * x + z * z) / 12, mass * (x * x + y * y) / 12)


def machine(c, note, name, bound):
    """The stacker crane machine: rail (held to the world), crane, carriage, turret, stage, arm."""
    g = frame(c)
    u = Usda("/robot/Looks")
    open_machine(u, "stacker_crane", {"travel": ["travel"], "lift": ["lift"], "turn": ["turn"], "reach": ["reach", "reach_top"]}, "rail", note, name)
    ra = anchor(c)
    body(u, 1, "rail", ra, 400, (5, 20000, 20000), link="base_link")
    visual(u, 2, name, "rail", bound)
    end(u, 1)
    bx, by, bz0, bz1 = c.base
    cz = 0.38
    body(u, 1, "crane", (0, 0, cz), 950, box_inertia(950, (2.3, 0.4, 5.3)), values=[f"float gearbox:value:range = {c.travel:g}"])
    hull(u, 2, "girder", (0, 0, (bz0 + bz1) / 2 - cz), (2 * bx, 2 * by, bz1 - bz0))
    for s in (-1, 1):
        hull(u, 2, f"mast_{'pn'[s > 0]}", (s * c.mast_x, 0, (bz1 + c.beam[0]) / 2 - cz), (c.mast[0], c.mast[1], c.beam[0] - bz1))
    hull(u, 2, "top_beam", (0, 0, (c.beam[0] + c.beam[1]) / 2 - cz), (2 * c.mast_x + c.mast[0], c.mast[1], c.beam[1] - c.beam[0]))
    hull(u, 2, "cabinet", (-bx - 0.11 + (0.11 + bx - c.mast_x - c.mast[0] / 2 - 0.02) / 2, 0, bz1 + 0.65 - cz), (bx + 0.11 - c.mast_x - c.mast[0] / 2 - 0.02, 0.5, 1.3))
    visual(u, 2, name, "crane", bound)
    end(u, 1)
    z = c.carriage_z
    xi = g["xi"]
    body(u, 1, "carriage", (0, 0, z), 110, box_inertia(110, (1.3, 0.3, 0.42)), values=[f"float gearbox:value:range = {c.lift:g}"])
    hull(u, 2, "beam", (0, 0, 0), (2 * xi - 0.04, 0.2, 0.12))
    for s in (-1, 1):
        hull(u, 2, f"side_{'pn'[s > 0]}", (s * (xi - 0.02), 0, -0.09), (0.04, 0.3, 0.42))
    visual(u, 2, name, "carriage", bound)
    end(u, 1)
    zt = g["turret"]
    body(u, 1, "turret", (0, 0, zt), 30, (0.5, 0.15, 0.6), values=["float gearbox:value:range = 3.14159"])
    drum(u, 2, "ring", (0, 0, -0.01), 0.17, 0.02)
    hull(u, 2, "housing", (0, 0.19, -0.065), (0.16, 0.62, 0.07))
    visual(u, 2, name, "turret", bound)
    end(u, 1)
    sx, sy, sz = g["stage"]
    half = c.reach / 2
    body(u, 1, "stage", (sx, sy, sz), 8, box_inertia(8, (0.13, 0.57, 0.045)), values=[f"float gearbox:value:range = {half:g}"])
    hull(u, 2, "slide", (0, 0, 0), (0.13, 0.57, 0.045))
    visual(u, 2, name, "stage", bound)
    end(u, 1)
    hx, hy, hz = g["head"]
    zs = c.carriage_z - 0.325
    body(u, 1, "arm", (hx, hy, hz), 10, box_inertia(10, (0.14, 0.6, 0.12)), values=[f"float gearbox:value:range = {half:g}"])
    hull(u, 2, "top_stage", (0, sy - hy, zs - hz), (0.1, 0.57, 0.04))
    hull(u, 2, "head", (0, 0, 0), (0.14, 0.15, 0.075))
    visual(u, 2, name, "arm", bound)
    head_sensors(u, 2, (0, 0, 0))
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "rail", ra)
    joint(u, 2, "Prismatic", "travel", "rail", "crane", (-ra[0], -ra[1], cz - ra[2]), "X", (0, c.travel), (6000, 1.5))
    joint(u, 2, "Prismatic", "lift", "crane", "carriage", (0, 0, z - cz), "Z", (0, c.lift), (4000, 0.8))
    joint(u, 2, "Revolute", "turn", "carriage", "turret", (0, 0, zt - z), "Z", (-5, 185), (200, 1.0))
    joint(u, 2, "Prismatic", "reach", "turret", "stage", (sx, sy, sz - zt), "Y", (0, half), (600, 0.2))
    joint(u, 2, "Prismatic", "reach_top", "stage", "arm", (hx - sx, hy - sy, hz - sz), "Y", (0, half), (400, 0.2))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def build(work, c=CRANE, name="vertical_crane", note="the stacker crane of the vertical farm"):
    """Builds the crane's looks, writes them as `<name>_geom.usdc` and the
    machine as `<name>.usda` into `work`; returns {name: machine layer path}."""
    kept = (set(bpy.data.materials), set(bpy.data.images), set(bpy.data.meshes))
    mats = materials(kit.Looks(os.path.join(work, "maps")))
    scene = bpy.data.collections.new(f"{name}_geom")
    bpy.context.scene.collection.children.link(scene)
    objs = [part.finish(mats, scene) for part in crane_looks(c)]
    geom = os.path.join(work, f"{name}_geom.usdc")
    export_geom(scene, geom)
    bound = bindings(geom)
    for obj in objs:
        bpy.data.objects.remove(obj)
    bpy.data.collections.remove(scene)
    for mesh in set(bpy.data.meshes) - kept[2]:
        bpy.data.meshes.remove(mesh)
    for mat in set(bpy.data.materials) - kept[0]:
        bpy.data.materials.remove(mat)
    for image in set(bpy.data.images) - kept[1]:
        bpy.data.images.remove(image)
    path = os.path.join(work, f"{name}.usda")
    with open(path, "w") as f:
        f.write(machine(c, note, name, bound))
    return {name: path}
