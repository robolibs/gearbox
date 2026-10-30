"""MARSHA's printer, after AI SpaceFactory's Mars concept for it: a
stationary print rover standing beside the shell it prints, its mast
telescoping up as the shell rises, a slewing boom holding an arm over the
shell's axis that carries the print head round the wall course by course.

    chassis    held to the world: the hex frame on six splayed legs, each
               on a steered grousered wheel with a screw jack let down to a
               foot pad beside it; on the deck two tanks, the hose reel,
               the control cabinet and the battery pack; the inlet panel
               of the feed line, air and power at its back, the work lamps
               at its front; the mast's base section on its flange
    mast       `lift`: the mast's middle stage, at half the stroke
    mast_top   `lift_top`: the top stage and the slewing bearing on it
    boom       `slew` 0..180 degrees: the jib reaching over the shell's
               axis and the counter-jib under its counterweights behind,
               the slewing drive, the turntable's drive and lamps at the
               tip, the feed line and the cable along the top chords
    pendant    `turn`: the turntable under the boom's tip, turning about
               the shell's axis, and the bracket holding the arm's shoulder
    upper_arm  `shoulder` 0..75 degrees down from level
    forearm    `elbow` 0..-140 degrees, unfolding from under the upper arm
    head       `wrist` 0..100 degrees, keeping the print head plumb: the
               extruder's drive, the pellet hopper on its feed throat, the
               heated barrel and the conical nozzle, six black corrugated
               hoses blowing cooling air onto the bead, a camera on it
    screw      `screw`: the extruder's screw and its coupling

Every joint has a motor. `lift` (one controller moving both mast stages,
each over half the stroke), `slew`, `shoulder`, `elbow` and `wrist` have a
`builtin:joint_position` controller of their name taking a position 0..1 of
their range; `turn` and `screw` a `builtin:joint_velocity` one. At rest the
mast is down, the boom over the shell and the arm folded under the
turntable, its head hanging plumb. Frame: the mast's axis on the ground at
the origin, +X toward the shell's axis `reach` away, Z up; the machine is
held to the world by a joint with an empty body at `anchor`. The feed line
crosses each joint of the arm through a swivel on the joint's axis. Looks
are built from `mechanics` parts in finishes dusted by Mars; the bodies
collide as hidden boxes.
"""
import math
import os
from dataclasses import dataclass

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import kit
import mechanics as mk
import patterns
import weathering
from industry import auger, heads, torus
from kit import TAU, box, rod, spline, srgb_to_linear, truss, tube
from mechanics import UP, Part, at, bar, cyl, turned
from usda import Usda, end, fmt, schemas, xform
from vehicles import bindings, export_geom, visual
from washers import battery, beacon, camera, camera_head, estop, gusset, hull, joint, looking

NAME = "regolith_printer"
Z = Vector((0, 0, 1))


@dataclass
class Printer:
    """The printer's sizes, metres and degrees, in its frame."""
    # From the mast's axis to the shell's.
    reach: float = 8.5
    # Each mast stage's stroke.
    stage: float = 5.6
    # The turntable's bearing, the boom's underside, over the ground with the mast down.
    turn_z: float = 8.0
    # The shoulder's pivot from the turntable's: out in the arm's plane, and down.
    shoulder: tuple = (1.2, -0.85)
    upper: float = 3.1
    fore: float = 3.0
    # From the wrist's pivot down to the nozzle's tip.
    head: float = 1.5
    # The elbow's bend at rest, the forearm folded back under the upper arm.
    fold: float = 150.0
    # Each positioning joint's travel from rest.
    slew_deg: float = 180.0
    shoulder_deg: float = 75.0
    elbow_deg: float = -140.0
    wrist_deg: float = 100.0


PRINTER = Printer()
# The couplings of the inlet panel the ground kit's lines end at.
INLETS = {"feed": (-1.45, -0.3, 1.18), "air": (-1.45, 0.0, 1.18), "power": (-1.45, 0.3, 1.18)}
HEX, BELLY, DECK = 1.5, 0.95, 1.45
LEG_R, JACK_R, WHEEL_R, WHEEL_W, AXLE = 2.75, 1.95, 0.42, 0.3, 0.42
LEGS = [math.radians(30 + 60 * k) for k in range(6)]
# The head's camera, in its frame, and where it looks: at the nozzle's tip.
EYE, GAZE = (0.18, 0.312, -1.0), (-0.18, -0.312, -0.5)
# Each joint: (name, parent, child, kind, axis, limits, motor's force and speed).
JOINTS = (
    ("lift", "chassis", "mast", "Prismatic", "Z", (0, PRINTER.stage), (60000, 0.12)),
    ("lift_top", "mast", "mast_top", "Prismatic", "Z", (0, PRINTER.stage), (40000, 0.12)),
    ("slew", "mast_top", "boom", "Revolute", "Z", (0, PRINTER.slew_deg), (60000, 0.05)),
    ("turn", "boom", "pendant", "Revolute", "Z", None, (20000, 0.2)),
    ("shoulder", "pendant", "upper_arm", "Revolute", "Y", (0, PRINTER.shoulder_deg), (60000, 0.15)),
    ("elbow", "upper_arm", "forearm", "Revolute", "Y", (PRINTER.elbow_deg, 0), (40000, 0.15)),
    ("wrist", "forearm", "head", "Revolute", "Y", (0, PRINTER.wrist_deg), (15000, 0.2)),
    ("screw", "head", "screw", "Revolute", "Z", None, (400, 6.0)),
)

# Key: (material name, sRGB colour, roughness, metal, map settings).
FINISHES = {
    "white": ("printer_white", (0.84, 0.83, 0.79), 0.55, 0.0, {"seed": 91, "dust": 0.4}),
    "grey": ("printer_grey", (0.45, 0.46, 0.47), 0.55, 0.2, {"seed": 92, "dust": 0.35}),
    "dark": ("printer_dark", (0.14, 0.14, 0.15), 0.5, 0.5, {"seed": 93, "dust": 0.35}),
    "black": ("printer_black", (0.07, 0.07, 0.075), 0.45, 0.5, {"seed": 94, "dust": 0.3}),
    "plastic": ("printer_plastic", (0.045, 0.045, 0.048), 0.7, 0.0, {"seed": 95, "dust": 0.3}),
    "steel": ("printer_steel", (0.66, 0.67, 0.68), 0.3, 1.0, {"seed": 96, "dust": 0.25, "brush": 0.06}),
    "alu": ("printer_alu", (0.76, 0.77, 0.78), 0.36, 0.9, {"seed": 97, "dust": 0.3, "brush": 0.05}),
    "barrel": ("printer_barrel", (0.82, 0.82, 0.83), 0.2, 1.0, {"seed": 98, "dust": 0.15, "brush": 0.1}),
    "brass": ("printer_nozzle", (0.80, 0.60, 0.32), 0.3, 1.0, {"seed": 99, "dust": 0.15}),
    "heater": ("printer_heater", (0.52, 0.47, 0.42), 0.5, 0.7, {"seed": 100, "dust": 0.2}),
    "orange": ("printer_orange", (0.86, 0.38, 0.07), 0.5, 0.0, {"seed": 101, "dust": 0.3}),
    "yellow": ("printer_yellow", (0.92, 0.68, 0.08), 0.5, 0.0, {"seed": 102, "dust": 0.3}),
    "red": ("printer_red", (0.72, 0.06, 0.04), 0.4, 0.0, {"seed": 103, "dust": 0.2}),
    "hose": ("printer_hose", (0.04, 0.04, 0.042), 0.6, 0.0, {"seed": 104, "dust": 0.35}),
    "feed": ("printer_feed", (0.13, 0.32, 0.66), 0.45, 0.0, {"seed": 105, "dust": 0.25}),
    "tank": ("printer_tank", (0.24, 0.25, 0.26), 0.45, 0.3, {"seed": 106, "dust": 0.3}),
    "wheel": ("printer_wheel", (0.36, 0.36, 0.37), 0.5, 0.6, {"seed": 107, "dust": 0.7}),
    "cast": ("printer_counterweight", (0.52, 0.45, 0.39), 0.9, 0.0, {"seed": 108, "dust": 0.2}),
}


def dusty(colour, rough, seed, dust=0.3, brush=0.0, n=512):
    """Maps of a finish that has stood out on Mars: a film of dust over it,
    thicker in soft patches, specks and streaks run down it, dulling what it
    covers, over a fine grain and, on bare metal, brushing lines."""
    def make():
        c = srgb_to_linear(np.array(colour, dtype=np.float32))
        patch = patterns.noise(n, cells=3, seed=seed, octaves=3)
        fine = patterns.noise(n, cells=128, seed=seed + 1, octaves=2)
        speck = patterns.noise(n, cells=64, seed=seed + 4, octaves=2)
        runs = np.repeat(patterns.noise(n, cells=40, seed=seed + 2, octaves=2)[:1], n, axis=0)
        fade = patterns.noise(n, cells=4, seed=seed + 3, octaves=2)
        lines = np.repeat(np.random.default_rng(seed).random(n).astype(np.float32)[None, :], n, axis=0)
        cover = np.clip(dust * (0.45 + 0.35 * patch + 0.25 * speck) + dust * 0.6 * np.clip(runs - 0.5, 0.0, 1.0) * fade, 0.0, 0.85)
        base = c * (0.96 + 0.08 * fine + brush * (lines - 0.5))[..., None]
        diff = base * (1.0 - cover[..., None]) + np.array(weathering.DUST, dtype=np.float32) * (0.9 + 0.2 * fine[..., None]) * cover[..., None]
        r = np.clip(rough + (0.93 - rough) * cover + 0.04 * (fine - 0.5) + 0.5 * brush * np.abs(lines - 0.5), 0.03, 1.0)
        height = (0.25 * fine + 0.6 * cover + 2.0 * brush * (lines - 0.5)).astype(np.float32)
        return np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height(height, 0.6), r.astype(np.float32)
    return make


def tread(n=512, cells=12):
    """Diamond tread plate: raised lozenges in pairs turned across each other, dust in the gaps."""
    def make():
        u, v = patterns.grid(n)
        fu, fv = u * cells % 1.0 - 0.5, v * cells % 1.0 - 0.5
        flip = ((np.floor(u * cells) + np.floor(v * cells)) % 2) == 0
        along = np.where(flip, fu + fv, fu - fv) / 1.414
        across = np.where(flip, fu - fv, fu + fv) / 1.414
        boss = np.clip(1.0 - (across / 0.07) ** 2, 0.0, 1.0) * np.clip(1.0 - (along / 0.3) ** 4, 0.0, 1.0)
        dirt = patterns.noise(n, cells=4, seed=111, octaves=3)
        cover = np.clip(0.5 * dirt - 0.1 + 0.3 * (1.0 - boss), 0.0, 0.8)
        c = srgb_to_linear(np.array((0.55, 0.56, 0.57), dtype=np.float32))
        diff = c * (0.8 + 0.25 * boss)[..., None] * (1.0 - cover[..., None]) + np.array(weathering.DUST, dtype=np.float32) * cover[..., None]
        rough = np.clip(0.35 + 0.5 * cover, 0.0, 1.0)
        return diff.astype(np.float32), patterns.normals_from_height(boss.astype(np.float32), 2.0), rough.astype(np.float32)
    return make


def materials(looks):
    """The printer's finishes, each dusted, and its tread plate, hazard
    stripes, lens, sight glass and lamps."""
    m = {key: looks.get(name, colour, rough, metal, maps=dusty(colour, rough, **maps)) for key, (name, colour, rough, metal, maps) in FINISHES.items()}
    m["tread"] = looks.get("printer_tread", (0.55, 0.56, 0.57), 0.45, 0.9, maps=tread())
    m["hazard"] = looks.get("printer_hazard", (0.92, 0.66, 0.05), 0.55, 0.0, maps=patterns.stripes)
    m["lens"] = looks.get("printer_lens", (0.02, 0.025, 0.03), 0.04, 0.0)
    m["glass"] = looks.get("printer_sight", (0.05, 0.06, 0.07), 0.08, 0.0)
    m["led"] = looks.get("printer_led", (1.0, 0.97, 0.9), 0.3, 0.0, emit=((1.0, 0.97, 0.9), 6.0))
    m["amber"] = looks.get("printer_amber", (1.0, 0.55, 0.1), 0.3, 0.0, emit=((1.0, 0.55, 0.1), 8.0))
    return m


# Kinematics


def origins(p=PRINTER):
    """Each body's origin in the machine's frame at rest, at its joint's pivot."""
    s = Vector((p.reach + p.shoulder[0], 0.0, p.turn_z + p.shoulder[1]))
    e = s + Vector((p.upper, 0.0, 0.0))
    f = math.radians(p.fold)
    w = e + Vector((p.fore * math.cos(f), 0.0, -p.fore * math.sin(f)))
    return {"chassis": Vector((0, 0, 1.2)), "mast": Vector((0, 0, 0.85)), "mast_top": Vector((0, 0, 1.0)), "boom": Vector((0, 0, p.turn_z)),
            "pendant": Vector((p.reach, 0, p.turn_z)), "upper_arm": s, "forearm": e, "head": w, "screw": w + Vector((0, 0, -0.48))}


def pose(q, p=PRINTER):
    """Each body's frame in the machine's at joint values `q` (metres along
    a mast stage, radians), `lift` moving both stages."""
    o = origins(p)
    out = {"chassis": Matrix.Translation(o["chassis"])}
    for name, parent, child, kind, axis, *_ in JOINTS:
        v = q.get("lift" if name == "lift_top" else name, 0.0)
        move = Matrix.Translation(Vector({"X": (1, 0, 0), "Y": (0, 1, 0), "Z": (0, 0, 1)}[axis]) * v) if kind == "Prismatic" else Matrix.Rotation(v, 4, axis)
        out[child] = out[parent] @ Matrix.Translation(o[child] - o[parent]) @ move
    return out


def nozzle(q, p=PRINTER):
    """The nozzle's tip in the machine's frame at joint values `q`."""
    return pose(q, p)["head"] @ Vector((0, 0, -p.head))


def anchor(p=PRINTER):
    """Where the printer's anchor holds its chassis, in its frame."""
    return tuple(origins(p)["chassis"])


# Geometry


def taper(x0, x1, w0, h0, w1, h1, tile=0.5):
    """A box along X about the X axis, `w0` wide and `h0` deep at x0, `w1` and `h1` at x1."""
    bm = bmesh.new()
    lo, hi = [[bm.verts.new((x, sy * w / 2, sz * h / 2)) for sy, sz in ((-1, -1), (1, -1), (1, 1), (-1, 1))] for x, w, h in ((x0, w0, h0), (x1, w1, h1))]
    for i in range(4):
        bm.faces.new((lo[i], lo[(i + 1) % 4], hi[(i + 1) % 4], hi[i]))
    bm.faces.new(lo[::-1])
    bm.faces.new(hi)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    kit.project_uv(bm, tile)
    return bm


def plate(points, y0, y1, tile=0.5):
    """A plate in the XZ plane over the (x, z) outline `points`, from y0 to y1."""
    bm = kit.prism(points, y0, y1, tile)
    bm.transform(Matrix(((1, 0, 0, 0), (0, 0, 1, 0), (0, 1, 0, 0), (0, 0, 0, 1))))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    kit.project_uv(bm, tile)
    return bm


def nose(x0, x1, half, r, steps=8):
    """The (x, z) outline of a fork's cheek from x0, `half` deep, out to a
    round end of radius r about (x1, 0)."""
    arc = [(x1 + r * math.sin(math.pi * k / steps), -r * math.cos(math.pi * k / steps)) for k in range(steps + 1)]
    return [(x0, -half)] + arc + [(x0, half)]


def corrugated(points, r, pitch=0.024, segments=10):
    """A corrugated hose along a smooth curve through `points`, rings `pitch` apart with a groove between each."""
    path = spline(points, pitch / 2)
    bm = bmesh.new()
    rings = []
    for k, (c, _, n, b) in enumerate(kit.frames(path)):
        rr = r if k % 2 == 0 else 0.8 * r
        rings.append([bm.verts.new(c + (n * math.cos(a) + b * math.sin(a)) * rr) for a in (TAU * i / segments for i in range(segments))])
    for lo, hi in zip(rings, rings[1:]):
        for i in range(segments):
            j = (i + 1) % segments
            bm.faces.new((lo[i], lo[j], hi[j], hi[i]))
    bm.faces.new(rings[0][::-1])
    bm.faces.new(rings[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    kit.project_uv(bm, 0.2)
    return bm


def line(points, r, step=0.05, segments=10):
    """A hose or cable along a smooth curve through `points`."""
    return tube(spline(points, step), r, segments)


def ring_of(r, z0, z1, inner, segments=32):
    """A ring about Z from `inner` out to r, z0 to z1."""
    return turned([(inner, z0), (r, z0), (r, z1), (inner, z1), (inner, z0)], segments)


def wheel(part, m):
    """A grousered wheel on an axle along the builder's Z, the grousers'
    tips WHEEL_R out: a hard-anodised rim with ski edges, two rings of
    blade spokes, the hub, its motor cap and bolts, eighteen grousers."""
    h, rim = WHEEL_W / 2, WHEEL_R - 0.025
    part.add("wheel", turned([(rim - 0.03, -h), (rim + 0.008, -h), (rim, -h + 0.02), (rim, h - 0.02), (rim + 0.008, h), (rim - 0.03, h), (rim - 0.03, -h)], 40), m)
    length = rim - 0.15
    for k in range(6):
        for z in (-0.07, 0.07):
            part.add("wheel", box((length, 0.045, 0.016), (0.12 + length / 2, 0, z), tile=0.2), Matrix(m) @ Matrix.Rotation(TAU * (k + 0.5 * (z > 0)) / 6, 4, "Z"))
    for k in range(18):
        part.add("wheel", box((0.026, 0.035, WHEEL_W), (rim + 0.013, 0, 0), tile=0.2), Matrix(m) @ Matrix.Rotation(TAU * k / 18, 4, "Z"))
    part.add("dark", cyl(0.12, -h - 0.02, h + 0.02, 20), m)
    part.add("dark", cyl(0.1, h + 0.02, h + 0.1, 20), m)
    mk.bolts(part, Matrix(m) @ Matrix.Translation((0, 0, h + 0.1)), [(0.07 * math.cos(TAU * k / 8), 0.07 * math.sin(TAU * k / 8)) for k in range(8)], 0.009)


# Bodies


def chassis_part(p=PRINTER):
    """The chassis: the hex frame, its deck, skins and access panels; the
    six legs, each a beam and strut out to a steering head with its motor
    over a caster fork and its wheel, a screw jack let down to a foot pad
    beside it; the mast's base section on its gusseted flange, with
    stiffener rings, the collar and its guide rollers, a junction box on a
    bracket and the conduit up it; two tanks strapped on saddles, the hose
    reel on its stands, the control cabinet with its beacon, antenna and
    e-stop, the battery pack; the work lamps and the inlet panel."""
    part = Part("chassis", origins(p)["chassis"])
    corners = [(HEX * math.cos(a), HEX * math.sin(a)) for a in LEGS]
    apothem = HEX * math.cos(math.pi / 6)
    part.add("white", kit.prism(corners, BELLY + 0.02, DECK - 0.02, 0.5))
    part.add("dark", kit.prism(corners, BELLY - 0.02, BELLY + 0.02, 0.5))
    part.add("tread", kit.prism(corners, DECK - 0.02, DECK + 0.025, 0.5))
    for k, (a, c) in enumerate(zip(LEGS, corners[1:] + corners[:1])):
        start = Vector((*corners[k], 0))
        for z in (BELLY + 0.05, DECK - 0.05):
            part.add("grey", bar(start + Z * z, Vector((*c, z)), mk.rect(0.09, 0.09), UP, 0.3))
        part.add("grey", box((0.12, 0.12, DECK - BELLY), (corners[k][0], corners[k][1], (DECK + BELLY) / 2), tile=0.3))
        face = math.radians(60 * k + 60)
        if k not in (2, 5):
            m = Matrix.Rotation(face, 4, "Z") @ Matrix.Translation((apothem + 0.012, 0, 1.2))
            part.add("grey", box((0.02, 0.9, 0.32), tile=0.4, bevel=0.004), m)
            part.add("steel", box((0.03, 0.12, 0.025), (0.02, 0.32, 0.0), tile=0.1), m)
            mk.bolts(part, m @ at((0.01, 0, 0), (1, 0, 0)), [(sy * 0.42, sz * 0.13) for sy in (-1, 1) for sz in (-1, 1)], 0.008)
    for a in LEGS:
        d, t = Vector((math.cos(a), math.sin(a), 0)), Vector((-math.sin(a), math.cos(a), 0))
        hub = d * LEG_R
        part.add("white", bar(d * 1.38 + Z * 1.28, d * 2.62 + Z * 1.06, mk.rect(0.22, 0.26), UP, 0.4))
        part.add("grey", rod(d * 1.15 + Z * 0.97, d * 2.6 + Z * 0.86, 0.05, 10))
        part.add("grey", cyl(0.17, 0.82, 1.14, 20), Matrix.Translation(hub))
        part.add("dark", cyl(0.19, 1.14, 1.18, 20), Matrix.Translation(hub))
        mk.motor(part, at(hub + Z * 1.18, (0, 0, -1)), 0.14, 0.18, "grey", gear_len=0.05)
        part.add("grey", box((0.3, 0.52, 0.08), (hub.x, hub.y, 0.8), tile=0.3, bevel=0.01))
        for s in (-1, 1):
            part.add("grey", box((0.2, 0.035, 0.46), (hub.x, hub.y + s * 0.21, 0.56), tile=0.3))
            part.add("dark", cyl(0.08, 0.227, 0.26, 16), at(Vector((hub.x, hub.y, AXLE)), (0, s, 0)))
        wheel(part, at(Vector((hub.x, hub.y, AXLE)), (0, 1, 0)))
        jack = d * JACK_R
        part.add("grey", box((0.22, 0.22, 0.24), (jack.x, jack.y, 0.91), rot=Matrix.Rotation(a, 4, "Z"), tile=0.3, bevel=0.01))
        mk.motor(part, at(jack + t * 0.11 + Z * 0.91, -t), 0.1, 0.14, "dark")
        part.add("white", cyl(0.075, 0.42, 0.79, 16), Matrix.Translation(jack))
        part.add("hazard", cyl(0.078, 0.42, 0.52, 16), Matrix.Translation(jack))
        part.add("steel", cyl(0.05, 0.12, 0.44, 14), Matrix.Translation(jack))
        part.add("dark", turned([(0.0, 0.06), (0.065, 0.08), (0.065, 0.16), (0.0, 0.18)], 14), Matrix.Translation(jack))
        part.add("dark", turned([(0.0, 0.0), (0.24, 0.0), (0.24, 0.035), (0.2, 0.06), (0.0, 0.06)], 24), Matrix.Translation(jack))
    # The mast's base section.
    part.add("white", cyl(0.32, 0.8, 6.95, 32))
    part.add("grey", ring_of(0.55, DECK + 0.025, DECK + 0.08, 0.32, 40))
    for k in range(12):
        a = TAU * k / 12
        part.add("grey", gusset(0.2, 0.34), Matrix.Translation((0.435 * math.cos(a), 0.435 * math.sin(a), DECK + 0.08)) @ Matrix.Rotation(a - math.pi / 2, 4, "Z"))
    mk.bolts(part, Matrix.Translation((0, 0, DECK + 0.08)), [(0.515 * math.cos(TAU * (k + 0.5) / 24), 0.515 * math.sin(TAU * (k + 0.5) / 24)) for k in range(24)], 0.013)
    for z in (3.0, 5.0):
        part.add("grey", ring_of(0.345, z, z + 0.05, 0.31))
    part.add("grey", ring_of(0.39, 6.95, 7.15, 0.26, 40))
    part.add("black", ring_of(0.29, 7.15, 7.18, 0.252, 32))
    for k in range(8):
        a = TAU * (k + 0.5) / 8
        part.add("black", box((0.06, 0.08, 0.08), (0.345 * math.cos(a), 0.345 * math.sin(a), 7.19), rot=Matrix.Rotation(a, 4, "Z"), tile=0.2, bevel=0.006))
    mk.cover(part, Matrix.Translation((-0.6, 0.0, 3.6)), (0.3, 0.36, 0.42), key="grey", vents=3, bevel=0.012)
    for z in (3.5, 3.72):
        part.add("grey", box((0.18, 0.06, 0.04), (-0.4, 0.0, z), tile=0.2))
    part.add("dark", rod((-0.37, 0.12, DECK + 0.08), (-0.37, 0.12, 6.9), 0.03, 10))
    for z in (2.2, 3.3, 4.4, 5.5, 6.6):
        part.add("grey", box((0.08, 0.1, 0.04), (-0.35, 0.11, z), tile=0.2))
    # The tanks on the deck.
    for s in (-1, 1):
        c = Vector((0.72, s * 0.78, DECK + 0.11))
        part.add("grey", box((0.5, 0.5, 0.09), (c.x, c.y, DECK + 0.07), tile=0.3))
        part.add("tank", turned(heads(0.33, 1.95), 28), Matrix.Translation(c))
        for z in (0.5, 1.45):
            part.add("steel", ring_of(0.345, c.z + z, c.z + z + 0.05, 0.33, 28))
        part.add("steel", cyl(0.04, 1.95, 2.08, 12), Matrix.Translation(c))
        part.add("red", turned([(0.03, 2.1), (0.09, 2.1), (0.09, 2.12), (0.03, 2.12), (0.03, 2.1)], 16), Matrix.Translation(c))
        part.add("steel", cyl(0.045, 0.0, 0.015, 16), at(c + Vector((-0.255, s * 0.225, 1.2)), (-0.75, s * 0.66, 0)))
        part.add("steel", line([c + Vector((-0.3, -s * 0.12, 0.28)), Vector((0.4, s * 0.36, DECK + 0.25)), Vector((0.36, s * 0.22, DECK + 0.1))], 0.025))
    # The hose reel.
    reel = Vector((-0.88, 0.0, 2.28))
    axis = at(reel, (0, 1, 0))
    part.add("dark", cyl(0.2, -0.4, 0.4, 24), axis)
    wraps = [(0.2, -0.36)] + [(0.34 + (0.035 if i % 2 else 0.0), -0.36 + 0.72 * i / 20) for i in range(21)] + [(0.2, 0.36), (0.2, -0.36)]
    part.add("feed", turned(wraps, 32), axis)
    for s in (-1, 1):
        part.add("grey", ring_of(0.46, s * 0.385 - 0.0125, s * 0.385 + 0.0125, 0.2, 40), axis)
        for dx in (-0.34, 0.34):
            part.add("white", rod((reel.x + dx, s * 0.45, DECK + 0.025), (reel.x, s * 0.45, reel.z), 0.03, 8))
        part.add("white", rod((reel.x - 0.25, s * 0.45, DECK + 0.3), (reel.x + 0.25, s * 0.45, DECK + 0.3), 0.025, 8))
    part.add("steel", cyl(0.04, -0.5, 0.5, 12), axis)
    mk.motor(part, at(reel + Vector((0, 0.5, 0)), (0, -1, 0)), 0.14, 0.18, "grey", gear_len=0.05)
    part.add("steel", cyl(0.06, 0.5, 0.6, 16), at(reel, (0, -1, 0)))
    part.add("feed", line([(-1.25, -0.3, DECK + 0.03), (-1.15, -0.45, DECK + 0.25), (-0.95, -0.62, 1.95), (-0.88, -0.62, reel.z - 0.06)], 0.03))
    part.add("feed", line([(reel.x + 0.28, 0.1, reel.z - 0.24), (-0.48, 0.18, DECK + 0.35), (-0.36, 0.2, DECK + 0.1)], 0.03))
    part.add("plastic", line([(-1.25, 0.3, DECK + 0.03), (-0.95, 0.62, DECK + 0.05), (-0.48, 0.92, DECK + 0.2)], 0.025))
    # The control cabinet and the battery pack.
    mk.cover(part, Matrix.Translation((-0.25, 0.98, DECK + 0.475)), (0.5, 0.3, 0.9), key="white", vents=4, bevel=0.012)
    beacon(part, Matrix.Translation((-0.12, 0.98, DECK + 0.925)), 0.05)
    part.add("plastic", rod((-0.42, 1.06, DECK + 0.925), (-0.42, 1.06, DECK + 1.9), 0.007, 6))
    part.add("black", cyl(0.014, DECK + 1.9, DECK + 1.93, 8), Matrix.Translation((-0.42, 1.06, 0)))
    estop(part, at((-0.5, 0.98, DECK + 0.7), (-1, 0, 0)), 1.6)
    battery(part, Matrix.Translation((-0.25, -0.98, DECK + 0.385)), (0.5, 0.3, 0.72))
    # The work lamps and the inlet panel.
    for y in (-0.45, 0.45):
        part.add("dark", box((0.07, 0.26, 0.15), (apothem + 0.035, y, 1.2), tile=0.2, bevel=0.008))
        part.add("led", box((0.006, 0.21, 0.1), (apothem + 0.073, y, 1.2), tile=0.2))
    part.add("grey", box((0.03, 0.95, 0.36), (-apothem - 0.015, 0, 1.2), tile=0.3, bevel=0.004))
    for key, (x, y, z) in INLETS.items():
        part.add("steel" if key != "power" else "dark", cyl(0.055, 0.0, -x - apothem - 0.03, 16), at((-apothem - 0.03, y, z), (-1, 0, 0)))
        part.add("yellow", box((0.006, 0.12, 0.05), (-apothem - 0.033, y, z + 0.11), tile=0.1))
    for y in (-0.38, -0.22):
        part.add("orange", box((0.08, 0.02, 0.1), (-apothem - 0.1, y, INLETS["feed"][2] + 0.05), tile=0.1))
    return part


def mast_part(p=PRINTER):
    """The mast's middle stage: its tube with a height scale, the collar at
    its head with the guide rollers riding the top stage and a wiper seal."""
    part = Part("mast", origins(p)["mast"])
    part.add("white", cyl(0.25, 0.85, 7.35, 32))
    part.add("grey", ring_of(0.31, 7.33, 7.52, 0.2, 36))
    part.add("black", ring_of(0.235, 7.52, 7.55, 0.192, 32))
    part.add("yellow", box((0.04, 0.004, 6.0), (0, 0.25, 4.3), tile=1.0))
    for k in range(13):
        part.add("black", box((0.05, 0.006, 0.02 if k % 2 else 0.04), (0, 0.251, 1.4 + 0.5 * k), tile=0.1))
    for k in range(6):
        a = TAU * k / 6
        part.add("black", box((0.06, 0.07, 0.07), (0.255 * math.cos(a), 0.255 * math.sin(a), 7.555), rot=Matrix.Rotation(a, 4, "Z"), tile=0.2, bevel=0.006))
    return part


def mast_top_part(p=PRINTER):
    """The mast's top stage: its tube with a height scale, the flange at its
    head and on it the slewing bearing's outer race, toothed outside for
    the boom's slewing pinion, the flange bolted round."""
    part = Part("mast_top", origins(p)["mast_top"])
    part.add("white", cyl(0.19, 1.0, 7.72, 32))
    part.add("grey", turned([(0.0, 7.6), (0.19, 7.6), (0.46, 7.72), (0.46, 7.8), (0.0, 7.8)], 48))
    part.add("steel", ring_of(0.38, 7.8, 7.93, 0.25, 48))
    part.add("steel", mk.gear(0.38, 0.42, 7.8, 7.93, 64))
    mk.bolts(part, Matrix.Translation((0, 0, 7.8)), [(0.44 * math.cos(TAU * k / 32), 0.44 * math.sin(TAU * k / 32)) for k in range(32)], 0.009)
    part.add("yellow", box((0.04, 0.004, 6.2), (0, 0.19, 4.4), tile=1.0))
    for k in range(13):
        part.add("black", box((0.05, 0.006, 0.02 if k % 2 else 0.04), (0, 0.191, 1.5 + 0.5 * k), tile=0.1))
    return part


def boom_part(p=PRINTER):
    """The boom: its turret on the slewing bearing's inner race, the slewing
    drive's motor and pinion; the jib, a box truss reaching past the
    shell's axis, and the counter-jib behind under four cast regolith
    counterweights; at the tip the turntable bearing's outer race under a
    plate, the turntable drive's motor and pinion, the work lamps, the end
    frame and the beacon; the rotary union on the turret and the feed line
    and the cable from it along the top chords on clamps, down to the tip."""
    z, r = p.turn_z, p.reach
    part = Part("boom", origins(p)["boom"])
    part.add("steel", ring_of(0.36, 7.93, z, 0.2, 40))
    part.add("grey", box((0.95, 0.72, 0.26), (0, 0, z + 0.13), tile=0.5, bevel=0.02))
    for s in (-1, 1):
        part.add("white", plate([(-0.62, 0.08), (0.62, 0.08), (0.42, 0.78), (-0.42, 0.78)], s * 0.29, s * 0.32), Matrix.Translation((0, 0, z)))
    part.add("white", truss((0.6, 0, z + 0.425), (r + 0.45, 0, z + 0.425), 0.6, 0.65, 12, 0.04, 0.02))
    part.add("white", truss((-0.6, 0, z + 0.35), (-3.4, 0, z + 0.35), 0.6, 0.5, 4, 0.04, 0.02))
    for k in range(4):
        x = -3.3 + 0.235 * k
        part.add("cast", box((0.22, 0.96, 0.95), (x, 0, z - 0.02), tile=0.5, bevel=0.015))
        part.add("steel", box((0.04, 0.12, 0.05), (x, 0, z + 0.48), tile=0.1))
    # The slewing drive.
    pinion = Vector((0.0, 0.47, 0.0))
    part.add("steel", mk.gear(0.06, 0.08, 7.8, 7.93, 14), Matrix.Translation(pinion))
    part.add("steel", cyl(0.03, 7.93, z + 0.28, 10), Matrix.Translation(pinion))
    part.add("grey", box((0.24, 0.24, 0.04), (0.0, 0.45, z + 0.26), tile=0.2))
    mk.motor(part, at(pinion + Z * (z + 0.28), (0, 0, -1)), 0.18, 0.26, gear_len=0.06)
    part.add("steel", cyl(0.1, z + 0.26, z + 0.5, 16))
    part.add("dark", cyl(0.12, z + 0.5, z + 0.54, 16))
    # The tip.
    part.add("grey", box((1.15, 1.12, 0.1), (r, 0, z + 0.05), tile=0.5))
    part.add("steel", ring_of(0.62, z - 0.08, z, 0.52, 48), Matrix.Translation((r, 0, 0)))
    mk.bolts(part, at((r, 0, z - 0.08), (0, 0, -1)), [(0.57 * math.cos(TAU * k / 32), 0.57 * math.sin(TAU * k / 32)) for k in range(32)], 0.01)
    drive = Vector((r + 0.58 * math.cos(math.radians(120)), 0.58 * math.sin(math.radians(120)), 0.0))
    part.add("steel", mk.gear(0.06, 0.08, z - 0.2, z - 0.1, 14), Matrix.Translation(drive))
    part.add("steel", cyl(0.03, z - 0.1, z + 0.1, 10), Matrix.Translation(drive))
    mk.motor(part, at(drive + Z * (z + 0.1), (0, 0, -1)), 0.18, 0.26, gear_len=0.06)
    part.add("grey", box((0.05, 0.64, 0.72), (r + 0.47, 0, z + 0.425), tile=0.3))
    beacon(part, Matrix.Translation((r + 0.3, 0, z + 0.79)), 0.05)
    for y in (-0.3, 0.3):
        part.add("dark", box((0.22, 0.14, 0.08), (r - 0.75, y, z + 0.02), tile=0.2, bevel=0.006))
        part.add("led", box((0.18, 0.1, 0.006), (r - 0.75, y, z - 0.023), tile=0.2))
    # The feed line and the cable along the jib.
    for y, rr, key in ((0.16, 0.035, "feed"), (-0.16, 0.025, "plastic")):
        part.add(key, line([(0.05, y * 0.4, z + 0.5), (0.25, y * 0.9, z + 0.8), (0.8, y, z + 0.75 + 0.02 + rr), (r - 0.6, y, z + 0.77 + rr),
                            (r - 0.2, y * 0.6, z + 0.5), (r, y * 0.2, z + 0.16)], rr, 0.08))
    for k in range(7):
        part.add("black", box((0.05, 0.42, 0.03), (0.9 + 1.2 * k, 0, z + 0.79), tile=0.1))
    return part


def pendant_part(p=PRINTER):
    """The turntable under the boom's tip: the bearing's inner race and the
    ring gear its drive turns, the hanger beam under it with the
    counterweight behind the axis, the cheek plates holding the shoulder's
    axle, its gear motor outside, the feed line's swivel on the other side
    and the feed line along the hanger to it."""
    o = origins(p)["pendant"]
    part = Part("pendant", o)
    m = Matrix.Translation(o)
    sx, sz = p.shoulder
    part.add("steel", ring_of(0.52, -0.08, 0.0, 0.3, 40), m)
    part.add("dark", mk.gear(0.48, 0.54, -0.2, -0.1, 72), m)
    part.add("grey", cyl(0.48, -0.2, -0.08, 40), m)
    part.add("steel", cyl(0.08, -0.08, 0.12, 16), m)
    part.add("white", box((2.0, 0.48, 0.34), (0.5, 0, -0.37), tile=0.5, bevel=0.02), m)
    part.add("dark", box((0.32, 0.5, 0.42), (-0.62, 0, -0.42), tile=0.5, bevel=0.015), m)
    cheek = [(sx - 0.48, -0.2), (sx + 0.36, -0.2)] + [(sx + 0.36 * math.cos(-math.pi * k / 8), sz + 0.36 * math.sin(-math.pi * k / 8)) for k in range(9)]
    for s in (-1, 1):
        part.add("white", plate(cheek, s * 0.30, s * 0.34), m)
        part.add("grey", cyl(0.2, 0.34, 0.39, 24), m @ at((sx, 0, sz), (0, s, 0)))
    mk.motor(part, m @ at((sx, 0.39, sz), (0, -1, 0)), 0.3, 0.36, gear_len=0.14)
    part.add("steel", cyl(0.06, 0.39, 0.48, 16), m @ at((sx, 0, sz), (0, -1, 0)))
    part.add("feed", line([(0.15, -0.25, -0.32), (0.6, -0.33, -0.42), (0.95, -0.44, -0.6), (sx, -0.44, sz + 0.06)], 0.026), m)
    for x in (0.3, 0.9):
        part.add("black", box((0.06, 0.1, 0.06), (x, -0.27, -0.36 - 0.07 * x), tile=0.1), m)
    return part


def upper_arm_part(p=PRINTER):
    """The upper arm, level at rest: its hub on the shoulder's axle, a
    tapered box beam with bolted side covers, the elbow's fork, its gear
    motor outside and the feed line's swivel inside; the feed line along
    its -Y side on clamps."""
    o = origins(p)["upper_arm"]
    part = Part("upper_arm", o)
    m = Matrix.Translation(o)
    length = p.upper
    part.add("grey", cyl(0.27, -0.29, 0.29, 28), m @ at((0, 0, 0), (0, 1, 0)))
    part.add("white", taper(0.15, length - 0.7, 0.36, 0.46, 0.3, 0.38), m)
    for s in (-1, 1):
        part.add("white", plate(nose(length - 0.9, length, 0.19, 0.24), s * 0.255, s * 0.295), m)
        part.add("grey", cyl(0.17, 0.295, 0.33, 24), m @ at((length, 0, 0), (0, s, 0)))
        cover = m @ Matrix.Translation((1.2, s * 0.172, 0.0))
        part.add("grey", box((0.8, 0.01, 0.24), tile=0.3), cover)
        mk.bolts(part, cover @ at((0, s * 0.005, 0), (0, s, 0)), [(sx * 0.36, sz * 0.1) for sx in (-1, 0, 1) for sz in (-1, 1)], 0.008)
    mk.motor(part, m @ at((length, 0.33, 0), (0, -1, 0)), 0.24, 0.3, gear_len=0.1)
    part.add("steel", cyl(0.05, 0.33, 0.42, 16), m @ at((length, 0, 0), (0, -1, 0)))
    part.add("feed", line([(0.0, -0.5, 0.0), (0.2, -0.46, 0.16), (0.6, -0.24, 0.24), (length - 1.0, -0.22, 0.2), (length - 0.45, -0.36, 0.16),
                           (length, -0.38, 0.06)], 0.026), m)
    for x in (0.7, 1.4, 2.0):
        part.add("black", box((0.06, 0.08, 0.06), (x, -0.2, 0.2 - 0.02 * x), tile=0.1), m)
    return part


def forearm_part(p=PRINTER):
    """The forearm, folded back under the upper arm at rest: its hub on the
    elbow's axle and its neck, a tapered box beam, the wrist's fork, its
    gear motor outside and the feed line's swivel inside; the feed line
    along its -Y side."""
    o = origins(p)["forearm"]
    part = Part("forearm", o)
    m = Matrix.Translation(o) @ Matrix.Rotation(math.radians(p.fold), 4, "Y")
    length = p.fore
    part.add("grey", cyl(0.22, -0.25, 0.25, 24), m @ at((0, 0, 0), (0, 1, 0)))
    part.add("grey", box((0.5, 0.26, 0.2), (0.3, 0, 0), tile=0.3), m)
    part.add("white", taper(0.55, length - 0.25, 0.3, 0.36, 0.24, 0.28), m)
    for s in (-1, 1):
        part.add("white", plate(nose(length - 0.5, length, 0.14, 0.2), s * 0.23, s * 0.265), m)
        part.add("grey", cyl(0.14, 0.265, 0.3, 24), m @ at((length, 0, 0), (0, s, 0)))
    mk.motor(part, m @ at((length, 0.3, 0), (0, -1, 0)), 0.18, 0.24, gear_len=0.08)
    part.add("steel", cyl(0.045, 0.3, 0.38, 16), m @ at((length, 0, 0), (0, -1, 0)))
    part.add("feed", line([(0.0, -0.44, 0.0), (0.25, -0.4, 0.16), (0.7, -0.2, 0.2), (length - 0.6, -0.19, 0.17), (length - 0.3, -0.31, 0.12),
                           (length, -0.33, 0.05)], 0.024), m)
    for x in (1.0, 1.8):
        part.add("black", box((0.06, 0.08, 0.06), (x, -0.17, 0.17), tile=0.1), m)
    return part


def head_part(p=PRINTER):
    """The print head, plumb under the wrist: the wrist's housing and the
    mount block; aluminium side plates carrying the extruder's right-angle
    gearbox and its servo, the lantern round the screw's coupling and the
    feed throat; the pellet hopper on the throat with its sight glass and
    the feed line down into it; the barrel in four heater bands with their
    terminal boxes and harness, the conical nozzle; an orange blower and
    the manifold round the barrel's head, from which six black corrugated
    hoses loop down onto the nozzle's air ring; the camera watching the
    bead and the junction box."""
    o = origins(p)["head"]
    part = Part("head", o)
    m = Matrix.Translation(o)
    part.add("grey", cyl(0.15, -0.22, 0.22, 24), m @ at((0, 0, 0), (0, 1, 0)))
    part.add("grey", box((0.26, 0.3, 0.14), (0, 0, -0.17), tile=0.3, bevel=0.01), m)
    for s in (-1, 1):
        part.add("alu", box((0.26, 0.015, 0.6), (0, s * 0.1575, -0.53), tile=0.3), m)
    part.add("dark", box((0.28, 0.3, 0.24), (0, 0, -0.36), tile=0.3, bevel=0.01), m)
    mk.motor(part, m @ at((0, 0.165, -0.36), (0, -1, 0)), 0.2, 0.3, gear_len=0.06)
    for x in (-0.1, 0.1):
        for y in (-0.1, 0.1):
            part.add("steel", rod((x, y, -0.48), (x, y, -0.6), 0.012, 6), m)
    part.add("steel", box((0.24, 0.24, 0.16), (0, 0, -0.68), tile=0.3, bevel=0.008), m)
    # The hopper.
    hopper = m @ Matrix.Translation((0, -0.33, 0))
    part.add("steel", box((0.12, 0.22, 0.08), (0, -0.21, -0.69), tile=0.2), m)
    part.add("steel", turned([(0.0, -0.72), (0.05, -0.72), (0.16, -0.48), (0.16, -0.3), (0.175, -0.3), (0.175, -0.27), (0.0, -0.27)], 24), hopper)
    part.add("dark", cyl(0.18, -0.27, -0.25, 24), hopper)
    part.add("glass", box((0.05, 0.006, 0.12), (0, -0.492, -0.4), tile=0.2), m)
    part.add("feed", line([(0.0, -0.39, 0.0), (0.02, -0.46, -0.08), (0.0, -0.42, -0.18), (0.0, -0.33, -0.25)], 0.022, 0.03), m)
    # The barrel and the nozzle.
    part.add("barrel", cyl(0.07, -1.3, -0.76, 24), m)
    for z in (-0.84, -0.96, -1.08, -1.2):
        part.add("heater", ring_of(0.09, z - 0.04, z + 0.04, 0.07, 24), m)
        part.add("dark", box((0.04, 0.05, 0.045), (0.11, 0, z), tile=0.1), m)
    part.add("plastic", line([(0.13, 0, -1.2), (0.13, 0, -0.84), (0.17, 0, -0.7), (0.19, -0.03, -0.62)], 0.014, 0.04, 8), m)
    part.add("dark", box((0.1, 0.12, 0.16), (0.19, -0.05, -0.56), tile=0.2, bevel=0.005), m)
    part.add("steel", cyl(0.095, -1.33, -1.3, 24), m)
    part.add("brass", turned([(0.0, -1.5), (0.012, -1.5), (0.016, -1.48), (0.065, -1.33), (0.0, -1.33)], 24), m)
    # The cooling air.
    scroll = m @ at((-0.27, 0, -0.42), (0, 1, 0))
    part.add("orange", turned([(0.0, -0.05), (0.09, -0.05), (0.1, -0.03), (0.1, 0.03), (0.09, 0.05), (0.0, 0.05)], 24), scroll)
    mk.motor(part, m @ at((-0.27, 0.05, -0.42), (0, -1, 0)), 0.08, 0.1, "dark")
    part.add("alu", box((0.14, 0.04, 0.04), (-0.19, 0, -0.36), tile=0.2), m)
    part.add("dark", box((0.12, 0.3, 0.2), (-0.25, 0, -0.62), tile=0.2, bevel=0.01), m)
    part.add("orange", box((0.07, 0.08, 0.06), (-0.25, 0, -0.49), tile=0.2), m)
    for s in (-1, 1):
        for start, out, low in (((-0.24, 0.13, -0.66), (-0.12, 0.44, -0.86), (0.06, 0.4, -1.12)), ((-0.27, 0.07, -0.72), (-0.34, 0.22, -0.98), (-0.22, 0.17, -1.24))):
            end = math.radians(50 if start[1] > 0.1 else 130)
            path = [(start[0], s * start[1], start[2]), (out[0], s * out[1], out[2]), (low[0], s * low[1], low[2]),
                    (0.16 * math.cos(end), s * 0.16 * math.sin(end), -1.33), (0.105 * math.cos(end), s * 0.105 * math.sin(end), -1.385)]
            part.add("hose", corrugated(path, 0.028), m)
    part.add("black", torus(0.105, 0.02, 24, 8), m @ Matrix.Translation((0, 0, -1.39)))
    for k in range(3):
        a = TAU * (k + 0.25) / 3
        part.add("steel", rod((0.1 * math.cos(a), 0.1 * math.sin(a), -1.39), (0.085 * math.cos(a), 0.085 * math.sin(a), -1.3), 0.008, 6), m)
    camera_head(part, m @ at(EYE, -Vector(GAZE)), 1.6)
    part.add("alu", rod((0.1, 0.165, -0.8), (0.16, 0.28, -0.95), 0.012, 6), m)
    return part


def screw_part(p=PRINTER):
    """The extruder's screw: the coupling under the gearbox with its bolts
    and an orange index mark, the shaft down through the throat and the
    flighted screw inside the barrel."""
    o = origins(p)["screw"]
    part = Part("screw", o)
    m = Matrix.Translation(o)
    part.add("steel", cyl(0.075, -0.1, 0.0, 20), m)
    mk.bolts(part, m, [(0.05 * math.cos(TAU * k / 6), 0.05 * math.sin(TAU * k / 6)) for k in range(6)], 0.009)
    part.add("orange", box((0.02, 0.03, 0.09), (0.078, 0, -0.05), tile=0.1), m)
    part.add("steel", cyl(0.035, -0.3, -0.1, 12), m)
    shaft, flight = auger(0.52, 0.03, 0.058, 0.07, 14)
    down = m @ Matrix.Translation((0, 0, -0.28)) @ Matrix.Rotation(math.pi / 2, 4, "Y")
    part.add("steel", shaft, down)
    part.add("steel", flight, down)
    return part


def parts(p=PRINTER):
    """The printer's bodies as Parts, in its frame at rest."""
    return [chassis_part(p), mast_part(p), mast_top_part(p), boom_part(p), pendant_part(p), upper_arm_part(p), forearm_part(p), head_part(p), screw_part(p)]


# The machine layer


def open_machine(u, controllers, note, name):
    """The machine's header: its prim, its controllers as (name, type,
    joints), a controller naming several joints moving them together, and
    its looks."""
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "scripts/mars_base/printer.py: {note}"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/printer.py and build again.")
    u.emit(0, "# A positioning joint takes a position 0..1 of its range on the link it moves: `gearbox machine set-value mast position 0.5")
    u.emit(0, "# --machine <id>` raises both mast stages; the pendant and the screw a velocity in rad/s: `... set-value pendant velocity 0.07`.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'PhysicsArticulationRootAPI', *(f'GearboxControllerAPI:{c}' for c, _, _ in controllers)])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, f'token gearbox:machine:kind = "{NAME}"')
    u.emit(1, "rel gearbox:machine:body = </robot/chassis>")
    for c, kind, targets in controllers:
        u.emit(1, f'token gearbox:controller:{c}:type = "builtin:{kind}"')
        rels = ", ".join(f"</robot/Joints/{t}>" for t in targets)
        u.emit(1, f"rel gearbox:controller:{c}:target = {rels if len(targets) == 1 else f'[{rels}]'}")
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{name}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")


def link_body(u, d, name, at_, mass, inertia, com=(0, 0, 0), values=(), link=None):
    """Opens a rigid body at `at_` with its mass, centre of mass and inertia; close it with `usda.end`."""
    xform(u, d, name, tuple(at_), ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"], [
        *([f'token gearbox:link:name = "{link}"', 'token gearbox:link:role = "base"'] if link else []),
        f"float physics:mass = {mass}",
        f"point3f physics:centerOfMass = {fmt(com)}",
        f"float3 physics:diagonalInertia = {fmt(inertia)}",
        *values,
    ])


def machine(p, note, name, bound):
    """The printer's machine: chassis (held to the world), the mast's two
    stages, boom, pendant, upper arm, forearm, head and screw."""
    o = origins(p)
    u = Usda("/robot/Looks")
    controllers = (("lift", "joint_position", ("lift", "lift_top")), ("slew", "joint_position", ("slew",)), ("turn", "joint_velocity", ("turn",)),
                   ("shoulder", "joint_position", ("shoulder",)), ("elbow", "joint_position", ("elbow",)), ("wrist", "joint_position", ("wrist",)),
                   ("screw", "joint_velocity", ("screw",)))
    open_machine(u, controllers, note, name)
    rel = lambda body_, v: tuple(Vector(v) - o[body_])
    span = lambda deg: f"float gearbox:value:range = {math.radians(deg):.5f}"
    link_body(u, 1, "chassis", o["chassis"], 4200, (11000, 11000, 20000), link="base_link")
    hull(u, 2, "frame", rel("chassis", (0, 0, 1.2)), (2.7, 2.6, 0.5))
    for k, a in enumerate(LEGS):
        d = Vector((math.cos(a), math.sin(a), 0))
        hull(u, 2, f"leg{k}", rel("chassis", d * LEG_R + Z * 0.6), (0.9, 0.44, 1.1))
        hull(u, 2, f"jack{k}", rel("chassis", d * JACK_R + Z * 0.115), (0.44, 0.44, 0.17))
    hull(u, 2, "mast_base", rel("chassis", (0, 0, 4.31)), (0.8, 0.8, 5.7))
    for s in (-1, 1):
        hull(u, 2, f"tank{'pn'[s > 0]}", rel("chassis", (0.72, s * 0.78, 2.55)), (0.7, 0.7, 2.1))
    hull(u, 2, "reel", rel("chassis", (-0.88, 0, 2.0)), (1.0, 1.1, 1.1))
    visual(u, 2, name, "chassis", bound)
    end(u, 1)
    link_body(u, 1, "mast", o["mast"], 520, (1840, 1840, 50), (0, 0, 3.25), [f"float gearbox:value:range = {p.stage:g}"])
    hull(u, 2, "stage", (0, 0, 3.35), (0.62, 0.62, 6.7))
    visual(u, 2, name, "mast", bound)
    end(u, 1)
    link_body(u, 1, "mast_top", o["mast_top"], 420, (1580, 1580, 25), (0, 0, 3.4), [f"float gearbox:value:range = {p.stage:g}"])
    hull(u, 2, "stage", (0, 0, 3.36), (0.4, 0.4, 6.72))
    hull(u, 2, "bearing", (0, 0, 6.825), (0.92, 0.92, 0.21))
    visual(u, 2, name, "mast_top", bound)
    end(u, 1)
    r = p.reach
    link_body(u, 1, "boom", o["boom"], 1500, (400, 19800, 19800), (0.2, 0, 0.3), [span(p.slew_deg)])
    hull(u, 2, "turret", (0, 0, 0.39), (1.25, 0.72, 0.78))
    hull(u, 2, "jib", ((0.6 + r + 0.5) / 2, 0, 0.425), (r - 0.1, 0.66, 0.72))
    hull(u, 2, "counter_jib", (-2.0, 0, 0.35), (2.8, 0.66, 0.55))
    hull(u, 2, "counterweight", (-2.94, 0, -0.02), (0.96, 0.98, 0.97))
    hull(u, 2, "tip", (r, 0, 0.0), (1.25, 1.25, 0.2))
    visual(u, 2, name, "boom", bound)
    end(u, 1)
    link_body(u, 1, "pendant", o["pendant"], 300, (70, 140, 140), (0.45, 0, -0.4))
    hull(u, 2, "hanger", (0.39, 0, -0.37), (2.34, 0.6, 0.4))
    hull(u, 2, "cheeks", (p.shoulder[0], 0, -0.78), (0.96, 0.8, 0.86))
    hull(u, 2, "ring", (0, 0, -0.1), (1.1, 1.1, 0.2))
    visual(u, 2, name, "pendant", bound)
    end(u, 1)
    link_body(u, 1, "upper_arm", o["upper_arm"], 260, (8, 220, 220), (1.4, 0, 0), [span(p.shoulder_deg)])
    hull(u, 2, "beam", (p.upper / 2, 0, 0), (p.upper + 0.6, 0.6, 0.52))
    visual(u, 2, name, "upper_arm", bound)
    end(u, 1)
    fold = Matrix.Rotation(math.radians(p.fold), 3, "Y")
    link_body(u, 1, "forearm", o["forearm"], 180, (140, 140, 140), tuple(fold @ Vector((1.4, 0, 0))), [span(p.elbow_deg)])
    hull(u, 2, "beam", tuple(fold @ Vector((p.fore / 2, 0, 0))), (p.fore + 0.3, 0.56, 0.4), (0.0, p.fold, 0.0))
    visual(u, 2, name, "forearm", bound)
    end(u, 1)
    link_body(u, 1, "head", o["head"], 220, (45, 45, 15), (0, 0, -0.7), [span(p.wrist_deg)])
    hull(u, 2, "head", (-0.04, 0.01, -0.66), (0.84, 1.24, 1.12))
    visual(u, 2, name, "head", bound)
    camera(u, 2, EYE, looking(GAZE), 3.0)
    end(u, 1)
    link_body(u, 1, "screw", o["screw"], 12, (0.6, 0.6, 0.03), (0, 0, -0.4))
    visual(u, 2, name, "screw", bound)
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "chassis", anchor(p))
    for name_, parent, child, kind, axis, limits, motor in JOINTS:
        joint(u, 2, kind, name_, parent, child, rel(parent, o[child]), axis, limits, motor)
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def build(work, p=PRINTER, note="the regolith printer of MARSHA"):
    """Builds the printer's looks, writes them as `<NAME>_geom.usdc` and the
    machine as `<NAME>.usda` into `work`; returns {NAME: machine layer}."""
    os.makedirs(work, exist_ok=True)
    kept = (set(bpy.data.materials), set(bpy.data.images), set(bpy.data.meshes))
    mats = materials(kit.Looks(os.path.join(work, "maps")))
    scene = bpy.data.collections.new(f"{NAME}_geom")
    bpy.context.scene.collection.children.link(scene)
    objs = [part.finish(mats, scene) for part in parts(p)]
    geom = os.path.join(work, f"{NAME}_geom.usdc")
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
    path = os.path.join(work, f"{NAME}.usda")
    with open(path, "w") as f:
        f.write(machine(p, note, NAME, bound))
    return {NAME: path}
