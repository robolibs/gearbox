"""The mobile harvester of the arch greenhouses: a robot that drives the
aisle beside a row of tables and sows, harvests and clears the trays on
them with a collaborative arm, after the mobile manipulators that work
greenhouse benches (Clearpath's Husky and Ridgeback carrying UR arms, MiR
with UR, Saga's Thorvald, Iron Ox's Grover, Root AI's Virgo, Tortuga's
pickers) and the lift columns built for UR arms (Ewellix's LIFTKIT).

    base     a skid-steered, four-wheel-drive tub on solid tyres, its wheels
             in wells under a deck housing with a status light band,
             bumpers with safety edges, two corner safety laser scanners
             seeing all round, a depth camera ahead, e-stops at both ends,
             an operator panel and charging contacts astern
    column   a two-stage telescopic lift on the deck's left, carrying the
             arm 0.4 m up over tall canopy; its control unit beside it
    arm      a UR10e (`arms.UR10E`) on the column, reaching every place of
             a 1.3 m by 1.35 m table from the aisle edge
    tool     a parallel gripper with soft food-grade fingers, a stem
             cutter, a suction cup on a spring plunger for plugs and seed,
             a wrist depth camera naming the plants and a light bar
    deck     a harvest crate astern, a waste bin and a seed magazine

Frame, as the other wheeled machines: +X left, +Y astern, +Z up, the
machine driving toward -Y, the tyres touching Z = 0. A `builtin:
diff_drive_cmd_vel` controller drives the four wheels; `lift`, the arm's
six joints and `grip` take `builtin:joint_position` positions 0..1 of their
ranges (`lift` and `grip` move both their joints together).

Its looks are built in Blender and written into a geometry layer each
body's visual references; the bodies collide as hidden boxes and drums.
"""
import math
import os
from dataclasses import dataclass, field

import bmesh
from mathutils import Matrix, Vector

import arms
import kit
import mechanics as mk
from kit import box, tube
from mechanics import Part, at, cyl, turned
from usda import Usda, end, fmt, schemas

# Key: (material name, sRGB colour, roughness, metal, map settings or None, glow or None).
FINISHES = {
    "rubber": ("harvester_rubber", (0.05, 0.05, 0.052), 0.82, 0.0, {"seed": 91, "grime": 0.05}, None),
    "tyre": ("harvester_tyre", (0.16, 0.16, 0.17), 0.75, 0.0, {"seed": 92, "grime": 0.08}, None),
    "yellow": ("harvester_yellow", (0.92, 0.70, 0.06), 0.45, 0.0, {"seed": 93, "grime": 0.05}, None),
    "red": ("harvester_red", (0.72, 0.05, 0.04), 0.35, 0.0, {"seed": 94, "grime": 0.03}, None),
    "crate": ("harvester_crate", (0.16, 0.34, 0.20), 0.6, 0.0, {"seed": 95, "grime": 0.06}, None),
    "bin": ("harvester_bin", (0.30, 0.31, 0.33), 0.65, 0.0, {"seed": 96, "grime": 0.08}, None),
    "soft": ("harvester_silicone", (0.20, 0.42, 0.78), 0.55, 0.0, {"seed": 97, "grime": 0.03}, None),
    "copper": ("harvester_copper", (0.80, 0.50, 0.30), 0.3, 1.0, {"seed": 98, "grime": 0.05}, None),
    "foam": ("harvester_liner", (0.62, 0.63, 0.62), 0.9, 0.0, {"seed": 99, "grime": 0.06}, None),
    "status": ("harvester_status", (0.25, 0.95, 0.55), 0.3, 0.0, None, ((0.25, 0.95, 0.55), 4.0)),
    "amber": ("harvester_amber", (1.0, 0.62, 0.1), 0.3, 0.0, None, ((1.0, 0.62, 0.1), 1.5)),
    "beacon": ("harvester_beacon", (0.85, 0.08, 0.06), 0.3, 0.0, None, ((0.85, 0.08, 0.06), 0.6)),
    "screen": ("harvester_screen", (0.10, 0.30, 0.42), 0.1, 0.0, None, ((0.10, 0.30, 0.42), 1.2)),
}
ROLLS = (("front_left", 1, -1), ("front_right", -1, -1), ("rear_left", 1, 1), ("rear_right", -1, 1))
# The safety scanners: the corner each stands in (sign of x, of y) and its heading, degrees.
SCANNERS = ((1, -1, -45.0), (-1, 1, 135.0))
# What stands on the deck: its corners (x, y) in the machine's frame and its height.
BIN = ((-0.33, -0.50), (-0.05, -0.20), 0.22)
MAGAZINE = ((-0.32, -0.16), (-0.08, 0.06), 0.10)
CRATE = ((-0.30, 0.11), (0.30, 0.51), 0.22)
ARM = arms.UR10E
# The arm's working ranges (degrees), and its rest: folded over the deck, the tool in its holster.
ARM_RANGES = {"shoulder_pan": (-80.0, 260.0), "shoulder_lift": (-150.0, 30.0), "elbow": (-15.0, 165.0),
              "wrist_1": (-360.0, 0.0), "wrist_2": (-260.0, 80.0), "wrist_3": (-170.0, 170.0)}
ARM_REST = (95.0, -25.0, 156.0, -211.0, -90.0, 5.0)
# The arm's ready pose (degrees): upright over the deck, the tool clear above it.
ARM_READY = (95.0, -90.0, 30.0, -120.0, -90.0, 5.0)


@dataclass
class Harvester:
    """The harvester's sizes, metres, in its frame at rest."""
    wheel_r: float = 0.15
    tyre_w: float = 0.10
    track: float = 0.60
    base: float = 0.56
    half: tuple = (0.35, 0.53)
    tub: float = 0.235
    skirt: float = 0.09
    shell: float = 0.32
    deck: float = 0.40
    column: tuple = (0.19, -0.18)
    outer: float = 0.45
    stroke: float = 0.20
    plate: float = 0.92
    tcp: float = 0.25
    jaw: tuple = (0.04, 0.08)
    masses: dict = field(default_factory=lambda: {"chassis": 220.0, "wheel": 5.0, "lift_stage": 5.0, "carriage": 8.0, "tool": 1.2, "jaw": 0.2})


HARVESTER = Harvester()


def mount(h):
    """The arm's base_link in the carriage's frame, and the carriage's place in the machine's at rest."""
    x, y = h.column
    return Matrix.Translation((0, 0, h.plate - (h.deck + 0.07))), Vector((x, y, h.deck + 0.07))


def arm_base(h):
    """The arm's base_link in the machine's frame at rest."""
    local, carriage = mount(h)
    return Matrix.Translation(carriage) @ local


def tool_frame(h, q=ARM_REST):
    """The tool's frame (tool0) in the machine's frame for arm angles `q`."""
    return arms.frames(ARM, arm_base(h), q)["tool0"]


def scanner_at(h, sx, sy):
    """Where a safety scanner's scan plane centre stands, in its corner notch."""
    return Vector((sx * (h.half[0] - 0.045), sy * (h.half[1] - 0.04), 0.2))


def tcp(h):
    """The tool centre point between the fingertips, in tool0's frame."""
    return Matrix.Translation((0, 0, h.tcp))


def placed(h, q=ARM_REST, lift=0.0, grip=0.0):
    """Where each body's Part stands in the machine's frame with the arm at
    `q` (degrees) and the lift and grip at positions 0..1."""
    local, origin = mount(h)
    x, y = h.column
    up = Vector((0, 0, lift * h.stroke))
    out = {"chassis": Matrix.Identity(4), "lift_stage": Matrix.Translation(Vector((x, y, h.deck + 0.045)) + up),
           "carriage": Matrix.Translation(origin + 2 * up), f"{ARM.kind}_base": Matrix.Translation(origin + 2 * up)}
    for name, sx, sy in ROLLS:
        out[f"wheel_{name}"] = Matrix.Translation((sx * h.track / 2, sy * h.base / 2, h.wheel_r))
    frames = arms.frames(ARM, Matrix.Translation(origin + 2 * up) @ local, q)
    for link in ARM.links:
        out[link.name] = frames[link.name]
    out["tool"] = frames["tool0"]
    for side, s in (("left", 1), ("right", -1)):
        out[f"jaw_{side}"] = frames["tool0"] @ Matrix.Translation((s * (h.jaw[0] + grip * h.jaw[1]), 0, 0.115))
    return out


# Looks


def materials(looks):
    mats = mk.materials(looks)
    mats.update(arms.materials(looks))
    for key, (name, colour, rough, metal, maps, glow) in FINISHES.items():
        mats[key] = looks.get(name, colour, rough, metal, maps=mk.finish(colour, rough, **maps) if maps else None, emit=glow)
    return mats


def rounded(w, d, r, n=3):
    """A rounded rectangle's outline, w across x and d across y, corners of radius r."""
    pts = []
    for cx, cy, a0 in ((w / 2 - r, d / 2 - r, 0.0), (-w / 2 + r, d / 2 - r, 90.0), (-w / 2 + r, -d / 2 + r, 180.0), (w / 2 - r, -d / 2 + r, 270.0)):
        for k in range(n + 1):
            a = math.radians(a0 + 90.0 * k / n)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def upright(part, key, x, y, z0, z1, w, d, r):
    """A vertical rounded-square tube from z0 to z1, capped."""
    part.add(key, kit.prism(rounded(w, d, r), z0, z1, 0.4), Matrix.Translation((x, y, 0)))


def estop(part, m):
    """An emergency stop: a yellow housing on a face, its red mushroom along the builder's +Z."""
    part.add("yellow", box((0.06, 0.06, 0.035), (0, 0, 0.0175), tile=0.2, bevel=0.005), m)
    part.add("yellow", cyl(0.024, 0.035, 0.042, 20), m)
    part.add("red", turned([(0.0, 0.042), (0.011, 0.042), (0.011, 0.05), (0.02, 0.052), (0.021, 0.058), (0.017, 0.064), (0.0, 0.066)], 24), m)


def scanner(part, m):
    """A safety laser scanner, its window looking along the builder's +X at z = 0."""
    part.add("yellow", box((0.07, 0.08, 0.085), (-0.03, 0, 0.0), tile=0.2, bevel=0.006), m)
    part.add("lens", cyl(0.034, -0.017, 0.017, 28), m)
    part.add("black", cyl(0.036, 0.017, 0.03, 28), m)
    part.add("black", cyl(0.036, -0.03, -0.017, 28), m)
    part.add("plastic", cyl(0.008, -0.075, -0.065, 10), m @ Matrix.Translation((0, 0.02, -0.03)) @ Matrix.Rotation(math.pi / 2, 4, "Y"))


def depth_camera(part, m, w=0.124, d=0.029, h=0.026):
    """A stereo depth camera looking along the builder's +Y: aluminium body, black glass, lenses and projector."""
    part.add("alu", box((w, d, h), (0, 0, 0), tile=0.2, bevel=0.006), m)
    part.add("lens", box((w - 0.012, 0.002, h - 0.008), (0, d / 2, 0), tile=0.2), m)
    for x in (-0.047 * w / 0.124, 0.047 * w / 0.124, 0.0):
        part.add("black", cyl(0.006 if x else 0.004, 0.0, 0.002, 14), m @ Matrix.Translation((x, d / 2 + 0.001, 0)) @ Matrix.Rotation(-math.pi / 2, 4, "X"))


def crate(part, lo, hi, key="crate"):
    """A ventilated stacking crate between corners `lo` and `hi`: floor, walls,
    rim, ribs, vent slots, hand holds and a liner."""
    (x0, y0, z0), (x1, y1, z1) = lo, hi
    t = 0.008
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    part.add(key, box((x1 - x0 - 2 * t, y1 - y0 - 2 * t, 0.012), (cx, cy, z0 + 0.016), tile=0.3))
    for s in (-1, 1):
        part.add(key, box((x1 - x0, t, z1 - z0 - 0.01), (cx, cy + s * ((y1 - y0) / 2 - t / 2), (z0 + z1 - 0.01) / 2), tile=0.3))
        part.add(key, box((t, y1 - y0 - 2 * t, z1 - z0 - 0.01), (cx + s * ((x1 - x0) / 2 - t / 2), cy, (z0 + z1 - 0.01) / 2), tile=0.3))
        part.add(key, box((x1 - x0 + 0.008, 0.016, 0.014), (cx, cy + s * ((y1 - y0) / 2 - 0.004), z1 - 0.007), tile=0.3, bevel=0.003))
        part.add(key, box((0.016, y1 - y0 + 0.008, 0.014), (cx + s * ((x1 - x0) / 2 - 0.004), cy, z1 - 0.007), tile=0.3, bevel=0.003))
        for k in range(7):
            x = x0 + (x1 - x0) * (k + 1) / 8
            part.add(key, box((0.008, 0.006, z1 - z0 - 0.03), (x, cy + s * ((y1 - y0) / 2 + 0.002), (z0 + z1) / 2 - 0.008), tile=0.3))
            if k < 6:
                xs = x0 + (x1 - x0) * (k + 1.5) / 8
                for zs in (z0 + 0.05, z0 + 0.1):
                    part.add("plastic", box((0.04, 0.003, 0.018), (xs, cy + s * ((y1 - y0) / 2 + 0.0005), zs), tile=0.2))
        for k in range(3):
            y = y0 + (y1 - y0) * (k + 1) / 4
            part.add(key, box((0.006, 0.008, z1 - z0 - 0.03), (cx + s * ((x1 - x0) / 2 + 0.002), y, (z0 + z1) / 2 - 0.008), tile=0.3))
        part.add("plastic", box((0.003, 0.11, 0.03), (cx + s * ((x1 - x0) / 2 + 0.0005), cy, z1 - 0.04), tile=0.2, bevel=0.001))
        for sy in (-1, 1):
            part.add(key, box((0.03, 0.03, z1 - z0), (cx + s * ((x1 - x0) / 2 - 0.012), cy + sy * ((y1 - y0) / 2 - 0.012), (z0 + z1) / 2), tile=0.3, bevel=0.004))
    part.add("foam", box((x1 - x0 - 2 * t - 0.004, y1 - y0 - 2 * t - 0.004, 0.012), (cx, cy, z0 + 0.028), tile=0.3))


def bin_(p, top):
    """The waste bin for cleared plants: walls with a rolled rim, a grip
    recess at each end, ribs, and the drain grid in its floor."""
    (x0, y0), (x1, y1), hgt = BIN
    cx, cy, z1, t = (x0 + x1) / 2, (y0 + y1) / 2, top + hgt, 0.006
    p.add("bin", box((x1 - x0 - 2 * t, y1 - y0 - 2 * t, 0.01), (cx, cy, top + 0.005), tile=0.3))
    for s in (-1, 1):
        p.add("bin", box((x1 - x0, t, hgt), (cx, cy + s * ((y1 - y0) / 2 - t / 2), top + hgt / 2), tile=0.3))
        p.add("bin", box((t, y1 - y0, hgt), (cx + s * ((x1 - x0) / 2 - t / 2), cy, top + hgt / 2), tile=0.3))
        p.add("plastic", box((0.003, 0.09, 0.025), (cx + s * ((x1 - x0) / 2 + 0.0005), cy, z1 - 0.035), tile=0.2, bevel=0.001))
        for k in range(3):
            p.add("bin", box((0.008, 0.005, hgt - 0.03), (x0 + (x1 - x0) * (k + 1) / 4, cy + s * ((y1 - y0) / 2 + 0.002), top + hgt / 2 - 0.01), tile=0.3))
    for s in (-1, 1):
        p.add("bin", box((x1 - x0 + 0.012, 0.018, 0.014), (cx, cy + s * ((y1 - y0) / 2 - 0.003), z1 - 0.007), tile=0.3, bevel=0.004))
        p.add("bin", box((0.018, y1 - y0 + 0.012, 0.014), (cx + s * ((x1 - x0) / 2 - 0.003), cy, z1 - 0.007), tile=0.3, bevel=0.004))
    for k in range(5):
        p.add("plastic", box((x1 - x0 - 0.05, 0.006, 0.003), (cx, y0 + 0.03 + k * (y1 - y0 - 0.06) / 4, top + 0.0115), tile=0.2))


def magazine(p, top):
    """The seed magazine: its case, a seed canister with a screw lid, the
    metering wheel's motor, a vibrating pick-up dish open to the suction cup."""
    (x0, y0), (x1, y1), hgt = MAGAZINE
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    p.add("grey", box((x1 - x0, y1 - y0, hgt), (cx, cy, top + hgt / 2), tile=0.3, bevel=0.008))
    p.add("plastic", box((x1 - x0 + 0.002, 0.004, 0.012), (cx, y0 - 0.001, top + 0.07), tile=0.2))
    can = Matrix.Translation((cx - 0.05, cy + 0.04, 0))
    p.add("black", cyl(0.05, top + hgt, top + hgt + 0.012, 32), can)
    p.add("paint", turned([(0.0, top + hgt + 0.012), (0.045, top + hgt + 0.012), (0.045, top + hgt + 0.13), (0.043, top + hgt + 0.134), (0.0, top + hgt + 0.134)], 32), can)
    p.add("plastic", box((0.004, 0.012, 0.1), (cx - 0.05 - 0.0445, cy + 0.04, top + hgt + 0.07), tile=0.2))
    p.add("black", turned([(0.0, top + hgt + 0.13), (0.048, top + hgt + 0.13), (0.048, top + hgt + 0.15), (0.044, top + hgt + 0.155), (0.0, top + hgt + 0.155)], 32), can)
    for k in range(16):
        a = 2 * math.pi * k / 16
        p.add("black", box((0.004, 0.006, 0.018), (0.048 * math.cos(a), 0.048 * math.sin(a), top + hgt + 0.14), tile=0.2), can)
    dish = Matrix.Translation((cx + 0.06, cy - 0.04, 0))
    p.add("black", cyl(0.042, top + hgt, top + hgt + 0.02, 28), dish)
    p.add("steel", turned([(0.0, top + hgt + 0.02), (0.038, top + hgt + 0.02), (0.038, top + hgt + 0.032), (0.034, top + hgt + 0.032),
                           (0.012, top + hgt + 0.024), (0.0, top + hgt + 0.024)], 28), dish)
    p.add("steel", tube([(cx - 0.05, cy + 0.04 - 0.045, top + hgt + 0.03), (cx + 0.02, cy - 0.01, top + hgt + 0.035), (cx + 0.05, cy - 0.035, top + hgt + 0.033)],
                        0.006, 8))
    mk.motor(p, at((x0 - 0.002, cy + 0.04, top + 0.05), (1, 0, 0)), 0.035, 0.04)
    p.add("plastic", tube([(x0 + 0.04, y1 + 0.001, top + 0.05), (x0 + 0.04, y1 + 0.03, top + 0.012), (x0 + 0.15, y1 + 0.03, top + 0.007)], 0.005, 8))


def holster(p, h):
    """The tool's rest: a post under its fingertips at the arm's rest pose,
    a rubber cradle the closed fingers sit in."""
    tip = tool_frame(h) @ Vector((0, 0, h.tcp + 0.03))
    top = h.deck + 0.006
    p.add("black", box((0.07, 0.07, 0.01), (tip.x, tip.y, top + 0.005), tile=0.2, bevel=0.002))
    mk.bolts(p, Matrix.Translation((tip.x, tip.y, top + 0.01)), [(sx * 0.025, sy * 0.025) for sx in (-1, 1) for sy in (-1, 1)], 0.004)
    p.add("alu", cyl(0.012, top + 0.01, tip.z - 0.02, 16), Matrix.Translation((tip.x, tip.y, 0)))
    p.add("rubber", turned([(0.0, tip.z - 0.03), (0.03, tip.z - 0.03), (0.034, tip.z - 0.005), (0.026, tip.z - 0.005), (0.018, tip.z - 0.018), (0.0, tip.z - 0.018)], 24),
          Matrix.Translation((tip.x, tip.y, 0)))


def deck_items(p, h):
    """What stands on the deck: the crate in its locating frame astern, the
    waste bin, the seed magazine and the tool's holster."""
    top = h.deck + 0.006
    (x0, y0), (x1, y1), hgt = CRATE
    for sx in (-1, 1):
        for sy in (-1, 1):
            cx, cy = sx * (x1 + 0.005), ((y0 + y1) / 2 + sy * ((y1 - y0) / 2 + 0.005))
            p.add("alu", box((0.03, 0.004, 0.05), (cx - sx * 0.013, cy, top + 0.025), tile=0.2))
            p.add("alu", box((0.004, 0.03, 0.05), (cx, cy - sy * 0.013, top + 0.025), tile=0.2))
    p.add("black", box((x1 - x0 - 0.02, y1 - y0 - 0.02, 0.01), (0.0, (y0 + y1) / 2, top + 0.005), tile=0.3))
    crate(p, (x0, y0, top + 0.01), (x1, y1, top + 0.01 + hgt))
    bin_(p, top)
    magazine(p, top)
    holster(p, h)


def column_base(p, h):
    """The lift's outer stage on its flange, and its control unit."""
    x, y = h.column
    top = h.deck + 0.006
    p.add("black", box((0.26, 0.26, 0.014), (x, y, top + 0.007), tile=0.3, bevel=0.003))
    mk.bolts(p, Matrix.Translation((x, y, top + 0.014)), [(sx * 0.11, sy * 0.11) for sx in (-1, 1) for sy in (-1, 1)], 0.008)
    upright(p, "paint", x, y, top + 0.014, h.deck + h.outer, 0.20, 0.20, 0.03)
    p.add("black", kit.prism(rounded(0.205, 0.205, 0.032), h.deck + h.outer - 0.03, h.deck + h.outer, 0.3), Matrix.Translation((x, y, 0)))
    p.add("plastic", kit.prism(rounded(0.2, 0.2, 0.03), h.deck + h.outer, h.deck + h.outer + 0.004, 0.3), Matrix.Translation((x, y, 0)))
    p.add("grey", box((0.003, 0.08, 0.05), (x - 0.1015, y - 0.02, h.deck + 0.3), tile=0.2))
    p.add("black", box((0.12, 0.003, 0.012), (x, y - 0.1015, h.deck + 0.36), tile=0.2))
    cy = -0.42
    p.add("grey", box((0.17, 0.11, 0.09), (x, cy, top + 0.045), tile=0.3, bevel=0.006))
    p.add("plastic", box((0.15, 0.003, 0.06), (x, cy - 0.0565, top + 0.05), tile=0.2))
    p.add("led", box((0.01, 0.003, 0.006), (x + 0.06, cy - 0.058, top + 0.07), tile=0.2))
    for gx in (-0.04, 0.0, 0.04):
        p.add("black", cyl(0.008, 0.0, 0.014, 10), at((x + gx, cy + 0.055, top + 0.03), (0, 1, 0)))
    for k, gx in enumerate((-0.04, 0.04)):
        p.add("plastic", tube([(x + gx, cy + 0.065, top + 0.03), (x + gx, cy + 0.11 + 0.02 * k, top + 0.012), (x + gx * 0.5, y - 0.12, top + 0.012),
                               (x + gx * 0.5, y - 0.1, top + 0.05)], 0.006, 8))


def shell(p, h):
    """The tub, the deck housing over the wheels, the skirts between the
    wells, the deck plate, bumpers, wheel-drive flanges, the light band."""
    hx, hy = h.half
    wy = h.base / 2
    well = (wy - h.wheel_r - 0.02, wy + h.wheel_r + 0.02)
    p.add("black", box((2 * h.tub, 2 * hy - 0.02, h.shell - h.skirt + 0.01), (0, 0, (h.skirt + h.shell) / 2), tile=0.5))
    p.add("paint", box((2 * hx, 2 * hy, h.deck - h.shell), (0, 0, (h.shell + h.deck) / 2), tile=0.5, bevel=0.012))
    p.add("alu", box((2 * hx - 0.03, 2 * hy - 0.03, 0.006), (0, 0, h.deck + 0.003), tile=0.3))
    screws = [(x, sy * (hy - 0.03)) for x in (-0.3, -0.15, 0.0, 0.15, 0.3) for sy in (-1, 1)] + [(sx * (hx - 0.03), y) for y in (-0.36, 0.0, 0.36) for sx in (-1, 1)]
    mk.bolts(p, Matrix.Translation((0, 0, h.deck + 0.006)), screws, 0.005)
    band = (0.15, 0.25)
    for sx in (-1, 1):
        for y0, y1 in ((-hy, -well[1]), (-well[0], well[0]), (well[1], hy)):
            x, y = sx * (hx + h.tub) / 2, (y0 + y1) / 2
            if (sx, 1 if y > 0 else -1) in [(a, b) for a, b, _ in SCANNERS] and abs(y) > well[1]:
                for z0, z1 in ((h.skirt, band[0]), (band[1], h.shell)):
                    p.add("paint", box((hx - h.tub, y1 - y0, z1 - z0), (x, y, (z0 + z1) / 2), tile=0.4, bevel=0.004))
                p.add("black", box((hx - h.tub - 0.09, y1 - y0, band[1] - band[0]), (sx * (h.tub + (hx - h.tub - 0.09) / 2), y, sum(band) / 2), tile=0.3))
                continue
            p.add("paint", box((hx - h.tub, y1 - y0, h.shell - h.skirt), (x, y, (h.skirt + h.shell) / 2), tile=0.4, bevel=0.006))
        for sy in (-1, 1):
            p.add("rubber", box((hx - h.tub + 0.004, well[1] - well[0], 0.012), (sx * (hx + h.tub) / 2, sy * wy, h.shell - 0.004), tile=0.3))
            for yy in well:
                p.add("rubber", box((hx - h.tub + 0.004, 0.012, h.shell - h.skirt), (sx * (hx + h.tub) / 2, sy * yy, (h.skirt + h.shell) / 2), tile=0.3))
            hub = Vector((sx * h.tub, sy * wy, h.wheel_r))
            p.add("black", cyl(0.06, 0.0, 0.03, 28), at(hub, (sx, 0, 0)))
            mk.bolts(p, at(hub + Vector((sx * 0.03, 0, 0)), (sx, 0, 0)), [(0.045 * math.cos(a), 0.045 * math.sin(a)) for a in [k * math.pi / 3 for k in range(6)]], 0.006)
            p.add("steel", cyl(0.022, 0.03, (h.track / 2 - h.tub) - 0.04, 16), at(hub, (sx, 0, 0)))
        # Side faces: light band, battery hatch, warning label, access-panel screws.
        p.add("status", box((0.003, 2 * hy - 0.06, 0.01), (sx * (hx + 0.0012), 0, h.deck - 0.022), tile=0.2))
        if sx > 0:
            battery_hatch(p, h)
        p.add("yellow", box((0.002, 0.05, 0.045), (sx * (hx + 0.001), 0.2, h.deck - 0.05), tile=0.2))
        p.add("black", box((0.0025, 0.03, 0.006), (sx * (hx + 0.0012), 0.2, h.deck - 0.05), tile=0.2))
        for y in (-(hy + well[1]) / 2, -(well[0] - 0.015), well[0] - 0.015, (hy + well[1]) / 2):
            for z in (h.skirt + 0.02, h.shell - 0.02):
                p.add("steel", cyl(0.0045, 0.0, 0.002, 8), at((sx * hx, y, z), (sx, 0, 0)))
    for sy in (-1, 1):
        p.add("status", box((2 * hx - 0.06, 0.003, 0.01), (0, sy * (hy + 0.0012), h.deck - 0.022), tile=0.2))
        bump = sy * (hy + 0.018)
        p.add("rubber", box((2 * hx - 0.1, 0.036, 0.19), (sy * 0.04, bump, 0.2), tile=0.3, bevel=0.012))
        p.add("yellow", box((2 * hx - 0.12, 0.004, 0.012), (sy * 0.04, bump + sy * 0.018, 0.285), tile=0.2))
        for bx in (-0.22, -0.07, 0.07, 0.22):
            p.add("steel", cyl(0.007, 0.0, 0.004, 8), at((bx + sy * 0.04, bump + sy * 0.018, 0.13), (0, sy, 0)))


def battery_hatch(p, h):
    """The battery bay's hatch on the left side between the wells: a gasketed
    panel with two quarter-turn latches and its rating label, the cooling
    louvres under it."""
    x = h.half[0] + 0.002
    zc = h.shell - 0.075
    p.add("black", box((0.003, 0.172, 0.132), (x, 0, zc), tile=0.2))
    p.add("paint", box((0.004, 0.164, 0.124), (x + 0.0015, 0, zc), tile=0.2, bevel=0.002))
    for y in (-0.06, 0.06):
        p.add("black", cyl(0.009, 0.0, 0.004, 16), at((x + 0.0035, y, zc + 0.045), (1, 0, 0)))
        p.add("steel", box((0.002, 0.012, 0.002), (x + 0.0075, y, zc + 0.045), tile=0.2))
    p.add("yellow", box((0.001, 0.07, 0.035), (x + 0.004, 0, zc - 0.02), tile=0.2))
    p.add("black", box((0.0012, 0.05, 0.006), (x + 0.0042, 0, zc - 0.014), tile=0.2))
    p.add("black", box((0.0012, 0.035, 0.004), (x + 0.0042, 0, zc - 0.026), tile=0.2))
    for k in range(6):
        p.add("plastic", box((0.004, 0.012, 0.04), (x, -0.065 + k * 0.026, h.skirt + 0.04), tile=0.2))


def front_and_rear(p, h):
    """The safety scanners at the front-left and rear-right corners, the
    depth camera, e-stops, the stack light, aerials, charging contacts and
    the operator panel on the right side."""
    hx, hy = h.half
    for sx, sy, yaw in SCANNERS:
        m = Matrix.Translation(scanner_at(h, sx, sy)) @ Matrix.Rotation(math.radians(yaw), 4, "Z")
        scanner(p, m)
        p.add("black", box((0.09, 0.07, 0.006), (-0.035, 0, -0.047), tile=0.2), m)
    cam = at((0, -(hy + 0.03), h.deck - 0.05), (0, 0, 1), (1, 0, 0)) @ Matrix.Rotation(math.radians(-12), 4, "X") @ Matrix.Rotation(math.pi, 4, "Z")
    depth_camera(p, cam)
    p.add("black", box((0.06, 0.035, 0.008), (0, -(hy + 0.0175), h.deck - 0.068), tile=0.2))
    for x, sy in ((-0.24, -1), (0.24, 1), (-0.24, 1)):
        estop(p, at((x, sy * hy, h.deck - 0.045), (0, sy, 0), (1, 0, 0)))
    sx, sy = 0.31, -0.5
    top = h.deck + 0.006
    p.add("black", cyl(0.022, top, top + 0.012, 20), Matrix.Translation((sx, sy, 0)))
    p.add("black", cyl(0.008, top + 0.012, top + 0.08, 10), Matrix.Translation((sx, sy, 0)))
    for k, key in enumerate(("status", "amber", "beacon")):
        p.add(key, cyl(0.02, top + 0.08 + 0.034 * k, top + 0.11 + 0.034 * k, 20), Matrix.Translation((sx, sy, 0)))
        p.add("black", cyl(0.0205, top + 0.11 + 0.034 * k, top + 0.114 + 0.034 * k, 20), Matrix.Translation((sx, sy, 0)))
    p.add("black", turned([(0.0, top + 0.182), (0.02, top + 0.182), (0.016, top + 0.19), (0.0, top + 0.192)], 20), Matrix.Translation((sx, sy, 0)))
    for ax in (-0.325, 0.325):
        p.add("black", cyl(0.01, top, top + 0.012, 12), Matrix.Translation((ax, hy - 0.02, 0)))
        p.add("plastic", turned([(0.0, top + 0.012), (0.006, top + 0.012), (0.005, top + 0.13), (0.0, top + 0.135)], 10), Matrix.Translation((ax, hy - 0.02, 0)))
    for cx in (-0.08, 0.08):
        p.add("copper", box((0.09, 0.006, 0.022), (cx, hy + 0.039, 0.135), tile=0.2, bevel=0.002))
    face = -(hx + 0.002)
    p.add("grey", box((0.004, 0.16, 0.16), (face, 0, (h.skirt + h.shell) / 2), tile=0.2, bevel=0.002))
    p.add("black", box((0.004, 0.09, 0.05), (face - 0.002, 0.02, 0.255), tile=0.2))
    p.add("screen", box((0.002, 0.08, 0.04), (face - 0.004, 0.02, 0.255), tile=0.2))
    p.add("status", cyl(0.011, 0.0, 0.008, 18), at((face, -0.05, 0.255), (-1, 0, 0)))
    p.add("black", cyl(0.012, 0.0, 0.012, 18), at((face, -0.05, 0.19), (-1, 0, 0)))
    p.add("steel", box((0.004, 0.004, 0.012), (face - 0.012, -0.05, 0.19), tile=0.2))
    p.add("grey", box((0.03, 0.05, 0.035), (face - 0.013, 0.03, 0.17), tile=0.2, bevel=0.003))
    p.add("red", box((0.004, 0.035, 0.02), (face - 0.029, 0.03, 0.17), tile=0.2))


def chassis_part(h):
    p = Part("chassis", (0, 0, 0))
    shell(p, h)
    front_and_rear(p, h)
    column_base(p, h)
    deck_items(p, h)
    return p


def wheel_part(h, name, sx, sy):
    """A wheel on the axle along X: a grooved solid tyre on a steel disc, hub cap and nuts."""
    hub = Vector((sx * h.track / 2, sy * h.base / 2, h.wheel_r))
    p = Part(f"wheel_{name}", hub)
    m = at(hub, (sx, 0, 0))
    r, w = h.wheel_r, h.tyre_w
    rib = [(r, -w / 2 + 0.012)]
    for k in range(4):
        z = -w / 2 + 0.02 + k * 0.02
        rib += [(r, z), (r - 0.004, z + 0.002), (r - 0.004, z + 0.006), (r, z + 0.008)]
    profile = [(0.106, -w / 2), (r - 0.012, -w / 2), (r - 0.003, -w / 2 + 0.004)] + rib + [(r, w / 2 - 0.012), (r - 0.003, w / 2 - 0.004),
                                                                                      (r - 0.012, w / 2), (0.106, w / 2), (0.106, -w / 2)]
    p.add("tyre", turned(profile, 48), m)
    p.add("grey", turned([(0.03, -w / 2 + 0.006), (0.108, -w / 2 + 0.006), (0.108, w / 2 - 0.006), (0.1, w / 2 - 0.004), (0.06, w / 2 - 0.01),
                          (0.03, w / 2 - 0.01), (0.03, -w / 2 + 0.006)], 36), m)
    for k in range(5):
        a = 2 * math.pi * k / 5
        p.add("black", cyl(0.012, w / 2 - 0.0105, w / 2 - 0.0095, 12), m @ Matrix.Translation((0.082 * math.cos(a), 0.082 * math.sin(a), 0)))
        p.add("steel", mk.hexagon(0.008, w / 2 - 0.01, w / 2 - 0.002), m @ Matrix.Translation((0.042 * math.cos(a + 0.6), 0.042 * math.sin(a + 0.6), 0)))
    p.add("steel", turned([(0.0, w / 2 - 0.01), (0.028, w / 2 - 0.01), (0.026, w / 2 + 0.004), (0.016, w / 2 + 0.01), (0.0, w / 2 + 0.011)], 24), m)
    return p


def stage_part(h):
    """The lift's middle stage at rest, collared at its top."""
    x, y = h.column
    z0, z1 = h.deck + 0.045, h.deck + h.outer + 0.035
    p = Part("lift_stage", (x, y, z0))
    upright(p, "alu", x, y, z0, z1, 0.17, 0.17, 0.025)
    p.add("black", kit.prism(rounded(0.176, 0.176, 0.027), z1 - 0.016, z1, 0.3), Matrix.Translation((x, y, 0)))
    for s in (-1, 1):
        p.add("plastic", box((0.004, 0.08, 0.02), (x + s * 0.0865, y, z0 + 0.03), tile=0.2))
        p.add("plastic", box((0.08, 0.004, 0.02), (x, y + s * 0.0865, z0 + 0.03), tile=0.2))
    return p


def carriage_part(h):
    """The lift's inner stage at rest: the tube, the top plate the arm's base
    stands on, the arm cable's gland and loop."""
    x, y = h.column
    local, origin = mount(h)
    z0, z1 = origin.z, h.plate - 0.015
    p = Part("carriage", origin)
    upright(p, "paint", x, y, z0, z1, 0.14, 0.14, 0.02)
    p.add("black", box((0.21, 0.21, 0.015), (x, y, z1 + 0.0075), tile=0.3, bevel=0.003))
    mk.bolts(p, Matrix.Translation((x, y, z1 + 0.015)), [(sx * 0.085, sy * 0.085) for sx in (-1, 1) for sy in (-1, 1)], 0.007)
    gx, gy = x - 0.08, y + 0.055
    p.add("black", cyl(0.011, z1 + 0.015, z1 + 0.03, 12), Matrix.Translation((gx, gy, 0)))
    p.add("plastic", tube([(gx, gy, z1 + 0.03), (gx, gy, z1 + 0.05), (gx + 0.02, gy - 0.01, z1 + 0.065), (x - 0.07, y + 0.03, z1 + 0.06)], 0.008, 10))
    return p


def tool_part(h):
    """The tool in tool0's frame: quick changer, gripper body and jaw rail,
    the vacuum ejector and plunger with its suction cup, the wrist camera on
    its bracket with the light bar, and the tool cable."""
    p = Part("tool", (0, 0, 0))
    p.add("alu", cyl(0.0375, 0.0, 0.012, 32))
    p.add("black", cyl(0.0375, 0.012, 0.022, 32))
    mk.bolts(p, Matrix.Translation((0, 0, 0.022)), [(0.028 * math.cos(a), 0.028 * math.sin(a)) for a in [k * math.pi / 2 + 0.78 for k in range(4)]], 0.004)
    p.add("grey", box((0.26, 0.07, 0.085), (0, 0, 0.0645), tile=0.2, bevel=0.008))
    p.add("black", box((0.262, 0.03, 0.06), (0, 0, 0.0645), tile=0.2))
    p.add("steel", box((0.25, 0.03, 0.008), (0, 0, 0.111), tile=0.2))
    p.add("plastic", box((0.02, 0.002, 0.012), (0.08, -0.0355, 0.09), tile=0.2))
    p.add("led", box((0.006, 0.002, 0.004), (0.06, -0.0355, 0.09), tile=0.2))
    p.add("black", box((0.05, 0.03, 0.03), (0.0, 0.05, 0.06), tile=0.2, bevel=0.003))
    p.add("steel", cyl(0.006, 0.0, 0.03, 10), at((0.025, 0.05, 0.06), (1, 0, 0)))
    p.add("plastic", tube([(-0.02, 0.05, 0.075), (-0.02, 0.05, 0.11), (-0.01, 0.02, 0.13), (0.0, 0.0, 0.135)], 0.004, 8))
    p.add("steel", cyl(0.006, 0.115, 0.205, 12))
    p.add("black", cyl(0.009, 0.115, 0.14, 12))
    p.add("soft", turned([(0.0, 0.2), (0.008, 0.2), (0.012, 0.21), (0.009, 0.215), (0.014, 0.225), (0.016, 0.235), (0.0, 0.232)], 20))
    bracket = Matrix.Translation((0.0, -0.035, 0.03))
    p.add("black", box((0.05, 0.035, 0.004), (0, -0.0175, 0.0), tile=0.2), bracket)
    p.add("black", box((0.05, 0.004, 0.05), (0, -0.033, 0.025), tile=0.2), bracket)
    cam = Matrix.Translation((0.0, -0.08, 0.075)) @ Matrix.Rotation(math.radians(-90 + 15), 4, "X")
    depth_camera(p, cam, 0.042, 0.023, 0.042)
    p.add("black", box((0.06, 0.012, 0.012), (0.0, -0.08, 0.04), tile=0.2))
    p.add("led", box((0.05, 0.004, 0.006), (0.0, -0.08, 0.0335), tile=0.2), Matrix.Translation((0, 0, 0)))
    p.add("plastic", tube([(0.06, 0.035, 0.03), (0.07, 0.05, 0.012), (0.07, 0.06, -0.02)], 0.004, 8))
    return p


def finger(s, root, length, curl, width=0.045, thick=0.022, n=42, folds=7):
    """A soft pneumatic finger from `root` down the tool's +Z, curling toward
    its axis (toward -s in X) by `curl` radians at the tip: a rounded section
    tapering to the tip, its back folded into bellows."""
    bm = bmesh.new()
    section = rounded(1.0, 1.0, 0.3, 2)
    rings, p = [], Vector(root)
    for i in range(n + 1):
        t = i / n
        a = curl * t * t
        out = Vector((s * math.cos(a), 0, math.sin(a)))
        fold = 1.0 + 0.4 * max(0.0, math.sin(math.pi * folds * (t - 0.06) / 0.86)) if 0.06 < t < 0.92 else 1.0
        taper = 1.0 - 0.3 * t
        rings.append([bm.verts.new(p + out * (u * thick * taper * (fold if u > 0 else 1.0)) + Vector((0, v * width * (1 - 0.15 * t), 0)))
                      for u, v in section])
        p = p + Vector((-s * math.sin(a), 0, math.cos(a))) * (length / n)
    m = len(section)
    for lo, hi in zip(rings, rings[1:]):
        for k in range(m):
            bm.faces.new((lo[k], lo[(k + 1) % m], hi[(k + 1) % m], hi[k]))
    bm.faces.new(rings[0][::-1])
    bm.faces.new(rings[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    kit.project_uv(bm, 0.1)
    return bm


def jaw_part(h, side):
    """A jaw at rest, closed: its carriage on the rail, a soft finger
    curling in at the tip; the right jaw carries the stem cutter."""
    s = 1 if side == "left" else -1
    x = s * h.jaw[0]
    p = Part(f"jaw_{side}", (x, 0, 0.115))
    p.add("black", box((0.035, 0.055, 0.022), (x, 0, 0.126), tile=0.2, bevel=0.003))
    p.add("steel", box((0.026, 0.05, 0.008), (x - s * 0.002, 0, 0.141), tile=0.2))
    mk.bolts(p, at((x - s * 0.002, 0, 0.137), (0, 0, -1)), [(0.0, -0.016), (0.0, 0.016)], 0.003)
    p.add("soft", finger(s, (x - s * 0.004, 0, 0.143), 0.13, 0.3))
    if s < 0:
        p.add("steel", kit.prism([(0.0, -0.016), (0.02, 0.0), (0.0, 0.016)], -0.0015, 0.0015, 0.1), Matrix.Translation((x + 0.006, 0, 0.152))
              @ Matrix.Rotation(math.pi / 2, 4, "Y"))
    return p


def looks(h):
    """Every body's Part: machine-frame bodies posed at rest, the arm's links
    and the tool and jaws in their own frames."""
    parts = [chassis_part(h)] + [wheel_part(h, name, sx, sy) for name, sx, sy in ROLLS] + [stage_part(h), carriage_part(h)]
    arm = arms.looks(ARM)
    local, origin = mount(h)
    base = arm.pop("base")
    base_in_carriage = mk.Part(base.name, (0, 0, 0))
    for key, bm in base.pieces.items():
        base_in_carriage.add(key, bm, local)
    parts.append(base_in_carriage)
    parts += list(arm.values())
    parts += [tool_part(h), jaw_part(h, "left"), jaw_part(h, "right")]
    return parts


# The machine layer


def box_hull(u, d, name, centre, size):
    """A hidden box the body collides as."""
    u.emit(d, f'def Cube "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, 'token visibility = "invisible"')
    u.emit(d + 1, 'uniform token purpose = "guide"')
    u.emit(d + 1, "double size = 1")
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(d + 1, f"float3 xformOp:scale = {fmt(size)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:scale"]')
    u.emit(d, "}")


def sensor(u, d, name, kind, at_, rotate, attrs=()):
    u.emit(d, f'def Xform "{name}" (prepend apiSchemas = ["GearboxLinkAPI"])')
    u.emit(d, "{")
    u.emit(d + 1, 'token gearbox:link:role = "sensor"')
    u.emit(d + 1, f'token gearbox:sensor:kind = "{kind}"')
    for attr in attrs:
        u.emit(d + 1, attr)
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(at_)}")
    u.emit(d + 1, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
    u.emit(d, "}")


def looking(direction):
    """rotateXYZ degrees turning a sensor's +X along `direction`."""
    d = Vector(direction).normalized()
    q = d.to_track_quat("X", "Z" if abs(d.z) < 0.95 else "Y")
    return tuple(math.degrees(a) for a in q.to_euler("XYZ"))


def box_inertia(mass, size):
    x, y, z = size
    return (mass * (y * y + z * z) / 12, mass * (x * x + z * z) / 12, mass * (x * x + y * y) / 12)


def rigid(u, d, name, m, mass, inertia, role="link", link=None, com=(0, 0, 0), values=()):
    """Opens a rigid body at `m` (a translation, or a matrix); close it with `usda.end`."""
    m = m if isinstance(m, Matrix) else Matrix.Translation(Vector(m))
    arms.body_prim(u, d, name, m, mass, com, inertia, (1.0, 0.0, 0.0, 0.0), values, link, role)


def roll_joint(u, d, name, body1, pos0, torque):
    u.emit(d, f'def PhysicsRevoluteJoint "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsDriveAPI:angular"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, "rel physics:body0 = </robot/chassis>")
    u.emit(d + 1, f"rel physics:body1 = </robot/{body1}>")
    u.emit(d + 1, 'uniform token physics:axis = "X"')
    u.emit(d + 1, f"point3f physics:localPos0 = {fmt(pos0)}")
    u.emit(d + 1, "point3f physics:localPos1 = (0, 0, 0)")
    u.emit(d + 1, "quatf physics:localRot0 = (1, 0, 0, 0)")
    u.emit(d + 1, "quatf physics:localRot1 = (1, 0, 0, 0)")
    u.emit(d + 1, "bool physics:collisionEnabled = 0")
    u.emit(d + 1, 'uniform token drive:angular:physics:type = "force"')
    u.emit(d + 1, "float drive:angular:physics:stiffness = 0")
    u.emit(d + 1, "float drive:angular:physics:damping = 8")
    u.emit(d + 1, f"float drive:angular:physics:maxForce = {torque}")
    u.emit(d + 1, "float drive:angular:physics:targetVelocity = 0")
    u.emit(d + 1, 'uniform token[] gearbox:joint:angular:commandInterfaces = ["velocity"]')
    u.emit(d, "}")


def slide_joint(u, d, name, body0, body1, pos0, axis, limits, force, speed):
    u.emit(d, f'def PhysicsPrismaticJoint "{name}"')
    u.emit(d, "{")
    u.emit(d + 1, f"rel physics:body0 = </robot/{body0}>")
    u.emit(d + 1, f"rel physics:body1 = </robot/{body1}>")
    u.emit(d + 1, f'uniform token physics:axis = "{axis}"')
    u.emit(d + 1, f"point3f physics:localPos0 = {fmt(pos0)}")
    u.emit(d + 1, "quatf physics:localRot0 = (1, 0, 0, 0)")
    u.emit(d + 1, "point3f physics:localPos1 = (0, 0, 0)")
    u.emit(d + 1, "quatf physics:localRot1 = (1, 0, 0, 0)")
    u.emit(d + 1, f"float physics:lowerLimit = {limits[0]:g}")
    u.emit(d + 1, f"float physics:upperLimit = {limits[1]:g}")
    u.emit(d + 1, "bool physics:collisionEnabled = 0")
    u.emit(d + 1, f"float gearbox:motor:maxForce = {force:g}")
    u.emit(d + 1, f"float gearbox:motor:maxVelocity = {speed:g}")
    u.emit(d, "}")


def header(u, h, controllers, groups):
    wheels = ", ".join(f"</robot/Joints/roll_{name}>" for name, _, _ in ROLLS)
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, 'string creator = "scripts/mars_base/harvester.py: the mobile harvester of the arch greenhouses"')
    u.emit(2, f'string licence = "{ARM.licence}"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/harvester.py and build again. Drive it with `gearbox machine move --machine <id> ...`;")
    u.emit(0, "# each joint controller takes a position 0..1 of its range: `gearbox machine set-value <joint> position 0.5 --machine <id>`.")
    u.emit(0, 'def Xform "robot" (')
    apis = ["GearboxMachineAPI", "PhysicsArticulationRootAPI", "GearboxControllerAPI:drive", *(f"GearboxControllerAPI:{c}" for c in controllers)]
    u.emit(1, f"prepend apiSchemas = {schemas(apis)}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, 'token gearbox:machine:interfaceVersion = "0.1"')
    u.emit(1, 'token gearbox:machine:kind = "mobile_manipulator"')
    u.emit(1, 'token gearbox:machine:idPolicy = "prim_path"')
    u.emit(1, 'token gearbox:machine:upAxis = "Z"')
    u.emit(1, "rel gearbox:machine:body = </robot/chassis>")
    u.emit(1, "rel gearbox:machine:controlFrame = </robot/chassis/control_frame>")
    u.emit(1, f"rel gearbox:machine:role:poweredWheelJoints = [{wheels}]")
    u.emit(1, f"rel gearbox:machine:role:brakeJoints = [{wheels}]")
    u.emit(1, "bool gearbox:controller:drive:enabled = true")
    u.emit(1, 'token gearbox:controller:drive:type = "builtin:diff_drive_cmd_vel"')
    u.emit(1, 'token gearbox:controller:drive:driveMode = "wheels"')
    u.emit(1, 'token gearbox:controller:drive:commandInterface = "cmd_vel"')
    u.emit(1, 'token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity", "joint_state"]')
    u.emit(1, 'token[] gearbox:controller:drive:usesRoles = ["poweredWheelJoints"]')
    u.emit(1, 'token gearbox:controller:drive:frameConvention = "usd_z_up"')
    u.emit(1, 'token gearbox:controller:drive:namespacePolicy = "machine_id"')
    u.emit(1, "float gearbox:controller:drive:updateRateHz = 60")
    u.emit(1, "float gearbox:controller:drive:maxWheelTorqueNm = 60")
    u.emit(1, "float gearbox:controller:drive:maxPowerKw = 1.2")
    u.emit(1, f"float gearbox:controller:drive:wheelRadius = {h.wheel_r}")
    u.emit(1, f"float gearbox:controller:drive:trackWidth = {h.track}")
    u.emit(1, f"float gearbox:controller:drive:wheelBase = {h.base}")
    u.emit(1, "rel gearbox:controller:drive:body = </robot/chassis>")
    for c in controllers:
        targets = groups.get(c, [c])
        u.emit(1, f'token gearbox:controller:{c}:type = "builtin:joint_position"')
        if len(targets) == 1:
            u.emit(1, f"rel gearbox:controller:{c}:target = </robot/Joints/{targets[0]}>")
        else:
            u.emit(1, f"rel gearbox:controller:{c}:target = [{', '.join(f'</robot/Joints/{t}>' for t in targets)}]")
    u.emit(1, 'def "Looks" (')
    u.emit(2, "prepend references = @./harvester_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")


def machine(h, bound):
    """The machine layer: bodies, joints, the drive and the joint controllers."""
    u = Usda("/robot/Looks")
    groups = {"lift": ["lift", "lift_upper"], "grip": ["grip_left", "grip_right"]}
    controllers = ["lift", *arms.controllers(ARM), "grip"]
    header(u, h, controllers, groups)
    hx, hy = h.half
    kg = h.masses
    rigid(u, 1, "chassis", (0, 0, 0), kg["chassis"], box_inertia(kg["chassis"], (2 * hx, 2 * hy, h.deck - h.skirt)), "base", "base_link", (0, 0, 0.22))
    u.emit(2, 'def Xform "control_frame"')
    u.emit(2, "{")
    u.emit(3, "quatd xformOp:orient = (0.7071067811865476, 0, 0, -0.7071067811865476)")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:orient"]')
    u.emit(2, "}")
    arms.visual(u, 2, "harvester", "chassis", bound)
    box_hull(u, 2, "tub", (0, 0, (h.skirt + h.shell) / 2), (2 * h.tub, 2 * hy, h.shell - h.skirt))
    box_hull(u, 2, "housing", (0, 0, (h.shell + h.deck) / 2), (2 * hx, 2 * hy, h.deck - h.shell))
    for sy in (-1, 1):
        box_hull(u, 2, f"bumper_{'front' if sy < 0 else 'rear'}", (sy * 0.04, sy * (hy + 0.018), 0.2), (2 * hx - 0.1, 0.036, 0.19))
    x, y = h.column
    box_hull(u, 2, "column", (x, y, h.deck + h.outer / 2), (0.2, 0.2, h.outer))
    top = h.deck + 0.006
    for name, ((x0, y0), (x1, y1), hgt), extra in (("crate", CRATE, 0.01), ("bin", BIN, 0.0), ("magazine", MAGAZINE, 0.155)):
        box_hull(u, 2, name, ((x0 + x1) / 2, (y0 + y1) / 2, top + (hgt + extra) / 2), (x1 - x0, y1 - y0, hgt + extra))
    sensor(u, 2, "imu_link", "imu", (0, 0, 0.25), (0, 0, 0), ["float gearbox:sensor:rate_hz = 100"])
    sensor(u, 2, "front_camera", "camera", (0, -(hy + 0.046), h.deck - 0.05), (0, 12, -90), [
        'token gearbox:sensor:channels = "color_depth"',
        'token gearbox:sensor:render = "geometry"',
        "int gearbox:sensor:width = 160",
        "int gearbox:sensor:height = 100",
        "float gearbox:sensor:vfov_deg = 58",
        "float gearbox:sensor:range_m = 6",
        "float gearbox:sensor:rate_hz = 5",
    ])
    for name, (sx, sy, yaw) in zip(("front_lidar", "rear_lidar"), SCANNERS):
        sensor(u, 2, name, "lidar", scanner_at(h, sx, sy), (0, 0, yaw), [
            "int gearbox:sensor:rows = 1",
            "int gearbox:sensor:columns = 550",
            "float gearbox:sensor:hfov_deg = 275",
            "float gearbox:sensor:vfov_deg = 1",
            "float gearbox:sensor:range_m = 10",
            "float gearbox:sensor:rate_hz = 5",
        ])
    end(u, 1)
    r, w = h.wheel_r, h.tyre_w
    for name, sx, sy in ROLLS:
        hub = (sx * h.track / 2, sy * h.base / 2, r)
        m = kg["wheel"]
        rigid(u, 1, f"wheel_{name}", hub, m, (m * r * r / 2, m * (3 * r * r + w * w) / 12, m * (3 * r * r + w * w) / 12), "wheel",
              values=[f"float gearbox:value:tyre_width_m = {w}"])
        arms.visual(u, 2, "harvester", f"wheel_{name}", bound)
        arms.hull(u, 2, "tire_collider", (0, 0, 0), "X", r, w)
        end(u, 1)
    z0 = h.deck + 0.045
    rigid(u, 1, "lift_stage", (x, y, z0), kg["lift_stage"], box_inertia(kg["lift_stage"], (0.17, 0.17, h.outer)),
          values=[f"float gearbox:value:range = {h.stroke}", "float gearbox:value:position = 0"])
    box_hull(u, 2, "tube", (0, 0, (h.outer - 0.01) / 2), (0.17, 0.17, h.outer - 0.01))
    arms.visual(u, 2, "harvester", "lift_stage", bound)
    end(u, 1)
    local, origin = mount(h)
    mass = kg["carriage"] + ARM.base.mass
    rigid(u, 1, "carriage", origin, mass, box_inertia(mass, (0.21, 0.21, h.plate - origin.z)), com=(0, 0, h.plate - origin.z - 0.1),
          values=[f"float gearbox:value:range = {h.stroke}", "float gearbox:value:position = 0"])
    box_hull(u, 2, "tube", (0, 0, (h.plate - origin.z) / 2), (0.14, 0.14, h.plate - origin.z))
    box_hull(u, 2, "plate", (0, 0, h.plate - origin.z - 0.0075), (0.21, 0.21, 0.015))
    arms.hull(u, 2, "base_hull", (0, 0, h.plate - origin.z + 0.05), "Z", 0.095, 0.1)
    arms.visual(u, 2, "harvester", "carriage", bound)
    arms.visual(u, 2, "harvester", f"{ARM.kind}_base", bound, "arm_base")
    end(u, 1)
    base = arm_base(h)
    arms.bodies(u, 1, ARM, base, ARM_REST, "harvester", bound, limits=ARM_RANGES)
    t0 = tool_frame(h)
    rigid(u, 1, "tool", t0, kg["tool"], box_inertia(kg["tool"], (0.26, 0.07, 0.12)), com=(0, 0, 0.07))
    box_hull(u, 2, "body", (0, 0, 0.06), (0.26, 0.07, 0.1))
    box_hull(u, 2, "plunger", (0, 0, 0.175), (0.03, 0.03, 0.12))
    arms.visual(u, 2, "harvester", "tool", bound)
    u.emit(2, 'def Xform "tcp"')
    u.emit(2, "{")
    u.emit(3, f"double3 xformOp:translate = {fmt((0, 0, h.tcp))}")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate"]')
    u.emit(2, "}")
    sensor(u, 2, "wrist_camera", "camera", (0.0, -0.08, 0.075), looking((0, math.sin(math.radians(15)), math.cos(math.radians(15)))), [
        "bool gearbox:sensor:recognition = true",
        'token gearbox:sensor:channels = "color_depth"',
        'token gearbox:sensor:render = "geometry"',
        "int gearbox:sensor:width = 160",
        "int gearbox:sensor:height = 120",
        "float gearbox:sensor:vfov_deg = 58",
        "float gearbox:sensor:range_m = 1.5",
        "float gearbox:sensor:rate_hz = 2",
    ])
    end(u, 1)
    for side, s in (("left", 1), ("right", -1)):
        jaw = t0 @ Matrix.Translation((s * h.jaw[0], 0, 0.115))
        m = kg["jaw"]
        rigid(u, 1, f"jaw_{side}", jaw, m, box_inertia(m, (0.035, 0.055, 0.16)), com=(0, 0, 0.06),
              values=[f"float gearbox:value:range = {s * h.jaw[1]}", "float gearbox:value:position = 0"])
        box_hull(u, 2, "finger", (-s * 0.006, 0, 0.08), (0.024, 0.05, 0.15))
        arms.visual(u, 2, "harvester", f"jaw_{side}", bound)
        end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    for name, sx, sy in ROLLS:
        roll_joint(u, 2, f"roll_{name}", f"wheel_{name}", (sx * h.track / 2, sy * h.base / 2, r), 60)
    slide_joint(u, 2, "lift", "chassis", "lift_stage", (x, y, z0), "Z", (0, h.stroke), 1500, 0.04)
    slide_joint(u, 2, "lift_upper", "lift_stage", "carriage", (0, 0, origin.z - z0), "Z", (0, h.stroke), 1500, 0.04)
    arms.joints(u, 2, ARM, "carriage", local, ARM_REST, limits=ARM_RANGES)
    wrist = ARM.links[-1].name
    t_local = arms.tool0(ARM)
    u.emit(2, 'def PhysicsFixedJoint "tool_mount"')
    u.emit(2, "{")
    u.emit(3, f"rel physics:body0 = </robot/{wrist}>")
    u.emit(3, "rel physics:body1 = </robot/tool>")
    u.emit(3, f"point3f physics:localPos0 = {fmt(t_local.translation)}")
    u.emit(3, f"quatf physics:localRot0 = ({', '.join(f'{v:.6f}' for v in arms.quat(t_local))})")
    u.emit(3, "point3f physics:localPos1 = (0, 0, 0)")
    u.emit(3, "quatf physics:localRot1 = (1, 0, 0, 0)")
    u.emit(3, "bool physics:collisionEnabled = 0")
    u.emit(2, "}")
    for side, s in (("left", 1), ("right", -1)):
        slide_joint(u, 2, f"grip_{side}", "tool", f"jaw_{side}", (s * h.jaw[0], 0, 0.115), "X",
                    (0, h.jaw[1]) if s > 0 else (-h.jaw[1], 0), 140, 0.1)
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def build(work, h=HARVESTER):
    """Builds the harvester's looks, writes them as `harvester_geom.usdc` and
    the machine as `harvester.usda` into `work`; returns {name: layer path}."""
    os.makedirs(work, exist_ok=True)
    kept = arms.snapshot()
    mats = materials(kit.Looks(os.path.join(work, "maps")))
    bound = arms.write_geom(looks(h), mats, os.path.join(work, "harvester_geom.usdc"))
    arms.tidy(kept)
    path = os.path.join(work, "harvester.usda")
    with open(path, "w") as f:
        f.write(machine(h, bound))
    return {"harvester": path}
