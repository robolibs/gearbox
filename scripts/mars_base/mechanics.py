"""Machine parts the base's robots are built from, as bmeshes: aluminium
T-slot extrusions, profiled guide rails and their carriage blocks, guide
wheels, gear motors, racks, enclosed conductor rails, covers and tending
heads; and the few materials they wear, clean but handled.

Builders of one material return a bmesh. Builders of assemblies add their
pieces to a `Part` under material keys, placed by a matrix `m` taking the
builder's own frame into the robot's. Runs inside Blender.
"""
import math

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import patterns
from industry import frame_of, ring
from kit import box, lathe, merge, prism, rod, srgb_to_linear, sweep

# Key: (material name, sRGB colour, roughness, metal, map settings or None).
FINISHES = {
    "alu": ("gantry_anodised", (0.77, 0.78, 0.80), 0.34, 0.9, {"seed": 61, "brush": 0.05, "grime": 0.05}),
    "black": ("gantry_black", (0.075, 0.075, 0.08), 0.42, 0.6, {"seed": 62, "grime": 0.1}),
    "steel": ("gantry_steel", (0.68, 0.69, 0.70), 0.22, 1.0, {"seed": 63, "brush": 0.08, "grime": 0.04}),
    "paint": ("gantry_paint", (0.86, 0.86, 0.83), 0.48, 0.0, {"seed": 64, "grime": 0.07}),
    "orange": ("gantry_orange", (0.88, 0.40, 0.07), 0.5, 0.0, {"seed": 65, "grime": 0.07}),
    "plastic": ("gantry_plastic", (0.04, 0.04, 0.042), 0.7, 0.0, {"seed": 66, "grime": 0.04}),
    "grey": ("gantry_grey", (0.46, 0.47, 0.48), 0.55, 0.0, {"seed": 67, "grime": 0.06}),
    "lens": ("gantry_lens", (0.02, 0.025, 0.03), 0.04, 0.0, None),
    "led": ("gantry_led", (1.0, 0.97, 0.9), 0.3, 0.0, None),
}
GLOW = ((1.0, 0.97, 0.9), 6.0)
UP, ALONG = Vector((0, 0, 1)), Vector((1, 0, 0))


def finish(colour, rough, seed, brush=0.0, grime=0.06, n=512):
    """Maps of a clean but handled surface: a fine grain, faint mottled
    grime, brushing lines along v, a roughness that wanders with them."""
    def make():
        c = srgb_to_linear(np.array(colour, dtype=np.float32))
        fine = patterns.noise(n, cells=128, seed=seed, octaves=2)
        dirt = patterns.noise(n, cells=4, seed=seed + 1, octaves=3)
        lines = np.random.default_rng(seed).random(n).astype(np.float32)
        streak = brush * (np.repeat(lines[None, :], n, axis=0) - 0.5)
        shade = 1.0 - grime * (dirt - 0.4) + 0.05 * (fine - 0.5) + streak
        diff = np.clip(c * shade[..., None], 0.0, 1.0)
        r = np.clip(rough + 0.08 * (dirt - 0.5) + 0.04 * (fine - 0.5) + 0.6 * np.abs(streak), 0.02, 1.0)
        height = (0.2 * fine + 2.0 * streak).astype(np.float32)
        return diff.astype(np.float32), patterns.normals_from_height(height, 0.5), r.astype(np.float32)
    return make


def materials(looks):
    return {key: looks.get(name, colour, rough, metal, maps=finish(colour, rough, **maps) if maps else None,
                           emit=GLOW if key == "led" else None)
            for key, (name, colour, rough, metal, maps) in FINISHES.items()}


class Part:
    """One link's look: geometry gathered per material in the robot's frame
    at rest, put about the link's origin when finished."""

    def __init__(self, name, origin):
        self.name, self.origin, self.pieces = name, Vector(origin), {}

    def add(self, key, bm, m=None):
        merge(self.pieces.setdefault(key, bmesh.new()), bm, m)
        return self

    def finish(self, mats, collection):
        """One triangulated mesh object, a material slot per key."""
        bm = bmesh.new()
        mesh = bpy.data.meshes.new(self.name)
        for index, (key, piece) in enumerate(self.pieces.items()):
            start = len(bm.faces)
            merge(bm, piece, Matrix.Translation(-self.origin))
            bm.faces.ensure_lookup_table()
            for face in bm.faces[start:]:
                face.material_index = index
            mesh.materials.append(mats[key])
        bmesh.ops.triangulate(bm, faces=bm.faces[:], quad_method="BEAUTY", ngon_method="BEAUTY")
        bm.to_mesh(mesh)
        bm.free()
        mesh.shade_smooth()
        mesh.set_sharp_from_angle(angle=math.radians(35.0))
        obj = bpy.data.objects.new(self.name, mesh)
        collection.objects.link(obj)
        return obj


# Placement


def at(p, z=UP, x=None):
    """The matrix putting a builder's frame at `p`, its +Z along `z` and,
    when given, its +X toward `x`."""
    z = Vector(z).normalized()
    x = Vector(x if x is not None else (ALONG if abs(z.x) < 0.9 else (0, 1, 0)))
    x = (x - z * x.dot(z)).normalized()
    m = Matrix((x, z.cross(x), z)).transposed().to_4x4()
    m.translation = Vector(p)
    return m


# Outlines, as (x toward a bar's `up`, y across)


def tslot(w, slots=1):
    """A square T-slot extrusion `w` across, `slots` grooves to a face."""
    h, c, o, k = w / 2, 0.04 * w, 0.22 * w / slots, 0.12 * w
    side = [(-h + c, -h)]
    for i in range(slots):
        u = -h + w * (i + 0.5) / slots
        side += [(u - o / 2, -h), (u - o / 2, -h + k), (u + o / 2, -h + k), (u + o / 2, -h)]
    side.append((h - c, -h))
    out = []
    for cs, sn in ((1, 0), (0, 1), (-1, 0), (0, -1)):
        out += [(x * cs - y * sn, x * sn + y * cs) for x, y in side]
    return out


def rail_outline(w):
    """A profiled guide rail `w` wide, from its foot at x = 0 to its head."""
    h, g, c = 0.85 * w, 0.12 * w, 0.06 * w
    right = [(0.0, w / 2), (0.5 * h, w / 2), (0.58 * h, w / 2 - g), (0.74 * h, w / 2 - g), (0.82 * h, w / 2), (h - c, w / 2), (h, w / 2 - c)]
    return right + [(x, -y) for x, y in reversed(right)]


def beam_outline(w, h, t):
    """An I section `h` deep and `w` wide, its foot at x = 0."""
    hw = w / 2
    return [(0, -hw), (0, hw), (t, hw), (t, t / 2), (h - t, t / 2), (h - t, hw), (h, hw), (h, -hw), (h - t, -hw), (h - t, -t / 2),
            (t, -t / 2), (t, -hw)]


def conductor_outline(w, h, slot=1.0):
    """An enclosed conductor rail's housing, w across and h deep about its
    centre, its collector slot in the side toward `slot` (±1 across)."""
    c, s, d = 0.18 * w, 0.16 * h, 0.2 * w
    pts = [(-h / 2, -w / 2 + c), (-h / 2, w / 2 - c), (-h / 2 + c, w / 2), (h / 2 - c, w / 2), (h / 2, w / 2 - c), (h / 2, -w / 2 + c),
           (h / 2 - c, -w / 2), (s, -w / 2), (s, -w / 2 + d), (-s, -w / 2 + d), (-s, -w / 2), (-h / 2 + c, -w / 2)]
    return [(x, -slot * y) for x, y in pts]


def rect(w, h, x0=None):
    """A rectangle `w` across and `h` toward up, centred, or from x0 up."""
    lo = -h / 2 if x0 is None else x0
    return [(lo, -w / 2), (lo, w / 2), (lo + h, w / 2), (lo + h, -w / 2)]


# Bars and solids


def bar(a, b, outline, up=UP, tile=0.5):
    """A straight section of `outline` from a to b, its outline's x toward
    `up`, capped, faces turned out."""
    a, b = Vector(a), Vector(b)
    bm = sweep([(x, -y) for x, y in outline], [(0, 0, 0), ((b - a).length, 0, 0)], tile)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    bm.transform(frame_of(a, b, up))
    return bm


def turned(profile, segments=16, tile=0.2):
    """A closed solid of revolution about Z, its seams welded and faces turned out."""
    bm = lathe(profile, segments, tile)
    bmesh.ops.remove_doubles(bm, verts=bm.verts[:], dist=1e-7)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


def cyl(r, z0, z1, segments=16):
    return ring(r, 0.0, z0, z1, segments)


def hexagon(r, z0, z1):
    return turned([(0.0, z0), (r, z0), (r, z1), (0.0, z1)], 6)


def gear(r_root, r_tip, z0, z1, teeth):
    """A spur gear about Z, external teeth from `r_root` out to `r_tip`."""
    pts = []
    for i in range(teeth):
        a = 2 * math.pi * i / teeth
        da = 2 * math.pi / teeth
        for f, r in ((0.0, r_root), (0.2, r_tip), (0.5, r_tip), (0.7, r_root)):
            pts.append((r * math.cos(a + f * da), r * math.sin(a + f * da)))
    return prism(pts, z0, z1, 0.3)


def rack(length, width, height, pitch):
    """A toothed rack along X from 0 to `length`, `width` across Y, its teeth up to z = `height`."""
    n = max(1, int(length / pitch))
    p, base = length / n, 0.5 * height
    outline = [(0.0, 0.0), (base, 0.0)]
    for i in range(n):
        x = i * p
        outline += [(base, x + 0.12 * p), (height, x + 0.32 * p), (height, x + 0.58 * p), (base, x + 0.78 * p)]
    outline += [(base, length), (0.0, length)]
    bm = sweep(outline, [(0, -width / 2, 0), (0, width / 2, 0)], 0.1)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


def louvred(r, z0, z1, rings=5):
    """A radiation shield of stacked plates about Z, like a climate probe's."""
    pts = [(0.0, z1)]
    step = (z1 - z0) / rings
    for k in range(rings):
        z = z1 - k * step
        pts += [(r, z), (r, z - 0.35 * step), (0.6 * r, z - 0.5 * step), (0.6 * r, z - step)]
    pts.append((0.0, z0))
    return turned(pts[::-1], 14)


# Assemblies


def wheel(part, m, r, w, tread="plastic"):
    """A guide wheel on an axle along the builder's Z: a crowned tread on a steel hub."""
    profile = [(0.45 * r, -w / 2), (0.86 * r, -w / 2), (r, -0.25 * w), (r, 0.25 * w), (0.86 * r, w / 2), (0.45 * r, w / 2)]
    part.add(tread, turned(profile + profile[:1], 20), m)
    part.add("steel", turned([(0.12 * r, -0.56 * w), (0.45 * r, -0.56 * w), (0.45 * r, 0.56 * w), (0.12 * r, 0.56 * w), (0.12 * r, -0.56 * w)]), m)


def motor(part, m, size, length, cover="orange", gear_len=0.0):
    """A servo motor, its shaft along the builder's +Z from z = 0 (from the
    gearbox's face when it has one): a square flange, the painted body and
    its encoder cap behind, a cable gland and its cable stub."""
    a = size
    z0 = 0.0
    if gear_len:
        part.add("black", turned([(0.0, 0.0), (0.42 * a, 0.0), (0.42 * a, 0.7 * gear_len), (0.36 * a, gear_len), (0.0, gear_len)], 18),
                 Matrix(m) @ Matrix.Translation((0, 0, -gear_len)))
        z0 = -gear_len
    part.add("steel", cyl(0.1 * a, 0.0, 0.25 * a, 10), m)
    part.add("black", box((a, a, 0.16 * a), (0, 0, z0 - 0.08 * a), tile=0.2, bevel=0.03 * a), m)
    body = z0 - 0.16 * a
    part.add(cover, box((0.94 * a, 0.94 * a, length), (0, 0, body - length / 2), tile=0.2, bevel=0.1 * a), m)
    part.add("black", cyl(0.36 * a, body - length - 0.3 * a, body - length, 16), m)
    side = Matrix(m) @ Matrix.Translation((0, 0, body - 0.3 * length)) @ Matrix.Rotation(math.pi / 2, 4, "Y")
    part.add("plastic", cyl(0.09 * a, -0.6 * a, -0.47 * a, 8), side)
    part.add("plastic", cyl(0.06 * a, -0.9 * a, -0.6 * a, 6), side)


def block(part, m, w, length):
    """A guide carriage block on a rail `w` wide running along the builder's
    X, the rail's foot at z = 0: the steel block, its seals and grease nipple."""
    h = 0.85 * w
    part.add("steel", box((length, 2.3 * w, 0.95 * w), (0, 0, 0.3 * h + 0.475 * w), tile=0.2), m)
    for s in (-1, 1):
        part.add("black", box((0.1 * w, 2.1 * w, 0.85 * w), (s * (length / 2 + 0.05 * w), 0, 0.3 * h + 0.47 * w), tile=0.2), m)
    part.add("steel", cyl(0.08 * w, 0.0, 0.25 * w, 6), Matrix(m) @ Matrix.Translation((length / 2 + 0.1 * w, 0, 0.3 * h + 0.47 * w)) @ Matrix.Rotation(math.pi / 2, 4, "Y"))


def guide_rail(part, a, b, w, up):
    """A profiled guide rail from a to b standing on its foot toward `up`, capped, with bolt heads along its head."""
    part.add("steel", bar(a, b, rail_outline(w), up, 0.3))
    a, b = Vector(a), Vector(b)
    n = max(1, int((b - a).length / 0.06))
    for k in range(n):
        p = a + (b - a) * ((k + 0.5) / n) + Vector(up).normalized() * 0.85 * w
        part.add("black", cyl(0.22 * w, -0.001, 0.0005, 8), at(p, up))


def bolts(part, m, points, r, key="steel"):
    """Hex bolt heads on a face, at `points` in the builder's XY, standing out of its +Z."""
    for x, y in points:
        part.add(key, hexagon(r, 0.0, 0.7 * r), Matrix(m) @ Matrix.Translation((x, y, 0.0)))


def bumper(part, m, r, length):
    """A rubber buffer on a steel plate, along the builder's +Z from z = 0."""
    part.add("steel", cyl(1.3 * r, 0.0, 0.12 * length, 12), m)
    part.add("plastic", turned([(0.0, 0.12 * length), (r, 0.12 * length), (r, 0.8 * length), (0.75 * r, length), (0.0, length)], 14), m)


def switch(part, m, k):
    """A roller limit switch, its lever reaching along the builder's +Z."""
    part.add("grey", box((0.03 * k, 0.016 * k, 0.03 * k), (0, 0, -0.015 * k), tile=0.2, bevel=0.002 * k), m)
    part.add("steel", rod((0, 0.0, 0.0), (0, 0, 0.022 * k), 0.0018 * k, 6), m)
    part.add("plastic", cyl(0.006 * k, -0.003 * k, 0.003 * k, 10), Matrix(m) @ Matrix.Translation((0, 0, 0.024 * k)) @ Matrix.Rotation(math.pi / 2, 4, "X"))


def cover(part, m, size, key="paint", vents=0, bevel=None):
    """A painted cover over a drive or controller, about the builder's
    origin: vent slots and a rating plate in its ±Y faces, a status lamp on -Y."""
    w, d, h = size
    part.add(key, box(size, tile=0.4, bevel=bevel if bevel is not None else 0.05 * min(size)), m)
    for k in range(vents):
        x = (k - (vents - 1) / 2) * 0.045 * w - 0.12 * w
        for s in (-1, 1):
            part.add("plastic", box((0.018 * w, 0.004, 0.42 * h), (x, s * (d / 2 + 0.0012), 0.0), tile=0.2), m)
    if vents:
        for s in (-1, 1):
            part.add("grey", box((0.16 * w, 0.003, 0.22 * h), (0.24 * w, s * (d / 2 + 0.001), -0.12 * h), tile=0.2), m)
        part.add("led", box((0.08 * w, 0.004, 0.06 * h), (0.38 * w, -(d / 2 + 0.0012), 0.3 * h), tile=0.2), m)


def conductor(part, x0, x1, y, z, w, h, slot, mounts, to):
    """An enclosed conductor rail along X at (y, z), its collector slot
    toward `slot` (±1 in Y), end caps, a feed box at x0, and a clip and
    strap at each of `mounts` rising to the height `to`."""
    part.add("grey", bar((x0, y, z), (x1, y, z), conductor_outline(w, h, slot), UP, 0.5))
    for x in (x0 - 0.008, x1 + 0.008):
        part.add("grey", box((0.016, 1.1 * w, 1.05 * h), (x, y, z), tile=0.2, bevel=0.003))
    part.add("grey", box((0.1, 2.2 * w, 1.8 * h), (x0 + 0.06, y - slot * 0.6 * w, z), tile=0.3, bevel=0.005))
    part.add("plastic", rod((x0 + 0.06, y - slot * 0.6 * w, z + 0.9 * h), (x0 + 0.06, y - slot * 0.6 * w, to), 0.008, 8))
    for x in mounts:
        part.add("grey", box((0.03, w + 0.008, h + 0.008), (x, y, z), tile=0.2, bevel=0.002))
        part.add("steel", box((0.024, 0.004, to - z), (x, y - slot * (w / 2 + 0.006), (to + z) / 2), tile=0.2))


def lens_ring(part, m, r_lens, r_ring):
    """A camera's lens barrel and glass under the builder's origin, with an
    LED ring light round it behind a black bezel, facing -Z."""
    part.add("plastic", cyl(r_lens * 1.35, -0.6 * r_lens, 0.0, 18), m)
    part.add("lens", cyl(r_lens, -0.65 * r_lens, -0.6 * r_lens, 18), m)
    part.add("black", turned([(1.6 * r_lens, 0.0), (1.08 * r_ring, 0.0), (1.08 * r_ring, -0.12 * r_lens), (1.6 * r_lens, -0.12 * r_lens),
                              (1.6 * r_lens, 0.0)], 24), m)
    part.add("led", turned([(1.75 * r_lens, -0.13 * r_lens), (r_ring, -0.13 * r_lens), (r_ring, -0.12 * r_lens), (1.75 * r_lens, -0.12 * r_lens),
                            (1.75 * r_lens, -0.13 * r_lens)], 24), m)


def nozzle(part, m, k):
    """A spray nozzle pointing along the builder's -Z: hex body and tip."""
    part.add("steel", hexagon(0.006 * k, -0.008 * k, 0.0), m)
    part.add("steel", turned([(0.0, -0.016 * k), (0.0025 * k, -0.016 * k), (0.004 * k, -0.008 * k), (0.0, -0.008 * k)], 10), m)


def gripper(part, m, k):
    """A small parallel gripper reaching along the builder's +Z: its body
    and two fingers with pads."""
    part.add("black", box((0.044 * k, 0.024 * k, 0.024 * k), (0, 0, 0.012 * k), tile=0.2, bevel=0.002 * k), m)
    for s in (-1, 1):
        part.add("alu", box((0.006 * k, 0.016 * k, 0.034 * k), (s * 0.011 * k, 0, 0.024 * k + 0.017 * k), tile=0.2), m)
        part.add("plastic", box((0.003 * k, 0.014 * k, 0.014 * k), (s * 0.0065 * k, 0, 0.024 * k + 0.026 * k), tile=0.2), m)
