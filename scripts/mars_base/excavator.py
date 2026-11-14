"""The farm's regolith excavator, after NASA Kennedy's RASSOR 2.0 and its
flight successor IPEx: a skid-steered chassis on four rigid grousered
wheels and, at each end, an arm on a shoulder actuator carrying a pair of
bucket drums that dig turning against each other.

    chassis       side plates and bulkheads round the battery and avionics
                  box, its radiator on top; the shoulder actuators between
                  their cheeks at both ends, the wheel actuators in the
                  side plates; a stereo camera and a lamp unit at each end,
                  a LiDAR, an antenna, a status lamp, an e-stop, the harness
    wheel_*       rigid hard-anodised wheels: rim with ski edges, blade
                  spokes, eighteen riveted grousers
    arm_*         the arm, its hub on the shoulder actuator and the drum
                  actuator at its tip, its harness in a conduit
    drum_*        two bucket drums on the drum actuator's through shaft,
                  each four slices of two scoops: expanding-spiral carbon
                  shells, steel teeth on the lips, baffles behind the
                  openings, aluminium section and end walls

Sizes are RASSOR 2.0's: 66 kg, drums 437 mm across and 353 mm wide holding
about 25 kg each, wheels 360 mm over the grousers. Frame, as the base's
other wheeled machines: +X left, +Y astern, +Z up, the machine driving
toward -Y, the wheels touching Z = 0. At rest the arms stand upright,
stowed. `arm_front` and `arm_rear` take a position 0..1 of their range:
0 upright, 0.33 over a bin's lip (`DUMP`), 0.40 carrying (`CARRY`), 0.8
level, 1 past level, the drums down in a cut or lifting the chassis.
`drum_front` and `drum_rear` take a velocity in rad/s: digging turns the
front drum positive and the rear negative; reversed, they dump.

Built in Blender from bmesh at those sizes; the looks are baked with Mars
wear into one atlas, posed carrying, and written as a geometry layer each
body's visual references; the bodies collide as hidden boxes and cylinders.
"""
import math
import os

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import kit
import mechanics as mk
import patterns
import weathering
from kit import box, prism, rod, srgb_to_linear, tube
from mechanics import Part, at, cyl, hexagon, turned
from usda import Usda, fmt, schemas
from vehicles import bindings, box_inertia, export_geom, visual

NAME = "regolith_excavator"
TEXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "assets", "mars_base", "textures")

WHEEL_R, RIM_R, WHEEL_W = 0.18, 0.163, 0.16
WHEEL_X, WHEEL_Y = 0.265, 0.25
SHOULDER_Y, SHOULDER_Z, ARM = 0.27, 0.20, 0.57
DRUM_R, DRUM_W, DRUM_X = 0.2185, 0.353, 0.067
SCOOP_H, SLICES = 0.048, 4
TRAVEL_DEG = 112.0
CARRY, DUMP = 0.40, 0.33
DRUM_RATE = 2.1
MASS = {"chassis": 34.0, "wheel": 2.5, "arm": 3.5, "drum": 7.5}
ENDS = (("front", -1.0), ("rear", 1.0))
SIDES = (("left", 1.0), ("right", -1.0))
YZ = at((0, 0, 0), (1, 0, 0))

# Key: (material name, sRGB colour, roughness, metal, map settings).
FINISHES = {
    "alu": ("rassor_aluminium", (0.74, 0.75, 0.77), 0.36, 0.9, {"seed": 71, "brush": 0.05, "grime": 0.05}),
    "hardcoat": ("rassor_hardcoat", (0.33, 0.33, 0.34), 0.48, 0.65, {"seed": 72, "grime": 0.08}),
    "black": ("rassor_black", (0.07, 0.07, 0.075), 0.42, 0.6, {"seed": 73, "grime": 0.08}),
    "steel": ("rassor_steel", (0.70, 0.71, 0.72), 0.24, 1.0, {"seed": 74, "brush": 0.08, "grime": 0.04}),
    "white": ("rassor_white", (0.86, 0.86, 0.84), 0.5, 0.0, {"seed": 75, "grime": 0.06}),
    "plastic": ("rassor_cable", (0.035, 0.035, 0.038), 0.7, 0.0, {"seed": 76, "grime": 0.03}),
    "felt": ("rassor_felt", (0.80, 0.74, 0.58), 0.95, 0.0, {"seed": 77, "grime": 0.1}),
    "yellow": ("rassor_yellow", (0.92, 0.70, 0.08), 0.5, 0.0, {"seed": 78, "grime": 0.06}),
    "red": ("rassor_red", (0.70, 0.06, 0.04), 0.4, 0.0, {"seed": 79, "grime": 0.04}),
    "lens": ("rassor_lens", (0.02, 0.025, 0.03), 0.04, 0.0, None),
}


def _twill(colour=(0.085, 0.085, 0.09), n=512, cells=32):
    """Carbon fibre in a 2/2 twill: tows crowned across their width, the
    over and under tows taking the light differently."""
    def make():
        u, v = patterns.grid(n)
        iu, iv = np.floor(u * cells), np.floor(v * cells)
        fu, fv = u * cells % 1.0, v * cells % 1.0
        over = ((iu + iv) % 4) < 2
        crown = np.where(over, np.sin(np.pi * fv), np.sin(np.pi * fu))
        sheen = np.where(over, 1.0, 0.72) * (0.75 + 0.35 * crown)
        c = srgb_to_linear(np.array(colour, dtype=np.float32))
        diff = np.clip(c * sheen[..., None], 0.0, 1.0).astype(np.float32)
        rough = np.clip(0.28 + 0.12 * (1.0 - crown) + 0.05 * patterns.noise(n, 16, 81), 0.0, 1.0).astype(np.float32)
        return diff, patterns.normals_from_height((0.6 * crown).astype(np.float32), 1.5), rough
    return make


def _osr(n=512, tiles=8):
    """A radiator of optical solar reflector tiles: silvered glass squares in dark bonding gaps."""
    def make():
        u, v = patterns.grid(n)
        fu, fv = u * tiles % 1.0, v * tiles % 1.0
        gap = (np.minimum(fu, 1.0 - fu) < 0.03) | (np.minimum(fv, 1.0 - fv) < 0.03)
        tone = patterns.noise(n, tiles, 82, 1)
        diff = np.where(gap[..., None], np.float32(0.05), (0.78 + 0.08 * tone)[..., None] * np.ones(3, dtype=np.float32))
        rough = np.where(gap, 0.7, 0.06 + 0.04 * tone).astype(np.float32)
        return diff.astype(np.float32), patterns.normals_from_height(-gap.astype(np.float32), 2.0), rough
    return make


def materials(looks):
    m = {key: looks.get(name, colour, rough, metal, maps=mk.finish(colour, rough, **maps) if maps else None)
         for key, (name, colour, rough, metal, maps) in FINISHES.items()}
    m["carbon"] = looks.get("rassor_carbon", (0.085, 0.085, 0.09), 0.3, 0.25, maps=_twill())
    m["osr"] = looks.get("rassor_radiator", (0.8, 0.8, 0.82), 0.08, 0.9, maps=_osr())
    m["mli"] = looks.get("rassor_mli", (0.84, 0.72, 0.50), 0.32, 1.0, "bi_stretch", contrast=1.1)
    m["led"] = looks.get("rassor_led", (1.0, 0.97, 0.9), 0.3, 0.0, emit=((1.0, 0.97, 0.9), 6.0))
    m["amber"] = looks.get("rassor_amber", (1.0, 0.55, 0.08), 0.3, 0.0, emit=((1.0, 0.55, 0.08), 5.0))
    return m


# Places and poses


def wheels():
    """Each wheel: its name and hub, front first."""
    return [(f"{end}_{side}", Vector((sx * WHEEL_X, sy * WHEEL_Y, WHEEL_R))) for end, sy in ENDS for side, sx in SIDES]


def shoulder(s):
    return Vector((0.0, s * SHOULDER_Y, SHOULDER_Z))


def lowering(s):
    """The sign of the joint angle that lowers the arm at end `s`."""
    return -s


def origins():
    """Each body's origin in the machine's frame at rest."""
    out = {"chassis": Vector((0, 0, 0))}
    for name, hub in wheels():
        out[f"wheel_{name}"] = hub
    for end, s in ENDS:
        out[f"arm_{end}"] = shoulder(s)
        out[f"drum_{end}"] = shoulder(s) + Vector((0, 0, ARM))
    return out


def pose(arms=(0.0, 0.0), drums=(0.0, 0.0), roll=0.0):
    """Each body's matrix in the machine's frame, its arms at positions 0..1
    of their range, its drums and wheels turned by angles in radians."""
    o = origins()
    out = {"chassis": Matrix.Identity(4)}
    for name, hub in wheels():
        out[f"wheel_{name}"] = Matrix.Translation(hub) @ Matrix.Rotation(roll, 4, "X")
    for (end, s), p, spin in zip(ENDS, arms, drums):
        arm = Matrix.Translation(o[f"arm_{end}"]) @ Matrix.Rotation(lowering(s) * math.radians(TRAVEL_DEG) * p, 4, "X")
        out[f"arm_{end}"] = arm
        out[f"drum_{end}"] = arm @ Matrix.Translation((0, 0, ARM)) @ Matrix.Rotation(spin, 4, "X")
    return out


# Builders


def _slab(points, x0, x1, m=None):
    """A plate between x0 and x1 over a counter-clockwise outline of (y, z) points."""
    bm = prism(points, x0, x1, 0.2)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    bm.transform(YZ if m is None else Matrix(m) @ YZ)
    return bm


def _along_x(part, key, r, x0, x1, y, z, segments=24):
    part.add(key, cyl(r, x0, x1, segments), Matrix.Translation((0, y, z)) @ YZ)


def _ring_x(part, key, r_out, r_in, x0, x1, y, z, segments=32):
    part.add(key, mk.ring(r_out, r_in, x0, x1, segments), Matrix.Translation((0, y, z)) @ YZ)


def _bolt_circle(part, x, y, z, r, n, face, bolt=0.004, key="steel", start=0.0):
    """Hex bolt heads round a circle of radius r about an X axis through (y, z), on the face at x looking along `face` (±1 in X)."""
    for k in range(n):
        a = start + 2 * math.pi * k / n
        part.add(key, hexagon(bolt, 0.0, 0.7 * bolt), at((x, y + r * math.cos(a), z + r * math.sin(a)), (face, 0, 0)))


def _heads(part, points, normal, r, key="steel"):
    """Hex bolt heads at `points`, standing out along `normal`."""
    for q in points:
        part.add(key, hexagon(r, 0.0, 0.7 * r), at(q, normal))


def _polar(y, z, r, a):
    return (y + r * math.cos(a), z + r * math.sin(a))


# Chassis


def chassis_part():
    """The chassis: side plates with their ribs, belly pan, bulkheads and
    deck; at each end the shoulder actuator's stator halves between their
    cheeks; the wheel actuators and their seals; the battery and avionics
    box under its radiator; cameras, lamps, LiDAR, antenna, status lamp,
    e-stop, the charging coil under the belly, and the harness."""
    p = Part("chassis", (0, 0, 0))
    side = [(-0.30, 0.12), (-0.26, 0.075), (0.26, 0.075), (0.30, 0.12), (0.30, 0.27), (0.27, 0.30), (-0.27, 0.30), (-0.30, 0.27)]
    for sx in (-1, 1):
        x0, x1 = sorted((sx * 0.155, sx * 0.161))
        p.add("alu", _slab(side, x0, x1))
        face = sx * 0.161
        rx0, rx1 = sorted((face, face + sx * 0.006))
        for (ya, za), (yb, zb) in ((( -0.2, 0.09), (0.2, 0.09)), ((-0.2, 0.285), (0.2, 0.285)), ((-0.19, 0.095), (0.0, 0.28)), ((0.19, 0.095), (0.0, 0.28)),
                                   ((-0.19, 0.28), (0.0, 0.095)), ((0.19, 0.28), (0.0, 0.095))):
            d = Vector((yb - ya, zb - za))
            n = Vector((-d.y, d.x)).normalized() * 0.006
            a0, a1 = Vector((ya, za)), Vector((yb, zb))
            p.add("alu", _slab([tuple(a0 - n), tuple(a1 - n), tuple(a1 + n), tuple(a0 + n)], rx0, rx1))
        _heads(p, [(face + sx * 0.006, y, z) for y in (-0.2, -0.1, 0.0, 0.1, 0.2) for z in (0.09, 0.285)], (sx, 0, 0), 0.004)
        for _, hub in wheels():
            if hub.x * sx < 0:
                continue
            _along_x(p, "black", 0.048, *sorted((sx * 0.161, sx * 0.183)), hub.y, hub.z, 28)
            _ring_x(p, "black", 0.058, 0.04, *sorted((sx * 0.161, sx * 0.167)), hub.y, hub.z, 28)
            _ring_x(p, "felt", 0.05, 0.03, *sorted((sx * 0.183, sx * 0.186)), hub.y, hub.z, 28)
            _bolt_circle(p, sx * 0.167, hub.y, hub.z, 0.052, 8, sx, 0.0035)
            gland = (sx * 0.161, hub.y - math.copysign(0.04, hub.y), hub.z + 0.065)
            p.add("plastic", cyl(0.009, 0.0, 0.012, 10), at(gland, (sx, 0, 0)))
            p.add("plastic", tube([Vector(gland) + Vector((sx * 0.012, 0, 0)), Vector(gland) + Vector((sx * 0.02, 0, 0.01)),
                                   (sx * 0.17, hub.y - math.copysign(0.02, hub.y), hub.z + 0.035)], 0.005, 8))
    p.add("alu", box((0.31, 0.43, 0.006), (0, 0, 0.078), tile=0.3))
    p.add("alu", box((0.322, 0.43, 0.006), (0, 0, 0.303), tile=0.3))
    p.add("black", cyl(0.07, 0.069, 0.075, 32), Matrix.Translation((0, 0.06, 0)))
    for end, s in ENDS:
        y = s * 0.215
        p.add("alu", box((0.31, 0.006, 0.22), (0, y - s * 0.003, 0.19), tile=0.3))
        sh = shoulder(s)
        if s < 0:
            cheek = [(y, 0.29), (sh.y, sh.z + 0.075)] + [_polar(sh.y, sh.z, 0.075, math.pi / 2 + math.pi * k / 12) for k in range(1, 12)] + [(sh.y, sh.z - 0.075), (y, 0.115)]
        else:
            cheek = [(y, 0.115), (sh.y, sh.z - 0.075)] + [_polar(sh.y, sh.z, 0.075, -math.pi / 2 + math.pi * k / 12) for k in range(1, 12)] + [(sh.y, sh.z + 0.075), (y, 0.29)]
        for sx in (-1, 1):
            x0, x1 = sorted((sx * 0.088, sx * 0.098))
            p.add("alu", _slab(cheek, x0, x1))
            _along_x(p, "black", 0.056, *sorted((sx * 0.040, sx * 0.088)), sh.y, sh.z, 32)
            _ring_x(p, "black", 0.066, 0.05, *sorted((sx * 0.080, sx * 0.088)), sh.y, sh.z, 32)
            _ring_x(p, "felt", 0.058, 0.045, *sorted((sx * 0.037, sx * 0.040)), sh.y, sh.z, 32)
            _bolt_circle(p, sx * 0.098, sh.y, sh.z, 0.06, 8, sx, 0.0035, start=math.pi / 8)
            _along_x(p, "black", 0.022, *sorted((sx * 0.098, sx * 0.112)), sh.y, sh.z, 16)
        _along_x(p, "black", 0.061, -0.088, -0.068, sh.y, sh.z, 32)
        for sx in (-1, 1):
            p.add("alu", rod((sx * 0.155, s * 0.29, 0.1), (sx * 0.098, s * 0.29, 0.1), 0.01, 10))
        p.add("alu", rod((-0.088, s * 0.29, 0.1), (0.088, s * 0.29, 0.1), 0.01, 10))
        for x in (-0.12, -0.065, 0.065, 0.12):
            p.add("black", cyl(0.011, 0.0, 0.014, 14), at((x, y, 0.125), (0, s, 0)))
            p.add("steel", cyl(0.013, 0.0, 0.004, 14), at((x, y, 0.125), (0, s, 0)))
        _heads(p, [(x, y, z) for x in (-0.14, 0.0, 0.14) for z in (0.095, 0.285)], (0, s, 0), 0.0035)
        _camera(p, s)
        _lamp_unit(p, s)
        p.add("plastic", tube([(0.115, s * 0.16, 0.32), (0.115, s * 0.205, 0.318), (0.07, s * 0.235, 0.29), (0.062, s * 0.255, 0.262)], 0.006, 8))
        p.add("plastic", tube([(-0.115, s * 0.16, 0.32), (-0.115, s * 0.205, 0.318), (-0.07, s * 0.235, 0.29), (-0.062, s * 0.255, 0.262)], 0.006, 8))
        for sx in (-1, 1):
            p.add("steel", box((0.014, 0.012, 0.004), (sx * 0.115, s * 0.2, 0.309), tile=0.1))
    _avionics(p)
    return p


def _camera(p, s):
    """A stereo camera on the deck's corner at end `s`, looking out and down: housing, two lenses behind dust-shield glass."""
    y = s * 0.195
    m = Matrix.Translation((0.105, y, 0.335)) @ Matrix.Rotation(s * math.radians(-15), 4, "X")
    p.add("white", box((0.115, 0.04, 0.052), (0, 0, 0), tile=0.1, bevel=0.004), m)
    p.add("black", box((0.105, 0.004, 0.044), (0, s * 0.021, 0), tile=0.1), m)
    for x in (-0.03, 0.03):
        p.add("black", cyl(0.012, 0.0, 0.008, 18), m @ at((x, s * 0.023, 0), (0, s, 0)))
        p.add("lens", cyl(0.009, 0.008, 0.009, 18), m @ at((x, s * 0.023, 0), (0, s, 0)))
    p.add("lens", box((0.098, 0.002, 0.036), (0, s * 0.033, 0), tile=0.1), m)
    p.add("black", box((0.11, 0.004, 0.006), (0, s * 0.032, 0.021), tile=0.1), m)
    p.add("alu", box((0.08, 0.03, 0.012), (0, -s * 0.008, -0.023), tile=0.1), Matrix.Translation((0.105, y, 0.335)))
    p.add("plastic", tube([(0.15, y - s * 0.015, 0.32), (0.16, y - s * 0.03, 0.31), (0.135, y - s * 0.035, 0.309)], 0.004, 8))


def _lamp_unit(p, s):
    """A lamp unit on the deck's other corner at end `s`: three LEDs behind their TIR optics in a housing."""
    y = s * 0.195
    m = Matrix.Translation((-0.105, y, 0.33)) @ Matrix.Rotation(s * math.radians(-20), 4, "X")
    p.add("white", box((0.1, 0.035, 0.045), (0, 0, 0), tile=0.1, bevel=0.004), m)
    for x in (-0.03, 0.0, 0.03):
        p.add("black", cyl(0.012, 0.0, 0.004, 18), m @ at((x, s * 0.0175, 0), (0, s, 0)))
        p.add("led", cyl(0.009, 0.004, 0.005, 18), m @ at((x, s * 0.0175, 0), (0, s, 0)))
    p.add("alu", box((0.07, 0.028, 0.012), (0, -s * 0.004, -0.018), tile=0.1), Matrix.Translation((-0.105, y, 0.33)))
    for x in (-0.045, 0.045):
        p.add("black", box((0.006, 0.03, 0.03), (x, 0, -0.005), tile=0.1), m)


def _avionics(p):
    """The battery and avionics box on the deck: wrapped in MLI between its
    corner frames, the radiator of mirror tiles on top, connectors, and on
    it the LiDAR, the antenna and the status lamp; the e-stop beside it."""
    x, y, z0, z1 = 0.125, 0.165, 0.306, 0.405
    p.add("mli", box((2 * x - 0.008, 2 * y - 0.008, z1 - z0 - 0.004), (0, 0, (z0 + z1) / 2), tile=0.25, bevel=0.006))
    for sx in (-1, 1):
        for sy in (-1, 1):
            p.add("alu", box((0.014, 0.014, z1 - z0), (sx * (x - 0.007), sy * (y - 0.007), (z0 + z1) / 2), tile=0.1))
        p.add("alu", box((0.012, 2 * y, 0.012), (sx * (x - 0.006), 0, z1 - 0.006), tile=0.2))
        p.add("alu", box((0.012, 2 * y, 0.01), (sx * (x - 0.006), 0, z0 + 0.005), tile=0.2))
    for sy in (-1, 1):
        p.add("alu", box((2 * x, 0.012, 0.012), (0, sy * (y - 0.006), z1 - 0.006), tile=0.2))
        p.add("alu", box((2 * x, 0.012, 0.01), (0, sy * (y - 0.006), z0 + 0.005), tile=0.2))
        _heads(p, [(xx, sy * y, z1 - 0.006) for xx in (-0.1, -0.05, 0.0, 0.05, 0.1)], (0, sy, 0), 0.003)
    p.add("alu", box((2 * x, 2 * y, 0.006), (0, 0, z1 + 0.003), tile=0.2))
    p.add("osr", box((2 * x - 0.02, 2 * y - 0.02, 0.003), (0, 0, z1 + 0.0075), tile=0.12))
    for xx in (-0.08, -0.04, 0.0, 0.04, 0.08):
        p.add("black", cyl(0.009, 0.0, 0.012, 14), at((xx, -y, z0 + 0.03), (0, -1, 0)))
    top = z1 + 0.009
    p.add("alu", cyl(0.042, top, top + 0.01, 32))
    p.add("black", cyl(0.044, top + 0.01, top + 0.04, 40))
    p.add("lens", cyl(0.041, top + 0.04, top + 0.066, 40))
    p.add("black", turned([(0.0, top + 0.066), (0.044, top + 0.066), (0.044, top + 0.078), (0.036, top + 0.083), (0.0, top + 0.083)], 40))
    p.add("plastic", tube([(0.0, 0.045, top + 0.02), (0.0, 0.07, top + 0.015), (0.0, 0.09, top + 0.0)], 0.004, 8))
    a = Matrix.Translation((0.095, 0.125, 0))
    p.add("alu", cyl(0.022, top, top + 0.005, 24), a)
    p.add("white", turned([(0.0, top + 0.005), (0.016, top + 0.005), (0.016, top + 0.05), (0.012, top + 0.058), (0.0, top + 0.06)], 24), a)
    b = Matrix.Translation((-0.095, 0.125, 0))
    p.add("black", cyl(0.016, top, top + 0.008, 20), b)
    p.add("amber", turned([(0.0, top + 0.008), (0.012, top + 0.008), (0.012, top + 0.02), (0.008, top + 0.026), (0.0, top + 0.028)], 20), b)
    p.add("yellow", box((0.026, 0.05, 0.03), (0.143, 0.03, 0.321), tile=0.1, bevel=0.003))
    p.add("red", turned([(0.0, 0.336), (0.008, 0.336), (0.008, 0.342), (0.015, 0.344), (0.015, 0.352), (0.0, 0.354)], 20), Matrix.Translation((0.143, 0.03, 0)))


# Wheels


def wheel_part(name, hub):
    """A rigid wheel on its hub: the rim with ski edges, eight blade spokes,
    the hub with its cap and the actuator's flange, eighteen grousers riveted on."""
    p = Part(f"wheel_{name}", hub)
    sx = 1.0 if hub.x > 0 else -1.0
    m = Matrix.Translation(hub) @ YZ
    w = WHEEL_W / 2
    R, t = RIM_R, 0.004
    rim = [(R - 0.011, -w), (R - 0.005, -w + 0.004), (R - 0.0015, -w + 0.012), (R, -w + 0.02), (R, w - 0.02), (R - 0.0015, w - 0.012),
           (R - 0.005, w - 0.004), (R - 0.011, w), (R - 0.014, w - 0.002), (R - 0.008, w - 0.006), (R - t, w - 0.02), (R - t, -w + 0.02),
           (R - 0.008, -w + 0.006), (R - 0.014, -w + 0.002)]
    p.add("hardcoat", turned(rim + rim[:1], 72, 0.2), m)
    spoke_r = (0.05, R - t - 0.001)
    for k in range(8):
        a = 2 * math.pi * (k + 0.5) / 8
        spoke = box((0.11, spoke_r[1] - spoke_r[0], 0.004), (0, sum(spoke_r) / 2, 0), tile=0.1)
        p.add("hardcoat", spoke, Matrix.Translation(hub) @ Matrix.Rotation(a, 4, "X"))
        p.add("hardcoat", box((0.09, 0.006, 0.016), (0, spoke_r[1] - 0.003, 0), tile=0.1), Matrix.Translation(hub) @ Matrix.Rotation(a, 4, "X"))
    hz0, hz1 = sorted((-sx * (WHEEL_X - 0.188), sx * 0.045))
    p.add("hardcoat", cyl(0.05, hz0, hz1, 32), m)
    outer = Matrix.Translation(hub) @ at((0, 0, 0), (sx, 0, 0))
    p.add("hardcoat", cyl(0.054, 0.045, 0.052, 32), outer)
    p.add("hardcoat", turned([(0.0, 0.052), (0.032, 0.052), (0.031, 0.058), (0.024, 0.062), (0.0, 0.063)], 24), outer)
    p.add("black", hexagon(0.012, 0.063, 0.07), outer)
    for k in range(6):
        a = 2 * math.pi * k / 6
        p.add("steel", hexagon(0.0045, 0.052, 0.0565), outer @ Matrix.Translation((0.042 * math.cos(a), 0.042 * math.sin(a), 0)))
    flange = Matrix.Translation(hub) @ at((0, 0, 0), (-sx, 0, 0))
    p.add("hardcoat", cyl(0.06, WHEEL_X - 0.194, WHEEL_X - 0.188, 32), flange)
    _bolt_circle(p, sx * 0.194, hub.y, hub.z, 0.054, 8, sx, 0.0035)
    grousers = 18
    for k in range(grousers):
        a = 2 * math.pi * k / grousers
        g = Matrix.Translation(hub) @ Matrix.Rotation(a, 4, "X")
        web = [(-w + 0.016, RIM_R), (w - 0.016, RIM_R), (w - 0.022, WHEEL_R), (-w + 0.022, WHEEL_R)]
        p.add("hardcoat", prism(web, -0.002, 0.002, 0.1), g @ Matrix(((1, 0, 0, 0), (0, 0, 1, 0), (0, 1, 0, 0), (0, 0, 0, 1))))
        p.add("hardcoat", box((2 * w - 0.032, 0.022, 0.003), (0, -0.012, RIM_R + 0.0015), tile=0.1), g)
        for xx in (-0.045, 0.0, 0.045):
            p.add("steel", turned([(0.0, 0.0), (0.004, 0.0), (0.0035, 0.002), (0.0, 0.0025)], 8), g @ Matrix.Translation((xx, -0.017, RIM_R + 0.003)))
    return p


# Arms


def arm_part(end, s):
    """An arm at rest, standing up from its shoulder: the hub on the
    shoulder actuator's rotor, the tapered beam under bolted cover plates,
    the harness in its conduit and clips, and the drum actuator in its
    clamp at the tip with its seals."""
    sh = shoulder(s)
    p = Part(f"arm_{end}", sh)
    _along_x(p, "alu", 0.062, -0.036, 0.036, sh.y, sh.z, 40)
    for face in (-1, 1):
        _bolt_circle(p, face * 0.036, sh.y, sh.z, 0.048, 8, face, 0.0035)
        _along_x(p, "black", 0.026, *sorted((face * 0.036, face * 0.039)), sh.y, sh.z, 24)
    tip = sh.z + ARM
    beam = [(sh.y - 0.04, sh.z + 0.03), (sh.y + 0.04, sh.z + 0.03), (sh.y + 0.03, tip - 0.05), (sh.y - 0.03, tip - 0.05)]
    p.add("alu", _slab(beam, -0.024, 0.024))
    for face in (-1, 1):
        plate = [(sh.y - 0.028, sh.z + 0.08), (sh.y + 0.028, sh.z + 0.08), (sh.y + 0.022, tip - 0.09), (sh.y - 0.022, tip - 0.09)]
        p.add("black", _slab(plate, *sorted((face * 0.024, face * 0.028))))
        _heads(p, [(face * 0.028, sh.y + dy * (0.017 - 0.004 * t), sh.z + 0.095 + t * (ARM - 0.2)) for t in (0.0, 0.5, 1.0) for dy in (-1, 1)],
               (face, 0, 0), 0.0032)
    _along_x(p, "alu", 0.068, -0.03, 0.03, sh.y, tip, 40)
    _along_x(p, "black", 0.058, -0.066, -0.03, sh.y, tip, 40)
    _along_x(p, "black", 0.058, 0.03, 0.066, sh.y, tip, 40)
    for face in (-1, 1):
        _ring_x(p, "felt", 0.06, 0.05, *sorted((face * 0.063, face * 0.0665)), sh.y, tip, 36)
        _ring_x(p, "black", 0.0605, 0.045, *sorted((face * 0.045, face * 0.049)), sh.y, tip, 36)
        _bolt_circle(p, face * 0.03, sh.y, tip, 0.064, 6, face, 0.0032, start=math.pi / 6)
    yc = sh.y + s * 0.045
    path = [(0.0, sh.y + s * 0.03, sh.z + 0.07), (0.0, yc, sh.z + 0.12), (0.0, yc, tip - 0.11), (0.0, sh.y + s * 0.05, tip - 0.07)]
    p.add("plastic", tube(path, 0.009, 10))
    for t in range(5):
        z = sh.z + 0.15 + t * (ARM - 0.3) / 4
        p.add("steel", box((0.024, 0.006, 0.012), (0.0, yc + s * 0.009, z), tile=0.1))
        p.add("steel", box((0.026, 0.02, 0.004), (0.0, (yc + sh.y + s * 0.03) / 2, z), tile=0.1))
    return p


# Drums


def _radius(a, lip, hand):
    """The shell's radius at angle `a` of a slice whose lips stand at `lip` and `lip + pi`, digging toward `hand`."""
    d = (hand * (lip - a)) % math.pi
    return DRUM_R - SCOOP_H * d / math.pi


def _shell(part, key, x0, x1, y, z, arc, t, tile=0.12):
    """A curved plate between x0 and x1 about an X axis through (y, z), over
    `arc` (angle, radius) points, `t` thick toward the axis."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    run = [0.0]
    for (a0, r0), (a1, r1) in zip(arc, arc[1:]):
        run.append(run[-1] + math.hypot(r1 * math.cos(a1) - r0 * math.cos(a0), r1 * math.sin(a1) - r0 * math.sin(a0)))
    rows = []
    for a, r in arc:
        rows.append([bm.verts.new((x, y + rr * math.cos(a), z + rr * math.sin(a))) for rr in (r, r - t) for x in (x0, x1)])
    for k in range(len(arc) - 1):
        lo, hi = rows[k], rows[k + 1]
        for i, j in ((0, 1), (3, 2), (2, 0), (1, 3)):
            face = bm.faces.new([lo[i], lo[j], hi[j], hi[i]])
            for loop, (u, v) in zip(face.loops, ((run[k], i), (run[k], j), (run[k + 1], j), (run[k + 1], i))):
                xs = (x0, x1, x0, x1)[v]
                loop[uv].uv = (u / tile, xs / tile)
    for ring_ in (rows[0], rows[-1]):
        face = bm.faces.new([ring_[0], ring_[1], ring_[3], ring_[2]])
        for loop in face.loops:
            loop[uv].uv = (loop.vert.co.x / tile, loop.vert.co.z / tile)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    part.add(key, bm)


def _wall(part, key, x0, x1, y, z, outline):
    """A flat wall between x0 and x1 across the drum over a closed outline of (angle, radius) points."""
    pts = [_polar(y, z, r, a) for a, r in outline]
    part.add(key, _slab(pts, x0, x1))


def drum_part(end, s):
    """A drum pair at rest, over its arm's tip: on either side of the
    drum actuator, a bucket drum of four slices, each slice two scoops whose
    expanding-spiral shells end in a lip of riveted steel teeth over the
    opening, a baffle behind it; aluminium section walls between the
    slices following their outlines, end walls with the tie rods' nuts and
    the hub cap, and the bearing collars on the actuator."""
    c = shoulder(s) + Vector((0, 0, ARM))
    p = Part(f"drum_{end}", c)
    hand = lowering(s)
    y, z = c.y, c.z
    step = math.pi / SLICES
    for side in (-1, 1):
        inner, outer = side * DRUM_X, side * (DRUM_X + DRUM_W)
        lo, hi = sorted((inner, outer))
        wall, end_wall = 0.004, 0.006
        span = (hi - lo - 2 * end_wall - (SLICES - 1) * wall) / SLICES
        base = (0.0 if side > 0 else step / 2) + (math.pi / 2 if s > 0 else 0.0)
        x = lo + end_wall
        lips = []
        for k in range(SLICES):
            lip = base + k * step
            lips.append(lip)
            xa, xb = x, x + span
            for j in range(2):
                L = lip + j * math.pi
                arc = [(L - hand * math.pi * i / 20, DRUM_R - SCOOP_H * i / 20) for i in range(21)]
                _shell(p, "carbon", xa, xb, y, z, arc, 0.003)
                baffle = [(L - hand * math.radians(70) * i / 12, DRUM_R - SCOOP_H - 0.003 - 0.07 * i / 12) for i in range(13)]
                _shell(p, "carbon", xa + 0.001, xb - 0.001, y, z, baffle, 0.0025)
                _teeth(p, xa, xb, y, z, L, hand)
            x = xb + wall
        x = lo + end_wall
        for k in range(SLICES - 1):
            x += span
            outline = [(2 * math.pi * i / 144, max(_radius(2 * math.pi * i / 144, lips[k], hand), _radius(2 * math.pi * i / 144, lips[k + 1], hand)) - 0.001)
                       for i in range(144)]
            _wall(p, "alu", x, x + wall, y, z, outline)
            x += wall
        for k, (xa, xb) in ((0, (lo, lo + end_wall)), (SLICES - 1, (hi - end_wall, hi))):
            outline = [(2 * math.pi * i / 144, _radius(2 * math.pi * i / 144, lips[k], hand) - 0.001) for i in range(144)]
            _wall(p, "alu", xa, xb, y, z, outline)
        face = side
        out_x = outer
        _ring_x(p, "alu", 0.195, 0.185, *sorted((out_x, out_x + face * 0.003)), y, z, 72)
        for k in range(6):
            a = 2 * math.pi * (k + 0.5) / 6
            rib = box((0.004, 0.12, 0.009), (0, 0.125, 0), tile=0.1)
            p.add("alu", rib, Matrix.Translation((out_x + face * 0.002, y, z)) @ Matrix.Rotation(a, 4, "X"))
        _along_x(p, "alu", 0.06, *sorted((out_x, out_x + face * 0.012)), y, z, 40)
        p.add("black", turned([(0.0, 0.0), (0.04, 0.0), (0.038, 0.01), (0.0, 0.012)], 32), Matrix.Translation((out_x + face * 0.012, y, z)) @ at((0, 0, 0), (face, 0, 0)))
        _bolt_circle(p, out_x + face * 0.012, y, z, 0.05, 6, face, 0.004)
        for k in range(5):
            a = 2 * math.pi * k / 5 + 0.3
            for xx, f in ((out_x, face), (inner, -face)):
                p.add("steel", hexagon(0.007, 0.0, 0.006), at((xx, y + 0.12 * math.cos(a), z + 0.12 * math.sin(a)), (f, 0, 0)))
        _ring_x(p, "alu", 0.085, 0.061, *sorted((inner, inner - face * 0.004)), y, z, 48)
        _bolt_circle(p, inner - face * 0.004, y, z, 0.072, 8, -face, 0.003)
    return p


def _teeth(p, xa, xb, y, z, lip, hand):
    """Steel teeth along a scoop's lip: a strip riveted on the shell's
    outside just behind the lip, its points reaching out ahead of it."""
    w = xb - xa
    tangent = Vector((0, -math.sin(lip), math.cos(lip))) * hand
    radial = Vector((0, math.cos(lip), math.sin(lip)))
    m = Matrix((Vector((1, 0, 0)), tangent, radial)).transposed().to_4x4()
    m.translation = Vector((0, y, z)) + radial * (DRUM_R + 0.0015)
    p.add("steel", box((w - 0.004, 0.018, 0.0025), ((xa + xb) / 2, -0.009, 0), tile=0.1), m)
    n = 4
    for i in range(n):
        x0 = xa + 0.002 + (w - 0.004) * i / n
        x1 = xa + 0.002 + (w - 0.004) * (i + 1) / n
        tooth = [(x0, 0.0), (x1, 0.0), ((x0 + x1) / 2 + 0.002, 0.012), ((x0 + x1) / 2 - 0.002, 0.012)]
        p.add("steel", prism(tooth, -0.00125, 0.00125, 0.05), m)
        p.add("steel", turned([(0.0, 0.0), (0.0025, 0.0), (0.002, 0.0014), (0.0, 0.0018)], 8), m @ Matrix.Translation(((x0 + x1) / 2, -0.011, 0.00125)))


# Wear


def _caked(obj, axis_dist):
    """A `caked` attribute: 1 on the vertices farther than `axis_dist` from the body's X axis through its origin."""
    co = np.empty(len(obj.data.vertices) * 3, np.float32)
    obj.data.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    values = (np.hypot(co[:, 1], co[:, 2]) > axis_dist).astype(np.float32) if axis_dist else np.zeros(len(co), np.float32)
    obj.data.attributes.new("caked", "FLOAT", "POINT").data.foreach_set("value", values)


def _dust(g):
    """Dust and the sky's reach at the excavator's scale: settled on what
    faces up, thrown on what rides low, caked on the drums and the wheels'
    treads; and ambient occlusion over a few centimetres."""
    geo = g.node("ShaderNodeNewGeometry")
    pos = g.node("ShaderNodeSeparateXYZ", Vector=geo.outputs["Position"])
    nor = g.node("ShaderNodeSeparateXYZ", Vector=geo.outputs["Normal"])
    up = g.ramp(nor.outputs["Z"], 0.25, 0.95)
    facing = g.ramp(nor.outputs["Z"], -0.8, 0.4)
    low = g.math("SUBTRACT", 1.0, g.ramp(pos.outputs["Z"], 0.05, 0.45))
    patch = g.ramp(g.node("ShaderNodeTexNoise", Vector=geo.outputs["Position"], Scale=7.0, Detail=3.0, Roughness=0.5).outputs["Fac"], 0.35, 0.68)
    speck = g.ramp(g.node("ShaderNodeTexNoise", Vector=geo.outputs["Position"], Scale=90.0, Detail=2.0).outputs["Fac"], 0.3, 0.7)
    blotch = g.ramp(g.node("ShaderNodeTexNoise", Vector=geo.outputs["Position"], Scale=22.0, Detail=3.0, Roughness=0.6).outputs["Fac"], 0.42, 0.72)
    caked = g.node("ShaderNodeAttribute", _attribute_name="caked", _attribute_type="GEOMETRY").outputs["Fac"]
    settled = g.math("MULTIPLY", up, g.math("ADD", 0.3, g.math("MULTIPLY", patch, 0.6)))
    thrown = g.math("MULTIPLY", g.math("MULTIPLY", low, facing), g.math("ADD", 0.25, g.math("MULTIPLY", speck, 0.5)))
    clods = g.math("MULTIPLY", caked, g.math("ADD", 0.1, g.math("MULTIPLY", g.math("MULTIPLY", blotch, speck), 0.6)))
    dust = g.math("ADD", g.math("ADD", g.math("MULTIPLY", settled, 0.5), g.math("MULTIPLY", thrown, 0.35)), clods)
    dust = g.math("MINIMUM", dust, 0.8)
    ao = g.node("ShaderNodeAmbientOcclusion", Distance=0.05, _samples=16)
    return dust, ao.outputs["AO"]


def wear(objs, out_dir, size=4096, samples=8):
    """Bakes Mars wear over `objs` as they stand into one atlas: each takes
    a `bake` UV packed with the others, the colour, roughness, metal and
    normals of every material that is not lit are baked into it, and those
    materials give way to one look of the bake. Returns that look."""
    view = bpy.context.view_layer
    view.update()
    for o in view.objects:
        if o is not None:
            o.select_set(False)
    for o in objs:
        o.data.uv_layers.new(name="bake")
        o.data.uv_layers.active = o.data.uv_layers["bake"]
        o.select_set(True)
    view.objects.active = objs[0]
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(60), island_margin=0.002, scale_to_bounds=False)
    bpy.ops.object.mode_set(mode="OBJECT")
    images = {}
    for key, srgb in (("colour", True), ("rough", False), ("metal", False), ("normal", False)):
        res = size if key == "colour" else size // 2
        image = bpy.data.images.new(f"{NAME}_{key}", res, res, alpha=False)
        image.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
        images[key] = image
    used = {m for o in objs for m in o.data.materials if m is not None and not weathering.keeps_own(m)}
    graphs = {}
    for mat in used:
        work = mat.copy()
        work.name = f"{mat.name}_bake"
        g = weathering.Graph(work)
        st = g.node("ShaderNodeUVMap", _uv_map="st").outputs["UV"]
        dust, ao = _dust(g)
        albedo = weathering.worn(g, weathering.source(g, mat, "Base Color", st), dust, ao)
        rough, metal = weathering.dulled(g, weathering.source(g, mat, "Roughness", st), weathering.source(g, mat, "Metallic", st), dust)
        emit = g.node("ShaderNodeEmission", Strength=1.0)
        target = g.node("ShaderNodeTexImage")
        bsdf = next(n for n in g.nodes if n.type == "BSDF_PRINCIPLED")
        output = next(n for n in g.nodes if n.type == "OUTPUT_MATERIAL")
        graphs[mat] = (work, g, {"colour": albedo, "rough": rough, "metal": metal}, emit, target, bsdf, output)
    kept = {o: list(o.data.materials) for o in objs}
    for o in objs:
        for i, m in enumerate(kept[o]):
            if m in graphs:
                o.data.materials[i] = graphs[m][0]
    scene = bpy.context.scene
    engine = scene.render.engine
    scene.render.engine = "CYCLES"
    scene.cycles.samples = samples
    scene.cycles.device = "CPU"
    scene.render.bake.margin = 4
    for key, image in images.items():
        for work, g, outputs, emit, target, bsdf, output in graphs.values():
            target.image = image
            g.nodes.active = target
            if key == "normal":
                g.links.new(bsdf.outputs[0], output.inputs["Surface"])
                continue
            value = outputs[key]
            if value.type != "RGBA":
                value = g.node("ShaderNodeCombineColor", Red=value, Green=value, Blue=value).outputs[0]
            g.links.new(value, emit.inputs["Color"])
            g.links.new(emit.outputs[0], output.inputs["Surface"])
        bpy.ops.object.bake(type="NORMAL" if key == "normal" else "EMIT", normal_space="TANGENT")
        image.filepath_raw = os.path.join(out_dir, f"{NAME}_{key}.jpg")
        image.file_format = "JPEG"
        scene.render.image_settings.quality = 90
        image.save()
    scene.render.engine = engine
    look = weathering.baked_look(f"{NAME}_look", images["colour"], images["rough"], images["metal"], images["normal"])
    for o in objs:
        me = o.data
        mats = [look] + [m for m in kept[o] if m not in graphs]
        index = [0 if m in graphs else mats.index(m) for m in kept[o]]
        old = np.empty(len(me.polygons), np.int32)
        me.polygons.foreach_get("material_index", old)
        me.materials.clear()
        for m in mats:
            me.materials.append(m)
        me.polygons.foreach_set("material_index", np.array(index, np.int32)[old])
        me.uv_layers.remove(me.uv_layers["st"])
        if "caked" in me.attributes:
            me.attributes.remove(me.attributes["caked"])
    for work, *_ in graphs.values():
        bpy.data.materials.remove(work)
    for o in objs:
        o.select_set(False)
    return look


def looks(work, weather=True):
    """The excavator's bodies as objects at their places, posed carrying,
    in a collection of their own; with `weather` their looks are baked with
    wear. Returns (collection, {body: object})."""
    mats = materials(kit.Looks(TEXTURES, os.path.join(work, "maps")))
    parts = [chassis_part()] + [wheel_part(name, hub) for name, hub in wheels()]
    for end, s in ENDS:
        parts += [arm_part(end, s), drum_part(end, s)]
    scene = bpy.data.collections.new(f"{NAME}_geom")
    bpy.context.scene.collection.children.link(scene)
    objs = {}
    for part in parts:
        obj = part.finish(mats, scene)
        _caked(obj, 0.19 if part.name.startswith("drum") else (0.155 if part.name.startswith("wheel") else 0.0))
        objs[part.name] = obj
    carried = pose(arms=(CARRY, CARRY))
    for name, obj in objs.items():
        obj.matrix_world = carried[name]
    if weather:
        bake = os.path.join(work, "bake")
        os.makedirs(bake, exist_ok=True)
        wear(list(objs.values()), bake)
    return scene, objs


# Machine


def _joint(u, name, body0, body1, pos0, limits, drive, motor, interface):
    u.emit(2, f'def PhysicsRevoluteJoint "{name}" (')
    u.emit(3, 'prepend apiSchemas = ["PhysicsDriveAPI:angular"]')
    u.emit(2, ")")
    u.emit(2, "{")
    u.emit(3, f"rel physics:body0 = </robot/{body0}>")
    u.emit(3, f"rel physics:body1 = </robot/{body1}>")
    u.emit(3, 'uniform token physics:axis = "X"')
    u.emit(3, f"point3f physics:localPos0 = {fmt(pos0)}")
    u.emit(3, "point3f physics:localPos1 = (0, 0, 0)")
    u.emit(3, "quatf physics:localRot0 = (1, 0, 0, 0)")
    u.emit(3, "quatf physics:localRot1 = (1, 0, 0, 0)")
    u.emit(3, "bool physics:collisionEnabled = 0")
    if limits:
        u.emit(3, f"float physics:lowerLimit = {limits[0]}")
        u.emit(3, f"float physics:upperLimit = {limits[1]}")
    kind, stiffness, damping, max_force = drive
    u.emit(3, 'uniform token drive:angular:physics:type = "force"')
    u.emit(3, f"float drive:angular:physics:stiffness = {stiffness}")
    u.emit(3, f"float drive:angular:physics:damping = {damping}")
    u.emit(3, f"float drive:angular:physics:maxForce = {max_force}")
    u.emit(3, f"float drive:angular:physics:target{'Position' if kind == 'position' else 'Velocity'} = 0")
    u.emit(3, f'uniform token[] gearbox:joint:angular:commandInterfaces = ["{interface}"]')
    if motor:
        u.emit(3, f'token gearbox:device:name = "{name}"')
        u.emit(3, f"float gearbox:motor:maxVelocity = {motor[0]}")
        u.emit(3, f"float gearbox:motor:acceleration = {motor[1]}")
    u.emit(2, "}")


def _body(u, name, at_, mass, inertia, link, role, values=()):
    u.emit(1, f'def Xform "{name}" (')
    u.emit(2, 'prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]')
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(2, f'token gearbox:link:name = "{link}"')
    u.emit(2, f'token gearbox:link:role = "{role}"')
    u.emit(2, f"float physics:mass = {mass}")
    u.emit(2, f"float3 physics:diagonalInertia = {fmt(inertia)}")
    for value in values:
        u.emit(2, value)
    u.emit(2, f"double3 xformOp:translate = {fmt(at_)}")
    u.emit(2, 'uniform token[] xformOpOrder = ["xformOp:translate"]')


def _cube(u, name, centre, size):
    u.emit(2, f'def Cube "{name}" (')
    u.emit(3, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(2, ")")
    u.emit(2, "{")
    u.emit(3, 'token visibility = "invisible"')
    u.emit(3, "double size = 1")
    u.emit(3, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(3, f"float3 xformOp:scale = {fmt(size)}")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:scale"]')
    u.emit(2, "}")


def _cylinder(u, name, radius, height):
    u.emit(2, f'def Cylinder "{name}" (')
    u.emit(3, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(2, ")")
    u.emit(2, "{")
    u.emit(3, 'token visibility = "invisible"')
    u.emit(3, 'uniform token axis = "X"')
    u.emit(3, f"double radius = {radius}")
    u.emit(3, f"double height = {height}")
    u.emit(2, "}")


def _sensor(u, name, kind, at_, rotate, attrs):
    u.emit(2, f'def Xform "{name}" (prepend apiSchemas = ["GearboxLinkAPI"])')
    u.emit(2, "{")
    u.emit(3, 'token gearbox:link:role = "sensor"')
    u.emit(3, f'token gearbox:sensor:kind = "{kind}"')
    for attr in attrs:
        u.emit(3, attr)
    u.emit(3, f"double3 xformOp:translate = {fmt(at_)}")
    u.emit(3, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
    u.emit(2, "}")


CAMERA = ['token gearbox:sensor:channels = "color_depth"', 'token gearbox:sensor:render = "geometry"', "int gearbox:sensor:width = 160",
          "int gearbox:sensor:height = 120", "float gearbox:sensor:vfov_deg = 62", "float gearbox:sensor:range_m = 20",
          "float gearbox:sensor:rate_hz = 5"]


def machine(bound):
    """The machine layer: bodies, joints and their drives, the skid-steer
    drive, a position controller on each arm and a velocity controller on
    each drum, and the sensors."""
    u = Usda("")
    o = origins()
    spin = [f"</robot/Joints/spin_{name}>" for name, _ in wheels()]
    controllers = ["drive"] + [f"{kind}_{end}" for kind in ("arm", "drum") for end, _ in ENDS]
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, 'upAxis = "Z"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, "customLayerData = {")
    u.emit(2, 'string creator = "scripts/mars_base/excavator.py: a RASSOR-class regolith excavator with counter-rotating bucket drums"')
    u.emit(1, "}")
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/excavator.py and build again. Drive it with `gearbox machine move --machine <id> ...`;")
    u.emit(0, f"# `gearbox machine set-value arm_front position {CARRY}` carries a drum, `... set-value drum_front velocity {DRUM_RATE}` digs.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'PhysicsArticulationRootAPI', *(f'GearboxControllerAPI:{c}' for c in controllers)])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, 'token gearbox:machine:interfaceVersion = "0.1"')
    u.emit(1, f'token gearbox:machine:kind = "{NAME}"')
    u.emit(1, 'token gearbox:machine:idPolicy = "prim_path"')
    u.emit(1, 'token gearbox:machine:upAxis = "Z"')
    u.emit(1, "rel gearbox:machine:body = </robot/chassis>")
    u.emit(1, "rel gearbox:machine:controlFrame = </robot/chassis/control_frame>")
    u.emit(1, f"rel gearbox:machine:role:poweredWheelJoints = [{', '.join(spin)}]")
    u.emit(1, f"rel gearbox:machine:role:brakeJoints = [{', '.join(spin)}]")
    u.emit(1, "bool gearbox:controller:drive:enabled = true")
    u.emit(1, 'token gearbox:controller:drive:type = "builtin:diff_drive_cmd_vel"')
    u.emit(1, 'token gearbox:controller:drive:driveMode = "wheels"')
    u.emit(1, f"float gearbox:controller:drive:wheelRadius = {WHEEL_R - 0.002}")
    u.emit(1, f"float gearbox:controller:drive:trackWidth = {2 * WHEEL_X}")
    u.emit(1, f"float gearbox:controller:drive:wheelBase = {2 * WHEEL_Y}")
    u.emit(1, "float gearbox:controller:drive:maxWheelTorqueNm = 45")
    u.emit(1, "float gearbox:controller:drive:maxPowerKw = 0.4")
    u.emit(1, "float gearbox:controller:drive:updateRateHz = 60")
    u.emit(1, 'token gearbox:controller:drive:namespacePolicy = "machine_id"')
    u.emit(1, 'token gearbox:controller:drive:commandInterface = "cmd_vel"')
    u.emit(1, 'token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity", "joint_state"]')
    u.emit(1, 'token gearbox:controller:drive:frameConvention = "usd_z_up"')
    u.emit(1, "rel gearbox:controller:drive:body = </robot/chassis>")
    for c in controllers[1:]:
        kind = "position" if c.startswith("arm") else "velocity"
        u.emit(1, f"bool gearbox:controller:{c}:enabled = true")
        u.emit(1, f'token gearbox:controller:{c}:type = "builtin:joint_{kind}"')
        u.emit(1, f'token gearbox:controller:{c}:commandInterface = "{kind}"')
        u.emit(1, 'token[] gearbox:controller:' + c + ':stateInterfaces = ["position", "velocity"]')
        u.emit(1, f"rel gearbox:controller:{c}:target = </robot/Joints/{c}>")
        u.emit(1, f"rel gearbox:controller:{c}:body = </robot/chassis>")
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{NAME}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")

    _body(u, "chassis", (0, 0, 0), MASS["chassis"], box_inertia(MASS["chassis"], (0.33, 0.6, 0.33)), "base_link", "base",
          ["float3 physics:centerOfMass = (0, 0, 0.24)"])
    u.emit(2, 'def Xform "control_frame"')
    u.emit(2, "{")
    u.emit(3, "quatd xformOp:orient = (0.7071067811865476, 0, 0, -0.7071067811865476)")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:orient"]')
    u.emit(2, "}")
    visual(u, 2, NAME, "chassis", bound)
    _cube(u, "frame_collider", (0, 0, 0.19), (0.33, 0.6, 0.23))
    _cube(u, "box_collider", (0, 0, 0.36), (0.25, 0.33, 0.11))
    _sensor(u, "imu_link", "imu", (0, 0, 0.2), (0, 0, 0), ["float gearbox:sensor:rate_hz = 100"])
    for end, s in ENDS:
        _sensor(u, f"{end}_camera", "camera", (0.105, s * 0.218, 0.335), (0, 15, -90 if s < 0 else 90), CAMERA)
    _sensor(u, "lidar", "lidar", (0, 0, 0.462), (0, 0, -90), [
        "int gearbox:sensor:rows = 16",
        "int gearbox:sensor:columns = 360",
        "float gearbox:sensor:hfov_deg = 360",
        "float gearbox:sensor:vfov_deg = 30",
        "float gearbox:sensor:range_m = 40",
        "float gearbox:sensor:rate_hz = 5",
    ])
    u.emit(1, "}")

    r = WHEEL_R - 0.002
    for name, hub in wheels():
        m = MASS["wheel"]
        _body(u, f"wheel_{name}", hub, m, (m * r * r / 2, m * (3 * r * r + WHEEL_W ** 2) / 12, m * (3 * r * r + WHEEL_W ** 2) / 12), f"wheel_{name}", "wheel", [
            'token gearbox:tyre:contactType = "solid"',
            f"float gearbox:value:rolling_radius_m = {r}",
            f"float gearbox:value:tyre_width_m = {WHEEL_W}",
            "float gearbox:value:tyre_pressure_bar = 1.8",
            "float gearbox:value:tyre_min_pressure_bar = 1.8",
            "float gearbox:value:tyre_max_pressure_bar = 1.8",
            "float gearbox:value:tyre_carcass_stiffness_pa_m = 4000000",
            "float gearbox:value:tyre_tread_stiffness_n_m3 = 26000000",
            "float gearbox:value:tyre_damping_ratio = 0.8",
            "float gearbox:value:tyre_hysteresis_fraction = 0.02",
            f"int gearbox:value:tyre_axle = {1 if hub.y < 0 else 2}",
        ])
        visual(u, 2, NAME, f"wheel_{name}", bound)
        _cylinder(u, "tire_collider", r, WHEEL_W)
        u.emit(1, "}")
    for end, s in ENDS:
        m = MASS["arm"]
        _body(u, f"arm_{end}", o[f"arm_{end}"], m, box_inertia(m, (0.13, 0.08, ARM)), f"arm_{end}", "link",
              [f"float gearbox:value:range = {lowering(s) * math.radians(TRAVEL_DEG):.5f}", "float3 physics:centerOfMass = (0, 0, 0.36)"])
        visual(u, 2, NAME, f"arm_{end}", bound)
        _cube(u, "beam_collider", (0, 0, ARM / 2), (0.05, 0.08, ARM - 0.14))
        u.emit(1, "}")
        m = MASS["drum"]
        length = 2 * (DRUM_X + DRUM_W)
        _body(u, f"drum_{end}", o[f"drum_{end}"], m, (0.6 * m * DRUM_R ** 2, m * (3 * DRUM_R ** 2 + length ** 2) / 12, m * (3 * DRUM_R ** 2 + length ** 2) / 12),
              f"drum_{end}", "link")
        visual(u, 2, NAME, f"drum_{end}", bound)
        _cylinder(u, "shell_collider", DRUM_R, length)
        u.emit(1, "}")

    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    for name, hub in wheels():
        _joint(u, f"spin_{name}", "chassis", f"wheel_{name}", hub, None, ("velocity", 0, 6, 45), None, "velocity")
    for end, s in ENDS:
        travel = (-2.0, TRAVEL_DEG) if lowering(s) > 0 else (-TRAVEL_DEG, 2.0)
        _joint(u, f"arm_{end}", "chassis", f"arm_{end}", o[f"arm_{end}"], travel, ("position", 60, 5, 250), (0.4, 0.8), "position")
        _joint(u, f"drum_{end}", f"arm_{end}", f"drum_{end}", (0, 0, ARM), None, ("velocity", 0, 4, 90), (2.6, 2.0), "velocity")
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def build(work):
    """Builds the excavator's looks, writes them as `<NAME>_geom.usdc` and
    the machine as `<NAME>.usda` into `work`; returns {NAME: machine layer}."""
    kept = (set(bpy.data.materials), set(bpy.data.images), set(bpy.data.meshes))
    os.makedirs(work, exist_ok=True)
    scene, objs = looks(work)
    for obj in objs.values():
        obj.matrix_world = Matrix.Identity(4)
    geom = os.path.join(work, f"{NAME}_geom.usdc")
    export_geom(scene, geom)
    bound = bindings(geom)
    for obj in objs.values():
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
        f.write(machine(bound))
    return {NAME: path}
