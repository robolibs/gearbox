"""In-situ resource use at the scale a few cargo landers can bring:
container-sized process units on skids, cryogenic propellant and water
storage inside a bund, an ice well drilled with coiled tubing, regolith dug,
screened and fed to an oven, and the pipe racks between them.

Also the plant's shared parts, which `power` builds with too: steel
sections, pipes with bends, valves, flanges, ladders, enclosures, fans,
heaps, signs and the materials they wear.
"""
import math
import random

import bmesh
import numpy as np
from mathutils import Matrix, Quaternion, Vector, noise

import patterns
from buildings import light_mast, mats, railing, stairs
from kit import (TAU, Building, box, cone, ibeam, lathe, merge, moved, project_uv, rect, rod, sphere, srgb_to_linear, sweep, tube, vault)

# Rack pipes: (radius, shoe) per material, the top of a rack's cross beams,
# and the spacing of pipes side by side.
PIPES = {"pipe_white": (0.11, 0.1), "pipe_yellow": (0.06, 0.0), "pipe_blue": (0.07, 0.0)}
BEAM = 0.85
GAP = 0.4
REPOSE = math.tan(math.radians(34.0))

# Tie-ins, in a module's frame: the centre line of a rack run where its
# flanges meet the module's. Runs leave the ISRU manifold along +X (three
# lines centred on blue, or white and blue centred between them), bring well
# water into the ISRU along +X, enter the tank farm along +Y, and leave the
# ice well along +X.
ISRU_TIE = Vector((4.7, -1.95, 0.0))
ISRU_TIE_PAIR = Vector((4.7, -2.15, 0.0))
WATER_TIE = Vector((-5.0, -1.6, 0.0))
TANK_TIE = Vector((2.0, -6.4, 0.0))
DRILL_TIE = Vector((8.0, 3.3, 0.0))
JOINT = 0.003

_PLANT = {}


# Materials


def _linear(colour):
    return np.asarray(srgb_to_linear(np.array(colour, dtype=np.float32)), dtype=np.float32)


def _worn(colour, seed, grime=0.1, rough=(0.55, 0.12)):
    """Maps for painted or bare metal: faint mottling and grime, a roughness that wanders."""
    def make():
        dirt = patterns.noise(cells=6, seed=seed)
        fine = patterns.noise(cells=48, seed=seed + 1, octaves=2)
        diff = _linear(colour) * (1.0 - grime * dirt - 0.05 * fine)[..., None]
        r = np.clip(rough[0] + rough[1] * (2.0 * dirt - 1.0) + 0.05 * fine, 0.05, 1.0)
        return diff.astype(np.float32), patterns.normals_from_height((0.12 * fine).astype(np.float32), 1.0), r.astype(np.float32)
    return make


def _louvre_maps():
    """Weather louvres: twenty blades to a tile, a dark slot under each."""
    u, v = patterns.grid()
    s = ((1.0 - v) * 20.0) % 1.0
    slot = s < 0.2
    blade = np.clip((s - 0.2) / 0.8, 0.0, 1.0)
    grime = patterns.noise(cells=8, seed=21)
    shade = np.where(slot, 0.06, 0.5 + 0.5 * blade) * (0.9 + 0.15 * grime)
    diff = _linear((0.62, 0.63, 0.64)) * shade[..., None]
    height = np.where(slot, 0.0, blade).astype(np.float32)
    return diff.astype(np.float32), patterns.normals_from_height(height, 5.0), np.full_like(u, 0.55)


def _label_maps():
    """A 2 x 2 atlas of plant signs: a danger plate, a radiation sign, an
    information plate and a pipe marker with flow arrows."""
    u, v = patterns.grid()
    v = 1.0 - v
    cu, cv = (u * 2.0) % 1.0, (v * 2.0) % 1.0
    cell = (np.floor(u * 2.0) + 2.0 * np.floor(v * 2.0)).astype(int)[..., None]
    edge = np.minimum(np.minimum(cu, 1.0 - cu), np.minimum(cv, 1.0 - cv))
    row = np.floor(cv * 8.0)
    line = (cv * 8.0) % 1.0
    glyph = ((cu * 34.0) % 1.0 < 0.7) & ((np.floor(cu * 34.0) + row * 5.0) % 6.0 != 0.0) & (line > 0.3) & (line < 0.72)
    text = glyph & (cu > 0.12) & (cu < 0.45 + 0.4 * ((row * 0.618) % 1.0))
    black, white = np.float32([0.02, 0.02, 0.02]), np.float32([0.80, 0.80, 0.77])
    red, yellow, blue = np.float32([0.50, 0.03, 0.02]), np.float32([0.80, 0.52, 0.02]), np.float32([0.02, 0.09, 0.32])
    danger = np.where((cv > 0.74)[..., None], red, white)
    danger = np.where(((text & (cv < 0.68)) | (edge < 0.025))[..., None], black, danger)
    dx, dy = cu - 0.5, cv - 0.5
    rr, th = np.hypot(dx, dy), np.arctan2(dy, dx)
    trefoil = ((rr > 0.1) & (rr < 0.36) & (np.cos(3.0 * (th - np.pi / 2)) > 0.0)) | (rr < 0.065) | (edge < 0.04)
    radiation = np.where(trefoil[..., None], black, yellow)
    info = np.where(((text & (cv < 0.86)) | (edge < 0.025))[..., None], white, blue)
    arrows = np.zeros_like(cu, dtype=bool)
    for c0 in (0.14, 0.86):
        arrows |= (cu > c0) & (cu < c0 + 0.1) & (np.abs(cv - 0.5) < (c0 + 0.1 - cu) * 3.0)
        arrows |= (cu > c0 - 0.1) & (cu <= c0) & (np.abs(cv - 0.5) < 0.07)
    big = ((cu * 18.0) % 1.0 < 0.7) & (np.abs(cv - 0.5) < 0.11) & (cu > 0.33) & (cu < 0.67)
    marker = np.where((arrows | big)[..., None], black, white)
    rgb = np.select([cell == k for k in range(4)], [danger, radiation, info, marker])
    rough = 0.45 + 0.1 * patterns.noise(cells=16, seed=23)
    return rgb.astype(np.float32), None, rough.astype(np.float32)


def _spoil_maps():
    """Dug regolith: mottled fines over grit, with darker basalt clasts."""
    broad = patterns.noise(cells=6, seed=31)
    mid = patterns.noise(cells=24, seed=32)
    grit = patterns.noise(cells=160, seed=33, octaves=2)
    clasts = np.clip((patterns.noise(cells=80, seed=34, octaves=2) - 0.66) * 6.0, 0.0, 1.0)
    shade = 0.72 + 0.3 * broad + 0.16 * mid + 0.12 * grit
    diff = _linear((0.34, 0.24, 0.18)) * shade[..., None]
    mix = 0.55 * clasts[..., None]
    diff = diff * (1.0 - mix) + _linear((0.22, 0.17, 0.14)) * (0.8 + 0.4 * grit)[..., None] * mix
    height = (0.5 * clasts + 0.25 * grit + 0.2 * mid).astype(np.float32)
    rough = np.clip(0.93 - 0.12 * clasts, 0.0, 1.0)
    return diff.astype(np.float32), patterns.normals_from_height(height, 5.0), rough.astype(np.float32)


def plant_mats(looks):
    """The base's materials and the plant's own, made once."""
    if _PLANT.get("looks") is looks:
        return _PLANT["set"]
    g = looks.get
    m = dict(mats(looks))
    for key, name, colour, rough, metal, seed, grime in (
        ("galv", "galvanised", (0.62, 0.63, 0.62), 0.45, 0.85, 41, 0.14),
        ("alu", "aluminium_cladding", (0.78, 0.79, 0.80), 0.38, 0.9, 42, 0.06),
        ("jacket", "vacuum_jacket", (0.74, 0.75, 0.77), 0.3, 1.0, 43, 0.05),
        ("paint_white", "plant_white", (0.84, 0.84, 0.81), 0.6, 0.1, 44, 0.14),
        ("paint_grey", "plant_grey", (0.56, 0.58, 0.58), 0.6, 0.2, 45, 0.12),
        ("paint_dark", "plant_dark", (0.20, 0.21, 0.22), 0.6, 0.3, 46, 0.1),
        ("paint_yellow", "plant_yellow", (0.88, 0.62, 0.10), 0.62, 0.15, 47, 0.16),
        ("paint_orange", "plant_orange", (0.82, 0.36, 0.08), 0.62, 0.15, 48, 0.16),
        ("paint_red", "plant_red", (0.62, 0.09, 0.06), 0.55, 0.1, 49, 0.1),
        ("carbon", "carbon_panel", (0.13, 0.13, 0.14), 0.5, 0.3, 50, 0.08),
    ):
        m[key] = g(name, colour, rough, metal, maps=_worn(colour, seed, grime, (rough, 0.1)))
    m.update({
        "foam": g("cryo_foam", (0.72, 0.47, 0.29), 0.85, 0.0, "rough_concrete", contrast=0.45),
        "footing": g("sintered_footing", (0.50, 0.43, 0.38), 0.9, 0.0, "rough_concrete", contrast=0.6),
        "bund": g("bund_wall", (0.54, 0.46, 0.40), 0.9, 0.0, "precast_concrete_wall", contrast=0.7),
        "vision": g("vision_glass", (0.05, 0.06, 0.07), 0.1, 0.0),
        "mli": g("mli_blanket", (0.84, 0.84, 0.86), 0.3, 1.0, "bi_stretch", contrast=1.5),
        "tread": g("tread_plate", (0.55, 0.55, 0.55), 0.45, 0.9, "metal_plate", contrast=0.5),
        "louvre": g("louvres", (0.62, 0.63, 0.64), 0.55, 0.4, maps=_louvre_maps),
        "label": g("plant_labels", (0.80, 0.80, 0.77), 0.5, 0.0, maps=_label_maps),
        "spoil": g("spoil", (0.34, 0.24, 0.18), 0.93, 0.0, maps=_spoil_maps),
        "basalt": g("basalt", (0.34, 0.25, 0.20), 0.85, 0.0, "rough_concrete", contrast=1.2),
        "heater": g("heater_cable", (0.85, 0.36, 0.08), 0.6, 0.0),
        "hose": g("hose", (0.09, 0.09, 0.09), 0.75, 0.0),
        "glaze": g("insulator_glaze", (0.42, 0.20, 0.11), 0.25, 0.0),
        "andon": g("andon_green", (0.1, 0.8, 0.2), 0.3, 0.0, emit=((0.1, 0.9, 0.2), 4.0)),
        "amber": g("andon_amber", (0.9, 0.55, 0.05), 0.35, 0.0),
    })
    m["spoil"]["gearbox_ground"] = True
    _PLANT.update(looks=looks, set=m)
    return m


# Sections and placement


def frame_of(a, b, up=(0, 0, 1)):
    """The matrix laying geometry built along +X from the origin onto a to b, its +Z toward `up`."""
    a, b = Vector(a), Vector(b)
    x = (b - a).normalized()
    up = Vector(up).normalized()
    if abs(x.dot(up)) > 0.99:
        up = Vector((1, 0, 0)) if abs(x.x) < 0.9 else Vector((0, 1, 0))
    z = (up - x * x.dot(up)).normalized()
    m = Matrix((x, z.cross(x), z)).transposed().to_4x4()
    m.translation = a
    return m


def member(a, b, profile, up=(0, 0, 1), tile=1.0):
    """A straight section of `profile` (x toward `up`, y across) from a to b."""
    length = (Vector(b) - Vector(a)).length
    bm = sweep(profile, [(0, 0, 0), (length, 0, 0)], tile)
    bm.transform(frame_of(a, b, up))
    return bm


def channel(w, h, t, flip=False):
    """A C profile h deep and w wide, its web on y = 0 and its flanges toward +y, or -y flipped."""
    pts = [(-h / 2, 0.0), (-h / 2, w), (-h / 2 + t, w), (-h / 2 + t, t), (h / 2 - t, t), (h / 2 - t, w), (h / 2, w), (h / 2, 0.0)]
    return [(x, -y) for x, y in reversed(pts)] if flip else pts


def angle(w, t):
    """An equal-leg L profile."""
    return [(0.0, 0.0), (w, 0.0), (w, t), (t, t), (t, w), (0.0, w)]


def aim(bm, at, axis):
    """Turns geometry built about +Z to point along `axis`, and moves it to `at`."""
    rot = Vector((0, 0, 1)).rotation_difference(Vector(axis).normalized()).to_matrix().to_4x4()
    bm.transform(Matrix.Translation(Vector(at)) @ rot)
    return bm


def ring(r_out, r_in, z0, z1, segments=24):
    """A thick ring about Z from z0 to z1, or a solid disc when r_in is 0."""
    if r_in <= 0.0:
        return lathe([(0.0, z0), (r_out, z0), (r_out, z1), (0.0, z1)], segments, 1.0)
    return lathe([(r_in, z0), (r_out, z0), (r_out, z1), (r_in, z1), (r_in, z0)], segments, 1.0)


def torus(radius, r, segments=24, sides=6, arc=TAU, start=0.0):
    """A torus about Z, or an arc of one."""
    profile = [(radius + r * math.cos(TAU * i / sides), r * math.sin(TAU * i / sides)) for i in range(sides + 1)]
    return lathe(profile, segments, 1.0, arc, start)


def heads(r, length, head=None, n=6):
    """The (radius, z) profile of a vessel with elliptical heads, pole to pole."""
    head = r / 2 if head is None else head
    steps = [math.pi / 2 * i / n for i in range(n + 1)]
    lo = [(r * math.sin(a), head * (1.0 - math.cos(a))) for a in steps]
    hi = [(r * math.cos(a), length - head + head * math.sin(a)) for a in steps]
    lo[0], hi[-1] = (0.0, 0.0), (0.0, length)
    return lo + hi


def grid(rows, floor=0.02, tile=2.0, sink=-0.06):
    """Quads over rows of (x, y, z) points, kept where a corner stands above
    `floor`; points below it sink to `sink`; faces turned up."""
    bm = bmesh.new()
    made = {}

    def vert(i, j):
        if (i, j) not in made:
            x, y, z = rows[j][i]
            made[(i, j)] = bm.verts.new((x, y, max(z, sink)))
        return made[(i, j)]

    for j in range(len(rows) - 1):
        for i in range(len(rows[j]) - 1):
            if max(rows[j][i][2], rows[j][i + 1][2], rows[j + 1][i][2], rows[j + 1][i + 1][2]) > floor:
                face = bm.faces.new([vert(i, j), vert(i + 1, j), vert(i + 1, j + 1), vert(i, j + 1)])
                face.normal_update()
                if face.normal.z < 0:
                    face.normal_flip()
    project_uv(bm, tile)
    return bm


def strip(a, b):
    """Quads between two polylines of as many points."""
    bm = bmesh.new()
    va, vb = [bm.verts.new(p) for p in a], [bm.verts.new(p) for p in b]
    for i in range(len(a) - 1):
        bm.faces.new([va[i], va[i + 1], vb[i + 1], vb[i]])
    project_uv(bm, 1.0)
    return bm


def smax(a, b, k):
    """A smooth maximum of a and b, blended over k."""
    h = max(k - abs(a - b), 0.0) / k
    return max(a, b) + h * h * k * 0.25


def clast(rng, at, size, squash=0.6):
    """A lumpy rock, partly sunk."""
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=1, radius=size)
    seed = Vector((rng.uniform(0, 50), rng.uniform(0, 50), rng.uniform(0, 50)))
    for v in bm.verts:
        v.co *= 1.0 + 0.3 * noise.noise(v.co * (2.5 / size) + seed)
        v.co.z *= squash
    moved(bm, tuple(Vector(at) - Vector((0, 0, size * squash * 0.3))), rng.uniform(0, TAU))
    project_uv(bm, 0.5)
    return bm


# Paths


def fillet(path, radius, steps=6):
    """A polyline with its corners rounded to arcs; `radius` is one value or
    one per point, used where that point is a corner."""
    pts = [Vector(p) for p in path]
    radii = list(radius) if isinstance(radius, (list, tuple)) else [radius] * len(pts)
    out = [pts[0]]
    for i in range(1, len(pts) - 1):
        p = pts[i]
        u, w = pts[i - 1] - p, pts[i + 1] - p
        lu, lw = u.length, w.length
        if lu < 1e-6 or lw < 1e-6:
            continue
        u, w = u / lu, w / lw
        theta = u.angle(w)
        if theta > math.pi - 0.02 or radii[i] <= 0.0:
            out.append(p)
            continue
        half = theta / 2
        d = min(radii[i] / math.tan(half), lu * 0.48, lw * 0.48)
        r = d * math.tan(half)
        centre = p + (u + w).normalized() * (r / math.sin(half))
        va, vc = p + u * d - centre, p + w * d - centre
        axis, turn = va.cross(vc).normalized(), va.angle(vc)
        n = max(2, int(math.ceil(steps * turn / (math.pi / 2))))
        out += [centre + Quaternion(axis, turn * k / n) @ va for k in range(n + 1)]
    out.append(pts[-1])
    clean = [out[0]]
    for q in out[1:]:
        if (q - clean[-1]).length > 1e-4:
            clean.append(q)
    return clean


def lengths(path):
    out = [0.0]
    for a, c in zip(path, path[1:]):
        out.append(out[-1] + (c - a).length)
    return out


def point_at(path, cum, s):
    """The point and unit tangent at arc length s along a polyline."""
    s = max(0.0, min(s, cum[-1]))
    for k in range(len(path) - 1):
        if s <= cum[k + 1] or k == len(path) - 2:
            t = (s - cum[k]) / max(cum[k + 1] - cum[k], 1e-9)
            return path[k].lerp(path[k + 1], t), (path[k + 1] - path[k]).normalized()


def offset(path, off):
    """A polyline moved sideways by `off`, to its left, mitred at its corners."""
    out = []
    for k, p in enumerate(path):
        ds = ([(p - path[k - 1]).normalized()] if k else []) + ([(path[k + 1] - p).normalized()] if k + 1 < len(path) else [])
        sides = [Vector((-d.y, d.x, 0.0)).normalized() for d in ds]
        s = sum(sides, Vector()).normalized()
        out.append(p + s * (off / max(s.dot(sides[0]), 0.2)))
    return out


def pipe(path, r, bend=None, segments=12, tile=1.0):
    """A pipe along a path, its corners bent at `bend`, three radii by default."""
    return tube(fillet(path, 3.0 * r if bend is None else bend), r, segments, tile)


# Fittings


def flange(at, axis, r, t=0.025):
    """A flange pair on a pipe of radius r."""
    return aim(lathe([(0.0, -t), (r * 1.8 + 0.02, -t), (r * 1.8 + 0.02, t), (0.0, t)], 16, 1.0), at, axis)


def collar(face, axis, r):
    """A flange on a pipe of radius r running out along `axis`, its face a
    hair short of the joint plane at `face`."""
    return aim(lathe([(0.0, -0.05 - JOINT), (r * 1.8 + 0.02, -0.05 - JOINT), (r * 1.8 + 0.02, -JOINT), (0.0, -JOINT)], 16, 1.0), face, axis)


def blind(at, axis, r):
    """A flange closed by a blind on the end of a pipe of radius r, facing along `axis`."""
    return aim(lathe([(0.0, -0.025), (r * 1.8 + 0.02, -0.025), (r * 1.8 + 0.02, 0.06), (0.0, 0.06)], 16, 1.0), at, axis)


def valve(b, m, at, axis, r, stem=(0, 0, 1), wheel="paint_red", body="paint_dark"):
    """A flanged gate valve on a pipe of radius r along `axis`: body, bonnet,
    stem and handwheel."""
    at, axis = Vector(at), Vector(axis).normalized()
    stem = Vector(stem)
    if abs(stem.normalized().dot(axis)) > 0.95:
        stem = axis.orthogonal()
    stem = (stem - axis * stem.dot(axis)).normalized()
    k = max(r, 0.04)
    parts = aim(lathe([(0.0, -2.4 * k), (1.9 * k, -2.4 * k), (1.9 * k, -1.9 * k), (1.4 * k, -1.8 * k), (1.6 * k, -0.8 * k),
                       (1.6 * k, 0.8 * k), (1.4 * k, 1.8 * k), (1.9 * k, 1.9 * k), (1.9 * k, 2.4 * k), (0.0, 2.4 * k)], 16, 1.0), at, axis)
    merge(parts, aim(lathe([(0.0, 0.0), (1.2 * k, 0.0), (1.2 * k, 1.6 * k), (0.8 * k, 2.4 * k), (0.8 * k, 3.4 * k), (0.0, 3.4 * k)], 12, 1.0), at, stem))
    b.put(parts, m[body])
    top = at + stem * (3.4 * k + 0.1)
    b.put(rod(at + stem * 3.4 * k, top + stem * 0.04, 0.012, 6), m["steel"])
    rim = max(0.09, 2.0 * k)
    hand = torus(rim, 0.012, 16, 5)
    for i in range(3):
        a = TAU * i / 3
        merge(hand, rod((0, 0, 0), (rim * math.cos(a), rim * math.sin(a), 0), 0.008, 4))
    merge(hand, ring(0.025, 0.0, -0.02, 0.02, 8))
    b.put(aim(hand, top, stem), m[wheel])


def gauge(b, m, at, up, face):
    """A pressure gauge on a short stem rising along `up` from `at`, its dial facing along `face`."""
    at, up = Vector(at), Vector(up).normalized()
    top = at + up * 0.12
    b.put(rod(at, top, 0.012, 6), m["steel"])
    b.put(aim(ring(0.055, 0.0, -0.015, 0.015, 16), top + up * 0.05, face), m["paint_dark"])
    b.put(aim(ring(0.047, 0.0, 0.0, 0.017, 16), top + up * 0.05, face), m["paint_white"])


def ubolt(at, d, r):
    """A U-bolt over a pipe of radius r along d, its legs down to the beam."""
    d = Vector(d).normalized()
    n, up = Vector((-d.y, d.x, 0.0)).normalized(), Vector((0, 0, 1))
    arc = [Vector(at) + n * ((r + 0.01) * math.cos(a)) + up * ((r + 0.01) * math.sin(a)) for a in [math.pi * i / 8 for i in range(9)]]
    return tube([arc[0] - up * r] + arc + [arc[-1] - up * r], 0.008, 5)


def shoe(at, d, r, h):
    """A pipe shoe: a steel T under a pipe of radius r, banded to it."""
    bm = box((0.32, 0.14, 0.012), (0, 0, -r - h + 0.006))
    merge(bm, box((0.3, 0.012, h), (0, 0, -r - h / 2)))
    merge(bm, moved(ring(r + 0.012, r - 0.004, -0.04, 0.04, 16), (0, 0, 0), 0.0, math.pi / 2))
    return moved(bm, tuple(at), math.atan2(d.y, d.x))


def marker(at, d, r, length=0.3, cell=3):
    """A printed band round a pipe along d, its arrows pointing along d."""
    bm = lathe([(r + 0.004, -length / 2), (r + 0.004, length / 2)], 16, 1.0)
    uv = bm.loops.layers.uv.get("st")
    u0, v0 = (cell % 2) * 0.5, (cell // 2) * 0.5
    for face in bm.faces:
        for loop in face.loops:
            around, along = loop[uv].uv
            loop[uv].uv = (u0 + 0.5 * along / length, v0 + 0.5 * around / (TAU * (r + 0.004)))
    return aim(bm, at, d)


def decal(w, h, cell, depth=0.006):
    """A thin plate facing +X, its face one cell of the sign atlas."""
    bm = box((depth, w, h), (depth / 2, 0, 0))
    uv = bm.loops.layers.uv.get("st")
    u0, v0 = (cell % 2) * 0.5, (cell // 2) * 0.5
    for face in bm.faces:
        front = face.normal.x > 0.9
        for loop in face.loops:
            co = loop.vert.co
            loop[uv].uv = (u0 + 0.5 * (co.y / w + 0.5), v0 + 0.5 * (co.z / h + 0.5)) if front else (u0 + 0.01, v0 + 0.01)
    return bm


def sign(b, m, at, yaw, w, h, cell):
    """A sign on a wall facing +X of its yaw."""
    b.put(moved(decal(w, h, cell), tuple(at), yaw), m["label"], smooth=False)


def stands(b, m, path, r, every=2.5, lowest=0.6):
    """Pipe stands under the level runs of a pipe: a post on a plate, a U-bolt over the pipe."""
    steel = bmesh.new()
    for a, c in zip(path, path[1:]):
        run = c - a
        if abs(run.z) > 0.01 or run.length < 1.2 or a.z < lowest:
            continue
        n = max(1, int(run.length / every))
        for k in range(n):
            p = a + run * ((k + 0.5) / n)
            merge(steel, rod((p.x, p.y, 0.02), (p.x, p.y, p.z - r), 0.03, 8))
            merge(steel, box((0.2, 0.2, 0.02), (p.x, p.y, 0.01)))
            merge(steel, box((0.14, 0.12, 0.02), (p.x, p.y, p.z - r - 0.01)))
            merge(steel, ubolt(p, run, r))
    b.put(steel, m["galv"])


# Equipment


def ladder(b, m, at, yaw, height, over=1.0, lean=0.0):
    """A rung ladder climbing `height` from `at`, its climber toward +X of its
    yaw, its stiles `over` past the top, caged from 2.3 m when it is tall;
    `lean` tips its top back toward -X."""
    width = 0.46
    parts = bmesh.new()
    for s in (-1, 1):
        merge(parts, member((0, s * width / 2, 0), (0, s * width / 2, height + over), rect(0.065, 0.014)))
    for k in range(1, int(height / 0.3) + 1):
        merge(parts, rod((0, -width / 2, k * 0.3), (0, width / 2, k * 0.3), 0.013, 6))
    if height > 2.6:
        R = 0.36
        cx = math.sqrt(R * R - (width / 2) ** 2)
        a0 = math.atan2(width / 2, -cx)
        arc = [a0 - 2.0 * a0 * i / 10 for i in range(11)]
        top = height + over
        for z in [2.3 + 0.9 * k for k in range(int((top - 2.3) / 0.9) + 1)] + [top]:
            merge(parts, tube([(cx + R * math.cos(a), R * math.sin(a), z) for a in arc], 0.01, 4))
        for t in (2, 4, 5, 6, 8):
            a = arc[t]
            merge(parts, member((cx + R * math.cos(a), R * math.sin(a), 2.3), (cx + R * math.cos(a), R * math.sin(a), top), rect(0.008, 0.04), up=(math.cos(a), math.sin(a), 0)))
    b.put(moved(parts, tuple(at), yaw, -lean), m["paint_yellow"])


def fan(r, depth=0.14):
    """A fan in its shroud under a wire guard, blowing up +Z: (shroud and blades, guard)."""
    body = ring(r + 0.03, r, 0.0, depth, 24)
    merge(body, ring(r * 0.2, 0.0, depth * 0.2, depth * 0.6, 12))
    for i in range(5):
        merge(body, moved(box((r * 0.78, r * 0.32, 0.008), (r * 0.58, 0, 0)), (0, 0, depth * 0.45), TAU * i / 5, 0.0, 0.35))
    guard = bmesh.new()
    for rr in (r * 0.95, r * 0.62, r * 0.3):
        merge(guard, moved(torus(rr, 0.006, 24, 4), (0, 0, depth + 0.01)))
    for i in range(8):
        a = TAU * i / 8
        merge(guard, rod((0, 0, depth + 0.01), (r * math.cos(a), r * math.sin(a), depth + 0.01), 0.006, 4))
    return body, guard


def cooler(b, m, at, yaw, size, fans=2, skin="paint_grey"):
    """A dry cooler on its own frame: finned coil faces on its long sides,
    fans in guards on top, a pipe pair at one end."""
    w, d, h = size
    at = tuple(at)
    b.put(moved(box((w - 0.06, d - 0.06, h - 0.12), (0, 0, h / 2 + 0.04), tile=1.0), at, yaw), m[skin], smooth=False)
    fins = bmesh.new()
    for s in (-1, 1):
        merge(fins, box((w - 0.2, 0.02, h - 0.3), (0, s * (d / 2 - 0.02), h / 2 + 0.02), tile=0.5))
    b.put(moved(fins, at, yaw), m["louvre"], smooth=False)
    frame = bmesh.new()
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(frame, box((0.06, 0.06, h), (sx * (w / 2 - 0.03), sy * (d / 2 - 0.03), h / 2)))
    for sy in (-1, 1):
        for z in (0.03, h - 0.03):
            merge(frame, box((w, 0.06, 0.06), (0, sy * (d / 2 - 0.03), z)))
    for sx in (-1, 1):
        merge(frame, box((0.06, d, 0.06), (sx * (w / 2 - 0.03), 0, h - 0.03)))
        merge(frame, box((0.12, 0.2, 0.03), (sx * (w / 2 - 0.03), 0, 0.015)))
    b.put(moved(frame, at, yaw), m["galv"], smooth=False)
    blades, guards = bmesh.new(), bmesh.new()
    r = min(d * 0.4, w / fans * 0.42)
    for k in range(fans):
        body, guard = fan(r)
        x = -w / 2 + w * (k + 0.5) / fans
        merge(blades, moved(body, (x, 0, h - 0.02)))
        merge(guards, moved(guard, (x, 0, h - 0.02)))
    b.put(moved(blades, at, yaw), m["paint_dark"])
    b.put(moved(guards, at, yaw), m["steel"])
    for k, y in enumerate((-0.15, 0.15)):
        z = h * 0.4 + 0.2 * k
        b.put(moved(pipe([(w / 2 - 0.04, y, z), (w / 2 + 0.25, y, z), (w / 2 + 0.25, y, 0.05)], 0.035), at, yaw), m["pipe_white"])


def container(b, m, at, yaw, skin, size=(6.06, 2.44, 2.59)):
    """A 20 ft ISO box about its floor's centre, its doors toward +X: walls
    in `skin`, corner posts and rails, castings, door leaves with lock rods,
    cam keepers and hinges, and a data plate."""
    L, W, H = size
    at = tuple(at)
    b.put(moved(box((L - 0.04, W - 0.04, H - 0.04), (0, 0, H / 2), tile=2.4), at, yaw), m[skin], smooth=False)
    frame, cast = bmesh.new(), bmesh.new()
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(frame, box((0.16, 0.16, H - 0.2), (sx * (L / 2 - 0.08), sy * (W / 2 - 0.08), H / 2)))
            for z in (0.06, H - 0.06):
                merge(cast, box((0.18, 0.165, 0.12), (sx * (L / 2 - 0.09), sy * (W / 2 - 0.0825), z)))
        merge(frame, box((0.12, W - 0.36, 0.22), (sx * (L / 2 - 0.06), 0, H - 0.11)))
        merge(frame, box((0.12, W - 0.36, 0.16), (sx * (L / 2 - 0.06), 0, 0.08)))
    for sy in (-1, 1):
        merge(frame, box((L - 0.36, 0.1, 0.14), (0, sy * (W / 2 - 0.05), H - 0.07)))
        merge(frame, box((L - 0.36, 0.1, 0.18), (0, sy * (W / 2 - 0.05), 0.09)))
    b.put(moved(frame, at, yaw), m[skin], smooth=False)
    b.put(moved(cast, at, yaw), m["paint_dark"], smooth=False)
    leaf = W / 2 - 0.17
    doors = bmesh.new()
    for s in (-1, 1):
        merge(doors, box((0.03, leaf, H - 0.42), (L / 2 + 0.005, s * (leaf / 2 + 0.005), H / 2), tile=2.4))
    b.put(moved(doors, at, yaw), m[skin], smooth=False)
    gear = bmesh.new()
    for y in (-0.88, -0.3, 0.3, 0.88):
        y *= W / 2.44
        merge(gear, rod((L / 2 + 0.05, y, 0.2), (L / 2 + 0.05, y, H - 0.2), 0.02, 6))
        for z in (0.2, H - 0.2):
            merge(gear, box((0.06, 0.09, 0.07), (L / 2 + 0.05, y, z)))
        merge(gear, box((0.05, 0.04, 0.32), (L / 2 + 0.08, y + 0.06, 1.05)))
    for s in (-1, 1):
        for z in (0.45, 1.0, 1.6, 2.15):
            merge(gear, rod((L / 2 + 0.03, s * (W / 2 - 0.16), z - 0.07), (L / 2 + 0.03, s * (W / 2 - 0.16), z + 0.07), 0.025, 8))
    b.put(moved(gear, at, yaw), m["steel"])
    sign(b, m, Vector(at) + Matrix.Rotation(yaw, 3, "Z") @ Vector((L / 2 + 0.02, 0.45 * W / 2.44, 1.75)), yaw, 0.3, 0.2, 2)


def side_door(b, m, at, yaw, w=0.9, h=2.0, skin="paint_grey", cell=0):
    """A personnel door facing +X of its yaw: frame, leaf, lever, hinges, a
    vision panel and a sign."""
    at = tuple(at)
    frame = box((0.05, 0.08, h + 0.06), (0.025, -w / 2 - 0.04, h / 2 + 0.03))
    merge(frame, box((0.05, 0.08, h + 0.06), (0.025, w / 2 + 0.04, h / 2 + 0.03)))
    merge(frame, box((0.05, w + 0.16, 0.08), (0.025, 0, h + 0.04)))
    b.put(moved(frame, at, yaw), m["paint_dark"], smooth=False)
    b.put(moved(box((0.035, w, h), (0.02, 0, h / 2), tile=1.0), at, yaw), m[skin], smooth=False)
    b.put(moved(box((0.01, 0.24, 0.34), (0.04, 0, h * 0.78)), at, yaw), m["vision"], smooth=False)
    hw = box((0.05, 0.14, 0.03), (0.07, w / 2 - 0.16, 1.02))
    merge(hw, box((0.02, 0.06, 0.16), (0.045, w / 2 - 0.1, 1.0)))
    for z in (0.25, h / 2, h - 0.25):
        merge(hw, rod((0.04, -w / 2 + 0.01, z - 0.06), (0.04, -w / 2 + 0.01, z + 0.06), 0.018, 6))
    b.put(moved(hw, at, yaw), m["steel"])
    sign(b, m, Vector(at) + Matrix.Rotation(yaw, 3, "Z") @ Vector((0.038, -0.1, h * 0.58)), yaw, 0.26, 0.2, cell)


def louvres(b, m, at, yaw, w, h):
    """A louvred vent facing +X of its yaw, in a frame."""
    at = tuple(at)
    b.put(moved(box((0.04, w, h), (0.02, 0, 0), tile=1.0), at, yaw), m["louvre"], smooth=False)
    frame = bmesh.new()
    for s in (-1, 1):
        merge(frame, box((0.05, 0.04, h + 0.04), (0.025, s * (w / 2 + 0.02), 0)))
        merge(frame, box((0.05, w + 0.08, 0.04), (0.025, 0, s * (h / 2 + 0.02))))
    b.put(moved(frame, at, yaw), m["paint_dark"], smooth=False)


def cabinet(b, m, at, yaw, size, skin="paint_grey", doors=1, cell=0):
    """An outdoor electrical cabinet facing +X of its yaw: doors with handles
    and signs, a rain hood, a gland plate under it."""
    d, w, h = size
    at = tuple(at)
    b.put(moved(box((d, w, h), (0, 0, h / 2 + 0.1), tile=1.0, bevel=0.01), at, yaw), m[skin], smooth=False)
    b.put(moved(box((d + 0.08, w + 0.08, 0.04), (0.02, 0, h + 0.12)), at, yaw), m[skin], smooth=False)
    b.put(moved(box((d - 0.04, w - 0.04, 0.1), (0, 0, 0.05)), at, yaw), m["paint_dark"], smooth=False)
    leaf = (w - 0.06) / doors
    hw = bmesh.new()
    for k in range(doors):
        y = -w / 2 + 0.03 + leaf * (k + 0.5)
        b.put(moved(box((0.015, leaf - 0.03, h - 0.12), (d / 2 + 0.007, y, h / 2 + 0.1), tile=1.0), at, yaw), m[skin], smooth=False)
        merge(hw, box((0.04, 0.03, 0.14), (d / 2 + 0.03, y + leaf / 2 - 0.08, h / 2 + 0.1)))
        sign(b, m, Vector(at) + Matrix.Rotation(yaw, 3, "Z") @ Vector((d / 2 + 0.016, y, h * 0.75 + 0.1)), yaw, min(0.24, leaf * 0.6), 0.17, cell)
    b.put(moved(hw, at, yaw), m["steel"])


def vessel(b, m, at, r, h, skin, lift=0.6, legs=4):
    """A vertical pressure vessel with elliptical heads standing on angle
    legs: seams at its tangent lines, a top nozzle and a nameplate."""
    at = Vector(at)
    base = at + Vector((0, 0, lift))
    b.put(moved(lathe(heads(r, h), 24, 2.0), tuple(base)), m[skin])
    seams = bmesh.new()
    for z in (r / 2, h - r / 2):
        merge(seams, moved(ring(r + 0.012, r - 0.01, z - 0.02, z + 0.02, 24), tuple(base)))
    merge(seams, moved(ring(0.05, 0.0, h - 0.05, h + 0.15, 10), tuple(base)))
    merge(seams, moved(ring(0.1, 0.0, h + 0.15, h + 0.19, 12), tuple(base)))
    b.put(seams, m["steel"])
    steel = bmesh.new()
    for i in range(legs):
        a = TAU * (i + 0.5) / legs
        c = Vector((math.cos(a), math.sin(a), 0))
        foot = at + c * (r * 0.95)
        merge(steel, member(foot, foot + Vector((0, 0, lift + r * 0.7)), angle(0.08, 0.008), up=c))
        merge(steel, moved(box((0.16, 0.16, 0.015), (0, 0, 0.008)), tuple(foot), a))
    b.put(steel, m["galv"])
    sign(b, m, base + Vector((r + 0.004, 0, h * 0.55)), 0.0, 0.22, 0.14, 2)


def drum(b, m, at, yaw, length, r, skin, saddles=(0.2, 0.8), lift=0.4, seams=1.0):
    """A horizontal vessel along +X of its yaw from `at`, on steel saddles
    over concrete plinths, its cladding banded every `seams` metres."""
    zc = lift + r
    b.put(moved(lathe(heads(r, length), 32, 2.0), tuple(Vector(at) + Vector((0, 0, zc))), yaw, math.pi / 2), m[skin])
    bands = bmesh.new()
    for k in range(1, int((length - r) / seams) + 1):
        merge(bands, moved(ring(r + 0.008, r - 0.01, -0.015, 0.015, 32), (r / 2 + k * seams - seams / 2, 0, zc), 0.0, math.pi / 2))
    b.put(moved(bands, tuple(at), yaw), m[skin])
    steel, plinths = bmesh.new(), bmesh.new()
    for t in saddles:
        merge(steel, moved(lathe([(r, -0.16), (r + 0.03, -0.16), (r + 0.03, 0.16), (r, 0.16), (r, -0.16)], 10, 1.0, math.radians(150), math.radians(-75)), (length * t, 0, zc), 0.0, math.pi / 2))
        merge(steel, box((0.03, r * 1.5, zc - r * 0.75 - lift), (length * t, 0, (zc - r * 0.75 + lift) / 2)))
        merge(steel, box((0.34, r * 1.6, 0.02), (length * t, 0, lift + 0.01)))
        merge(plinths, box((0.5, r * 1.8, lift), (length * t, 0, lift / 2)))
    b.put(moved(steel, tuple(at), yaw), m["steel"])
    b.put(moved(plinths, tuple(at), yaw), m["footing"], smooth=False)


def bottles(b, m, at, yaw, count=6):
    """Gas cylinders chained in a rack, in a row along +Y of its yaw, their fronts toward +X."""
    shells, caps, rack = bmesh.new(), bmesh.new(), bmesh.new()
    for k in range(count):
        y = (k - (count - 1) / 2) * 0.28
        merge(shells, moved(lathe([(0.0, 0.0), (0.11, 0.0), (0.115, 0.03), (0.115, 1.3), (0.08, 1.42), (0.04, 1.45), (0.0, 1.45)], 16, 1.0), (0, y, 0.05)))
        merge(caps, moved(lathe([(0.0, 1.44), (0.05, 1.44), (0.05, 1.6), (0.03, 1.62), (0.0, 1.62)], 10, 1.0), (0, y, 0.05)))
    span = count * 0.28 / 2 + 0.1
    for s in (-1, 1):
        merge(rack, member((-0.14, s * span, 0.0), (-0.14, s * span, 1.3), rect(0.05, 0.05)))
    for z in (0.6, 1.15):
        merge(rack, member((-0.14, -span, z), (-0.14, span, z), rect(0.04, 0.04)))
        merge(rack, tube([(-0.14, -span, z), (0.13, -span + 0.1, z), (0.13, span - 0.1, z), (-0.14, span, z)], 0.006, 4))
    merge(rack, box((0.4, span * 2, 0.05), (0, 0, 0.025)))
    b.put(moved(shells, tuple(at), yaw), m["pipe_blue"])
    b.put(moved(caps, tuple(at), yaw), m["steel"])
    b.put(moved(rack, tuple(at), yaw), m["galv"])


def tee(b, m, at, d, half, top):
    """A T-support under pipes running along d: an I-beam column on a footing,
    a cross beam `half` either side with its top at `top`, knee braces."""
    at = Vector(at)
    d = Vector((d.x, d.y, 0.0)).normalized()
    n = Vector((-d.y, d.x, 0.0))
    steel = member(at + Vector((0, 0, 0.25)), at + Vector((0, 0, top - 0.14)), ibeam(0.12, 0.12, 0.01), up=d)
    merge(steel, member(at - n * half + Vector((0, 0, top - 0.07)), at + n * half + Vector((0, 0, top - 0.07)), ibeam(0.1, 0.14, 0.01)))
    reach = min(half - 0.05, 0.4)
    for s in (-1, 1):
        merge(steel, member(at + Vector((0, 0, top - 0.55)), at + n * (s * reach) + Vector((0, 0, top - 0.14)), angle(0.05, 0.006), up=d))
    yaw = math.atan2(d.y, d.x)
    merge(steel, moved(box((0.3, 0.3, 0.02), (0, 0, 0.26)), tuple(at), yaw))
    b.put(steel, m["galv"])
    b.put(moved(box((0.5, 0.5, 0.32), (0, 0, 0.09)), tuple(at), yaw), m["footing"], smooth=False)


def rack_pipes(b, m, runs, ends):
    """Pipes along their paths, already at rack height, with their fittings:
    shoes or U-bolts where a beam passes under, cladding bands or flanged
    spools, markers every ten metres, and at each end what `ends` names:
    "flange" mates a module's flange at the joint, "seal" stops at a wall's
    seals with an isolation valve out from it, "cap" closes with a valve and
    a blind. `runs` is [(material, path, [(point, tangent) of each support])]."""
    for i, (colour, path, supports) in enumerate(runs):
        r, h = PIPES[colour]
        trimmed = list(path)
        for k, inner, end in ((0, 1, ends[0]), (-1, -2, ends[1])):
            if end != "cap":
                trimmed[k] = path[k] + (path[inner] - path[k]).normalized() * JOINT
        b.put(tube(trimmed, r, 14 if r > 0.08 else 12), m[colour])
        cum = lengths(path)
        fit = bmesh.new()
        if h > 0.0:
            for p, d in supports:
                merge(fit, shoe(p, d, r, h))
            for k in range(1, int(cum[-1])):
                p, d = point_at(path, cum, float(k))
                merge(fit, aim(ring(r + 0.006, r - 0.01, -0.012, 0.012, 16), p, d))
            b.put(fit, m["alu"])
        else:
            for p, d in supports:
                merge(fit, ubolt(p, d, r))
            spools = int(cum[-1] / 6.0)
            for k in range(1, spools + 1):
                p, d = point_at(path, cum, cum[-1] * k / (spools + 1))
                merge(fit, flange(p, d, r))
            b.put(fit, m["steel"])
        bands = bmesh.new()
        for k in range(int((cum[-1] - 2.0) / 10.0) + 1):
            p, d = point_at(path, cum, 1.5 + k * 10.0)
            merge(bands, marker(p, d, r))
        b.put(bands, m["label"], smooth=False)
        caps = bmesh.new()
        wheel = "paint_yellow" if colour == "pipe_yellow" else "paint_red"
        for end, tip, inner, s in ((ends[0], path[0], path[1], 0.0), (ends[1], path[-1], path[-2], cum[-1])):
            out = (tip - inner).normalized()
            if end == "flange":
                merge(caps, collar(tip, out, r))
                continue
            back = (0.7 if end == "cap" else 1.5) + 0.5 * i
            p, d = point_at(path, cum, abs(s - back))
            valve(b, m, p, d, r, wheel=wheel)
            if end == "cap":
                merge(caps, blind(tip, out, r))
        b.put(caps, m["steel"])


def auger(length, r_in, r_out, pitch=0.3, steps=16):
    """A flighted auger along +X from the origin: (shaft and flight)."""
    shaft = ring(r_in, 0.0, 0.0, length, 10)
    moved(shaft, (0, 0, 0), 0.0, math.pi / 2)
    merge(shaft, moved(cone(r_in, 0.01, 0.2, segments=10), (length, 0, 0), 0.0, math.pi / 2))
    flight = bmesh.new()
    uv = flight.loops.layers.uv.new("st")
    n = int((length - 0.2) / pitch * steps)
    rows = []
    for i in range(n + 1):
        a = TAU * i / steps
        x = 0.1 + (length - 0.25) * i / n
        rows.append([flight.verts.new((x, r * math.cos(a), r * math.sin(a))) for r in (r_in, r_out)])
    for i in range(n):
        face = flight.faces.new([rows[i][0], rows[i + 1][0], rows[i + 1][1], rows[i][1]])
        for loop, st in zip(face.loops, ((i, 0), (i + 1, 0), (i + 1, 1), (i, 1))):
            loop[uv].uv = (st[0] * 0.05, st[1] * 0.1)
    return shaft, flight


def bag(b, m, at, yaw, full=1.0):
    """A filled bulk bag on a steel pallet, its lift loops up."""
    at = Vector(at)
    h = 0.12 + 0.88 * full
    body = lathe([(0.0, 0.12), (0.64, 0.12), (0.71, 0.3), (0.72, h - 0.25), (0.66, h - 0.04), (0.3, h), (0.0, h + 0.02)], 4, 1.0, start=math.pi / 4)
    b.put(moved(body, tuple(at), yaw), m["fabric"], smooth=False)
    loops = bmesh.new()
    for i in range(4):
        a = math.pi / 4 + TAU * i / 4
        loop = moved(torus(0.08, 0.015, 10, 4, math.pi), (0, 0, 0), 0.0, 0.0, math.pi / 2)
        merge(loops, moved(loop, (0.43 * math.cos(a), 0.43 * math.sin(a), h - 0.08), a + math.pi / 2))
    b.put(moved(loops, tuple(at), yaw), m["fabric"])
    b.put(moved(box((1.1, 1.1, 0.12), (0, 0, 0.06)), tuple(at), yaw), m["galv"], smooth=False)


# Modules


def isru_unit(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """One landed ISRU unit, a MOXIE-style CO2 electrolysis and Sabatier
    plant in a 20 ft process container on a skid with levelling jacks: a
    filtered CO2 intake, cyclone and compressor at its back, a liquefier and
    separator on its roof, a radiator array of flat panels beside it, gas
    bottles, the product manifold at its front and the well water inlet at
    its back (both flanged tie-ins, see ISRU_TIE and WATER_TIE), and stairs
    to its side door."""
    m = plant_mats(looks)
    b = Building(name, parent, at, yaw)
    L, W, H = 6.06, 2.44, 2.59
    z0 = 0.55
    cx = 0.6
    x0, x1 = cx - L / 2, cx + L / 2
    top = z0 + H
    sx0, sx1, sw = -4.3, 3.9, 1.22

    # Skid: an I-beam frame, a tread deck at its back, six levelling jacks.
    skid = bmesh.new()
    for s in (-1, 1):
        merge(skid, member((sx0, s * sw, z0 - 0.15), (sx1, s * sw, z0 - 0.15), ibeam(0.15, 0.3, 0.014)))
    for x in (sx0 + 0.08, -2.3, -0.6, 1.2, 2.9, sx1 - 0.08):
        merge(skid, member((x, -sw + 0.08, z0 - 0.15), (x, sw - 0.08, z0 - 0.15), ibeam(0.12, 0.26, 0.012)))
    b.put(skid, m["paint_dark"])
    b.put(box((x0 - sx0, 2 * sw + 0.15, 0.02), ((sx0 + x0) / 2, 0, z0 + 0.01), tile=1.0), m["tread"], smooth=False)
    jacks, pads = bmesh.new(), bmesh.new()
    for x in (sx0 + 0.35, -0.6, sx1 - 0.35):
        for s in (-1, 1):
            p = Vector((x, s * (sw + 0.2), 0.0))
            merge(jacks, box((0.3, 0.14, 0.26), (x, s * (sw + 0.1), z0 - 0.15)))
            merge(jacks, box((0.15, 0.15, 0.44), tuple(p + Vector((0, 0, 0.37)))))
            merge(pads, rod(p + Vector((0, 0, 0.59)), p + Vector((0, s * 0.16, 0.66)), 0.015, 6))
            merge(pads, rod(p + Vector((0, 0, 0.05)), p + Vector((0, 0, 0.2)), 0.03, 8))
            merge(pads, moved(lathe([(0.0, 0.0), (0.2, 0.0), (0.2, 0.04), (0.06, 0.07), (0.0, 0.07)], 16, 1.0), tuple(p)))
    b.put(jacks, m["paint_yellow"], smooth=False)
    b.put(pads, m["steel"])

    # The process container, its side door, a landing and stairs.
    container(b, m, (cx, 0, z0), 0.0, "container_white")
    dx = 1.9
    side_door(b, m, (dx, W / 2, z0), math.pi / 2, 0.9, 2.0, "paint_white")
    b.put(box((1.4, 1.0, 0.05), (dx, W / 2 + 0.5, z0 - 0.025), tile=1.0), m["grate"], smooth=False)
    for x in (dx - 0.65, dx + 0.65):
        b.put(member((x, W / 2 + 0.95, 0.0), (x, W / 2 + 0.95, z0 - 0.05), rect(0.06, 0.06)), m["galv"], smooth=False)
    stairs(b, m, (dx - 0.7 - 0.84, W / 2 + 0.5, 0.0), 0.0, z0, 0.9)
    railing(b, m, [(dx - 0.7, W / 2 + 1.0, z0), (dx + 0.7, W / 2 + 1.0, z0), (dx + 0.7, W / 2 + 0.05, z0)], 1.0, 0.7)
    b.put(box((0.12, 0.26, 0.1), (dx, W / 2 + 0.07, z0 + 2.25), bevel=0.01), m["paint_dark"], smooth=False)
    b.put(box((0.1, 0.22, 0.02), (dx, W / 2 + 0.13, z0 + 2.2)), m["lamp"], smooth=False)
    b.places.append(("light", {"at": (dx, W / 2 + 0.4, z0 + 2.15), "kind": "door"}))
    louvres(b, m, (-0.3, W / 2, z0 + 2.0), math.pi / 2, 0.9, 0.5)
    louvres(b, m, (0.8, W / 2, z0 + 2.0), math.pi / 2, 0.9, 0.5)
    louvres(b, m, (-1.4, -W / 2, z0 + 0.6), -math.pi / 2, 0.9, 0.5)
    sign(b, m, (2.75, W / 2 + 0.01, z0 + 1.6), math.pi / 2, 0.5, 0.36, 0)
    sign(b, m, (-2.0, -W / 2 - 0.01, z0 + 1.7), -math.pi / 2, 0.4, 0.3, 2)

    # Power: a disconnect cabinet, conduits down to a ladder tray along the skid.
    cabinet(b, m, (-1.3, W / 2 + 0.17, z0 + 0.7), math.pi / 2, (0.3, 1.0, 1.1), doors=2)
    tray = bmesh.new()
    ty, tz = sw + 0.45, 0.32
    for s in (-1, 1):
        merge(tray, member((sx0 - 0.3, ty + s * 0.15, tz), (-1.0, ty + s * 0.15, tz), channel(0.03, 0.08, 0.005, s < 0)))
    for k in range(int((-1.0 - sx0 + 0.3) / 0.3) + 1):
        merge(tray, box((0.03, 0.3, 0.02), (sx0 - 0.3 + k * 0.3, ty, tz - 0.03)))
    for x in (-3.0, -1.6):
        merge(tray, member((x, ty, 0.0), (x, ty, tz - 0.04), rect(0.05, 0.05)))
    b.put(tray, m["galv"], smooth=False)
    cables = bmesh.new()
    for k in range(3):
        y = ty - 0.08 + 0.08 * k
        merge(cables, pipe([(-1.55 + 0.12 * k, W / 2 + 0.17, z0 + 0.8), (-1.55 + 0.12 * k, W / 2 + 0.17, tz + 0.1), (-1.85, y, tz), (sx0 - 0.2, y, tz),
                            (sx0 - 0.75, y, 0.3), (sx0 - 0.75, y, 0.05)], 0.025, 0.25, 8))
    b.put(cables, m["cable"])
    b.put(box((0.7, 0.7, 0.24), (sx0 - 0.85, ty, 0.07)), m["footing"], smooth=False)
    b.put(box((0.6, 0.6, 0.02), (sx0 - 0.85, ty, 0.2), tile=1.0), m["tread"], smooth=False)

    # Radiator: flat two-sided panels on a frame beside the container, headers top and bottom.
    ry = -2.55
    posts = [-2.2 + k * 1.2 for k in range(5)]
    panels, tubes = bmesh.new(), bmesh.new()
    for xa, xb in zip(posts, posts[1:]):
        merge(panels, box((xb - xa - 0.1, 0.025, 2.3), ((xa + xb) / 2, ry, 1.85), tile=1.0))
        for k in range(6):
            x = xa + 0.14 + (xb - xa - 0.28) * k / 5
            merge(tubes, rod((x, ry, 0.72), (x, ry, 2.98), 0.013, 6))
    for z in (0.66, 3.04):
        merge(tubes, tube([(posts[0], ry, z), (posts[-1], ry, z)], 0.045, 10))
    b.put(panels, m["paint_white"], smooth=False)
    b.put(tubes, m["alu"])
    frame, feet = bmesh.new(), bmesh.new()
    for x in posts:
        merge(frame, member((x, ry, 0.25), (x, ry, 3.25), ibeam(0.1, 0.1, 0.008), up=(1, 0, 0)))
        merge(feet, moved(box((0.45, 0.45, 0.3), (0, 0, 0.1)), (x, ry, 0)))
    merge(frame, member((posts[0], ry, 3.22), (posts[-1], ry, 3.22), channel(0.05, 0.1, 0.006)))
    for x in (posts[0], posts[2], posts[-1]):
        merge(frame, member((x, ry - 1.1, 0.12), (x, ry - 0.05, 2.6), angle(0.06, 0.006), up=(0, 1, 0)))
        merge(feet, moved(box((0.35, 0.35, 0.25), (0, 0, 0.06)), (x, ry - 1.1, 0)))
    b.put(frame, m["galv"])
    b.put(feet, m["footing"], smooth=False)
    for x, z, zw in ((0.4, 3.04, 2.75), (1.6, 0.66, 1.0)):
        b.put(pipe([(x, ry, z), (x, ry + 0.45, z), (x, -W / 2 - 0.45, zw), (x, -W / 2 - 0.02, zw)], 0.04, 0.3), m["pipe_white"])
        valve(b, m, (x, -W / 2 - 0.3, zw), (0, 1, 0), 0.04, stem=(1, 0, 0))

    # The CO2 intake: a raised hood over a filter box, a cyclone and a compressor.
    fx, fy = -3.45, 0.5
    b.put(box((1.1, 1.2, 1.2), (fx, fy, z0 + 0.6), tile=1.0, bevel=0.02), m["paint_white"], smooth=False)
    louvres(b, m, (fx - 0.55, fy, z0 + 0.65), math.pi, 0.9, 0.8)
    louvres(b, m, (fx, fy + 0.6, z0 + 0.65), math.pi / 2, 0.8, 0.8)
    b.put(pipe([(fx, fy, z0 + 1.2), (fx, fy, 4.3)], 0.17), m["paint_white"])
    b.put(moved(cone(0.38, 0.17, 0.35, segments=20, caps=False), (fx, fy, 4.25)), m["paint_white"])
    b.put(moved(ring(0.4, 0.36, 4.12, 4.26, 20), (fx, fy, 0)), m["paint_dark"])
    for k in range(3):
        a = TAU * k / 3
        b.put(rod((fx + 0.17 * math.cos(a), fy + 0.17 * math.sin(a), 4.0), (fx + 0.36 * math.cos(a), fy + 0.36 * math.sin(a), 4.2), 0.012, 4), m["steel"])
    cyx, cyy = -3.75, -0.7
    b.put(moved(lathe([(0.0, 0.0), (0.05, 0.0), (0.05, 0.1), (0.22, 0.55), (0.22, 1.1), (0.15, 1.18), (0.06, 1.2), (0.06, 1.3), (0.0, 1.3)], 20, 1.0), (cyx, cyy, z0 + 0.55)), m["jacket"])
    b.put(moved(ring(0.14, 0.0, 0.0, 0.42, 14), (cyx, cyy, z0 + 0.02)), m["paint_dark"])
    b.put(pipe([(fx - 0.2, fy - 0.6, z0 + 1.0), (cyx + 0.15, cyy + 0.18, z0 + 1.5)], 0.06), m["jacket"])
    b.put(pipe([(cyx, cyy, z0 + 1.85), (cyx, cyy, z0 + 2.05), (-3.2, cyy, z0 + 2.05), (-3.2, cyy, z0 + 0.66)], 0.05), m["jacket"])
    b.put(box((0.75, 0.7, 0.65), (-2.95, -0.75, z0 + 0.33), tile=1.0, bevel=0.02), m["paint_grey"], smooth=False)
    b.put(moved(ring(0.17, 0.0, 0.0, 0.5, 16), (-2.95, -0.75, z0 + 0.85), 0.0, math.pi / 2), m["paint_dark"])
    b.put(pipe([(-2.75, -0.45, z0 + 0.4), (x0 - 0.02, -0.45, z0 + 0.4)], 0.05), m["jacket"])
    gauge(b, m, (-2.5, -0.45, z0 + 0.45), (0, 0, 1), (-1, 0, 0))
    flanges = flange((-2.6, -0.45, z0 + 0.4), (1, 0, 0), 0.05)
    merge(flanges, flange((cyx, cyy, z0 + 1.95), (0, 0, 1), 0.05))
    b.put(flanges, m["steel"])
    bottles(b, m, (-3.3, W / 2 + 0.75, 0.0), math.pi / 2, 6)
    railing(b, m, [(sx0 + 0.05, -sw, z0), (sx0 + 0.05, sw, z0)], 1.0, 1.2)

    # Roof: a liquefier with fans, a separator, vents, a status light and an antenna.
    cooler(b, m, (1.9, -0.1, top), 0.0, (2.4, 1.7, 0.85), 2, "paint_white")
    vessel(b, m, (-1.4, -0.5, top), 0.32, 1.3, "jacket", lift=0.35, legs=3)
    b.put(pipe([(-1.4, -0.5, top + 0.36), (-1.4, -0.5, top + 0.2), (-0.6, -0.5, top + 0.2), (-0.6, -0.5, top + 0.02)], 0.035), m["jacket"])
    for x, y in ((-0.3, 0.7), (-2.0, 0.7)):
        b.put(moved(lathe([(0.0, 0.0), (0.09, 0.0), (0.09, 0.3), (0.2, 0.32), (0.2, 0.38), (0.0, 0.42)], 14, 1.0), (x, y, top)), m["paint_grey"])
    b.put(rod((x1 - 0.25, W / 2 - 0.25, top), (x1 - 0.25, W / 2 - 0.25, top + 0.5), 0.02, 6), m["steel"])
    for k, key in enumerate(("paint_red", "amber", "andon")):
        b.put(moved(ring(0.05, 0.0, 0.0, 0.08, 12), (x1 - 0.25, W / 2 - 0.25, top + 0.5 + 0.085 * k)), m[key])
    b.put(rod((x1 - 0.25, -W / 2 + 0.25, top), (x1 - 0.25, -W / 2 + 0.25, top + 1.6), 0.008, 4), m["paint_dark"])
    ladder(b, m, (-2.1, -W / 2 - 0.12, 0.0), -math.pi / 2, top)
    e = 0.08
    railing(b, m, [(-1.8, -W / 2 + e, top), (x1 - e, -W / 2 + e, top), (x1 - e, W / 2 - e, top), (x0 + e, W / 2 - e, top), (x0 + e, -W / 2 + 0.55, top)], 1.0, 1.2)

    # The product manifold at the front: oxygen, water and methane out to the
    # rack, each valved, its flange at the tie-in.
    for k, colour in enumerate(("pipe_white", "pipe_blue", "pipe_yellow")):
        r, h = PIPES[colour]
        x, y, z = 2.85 + 0.32 * k, ISRU_TIE.y + GAP * (k - 1), BEAM + h + r
        wall = 1.9 - 0.3 * k
        b.put(pipe([(x, -W / 2 - 0.02, wall), (x, y, wall), (x, y, z), (ISRU_TIE.x - 0.01, y, z)], r, 0.25), m[colour])
        valve(b, m, (sx1 + 0.3, y, z), (1, 0, 0), r, wheel="paint_yellow" if colour == "pipe_yellow" else "paint_red")
        gauge(b, m, (sx1 - 0.55, y, z + r), (0, 0, 1), (0, -1, 0))
        b.put(collar(Vector((ISRU_TIE.x, y, z)), (1, 0, 0), r), m["steel"])
        b.put(marker(Vector((sx1 - 0.25, y, z)), Vector((1, 0, 0)), r), m["label"], smooth=False)
    tee(b, m, (sx1 - 0.1, ISRU_TIE.y, 0), Vector((1, 0, 0)), 0.65, BEAM)

    # The well water inlet at the back, out of the end wall over the
    # compressor to its flange at the tie-in.
    r = PIPES["pipe_blue"][0]
    zw = BEAM + r
    inlet = [Vector(p) for p in ((x0 - 0.02, -1.05, 1.5), (-2.75, -1.05, 1.5), (-2.75, WATER_TIE.y, 1.5), (-2.75, WATER_TIE.y, zw), (WATER_TIE.x + 0.01, WATER_TIE.y, zw))]
    b.put(pipe(inlet, r, 0.2), m["pipe_blue"])
    b.put(collar(Vector((WATER_TIE.x, WATER_TIE.y, zw)), (-1, 0, 0), r), m["steel"])
    valve(b, m, (-4.35, WATER_TIE.y, zw), (1, 0, 0), r)
    gauge(b, m, (-3.3, WATER_TIE.y, zw + r), (0, 0, 1), (0, -1, 0))
    b.put(marker(Vector((-3.75, WATER_TIE.y, zw)), Vector((1, 0, 0)), r), m["label"], smooth=False)
    stands(b, m, inlet, r)

    # A lamp over the end doors.
    b.put(box((0.12, 0.4, 0.1), (x1 + 0.08, 0, top - 0.12), bevel=0.01), m["paint_dark"], smooth=False)
    b.put(box((0.02, 0.34, 0.06), (x1 + 0.15, 0, top - 0.14)), m["lamp"], smooth=False)
    b.places.append(("light", {"at": (x1 + 0.45, 0, top - 0.1), "kind": "door"}))

    b.collider(box((L, W, top), (cx, 0, top / 2)))
    b.collider(box((sx1 - sx0, 2 * sw + 0.4, z0), ((sx0 + sx1) / 2, 0, z0 / 2)))
    b.collider(box((x0 - sx0, 2 * sw, 1.8), ((sx0 + x0) / 2, 0, z0 + 0.9)))
    b.collider(box((posts[-1] - posts[0], 0.3, 3.25), ((posts[0] + posts[-1]) / 2, ry, 1.625)))
    b.collider(box((2.4, 1.7, 0.85), (1.9, -0.1, top + 0.425)))
    return b.finish()


def _sphere_tank(b, m, centre, r, skin, seams):
    """A spherical cryogenic tank on six columns: an equator girder, X
    bracing above pipe level, a manway, relief valves and a cryocooler on
    top, a railed platform and a walkway to a caged ladder at -X. Returns
    the relief outlet."""
    cx, cy = centre
    zc = r + 1.3
    c = Vector((cx, cy, zc))
    b.put(sphere(r, tuple(c), 48, 24), m[skin])
    girder = moved(ring(r + 0.035, r - 0.05, -0.1, 0.1, 48), tuple(c))
    if seams:
        skin_seams = bmesh.new()
        for lat in (-0.62, 0.62):
            rr = r * math.cos(lat)
            merge(skin_seams, moved(ring(rr + 0.01, rr - 0.03, r * math.sin(lat) - 0.015, r * math.sin(lat) + 0.015, 48), tuple(c)))
        for k in range(8):
            a = TAU * (k + 0.5) / 8
            merge(skin_seams, tube([c + Vector((math.cos(a) * math.cos(t), math.sin(a) * math.cos(t), math.sin(t))) * (r + 0.006) for t in [-0.62 + 1.24 * i / 10 for i in range(11)]], 0.012, 4))
        b.put(skin_seams, m[skin])
    columns, plinths = bmesh.new(), bmesh.new()
    feet = []
    for k in range(6):
        a = math.radians(30 + 60 * k)
        p = Vector((cx + r * math.cos(a), cy + r * math.sin(a), 0.0))
        feet.append(p)
        merge(columns, tube([p + Vector((0, 0, 0.33)), p + Vector((0, 0, zc))], 0.1, 12))
        merge(columns, moved(box((0.38, 0.38, 0.03), (0, 0, 0.315)), tuple(p), a))
        merge(columns, moved(box((0.02, 0.32, 0.5), (0.0, 0, zc - 0.3)), tuple(p), a + math.pi / 2))
        merge(plinths, moved(box((0.6, 0.6, 0.36), (0, 0, 0.12)), tuple(p), a))
    for k in range(6):
        p, q = feet[k], feet[(k + 1) % 6]
        lo, hi = Vector((0, 0, 1.25)), Vector((0, 0, zc - 0.7))
        merge(columns, rod(p + lo, q + hi, 0.02, 6))
        merge(columns, rod(p + hi, q + lo, 0.02, 6))
        merge(columns, moved(ring(0.07, 0.0, -0.01, 0.01, 10), tuple((p + q) / 2 + (lo + hi) / 2)))
    merge(columns, girder)
    b.put(columns, m["galv"])
    b.put(plinths, m["footing"], smooth=False)
    # Top: manway, relief valves, a cryocooler and the platform.
    top = zc + r
    zp = top + 0.25
    fit = moved(ring(0.3, 0.0, -0.1, 0.22, 20), (cx, cy, top))
    merge(fit, moved(ring(0.37, 0.0, 0.22, 0.27, 20), (cx, cy, top)))
    for k in range(16):
        a = TAU * k / 16
        merge(fit, box((0.03, 0.03, 0.04), (cx + 0.33 * math.cos(a), cy + 0.33 * math.sin(a), top + 0.29)))
    relief = Vector((cx + 0.15, cy + 0.6 * (1 if cy < 0 else -1), 0.0))
    for k in (-1, 1):
        p = relief + Vector((0.18 * k, 0, 0))
        z = zc + math.sqrt(max(r * r - (p.x - cx) ** 2 - (p.y - cy) ** 2, 0.0))
        merge(fit, tube([(p.x, p.y, z - 0.05), (p.x, p.y, zp + 0.2)], 0.035, 10))
        merge(fit, moved(lathe([(0.0, 0.0), (0.06, 0.0), (0.06, 0.22), (0.04, 0.3), (0.0, 0.3)], 12, 1.0), (p.x, p.y, zp + 0.2)))
    merge(fit, tube([relief + Vector((-0.18, 0, zp + 0.42)), relief + Vector((0.18, 0, zp + 0.42))], 0.035, 10))
    b.put(fit, m["jacket"])
    cooler(b, m, (cx + 0.45, cy - 0.45 * (1 if cy < 0 else -1), zp), 0.0, (0.55, 0.45, 0.42), 1, "paint_white")
    deck = moved(ring(0.95, 0.4, -0.03, 0.0, 24), (cx, cy, zp))
    merge(deck, box((r + 0.6 - 0.9, 0.7, 0.03), (cx - (0.9 + r + 0.6) / 2, cy, zp - 0.015)))
    b.put(deck, m["grate"], smooth=False)
    struts = bmesh.new()
    for k in range(6):
        a = TAU * k / 6 + 0.3
        p = Vector((cx + 0.8 * math.cos(a), cy + 0.8 * math.sin(a), 0.0))
        z = zc + math.sqrt(r * r - 0.64)
        merge(struts, rod((p.x, p.y, z - 0.05), (p.x, p.y, zp - 0.03), 0.025, 6))
    merge(struts, rod((cx - r - 0.1, cy, zp - 0.03), (cx - r * 0.8, cy, zc + r * 0.55), 0.03, 6))
    b.put(struts, m["galv"])
    rim = [(cx + 0.92 * math.cos(a), cy + 0.92 * math.sin(a), zp) for a in [math.radians(200 + 320 * i / 16) for i in range(17)]]
    railing(b, m, rim, 1.0, 0.6)
    for s in (-1, 1):
        railing(b, m, [(cx - 0.9, cy + s * 0.35, zp), (cx - r - 0.6, cy + s * 0.35, zp)], 1.0, 0.8)
    ladder(b, m, (cx - r - 0.62, cy, 0.0), math.pi, zp)
    a = math.radians(330)
    sign(b, m, (cx + (r + 0.105) * math.cos(a), cy + (r + 0.105) * math.sin(a), 1.8), a, 0.4, 0.3, 0)
    return relief + Vector((0, 0, zp + 0.42))


def propellant_tanks(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """The ISRU products in store: liquid oxygen in a vacuum-jacketed sphere
    and methane in a foam-insulated one, both on columns with cross bracing,
    zero-boil-off cryocoolers on top, ladders and platforms; water in two
    clad bullets on saddles under a shared platform; product headers from
    the rack's tie-in on the -Y wall (TANK_TIE) crossing over each other on
    risers, a vent stack, and a regolith-concrete bund with a stile over its
    +X wall."""
    m = plant_mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(box((18.0, 12.0, 0.08), (0, 0, 0.0), tile=2.0), m["pad"], smooth=False)
    walls = bmesh.new()
    for s in (-1, 1):
        merge(walls, box((18.3, 0.3, 0.6), (0, s * 6.0, 0.3), tile=1.5))
        merge(walls, box((0.3, 11.7, 0.6), (s * 9.0, 0, 0.3), tile=1.5))
        merge(walls, box((18.4, 0.36, 0.05), (0, s * 6.0, 0.625), tile=1.5))
        merge(walls, box((0.36, 11.64, 0.05), (s * 9.0, 0, 0.625), tile=1.5))
    b.put(walls, m["bund"], smooth=False)
    b.put(box((0.7, 0.7, 0.03), (8.3, 5.3, 0.05), tile=0.5), m["grate"], smooth=False)

    # The spheres: oxygen jacketed and seamed, methane under sprayed foam.
    r = 2.1
    lox, ch4 = (-4.6, -2.6), (-4.6, 2.6)
    vents = [_sphere_tank(b, m, lox, r, "paint_white", True), _sphere_tank(b, m, ch4, r, "foam", False)]

    # The water bullets and the platform over their crowns.
    wr, wl, wz = 1.1, 6.5, 0.7
    for y in (-1.6, 1.6):
        drum(b, m, (0.5, y, 0), 0.0, wl, wr, "alu", lift=wz)
        nz = bmesh.new()
        for x in (2.3, 4.6):
            merge(nz, tube([(x, y, wz + 2 * wr - 0.05), (x, y, wz + 2 * wr + 0.25)], 0.06, 12))
            merge(nz, flange((x, y, wz + 2 * wr + 0.25), (0, 0, 1), 0.06))
        merge(nz, aim(ring(0.3, 0.0, -0.05, 0.22, 20), (0.5 + wl - 0.05, y, wz + wr), (1, 0, 0)))
        merge(nz, aim(ring(0.36, 0.0, 0.22, 0.27, 20), (0.5 + wl - 0.05, y, wz + wr), (1, 0, 0)))
        merge(nz, tube([(0.5 + wl + 0.2, y + 0.3, wz + wr), (0.5 + wl + 0.2, y + 0.3, wz + wr + 0.5), (0.5 + wl - 0.1, y + 0.3, wz + wr + 0.5)], 0.02, 6))
        b.put(nz, m["steel"])
        valve(b, m, (2.3, y, wz + 2 * wr + 0.45), (0, 0, 1), 0.06, stem=(1, 0, 0))
        b.put(box((0.25, 0.18, 0.3), (4.6, y, wz + 2 * wr + 0.42), bevel=0.01), m["paint_grey"], smooth=False)
        sign(b, m, (0.5 + 1.0, y - wr * math.copysign(1, y) - 0.01, wz + wr), -math.pi / 2 if y < 0 else math.pi / 2, 0.5, 0.36, 2)
    zp = wz + 2 * wr + 0.25
    b.put(box((1.3, 4.0, 0.04), (5.65, 0, zp - 0.02), tile=1.0), m["grate"], smooth=False)
    b.put(box((1.15, 0.8, 0.04), (6.9, 0, zp - 0.02), tile=1.0), m["grate"], smooth=False)
    legs = bmesh.new()
    for y in (-0.36, 0.36):
        merge(legs, member((7.42, y, 0.04), (7.42, y, zp - 0.04), rect(0.06, 0.06)))
    for y in (-1.6, 1.6):
        merge(legs, member((5.65, y, wz + 2 * wr - 0.05), (5.65, y, zp - 0.04), rect(0.08, 0.08)))
    b.put(legs, m["galv"], smooth=False)
    railing(b, m, [(6.3, -0.4, zp), (6.3, -2.0, zp), (5.0, -2.0, zp), (5.0, 2.0, zp), (6.3, 2.0, zp), (6.3, 0.4, zp), (7.45, 0.4, zp)], 1.0, 0.7)
    railing(b, m, [(6.3, -0.4, zp), (7.45, -0.4, zp)], 1.0, 0.7)
    ladder(b, m, (7.5, 0, 0), 0.0, zp)

    # Headers from the tie-in on the -Y wall, where the rack run comes in
    # along +Y (white, blue, yellow from its right), to the tanks; water and
    # methane rise over the oxygen line.
    z = {c: BEAM + PIPES[c][1] + PIPES[c][0] for c in PIPES}
    lift = 1.75
    head = {c: Vector((TANK_TIE.x - GAP * (k - 1), TANK_TIE.y, z[c])) for k, c in enumerate(("pipe_white", "pipe_blue", "pipe_yellow"))}
    w0, b0, y0 = head["pipe_white"], head["pipe_blue"], head["pipe_yellow"]
    white = [w0, Vector((w0.x, -3.4, w0.z)), Vector((-0.9, -3.4, w0.z)), Vector((-0.9, lox[1], w0.z)), Vector((lox[0], lox[1], w0.z)), Vector((lox[0], lox[1], 1.32))]
    blue = [b0, Vector((b0.x, -5.6, b0.z)), Vector((b0.x, -5.6, lift)), Vector((b0.x, -3.1, lift)), Vector((1.25, -3.1, lift)), Vector((1.25, -3.1, 0.45)),
            Vector((1.25, 1.6, 0.45)), Vector((1.25, 1.6, wz + 0.02))]
    yellow = [y0, Vector((y0.x, -4.1, y0.z)), Vector((0.7, -4.1, y0.z)), Vector((0.4, -4.1, lift)), Vector((-1.6, -4.1, lift)), Vector((-1.6, ch4[1], lift)),
              Vector((-2.1, ch4[1], lift)), Vector((-2.4, ch4[1], y0.z)), Vector((ch4[0], ch4[1], y0.z)), Vector((ch4[0], ch4[1], 1.32))]
    for colour, path in (("pipe_white", white), ("pipe_blue", blue), ("pipe_yellow", yellow)):
        pr = PIPES[colour][0]
        b.put(pipe([path[0] + Vector((0, 0.01, 0))] + path[1:], pr, 3.5 * pr), m[colour])
        stands(b, m, path, pr)
        fit = collar(path[0], (0, -1, 0), pr)
        merge(fit, flange(path[-1] - (path[-1] - path[-2]).normalized() * 0.05, path[-1] - path[-2], pr))
        b.put(fit, m["steel"])
        b.put(marker(*point_at(path, lengths(path), 1.2), pr), m["label"], smooth=False)
        a, c = max(zip(path, path[1:]), key=lambda s: (s[1] - s[0]).length if abs(s[1].z - s[0].z) < 0.01 and s[0].z > 0.6 else 0.0)
        d = (c - a).normalized()
        valve(b, m, a.lerp(c, 0.4), d, pr, wheel="paint_yellow" if colour == "pipe_yellow" else "paint_red")
        gauge(b, m, a.lerp(c, 0.4) + d * 0.45 + Vector((0, 0, pr)), (0, 0, 1), Vector((-d.y, d.x, 0.0)))
    b.put(pipe([(1.25, -1.6, 0.45), (1.25, -1.6, wz + 0.02)], PIPES["pipe_blue"][0]), m["pipe_blue"])

    # The vent stack, guyed, and the relief lines over to it.
    sx, sy = -8.0, 0.0
    b.put(tube([(sx, sy, 0.3), (sx, sy, 8.2)], 0.1, 12), m["paint_white"])
    stack = moved(ring(0.17, 0.0, 7.2, 7.65, 16), (sx, sy, 0))
    merge(stack, tube([(sx - 0.4, sy, 8.3), (sx + 0.4, sy, 8.3)], 0.08, 10))
    merge(stack, moved(box((0.5, 0.5, 0.3), (0, 0, 0.15)), (sx, sy, 0)))
    for k in range(3):
        a = TAU * k / 3 + math.pi / 3
        foot = Vector((sx + 1.5 * math.cos(a), sy + 1.5 * math.sin(a), 0.05))
        merge(stack, rod(foot, Vector((sx, sy, 5.5)), 0.008, 4))
        merge(stack, moved(box((0.2, 0.2, 0.1), (0, 0, 0.05)), tuple(foot)))
    b.put(stack, m["steel"])
    for v in vents:
        b.put(pipe([v, Vector((v.x, v.y, 7.1)), Vector((sx, v.y, 7.1)), Vector((sx, sy + math.copysign(0.1, v.y), 7.1))], 0.04, 0.3), m["jacket"])

    # The stile over the +X bund wall, a control cabinet, and a light.
    stairs(b, m, (9.15 + 0.84, -3.0, 0.0), math.pi, 0.6, 0.8)
    stairs(b, m, (8.85 - 0.84, -3.0, 0.0), 0.0, 0.6, 0.8)
    b.put(box((0.5, 0.8, 0.04), (9.0, -3.0, 0.66), tile=1.0), m["grate"], smooth=False)
    sign(b, m, (9.16, -4.3, 0.32), 0.0, 0.45, 0.32, 0)
    cabinet(b, m, (3.4, -5.45, 0.0), math.pi / 2, (0.4, 1.2, 1.4), doors=2, cell=2)
    light_mast(b, m, (8.0, 5.0, 0), 7.0, 2, math.pi * 1.25)

    for c in (lox, ch4):
        b.collider(moved(lathe([(0.0, 0.0), (r, 0.0), (r, 2 * r + 1.3), (0.0, 2 * r + 1.3)], 10, 2.0), (c[0], c[1], 0)))
    b.collider(box((wl, 5.4, 2 * wr + wz), (0.5 + wl / 2, 0, (2 * wr + wz) / 2)))
    for s in (-1, 1):
        b.collider(box((18.3, 0.3, 0.65), (0, s * 6.0, 0.325)))
        b.collider(box((0.3, 12.0, 0.65), (s * 9.0, 0, 0.325)))
    return b.finish()


def pipe_run(looks, name, path, parent=None, colours=("pipe_white", "pipe_yellow", "pipe_blue"), ends=("cap", "cap")):
    """Pipes on an elevated rack along a path, side by side across it in the
    order of `colours` (the first to the path's right): T-supports on
    footings about every five metres, long-radius bends, nested expansion
    loops on long straights, shoes under the insulated line and U-bolts over
    bare ones, flanged spools and flow markers. `ends` says what each end
    meets: "flange", a module's tie-in; "seal", a wall's pipe seals; "cap",
    nothing, closed by a valve and a blind."""
    m = plant_mats(looks)
    b = Building(name, parent, (0, 0, 0), 0.0)
    base = [Vector(p) for p in path]
    count = len(colours)
    offs = [GAP * (i - (count - 1) / 2) for i in range(count)]
    reach = max(offs)
    bend, spacing = 1.6, 5.0
    loop_w, loop_d, loop_r = 1.3, 2.6, 0.35
    turns = [0.0] + [math.copysign(1.0, (base[k] - base[k - 1]).cross(base[k + 1] - base[k]).z) for k in range(1, len(base) - 1)] + [0.0]
    loops = {}
    for k, (a, c) in enumerate(zip(base, base[1:])):
        if (c - a).length >= 18.0:
            d = (c - a).normalized()
            loops[k] = ((a + c) / 2, d, Vector((-d.y, d.x, 0.0)))
    lines = []
    for i, colour in enumerate(colours):
        r, h = PIPES[colour]
        lift = Vector((0, 0, BEAM + h + r))
        q = offset(base, offs[i])
        pts, radii = [q[0]], [0.0]
        for k in range(1, len(base)):
            if k - 1 in loops:
                mid, d, n = loops[k - 1]
                wa, far = loop_w + i * GAP, reach + loop_d + i * GAP
                r14, r23 = loop_r + (count - 1 - i) * GAP, loop_r + i * GAP
                for along, across, rr in ((-wa, offs[i], r14), (-wa, far, r23), (wa, far, r23), (wa, offs[i], r14)):
                    pts.append(mid + d * along + n * across)
                    radii.append(rr)
            pts.append(q[k])
            radii.append(bend - offs[i] * turns[k])
        lines.append((colour, [p + lift for p in fillet(pts, radii)], lift))

    # Supports along the centre line, clear of the loops, and under each loop's far legs.
    centre = fillet(base, bend)
    cum = lengths(centre)
    n = max(1, int(math.ceil((cum[-1] - 1.2) / spacing)))
    spots = [point_at(centre, cum, 0.6 + (cum[-1] - 1.2) * k / n) for k in range(n + 1)]
    clear = loop_w + (count - 1) * GAP + 0.5
    spots = [(p, d) for p, d in spots if all((p - mid).length > clear for mid, _, _ in loops.values())]
    spots += [(mid + d * s * clear, d) for mid, d, _ in loops.values() for s in (-1, 1)]
    per_line = [[] for _ in colours]
    for p, d in spots:
        tee(b, m, p, d, reach + 0.3, p.z + BEAM)
        nrm = Vector((-d.y, d.x, 0.0))
        for i, (_, _, lift) in enumerate(lines):
            per_line[i].append((p + nrm * offs[i] + lift, d))
    for mid, d, nrm in loops.values():
        far = reach + loop_d + (count - 1) / 2 * GAP
        tee(b, m, mid + nrm * far, nrm, (count - 1) / 2 * GAP + 0.3, mid.z + BEAM)
        for i, (_, _, lift) in enumerate(lines):
            per_line[i].append((mid + nrm * (reach + loop_d + i * GAP) + lift, d))
    rack_pipes(b, m, [(colour, line, per_line[i]) for i, (colour, line, _) in enumerate(lines)], ends)
    return b.finish()


def drill_rig(looks, name, at=(0, 0, 0), yaw=0.0, parent=None, height=9.0):
    """An ice well, after Honeybee's RedWater: coiled tubing drilled through
    the overburden into buried ice and left in the hole to bring the melt
    up. A skid carries the tubing reel, the melt and pump house, a water
    tank and the heater umbilical's reel; a lattice tower over the well
    holds the injector head and its gooseneck, with a crown sheave and a
    caged ladder; spare augers lie racked beside it, and a heat-traced water
    line runs out to its flanged tie-in (DRILL_TIE)."""
    m = plant_mats(looks)
    b = Building(name, parent, at, yaw)
    deck = 0.6
    wx = 2.4

    # Skid with outrigger jacks.
    skid = bmesh.new()
    for s in (-1, 1):
        merge(skid, member((-3.2, s * 1.72, deck - 0.15), (1.4, s * 1.72, deck - 0.15), ibeam(0.15, 0.3, 0.014)))
    for x in (-3.12, -1.6, 0.0, 1.32):
        merge(skid, member((x, -1.64, deck - 0.15), (x, 1.64, deck - 0.15), ibeam(0.12, 0.26, 0.012)))
    b.put(skid, m["paint_dark"])
    b.put(box((4.6, 3.6, 0.02), (-0.9, 0, deck + 0.01), tile=1.0), m["tread"], smooth=False)
    jacks, pads = bmesh.new(), bmesh.new()
    for x in (-2.9, 1.1):
        for s in (-1, 1):
            merge(jacks, member((x, s * 1.72, 0.45), (x, s * 2.15, 0.45), rect(0.14, 0.14)))
            merge(jacks, moved(ring(0.08, 0.0, 0.12, 0.62, 12), (x, s * 2.15, 0)))
            merge(pads, moved(lathe([(0.0, 0.0), (0.25, 0.0), (0.25, 0.05), (0.07, 0.1), (0.0, 0.12)], 16, 1.0), (x, s * 2.15, 0)))
    b.put(jacks, m["paint_yellow"])
    b.put(pads, m["steel"])
    railing(b, m, [(-3.15, -1.75, deck), (-3.15, 1.75, deck)], 1.0, 1.2)

    # The tower over the well: tapered legs, girts, bracing (the reel's face
    # open to its gooseneck), platforms, the crown and its sheave.
    base, crown = 0.85, 0.3
    legs = [(sx, sy) for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))]

    def leg(i, z):
        t = z / height
        w = base + (crown - base) * t
        return Vector((wx + legs[i][0] * w, legs[i][1] * w, z))

    tower = bmesh.new()
    for i in range(4):
        merge(tower, member(leg(i, 0.3), leg(i, height), angle(0.1, 0.01), up=(-legs[i][0], -legs[i][1], 0)))
    levels = [0.3 + (height - 0.3) * k / 6 for k in range(7)]
    for z0, z1 in zip(levels, levels[1:]):
        for i in range(4):
            j = (i + 1) % 4
            merge(tower, rod(leg(i, z1), leg(j, z1), 0.03, 6))
            face_reel = legs[i][0] < 0 and legs[j][0] < 0
            if face_reel and z1 < 5.5:
                continue
            merge(tower, rod(leg(i, z0), leg(j, z1), 0.025, 6))
            merge(tower, rod(leg(j, z0), leg(i, z1), 0.025, 6))
    b.put(tower, m["paint_yellow"])
    footings = bmesh.new()
    for i in range(4):
        merge(footings, moved(box((0.6, 0.6, 0.4), (0, 0, 0.1)), tuple(leg(i, 0.0))))
    b.put(footings, m["footing"], smooth=False)
    zw = 1.9
    plat = bmesh.new()
    for s in (-1, 1):
        merge(plat, box((1.5, 0.5, 0.04), (wx, s * 0.5, zw - 0.02), tile=1.0))
        merge(plat, box((0.5, 0.5, 0.04), (wx + s * 0.5, 0, zw - 0.02), tile=1.0))
    merge(plat, box((0.9, 0.9, 0.04), (wx, 0, height - 0.42), tile=1.0))
    b.put(plat, m["grate"], smooth=False)
    w = base + (crown - base) * zw / height
    railing(b, m, [(wx + w - 0.05, 0.3, zw), (wx + w - 0.05, w - 0.05, zw), (wx - w + 0.05, w - 0.05, zw), (wx - w + 0.05, -w + 0.05, zw), (wx + w - 0.05, -w + 0.05, zw), (wx + w - 0.05, -0.3, zw)], 1.0, 0.8)
    ladder(b, m, (wx + base + 0.22, 0, 0.0), 0.0, height - 0.4, lean=math.atan((base - crown) / height))
    sheave = moved(ring(0.3, 0.0, -0.05, 0.05, 20), (wx - 0.2, 0.35, height + 0.3), 0.0, 0.0, math.pi / 2)
    merge(sheave, moved(box((0.12, 0.16, 0.45), (0, 0, 0)), (wx - 0.2, 0.35, height + 0.15)))
    merge(sheave, rod((wx - 0.2 + 0.3, 0.35, height + 0.3), (wx + 0.1, 0.35, 5.2), 0.008, 4))
    merge(sheave, box((0.22, 0.18, 0.35), (wx + 0.1, 0.35, 5.0), bevel=0.02))
    b.put(sheave, m["steel"])
    b.put(sphere(0.12, (wx, 0, height + 0.12), 10, 5), m["red"])
    for s in (-1, 1):
        head = leg(0 if s < 0 else 3, 6.0)
        b.put(box((0.18, 0.32, 0.18), tuple(head + Vector((-0.12, 0, 0))), bevel=0.02), m["paint_dark"], smooth=False)
        b.put(box((0.02, 0.28, 0.14), tuple(head + Vector((-0.22, 0, -0.01)))), m["lamp"], smooth=False)
    b.places.append(("light", {"at": (wx - 0.9, 0, 6.0), "kind": "flood", "yaw": math.pi}))
    sign(b, m, leg(1, 1.3) + Vector((0.06, 0, 0)), 0.0, 0.4, 0.3, 0)

    # The wellhead and its stack: casing head, rams, stripper, then the injector.
    stack = moved(ring(0.12, 0.0, -0.2, 0.35, 12), (wx, 0, 0))
    for z0, z1, rad in ((0.35, 0.42, 0.26), (0.45, 1.2, 0.27), (1.2, 1.26, 0.3), (1.26, zw + 0.1, 0.17)):
        merge(stack, moved(ring(rad, 0.0, z0, z1, 16), (wx, 0, 0)))
    for s in (-1, 1):
        merge(stack, box((0.3, 0.38, 0.3), (wx, s * 0.42, 0.85), bevel=0.02))
        merge(stack, moved(ring(0.09, 0.0, 0.0, 0.3, 10), (wx, s * 0.6, 0.85), 0.0, 0.0, math.pi / 2))
    b.put(stack, m["paint_red"])
    b.put(pipe([(wx - 0.15, 0, 0.3), (wx - 0.6, 0, 0.3), (1.8, -0.5, 0.35), (1.5, -0.9, 0.9), (1.32, -0.9, 1.05)], 0.05, 0.3), m["hose"])
    inj = box((0.7, 0.8, 1.0), (wx, 0, zw + 0.6), tile=1.0, bevel=0.03)
    b.put(inj, m["paint_orange"], smooth=False)
    gear = bmesh.new()
    for s in (-1, 1):
        merge(gear, moved(ring(0.13, 0.0, 0.0, 0.3, 14), (wx, s * 0.4, zw + 0.85), 0.0, 0.0, -s * math.pi / 2))
        merge(gear, box((0.04, 0.55, 0.85), (wx + s * 0.37, 0, zw + 0.6)))
    merge(gear, box((0.9, 0.95, 0.08), (wx, 0, zw + 0.06)))
    b.put(gear, m["paint_dark"], smooth=False)

    # The gooseneck over the injector, and the tubing from the reel over it.
    rr, reel_x, reel_z, wraps = 1.0, -2.1, deck + 1.25, 0.95
    gz = zw + 1.25
    gc = Vector((wx - rr, 0, gz))
    p = Vector((reel_x, 0, reel_z + wraps))
    d = p - gc
    dist = math.hypot(d.x, d.z)
    end = math.atan2(d.z, d.x) - math.acos(rr / dist)
    end = end + TAU if end < 0 else end
    arc = [gc + Vector((rr * math.cos(t), 0, rr * math.sin(t))) for t in [end * i / 14 for i in range(15)]]
    neck = bmesh.new()
    for s in (-1, 1):
        side = Vector((0, s * 0.11, 0))
        merge(neck, strip([gc + (a - gc) * (1.0 - 0.08 / rr) + side for a in arc], [gc + (a - gc) * (1.0 + 0.1 / rr) + side for a in arc]))
    for a in arc[1::2]:
        merge(neck, rod(a + Vector((0, -0.1, 0)) - (a - gc).normalized() * 0.04, a + Vector((0, 0.1, 0)) - (a - gc).normalized() * 0.04, 0.03, 8))
    merge(neck, rod(Vector((wx - 0.1, 0, zw + 1.1)), arc[7] - (arc[7] - gc).normalized() * 0.1, 0.03, 6))
    b.put(neck, m["paint_orange"])
    b.put(tube([p] + arc[::-1] + [Vector((wx, 0, zw + 1.1))], 0.03, 8), m["jacket"])

    # The tubing reel on its stands, with a drive and a level wind.
    reel = bmesh.new()
    for s in (-1, 1):
        merge(reel, moved(ring(1.12, 0.0, s * 0.7 - 0.025, s * 0.7 + 0.025, 32), (reel_x, 0, reel_z), 0.0, 0.0, math.pi / 2))
        merge(reel, member((reel_x - 0.9, s * 0.85, deck), (reel_x, s * 0.85, reel_z), rect(0.1, 0.1)))
        merge(reel, member((reel_x + 0.9, s * 0.85, deck), (reel_x, s * 0.85, reel_z), rect(0.1, 0.1)))
        merge(reel, box((0.25, 0.12, 0.25), (reel_x, s * 0.85, reel_z)))
    merge(reel, member((reel_x - 0.9, -0.85, deck + 0.05), (reel_x - 0.9, 0.85, deck + 0.05), rect(0.1, 0.1)))
    merge(reel, member((reel_x + 0.9, -0.85, deck + 0.05), (reel_x + 0.9, 0.85, deck + 0.05), rect(0.1, 0.1)))
    for z in (reel_z + wraps + 0.15, reel_z + wraps + 0.6):
        merge(reel, member((reel_x + 0.95, -0.8, z), (reel_x + 0.95, 0.8, z), rect(0.05, 0.05)))
    for s in (-1, 1):
        merge(reel, member((reel_x + 0.95, s * 0.8, deck), (reel_x + 0.95, s * 0.8, reel_z + wraps + 0.65), rect(0.06, 0.06)))
    merge(reel, box((0.2, 0.25, 0.5), (reel_x + 0.95, 0.0, reel_z + wraps + 0.38)))
    b.put(reel, m["paint_orange"])
    b.put(moved(ring(wraps, 0.0, -0.67, 0.67, 32), (reel_x, 0, reel_z), 0.0, 0.0, math.pi / 2), m["jacket"])
    b.put(moved(ring(0.2, 0.0, 0.9, 1.3, 14), (reel_x, 0, reel_z), 0.0, 0.0, math.pi / 2), m["paint_dark"])

    # The melt and pump house, the water tank and the umbilical reel.
    hx, hy = 0.2, -0.8
    b.put(box((2.2, 1.9, 2.0), (hx, hy, deck + 1.0), tile=1.0, bevel=0.03), m["paint_white"], smooth=False)
    b.put(box((2.3, 2.0, 0.08), (hx, hy, deck + 2.04)), m["paint_grey"], smooth=False)
    side_door(b, m, (hx - 0.4, hy - 0.95, deck), -math.pi / 2, 0.8, 1.85, "paint_grey")
    louvres(b, m, (hx + 1.1, hy - 0.3, deck + 1.4), 0.0, 0.8, 0.6)
    louvres(b, m, (hx - 1.1, hy, deck + 1.4), math.pi, 0.8, 0.6)
    b.put(moved(lathe([(0.0, 0.0), (0.1, 0.0), (0.1, 0.35), (0.2, 0.37), (0.2, 0.44), (0.0, 0.48)], 14, 1.0), (hx + 0.5, hy + 0.3, deck + 2.08)), m["paint_grey"])
    sign(b, m, (hx + 0.6, hy - 0.96, deck + 1.5), -math.pi / 2, 0.45, 0.32, 0)
    vessel(b, m, (0.55, 1.1, deck), 0.5, 1.6, "alu", lift=0.3, legs=3)
    ux, uy, uz = -0.55, 1.1, deck + 0.75
    reel2 = bmesh.new()
    for s in (-1, 1):
        merge(reel2, moved(ring(0.5, 0.0, s * 0.3 - 0.02, s * 0.3 + 0.02, 24), (ux, uy, uz), 0.0, 0.0, math.pi / 2))
        for t in (-1, 1):
            merge(reel2, member((ux + t * 0.4, uy + s * 0.34, deck), (ux, uy + s * 0.34, uz), rect(0.06, 0.06)))
    b.put(reel2, m["paint_dark"])
    b.put(moved(ring(0.4, 0.0, -0.28, 0.28, 24), (ux, uy, uz), 0.0, 0.0, math.pi / 2), m["heater"])
    b.put(tube(fillet([(ux, uy, uz + 0.4), (0.5, 0.75, 3.7), (wx - 0.45, 0.5, 3.75), (wx - 0.2, 0.3, zw + 1.0)], 0.6), 0.03, 8), m["heater"])

    # Spare augers on a rack beside the skid.
    rack = bmesh.new()
    for x in (-2.4, -0.8):
        for s in (-1, 1):
            merge(rack, member((x, 2.6 + s * 0.45, 0.0), (x, 2.6, 0.62), rect(0.06, 0.06)))
        merge(rack, member((x, 2.1, 0.62), (x, 3.1, 0.62), channel(0.05, 0.1, 0.006)))
    b.put(rack, m["paint_dark"])
    shafts, flights = bmesh.new(), bmesh.new()
    for k in range(4):
        shaft, flight = auger(2.2, 0.04, 0.12)
        merge(shafts, moved(shaft, (-2.7, 2.22 + k * 0.26, 0.79)))
        merge(flights, moved(flight, (-2.7, 2.22 + k * 0.26, 0.79)))
    b.put(shafts, m["steel"])
    b.put(flights, m["paint_grey"])

    # The water line, heat traced, on stands out to its flange at the tie-in;
    # the power feed in from a pull pit.
    r = PIPES["pipe_blue"][0]
    zw = BEAM + r
    line = [Vector(v) for v in ((0.55, 1.1, deck + 0.42), (0.9, 1.95, deck + 0.42), (1.4, DRILL_TIE.y, zw), (DRILL_TIE.x - 0.01, DRILL_TIE.y, zw))]
    b.put(pipe(line, r, 0.4), m["pipe_blue"])
    b.put(tube(fillet([v - Vector((0, 0, r + 0.012)) for v in line], 0.4), 0.012, 5), m["heater"])
    stands(b, m, fillet(line, 0.4), r, 2.0, 0.4)
    b.put(collar(Vector((DRILL_TIE.x, DRILL_TIE.y, zw)), (1, 0, 0), r), m["steel"])
    valve(b, m, (DRILL_TIE.x - 0.8, DRILL_TIE.y, zw), (1, 0, 0), r)
    b.put(tube(fillet([(hx - 1.1, hy - 0.6, deck + 0.4), (-1.4, -1.85, deck - 0.1), (-1.4, -2.6, 0.05)], 0.4), 0.04, 8), m["cable"])
    b.put(box((0.7, 0.7, 0.24), (-1.4, -2.75, 0.07)), m["footing"], smooth=False)
    b.put(box((0.6, 0.6, 0.02), (-1.4, -2.75, 0.2), tile=1.0), m["tread"], smooth=False)

    b.collider(box((4.6, 3.6, deck + 0.02), (-0.9, 0, (deck + 0.02) / 2)))
    b.collider(box((2 * base + 0.2, 2 * base + 0.2, height), (wx, 0, height / 2)))
    b.collider(box((2.2, 1.7, 2.4), (reel_x, 0, reel_z)))
    b.collider(box((2.2, 1.9, 2.0), (hx, hy, deck + 1.0)))
    b.collider(box((2.4, 1.1, 1.0), (-1.6, 2.6, 0.5)))
    return b.finish()


def regolith_works(looks, name, at=(0, 0, 0), yaw=0.0, parent=None, seed=3):
    """Regolith the excavators dig and how it reaches the plant: spoil heaps
    at their angle of repose, a ramp up to a grizzly-topped hopper, a
    vibrating feeder onto a troughed belt conveyor under hoods, a vibrating
    screen on a tower, its fines screwed up into the oven and its oversize
    dropped on a reject pile, the oven's gas into a receiver and its spent
    regolith bagged. Its road comes in from +X along y = 2."""
    m = plant_mats(looks)
    b = Building(name, parent, at, yaw)
    rng = random.Random(seed)

    # Heaps: dumps at the angle of repose, rounded, noisy, and the ramp to the hopper.
    cones = []
    for k in range(3):
        qx, qy = rng.uniform(-11.5, -6.5), (-5.5 + 5.5 * k) + rng.uniform(-1.0, 1.0)
        for _ in range(rng.randint(2, 3)):
            cones.append((qx + rng.uniform(-2.0, 2.0), qy + rng.uniform(-1.4, 1.4), rng.uniform(2.0, 3.2) * REPOSE))
    phase = Vector((rng.uniform(0, 100), rng.uniform(0, 100), 0.0))
    ramp_y, ramp_x0, ramp_x1, ramp_h = -2.5, -5.5, 0.25, 1.5

    def height(x, y):
        h = -1.0
        for px, py, peak in cones:
            d = math.hypot(x - px, y - py)
            h = smax(h, peak - REPOSE * (math.sqrt(d * d + 0.5) - math.sqrt(0.5)), 0.6)
        if x <= ramp_x1:
            rise = ramp_h * min(max((x - ramp_x0) / (ramp_x1 - ramp_x0), 0.0), 1.0)
            h = smax(h, rise - REPOSE * max(0.0, abs(y - ramp_y) - 1.5) - (0.0 if x > ramp_x0 else REPOSE * (ramp_x0 - x)), 0.4)
        grain = noise.fractal(Vector((x * 0.45, y * 0.45, 0.0)) + phase, 1.0, 2.0, 4)
        lumps = noise.fractal(Vector((x * 1.6, y * 1.6, 2.0)) + phase, 0.8, 2.0, 3)
        scoops = 1.0 - abs(noise.noise(Vector((x * 0.9, y * 0.9, 5.0)) + phase))
        return h + (0.22 * grain + 0.07 * lumps + 0.1 * scoops * scoops) * min(1.0, max(h, 0.0) / 0.6)

    step = 0.25
    rows = [[(x, y, height(x, y)) for x in [-15.0 + step * i for i in range(64)]] for y in [-9.6 + step * j for j in range(78)]]
    b.put(grid(rows, tile=2.0), m["spoil"])
    rocks = bmesh.new()
    for _ in range(46):
        x, y = rng.uniform(-14.5, 0.0), rng.uniform(-9.0, 9.0)
        merge(rocks, clast(rng, (x, y, max(height(x, y), 0.0)), rng.uniform(0.08, 0.32)))
    b.put(rocks, m["basalt"])
    for px, py, peak in cones:
        b.collider(moved(cone(peak / REPOSE * 0.8, 0.4, peak * 0.9, segments=8), (px, py, -0.05)))
    b.collider(moved(box((ramp_x1 - ramp_x0, 3.0, 0.3), (0, 0, 0)), ((ramp_x0 + ramp_x1) / 2, ramp_y, ramp_h / 2), 0.0, -math.atan(ramp_h / (ramp_x1 - ramp_x0))))

    # Retaining wall of interlocking blocks at the ramp's head, a wheel stop on it.
    blocks = bmesh.new()
    for row in range(3):
        for k in range(4 - row % 2):
            y = ramp_y - 1.65 + (k + 0.5 * (row % 2)) * 1.1 + 0.55
            merge(blocks, box((0.32, 1.06, 0.53), (ramp_x1 + 0.16, y, 0.27 + row * 0.55), bevel=0.02))
    b.put(blocks, m["footing"], smooth=False)
    b.put(box((0.25, 3.0, 0.2), (ramp_x1 - 0.15, ramp_y, ramp_h + 0.1)), m["hazard"], smooth=False)
    b.collider(box((0.32, 3.3, 1.65), (ramp_x1 + 0.16, ramp_y, 0.825)))

    # Hopper with a grizzly on four braced legs, a vibrating feeder under it.
    hx, hy, top, out = 1.6, ramp_y, 1.75, 0.95
    b.put(moved(lathe([(0.354, out), (1.556, top), (1.6, top), (1.6, top + 0.12)], 4, 1.0, start=math.pi / 4), (hx, hy, 0)), m["paint_orange"], smooth=False)
    ribs = bmesh.new()
    for t in (0.35, 0.7):
        z = out + (top - out) * t
        rr = 0.354 + (1.556 - 0.354) * t + 0.03
        merge(ribs, moved(lathe([(rr, z - 0.03), (rr + 0.02, z - 0.03), (rr + 0.02, z + 0.03), (rr, z + 0.03)], 4, 1.0, start=math.pi / 4), (hx, hy, 0)))
    merge(ribs, moved(ring(0.3, 0.27, out - 0.15, out + 0.02, 4), (hx, hy, 0), math.pi / 4))
    b.put(ribs, m["paint_orange"], smooth=False)
    frame = bmesh.new()
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(frame, member((hx + sx * 1.05, hy + sy * 1.05, 0.25), (hx + sx * 1.05, hy + sy * 1.05, top + 0.12), ibeam(0.12, 0.12, 0.01), up=(1, 0, 0)))
        merge(frame, member((hx + sx * 1.1, hy - 1.1, top + 0.15), (hx + sx * 1.1, hy + 1.1, top + 0.15), angle(0.08, 0.008)))
        merge(frame, member((hx - 1.1, hy + sx * 1.1, top + 0.15), (hx + 1.1, hy + sx * 1.1, top + 0.15), angle(0.08, 0.008)))
        merge(frame, rod((hx + sx * 1.05, hy - 1.05, 0.3), (hx + sx * 1.05, hy + 1.05, top - 0.2), 0.02, 6))
        merge(frame, rod((hx + sx * 1.05, hy + 1.05, 0.3), (hx + sx * 1.05, hy - 1.05, top - 0.2), 0.02, 6))
    for k in range(9):
        y = hy - 1.0 + 2.0 * k / 8
        merge(frame, rod((hx - 1.1, y, top + 0.2), (hx + 1.1, y, top + 0.2), 0.03, 6))
    b.put(frame, m["galv"])
    feet = bmesh.new()
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(feet, box((0.45, 0.45, 0.3), (hx + sx * 1.05, hy + sy * 1.05, 0.1)))
    b.put(feet, m["footing"], smooth=False)
    feeder = box((1.3, 0.5, 0.02), (0, 0, 0))
    for s in (-1, 1):
        merge(feeder, box((1.3, 0.02, 0.16), (0, s * 0.25, 0.08)))
    merge(feeder, moved(ring(0.09, 0.0, 0.0, 0.25, 12), (0.1, 0.27, 0.0), 0.0, 0.0, -math.pi / 2))
    b.put(moved(feeder, (hx + 0.55, hy, out - 0.2), 0.0, 0.08), m["paint_dark"], smooth=False)
    springs = bmesh.new()
    for x in (hx + 0.05, hx + 1.05):
        for s in (-1, 1):
            merge(springs, tube([(x + 0.04 * math.cos(t), hy + s * 0.25 + 0.04 * math.sin(t), 0.45 + 0.012 * t) for t in [i * 0.6 for i in range(26)]], 0.008, 4))
            merge(springs, member((x, hy + s * 0.25, 0.0), (x, hy + s * 0.25, 0.45), rect(0.06, 0.06)))
    b.put(springs, m["steel"])
    b.collider(box((2.3, 2.3, top + 0.2), (hx, hy, (top + 0.2) / 2)))

    # The belt conveyor, built level along +X then tilted up to the screen.
    tail, head = Vector((2.75, ramp_y, 0.62)), Vector((10.5, ramp_y, 3.6))
    span = (head - tail).length
    tilt = math.atan2(head.z - tail.z, head.x - tail.x)
    dirn, up = (head - tail).normalized(), Vector((-math.sin(tilt), 0.0, math.cos(tilt)))

    def lay(bm):
        return moved(bm, tuple(tail), 0.0, -tilt)

    steel, rollers, belt = bmesh.new(), bmesh.new(), bmesh.new()
    for s in (-1, 1):
        merge(steel, member((-0.2, s * 0.42, -0.2), (span + 0.2, s * 0.42, -0.2), channel(0.05, 0.16, 0.006, s < 0)))
    ux = 0.6
    while ux < span - 0.4:
        merge(rollers, rod((ux, -0.12, -0.055), (ux, 0.12, -0.055), 0.045, 10))
        for s in (-1, 1):
            a0 = Vector((ux, s * 0.135, -0.05))
            merge(rollers, rod(a0, a0 + Vector((0, s * 0.2 * math.cos(0.52), 0.2 * math.sin(0.52))), 0.045, 10))
            merge(steel, rod((ux, s * 0.38, -0.13), (ux, s * 0.38, 0.06), 0.012, 4))
        merge(steel, member((ux, -0.42, -0.14), (ux, 0.42, -0.14), angle(0.05, 0.005)))
        ux += 1.0
    ux = 1.2
    while ux < span - 0.4:
        merge(rollers, rod((ux, -0.34, -0.33), (ux, 0.34, -0.33), 0.04, 10))
        for s in (-1, 1):
            merge(steel, box((0.06, 0.01, 0.16), (ux, s * 0.4, -0.28)))
        ux += 2.4
    trough = [(-0.312, 0.105), (-0.13, 0.0), (0.13, 0.0), (0.312, 0.105), (0.312, 0.095), (0.13, -0.01), (-0.13, -0.01), (-0.312, 0.095)]
    merge(belt, sweep([(z, -y) for y, z in trough], [(0.0, 0, 0), (span, 0, 0)], 1.0))
    merge(belt, box((span, 0.6, 0.01), (span / 2, 0, -0.285)))
    for x in (0.0, span):
        merge(belt, moved(ring(0.145, 0.0, -0.33, 0.33, 16), (x, 0, -0.145), 0.0, 0.0, math.pi / 2))
    load = [(y, 0.012 + 0.07 * max(0.0, 1.0 - (y / 0.22) ** 2) ** 0.5) for y in [-0.22 + 0.44 * k / 8 for k in range(9)]]
    b.put(lay(sweep([(z, -y) for y, z in load], [(0.3, 0, 0), (span - 0.2, 0, 0)], 1.0)), m["spoil"])
    covers = bmesh.new()
    x = 0.9
    while x + 1.2 < span - 0.5:
        merge(covers, vault(x, x + 1.17, 0.44, -0.04, 12, 0.6))
        x += 1.2
    b.put(lay(covers), m["corrugated"])
    walk = box((span - 0.8, 0.6, 0.03), (span / 2, -0.75, -0.25), tile=1.0)
    b.put(lay(walk), m["grate"], smooth=False)
    rails = bmesh.new()
    xs = [0.5 + k * 1.2 for k in range(int((span - 1.0) / 1.2) + 1)]
    for x in xs:
        merge(rails, rod((x, -1.04, -0.25), (x, -1.04, 0.75), 0.022, 6))
        merge(rails, member((x, -0.42, -0.26), (x, -1.06, -0.26), angle(0.05, 0.005)))
    for z in (0.75, 0.25):
        merge(rails, tube([(xs[0], -1.04, z), (xs[-1], -1.04, z)], 0.022, 6))
    b.put(lay(rails), m["paint_yellow"])
    drive = box((0.35, 0.3, 0.35), (span, 0.6, -0.15), bevel=0.02)
    merge(drive, moved(ring(0.13, 0.0, 0.0, 0.45, 14), (span - 0.1, 0.62, -0.15), 0.0, -math.pi / 2))
    merge(drive, box((0.75, 0.9, 0.55), (span + 0.15, 0, -0.05), bevel=0.02))
    b.put(lay(drive), m["paint_orange"], smooth=False)
    b.put(lay(steel), m["galv"])
    b.put(lay(rollers), m["paint_dark"])
    b.put(lay(belt), m["hose"])
    trestles, footings = bmesh.new(), bmesh.new()
    for u in (2.6, 5.4):
        p = tail + dirn * u + up * (-0.28)
        for s in (-1, 1):
            merge(trestles, member((p.x, p.y + s * 0.45, 0.25), (p.x, p.y + s * 0.45, p.z), ibeam(0.1, 0.1, 0.008), up=(1, 0, 0)))
            merge(footings, box((0.4, 0.4, 0.3), (p.x, p.y + s * 0.45, 0.1)))
        merge(trestles, member((p.x, p.y - 0.45, p.z - 0.05), (p.x, p.y + 0.45, p.z - 0.05), channel(0.05, 0.1, 0.006)))
        merge(trestles, rod((p.x, p.y - 0.45, 0.35), (p.x, p.y + 0.45, p.z - 0.15), 0.018, 6))
        merge(trestles, rod((p.x, p.y + 0.45, 0.35), (p.x, p.y - 0.45, p.z - 0.15), 0.018, 6))
    b.put(trestles, m["galv"])
    b.put(footings, m["footing"], smooth=False)
    b.collider(moved(box((span, 1.2, 0.6), (span / 2, -0.2, -0.1)), tuple(tail), 0.0, -tilt))

    # The screen tower: columns, deck, a vibrating screen on springs, fines bin, ladder.
    tx0, tx1, ty0, ty1, tz = 10.2, 12.6, ramp_y - 1.1, ramp_y + 1.1, 3.0
    tower, feet = bmesh.new(), bmesh.new()
    for x in (tx0, tx1):
        for y in (ty0, ty1):
            merge(tower, member((x, y, 0.25), (x, y, tz), ibeam(0.15, 0.15, 0.012), up=(1, 0, 0)))
            merge(feet, box((0.5, 0.5, 0.3), (x, y, 0.1)))
    for z in (1.5, tz):
        for y in (ty0, ty1):
            merge(tower, member((tx0, y, z - 0.08), (tx1, y, z - 0.08), channel(0.06, 0.16, 0.008)))
        for x in (tx0, tx1):
            merge(tower, member((x, ty0, z - 0.08), (x, ty1, z - 0.08), channel(0.06, 0.16, 0.008)))
    for z0, z1 in ((0.3, 1.42), (1.58, tz - 0.16)):
        merge(tower, rod((tx0, ty1, z0), (tx1, ty1, z1), 0.02, 6))
        merge(tower, rod((tx1, ty1, z0), (tx0, ty1, z1), 0.02, 6))
    b.put(tower, m["galv"])
    b.put(feet, m["footing"], smooth=False)
    walk = box((tx1 - tx0, 0.55, 0.04), ((tx0 + tx1) / 2, ty1 - 0.28, tz + 0.02), tile=1.0)
    merge(walk, box((0.5, ty1 - ty0, 0.04), (tx1 - 0.25, ramp_y, tz + 0.02), tile=1.0))
    b.put(walk, m["grate"], smooth=False)
    railing(b, m, [(tx0, ty1, tz + 0.04), (tx1, ty1, tz + 0.04), (tx1, ramp_y - 0.2, tz + 0.04)], 1.0, 0.8)
    railing(b, m, [(tx1, ramp_y - 0.8, tz + 0.04), (tx1, ty0, tz + 0.04), (tx1 - 0.5, ty0, tz + 0.04)], 1.0, 0.8)
    ladder(b, m, (tx1 + 0.15, ramp_y - 0.5, 0.0), 0.0, tz)
    screen = bmesh.new()
    for s in (-1, 1):
        merge(screen, box((1.9, 0.03, 0.45), (0, s * 0.5, 0.2)))
    merge(screen, box((1.9, 1.0, 0.02), (0, 0, 0.05)))
    merge(screen, box((0.03, 1.0, 0.45), (-0.95, 0, 0.2)))
    for s in (-1, 1):
        merge(screen, moved(ring(0.12, 0.0, 0.0, 0.3, 12), (0.1, s * 0.52, 0.25), 0.0, 0.0, -s * math.pi / 2))
    b.put(moved(screen, (11.3, ramp_y, tz + 0.38), 0.0, math.radians(12)), m["paint_orange"], smooth=False)
    b.put(moved(box((1.85, 0.96, 0.02), (0, 0, 0.12), tile=0.5), (11.3, ramp_y, tz + 0.38), 0.0, math.radians(12)), m["grate"], smooth=False)
    coils = bmesh.new()
    for x in (10.55, 12.0):
        for s in (-1, 1):
            zs = tz + 0.04 + (0.24 if x < 11.0 else 0.0)
            merge(coils, tube([(x + 0.05 * math.cos(t), ramp_y + s * 0.45 + 0.05 * math.sin(t), zs + 0.012 * t) for t in [i * 0.6 for i in range(22)]], 0.01, 4))
    b.put(coils, m["steel"])
    b.put(moved(lathe([(0.28, 1.55), (1.41, tz - 0.12), (1.45, tz - 0.12)], 4, 1.0, start=math.pi / 4), (11.3, ramp_y, 0)), m["paint_orange"], smooth=False)
    chute = sweep([(0.0, -0.3), (0.0, 0.3), (0.25, 0.3), (0.25, 0.27), (0.02, 0.27), (0.02, -0.27), (0.25, -0.27), (0.25, -0.3)], [(12.25, ramp_y, tz + 0.2), (13.4, ramp_y, 1.6)], 1.0)
    b.put(chute, m["paint_orange"], smooth=False)
    pile = [[(x, y, 0.85 - REPOSE * math.hypot(x - 13.9, y - ramp_y) + 0.06 * noise.noise(Vector((x * 2.0, y * 2.0, 3.0)))) for x in [12.4 + 0.25 * i for i in range(14)]] for y in [ramp_y - 1.6 + 0.25 * j for j in range(14)]]
    b.put(grid(pile, tile=2.0), m["spoil"])
    b.collider(box((tx1 - tx0 + 0.3, ty1 - ty0 + 0.3, tz + 0.8), ((tx0 + tx1) / 2, ramp_y, (tz + 0.8) / 2)))
    b.collider(moved(cone(1.3, 0.3, 0.8, segments=8), (13.9, ramp_y, 0)))

    # The oven: fines screwed up into it, power, gas to a receiver, spent regolith bagged.
    ox, oy, lift, oh = 11.3, -6.1, 1.4, 2.2
    vessel(b, m, (ox, oy, 0), 0.8, oh, "alu", lift=lift, legs=4)
    screw_a, screw_b = Vector((ox, ramp_y, 1.3)), Vector((ox, oy + 0.35, lift + oh + 0.45))
    b.put(tube([screw_a, screw_b], 0.13, 14), m["paint_grey"])
    b.put(moved(ring(0.15, 0.0, 0.0, 0.4, 14), tuple(screw_b)), m["paint_dark"])
    b.put(tube([screw_b - Vector((0, 0.05, 0.0)), Vector((ox, oy, lift + oh + 0.2))], 0.09, 10), m["paint_grey"])
    mid = screw_a.lerp(screw_b, 0.5)
    b.put(member((mid.x, mid.y, 0.0), (mid.x, mid.y, mid.z - 0.14), rect(0.08, 0.08)), m["galv"], smooth=False)
    rx, ry = ox + 2.4, oy + 0.7
    vessel(b, m, (rx, ry, 0), 0.4, 1.4, "jacket", lift=0.5, legs=3)
    b.put(pipe([(ox + 0.3, oy, lift + oh + 0.1), (ox + 0.3, oy, 4.0), (ox + 0.3, ry, 4.0), (rx, ry, 4.0), (rx, ry, 2.05)], 0.05, 0.25), m["jacket"])
    valve(b, m, (rx, ry, 3.0), (0, 0, -1), 0.05, stem=(1, 0, 0))
    gauge(b, m, (rx, ry, 3.5), (1, 0, 0), (1, 0, 0))
    cabinet(b, m, (ox - 1.9, oy, 0.0), 0.0, (0.6, 1.2, 1.6), doors=2)
    b.put(tube(fillet([(ox - 1.6, oy - 0.2, 0.3), (ox - 1.2, oy - 0.2, 0.05), (ox - 0.85, oy - 0.2, 0.05), (ox - 0.8, oy - 0.2, lift + 0.6)], 0.25), 0.04, 8), m["cable"])
    b.put(tube(fillet([(ox - 1.6, oy + 0.2, 0.3), (ox - 1.2, oy + 0.2, 0.05), (ox - 0.85, oy + 0.2, 0.05), (ox - 0.8, oy + 0.2, lift + 0.9)], 0.25), 0.04, 8), m["cable"])
    b.put(tube([(ox, oy, lift + 0.05), (ox, oy, lift - 0.15), (ox + 1.2, oy - 0.9, 1.15)], 0.1, 10), m["paint_grey"])
    bag(b, m, (ox + 1.25, oy - 0.95, 0), 0.2, 0.7)
    for x, y, turn in ((ox + 2.5, oy - 0.95, 0.05), (ox + 2.5, oy - 2.15, -0.08), (8.4, -1.45, 0.1), (9.55, -1.45, -0.05)):
        bag(b, m, (x, y, 0), turn)
    b.collider(moved(lathe([(0.0, 0.0), (0.85, 0.0), (0.85, lift + oh), (0.0, lift + oh)], 8, 2.0), (ox, oy, 0)))
    b.collider(box((2.4, 2.4, 1.0), (ox + 1.9, oy - 1.55, 0.5)))
    light_mast(b, m, (8.2, -5.2, 0), 7.0, 2, 2.75)
    return b.finish()
