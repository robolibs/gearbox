"""Everything else a working base has lying about it: cargo, comms masts,
dishes, a weather station, marker poles, tyre tracks, and NASA's own models."""
import math
import os
import random

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import patterns
from buildings import _MATS, mats
from kit import (TAU, Building, box, cone, lathe, merge, moved, project_uv, rod, sphere, srgb_to_linear, tube)


# Maps


def weathered(colour, seed=0, chips=0.3, grime=0.3, runs=1.0):
    """Paint on steel after years outside, as (diff, nor, rough): blotchy
    grime, `runs` of streaks down the sheet, chips through to rust, orange
    peel. `colour` is sRGB."""
    n = patterns.N
    rng = np.random.default_rng(seed)
    paint = srgb_to_linear(np.array(colour, dtype=np.float32))
    dirt = patterns.noise(cells=6, seed=seed)
    peel = patterns.noise(cells=160, seed=seed + 1, octaves=2)
    lanes = rng.random(80).astype(np.float32)
    lanes = np.interp(np.arange(n) * 80.0 / n, np.arange(81), np.append(lanes, lanes[0])).astype(np.float32)
    streak = runs * lanes[None, :] ** 4 * patterns.noise(cells=3, seed=seed + 2)
    spots = patterns.noise(cells=72, seed=seed + 3, octaves=3)
    chip = np.clip((spots - (0.8 - 0.04 * chips)) / 0.015, 0.0, 1.0)
    shade = 1.0 - grime * (dirt - 0.5) - 0.2 * streak - 0.05 * peel
    rust = np.array([0.20, 0.08, 0.03], dtype=np.float32)
    diff = paint * shade[..., None]
    diff = diff + (rust - diff) * np.clip(0.3 * streak + chip, 0.0, 1.0)[..., None]
    rough = 0.55 + 0.2 * dirt + 0.2 * chip
    height = 0.25 * peel - 0.8 * chip
    return (np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 2.0),
            np.clip(rough, 0.0, 1.0).astype(np.float32))


def metal(colour, seed=0, brushed=False):
    """Bare metal, as (diff, nor, rough): galvanised spangle, or brushed
    lines along u, with handling marks."""
    c = srgb_to_linear(np.array(colour, dtype=np.float32))
    n = patterns.N
    rng = np.random.default_rng(seed)
    if brushed:
        lines = rng.random(n).astype(np.float32)
        grain = np.repeat(lines[:, None], n, axis=1) * 0.6 + 0.4 * patterns.noise(cells=256, seed=seed, octaves=1)
    else:
        grain = patterns.noise(cells=40, seed=seed, octaves=1)
    marks = patterns.noise(cells=8, seed=seed + 1)
    diff = c * (0.9 + 0.14 * grain + 0.2 * (marks - 0.5))[..., None]
    rough = 0.32 + 0.12 * grain + 0.15 * marks
    return (np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height((0.2 * grain).astype(np.float32), 1.0),
            np.clip(rough, 0.0, 1.0).astype(np.float32))


def timber(colour=(0.62, 0.47, 0.31), seed=0):
    """Sawn boards, sun-greyed: grain running along u, fibres, a few knots."""
    u, v = patterns.grid()
    wood = srgb_to_linear(np.array(colour, dtype=np.float32))
    warp = patterns.noise(cells=4, seed=seed)
    fibre = patterns.noise(cells=256, seed=seed + 1, octaves=1)
    grain = 0.5 + 0.5 * np.sin(TAU * (v * 24.0 + 1.2 * warp))
    knots = np.clip((patterns.noise(cells=28, seed=seed + 2, octaves=1) - 0.86) / 0.02, 0.0, 1.0)
    shade = (0.78 + 0.18 * grain + 0.12 * fibre) * (1.0 - 0.35 * knots)
    grey = np.float32(0.25)
    diff = (wood * 0.7 + grey * 0.3) * shade[..., None]
    height = 0.3 * grain + 0.2 * fibre - 0.5 * knots
    return (np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 1.5),
            (0.72 + 0.12 * grain).astype(np.float32))


def moulded(colour, seed=0):
    """Rotomoulded plastic: a fine stipple, scuffs and a little dust."""
    c = srgb_to_linear(np.array(colour, dtype=np.float32))
    stipple = patterns.noise(cells=256, seed=seed, octaves=1)
    scuff = patterns.noise(cells=12, seed=seed + 1)
    diff = c * (0.92 + 0.12 * stipple + 0.25 * (scuff - 0.5))[..., None]
    return (np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height((0.5 * stipple).astype(np.float32), 0.6),
            (0.62 + 0.15 * stipple).astype(np.float32))


def labels(seed=5):
    """A 2 x 2 atlas of labels: a shipping label, a flammable-goods diamond,
    a stamped data plate and a yellow caution placard, their text drawn as
    glyph-like marks."""
    n = patterns.N
    h = n // 2
    rng = np.random.default_rng(seed)
    img = np.empty((n, n, 3), dtype=np.float32)
    ink = np.array([0.02, 0.02, 0.02], dtype=np.float32)
    white = np.array([0.80, 0.80, 0.78], dtype=np.float32)

    def text(y0, x0, y1, x1, rows, colour):
        """Rows of glyphs, each a random 5 x 3 grid of strokes, with word gaps."""
        gh = (y1 - y0) // rows * 6 // 10
        gw = gh * 3 // 5
        for r in range(rows):
            y = y0 + r * (y1 - y0) // rows
            x = x0
            while x + gw <= x1:
                if rng.random() > 0.16:
                    bits = rng.random((5, 3)) < 0.55
                    for i in range(5):
                        for j in range(3):
                            if bits[i, j]:
                                img[y + i * gh // 5: y + (i + 1) * gh // 5, x + j * gw // 3: x + (j + 1) * gw // 3] = colour
                x += gw + gw // 3

    def frame(y0, x0, colour, inset):
        img[y0 + inset: y0 + h - inset, x0 + inset: x0 + inset + 10] = colour
        img[y0 + inset: y0 + h - inset, x0 + h - inset - 10: x0 + h - inset] = colour
        img[y0 + inset: y0 + inset + 10, x0 + inset: x0 + h - inset] = colour
        img[y0 + h - inset - 10: y0 + h - inset, x0 + inset: x0 + h - inset] = colour

    yy, xx = np.mgrid[0:h, 0:h]
    # Shipping label: header bar, text, a barcode.
    img[:h, :h] = white
    frame(0, 0, ink, 8)
    img[30:110, 30:h - 30] = ink
    text(45, 50, 100, h - 50, 1, white)
    text(140, 40, 360, h - 40, 6, ink)
    bars = np.repeat(rng.random(110) < 0.5, 4)[: h - 80]
    img[390:470, 40:h - 40][:, bars] = ink
    # Flammable diamond: red, a white inner border, a flame and its class.
    img[:h, h:] = np.array([0.55, 0.03, 0.02], dtype=np.float32)
    frame(0, h, white, 26)
    flame = (xx - 340) ** 2 + (yy - 170) ** 2 < 70 ** 2
    img[:h, h:][flame] = ink
    text(330, h + 150, 420, h + 230, 1, white)
    # Data plate: brushed steel, engraved rows, rivets.
    img[h:, :h] = (0.42 + 0.06 * patterns.noise(n=h, cells=128, seed=seed, octaves=1)[..., None])
    frame(h, 0, np.array([0.2, 0.2, 0.2], dtype=np.float32), 20)
    text(h + 60, 60, h + 450, h - 60, 8, np.array([0.12, 0.12, 0.12], dtype=np.float32))
    for cy, cx in ((40, 40), (40, h - 40), (h - 40, 40), (h - 40, h - 40)):
        img[h:, :h][(xx - cx) ** 2 + (yy - cy) ** 2 < 12 ** 2] = 0.65
    # Caution placard: yellow, black border, a warning triangle, text.
    img[h:, h:] = np.array([0.80, 0.50, 0.02], dtype=np.float32)
    frame(h, h, ink, 14)
    tri = (yy > 50) & (yy < 230) & (np.abs(xx - h // 2) < (yy - 50) * 0.6)
    hole = (yy > 90) & (yy < 215) & (np.abs(xx - h // 2) < (yy - 90) * 0.6 - 12)
    img[h:, h:][tri & ~hole] = ink
    img[h:, h:][(np.abs(xx - h // 2) < 8) & (yy > 130) & (yy < 185)] = ink
    text(h + 270, h + 50, h + 460, n - 50, 3, ink)
    height = np.zeros((n, n), dtype=np.float32)
    rough = np.full((n, n), 0.55, dtype=np.float32)
    rough[h:, :h] = 0.35
    return img, patterns.normals_from_height(height, 1.0), rough


def ruts(seed=11):
    """Rover tracks across a road's width (u) and a width's length along it
    (v): two pairs of ruts, chevron treads in one pair and bar treads in the
    other, berms pushed up beside them, a compacted crown, pebbles, and
    alpha fading out at the verges."""
    u, v = patterns.grid()
    grain = patterns.noise(cells=48, seed=seed)
    coarse = patterns.noise(cells=6, seed=seed + 1)
    stones = np.clip((patterns.noise(cells=90, seed=seed + 2, octaves=1) - 0.8) / 0.03, 0.0, 1.0)
    depth = np.zeros_like(u)
    tread = np.zeros_like(u)
    berm = np.zeros_like(u)
    for centre, w, chevron in ((0.3, 0.05, True), (0.7, 0.05, True), (0.235, 0.04, False), (0.765, 0.04, False)):
        d = np.abs(u - centre) / w
        trough = np.clip(1.0 - d * d, 0.0, 1.0)
        mark = ((v * 40.0 + d * 0.9) % 1.0 < 0.42) if chevron else ((v * 56.0) % 1.0 < 0.45)
        depth = np.maximum(depth, trough * (0.7 + 0.3 * mark))
        tread = np.maximum(tread, trough * mark)
        berm = np.maximum(berm, np.exp(-((d - 1.3) / 0.35) ** 2))
    crown = np.clip(1.0 - np.abs(u - 0.5) / 0.14, 0.0, 1.0)
    loose = 1.0 - np.clip(depth * 2.0, 0.0, 1.0)
    base = np.array([0.11, 0.066, 0.043], dtype=np.float32)
    shade = 0.86 + 0.25 * grain + 0.12 * (coarse - 0.5) - 0.3 * depth - 0.14 * tread + 0.08 * berm + 0.04 * crown - 0.3 * stones * loose
    edge = np.minimum(u, 1.0 - u)
    alpha = np.clip((edge - 0.04 - 0.08 * patterns.noise(cells=12, seed=seed + 3)) / 0.14, 0.0, 1.0) * 0.88
    diff = np.concatenate([np.clip(base * shade[..., None], 0.0, 1.0), alpha[..., None]], axis=-1)
    height = -depth + 0.3 * berm + 0.15 * grain + 0.4 * stones * loose
    rough = 0.82 + 0.1 * grain - 0.08 * tread
    return diff.astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 4.0), rough.astype(np.float32)


# Materials


SKINS = {"container_orange": (0.72, 0.32, 0.10), "container_white": (0.82, 0.82, 0.79), "container_blue": (0.12, 0.26, 0.47)}
DRUMS = {"drum_blue": (0.10, 0.27, 0.55), "drum_orange": (0.82, 0.38, 0.08), "drum_grey": (0.42, 0.44, 0.45), "drum_white": (0.82, 0.82, 0.80), "drum_red": (0.55, 0.10, 0.07)}
CASES = {"case_olive": (0.30, 0.32, 0.22), "case_black": (0.07, 0.07, 0.07), "case_orange": (0.85, 0.42, 0.10), "case_grey": (0.40, 0.42, 0.43)}


def _specs():
    """Each own material's (name, colour, rough, metal, extra arguments)."""
    s = {f"{k}_paint": (f"prop_{k}", c, 0.6, 0.0, {"maps": lambda c=c, i=i: weathered(c, 20 + i)}) for i, (k, c) in enumerate(SKINS.items())}
    s.update({k: (f"prop_{k}", c, 0.6, 0.0, {"maps": lambda c=c, i=i: weathered(c, 30 + i, chips=0.8, grime=0.2)}) for i, (k, c) in enumerate(DRUMS.items())})
    s.update({k: (f"prop_{k}", c, 0.65, 0.0, {"maps": lambda c=c, i=i: moulded(c, 40 + i)}) for i, (k, c) in enumerate(CASES.items())})
    s.update({
        "galv": ("prop_galvanised", (0.60, 0.61, 0.62), 0.42, 0.9, {"maps": lambda: metal((0.60, 0.61, 0.62), 70)}),
        "alu": ("prop_aluminium", (0.76, 0.77, 0.78), 0.38, 0.95, {"maps": lambda: metal((0.76, 0.77, 0.78), 71, True)}),
        "slab": ("prop_slab", (0.62, 0.47, 0.37), 0.9, 0.0, {"tex": "rough_concrete", "contrast": 1.0}),
        "timber": ("prop_timber", (0.58, 0.44, 0.30), 0.75, 0.0, {"maps": lambda: timber((0.58, 0.44, 0.30), 50)}),
        "ply": ("prop_plywood", (0.70, 0.56, 0.38), 0.75, 0.0, {"maps": lambda: timber((0.70, 0.56, 0.38), 51)}),
        "webbing": ("prop_webbing", (0.92, 0.66, 0.10), 0.8, 0.0, {"tex": "hessian_230", "contrast": 0.5}),
        "retro": ("prop_retroreflective", (0.86, 0.87, 0.88), 0.3, 0.5, {}),
        "pole": ("prop_fibreglass", (0.93, 0.40, 0.06), 0.55, 0.0, {"maps": lambda: moulded((0.93, 0.40, 0.06), 60)}),
        "pallet": ("prop_spill_pallet", (0.90, 0.68, 0.10), 0.65, 0.0, {"maps": lambda: moulded((0.90, 0.68, 0.10), 61)}),
        "cabinet": ("prop_cabinet", (0.76, 0.77, 0.75), 0.6, 0.0, {"maps": lambda: weathered((0.76, 0.77, 0.75), 62, chips=0.1, grime=0.2, runs=0.3)}),
        "dish_white": ("prop_dish_white", (0.86, 0.86, 0.84), 0.7, 0.0, {"maps": lambda: weathered((0.86, 0.86, 0.84), 63, chips=0.05, grime=0.15, runs=0.15)}),
        "radome": ("prop_radome", (0.88, 0.88, 0.86), 0.7, 0.0, {"maps": lambda: moulded((0.88, 0.88, 0.86), 64)}),
        "label": ("prop_labels", (0.8, 0.8, 0.8), 0.55, 0.0, {"maps": labels}),
        "road": ("prop_road", (0.6, 0.6, 0.6), 0.85, 0.0, {"maps": ruts}),
    })
    return s


class _Own(dict):
    """The base set with the props' own materials, each made when first
    asked for, in place of any base material of the same key."""

    def __init__(self, m, looks):
        self.specs = _specs()
        super().__init__({k: v for k, v in m.items() if k not in self.specs})
        self.looks = looks

    def __missing__(self, key):
        name, colour, rough, metal, extra = self.specs[key]
        self[key] = self.looks.get(name, colour, rough, metal, **extra)
        return self[key]


_OWN = {}


def own(m):
    """The base set `m` with the props' own materials, made with its looks."""
    if _OWN.get("base") is not m:
        _OWN.update(base=m, set=_Own(m, _MATS["looks"]))
    return _OWN["set"]


# Parts


def aim(bm, at, direction):
    """Turns a part built about +Z to point along `direction`, and moves it to `at`."""
    rot = Vector(direction).to_track_quat("Z", "X" if abs(Vector(direction).normalized().y) > 0.9 else "Y").to_matrix().to_4x4()
    bm.transform(Matrix.Translation(Vector(at)) @ rot)
    return bm


def web(lo, hi, thickness, tile=1.0):
    """A plate between two edges of (x, z) points, point for point, `thickness` across Y."""
    bm = bmesh.new()
    rows = [[bm.verts.new((a[0], -thickness / 2, a[1])), bm.verts.new((c[0], -thickness / 2, c[1])),
             bm.verts.new((c[0], thickness / 2, c[1])), bm.verts.new((a[0], thickness / 2, a[1]))] for a, c in zip(lo, hi)]
    for a, c in zip(rows, rows[1:]):
        for i in range(4):
            bm.faces.new([a[i], a[(i + 1) % 4], c[(i + 1) % 4], c[i]])
    bm.faces.new(rows[0])
    bm.faces.new(rows[-1][::-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    project_uv(bm, tile)
    return bm


def corrugated(length, height, pitch, depth, matrix, tile=3.0):
    """A trapezoid-corrugated sheet: ribs up local Z across local X, its
    crests at local Y = 0 facing +Y, set in place by `matrix`."""
    n = max(1, round(length / pitch))
    p = length / n
    flat, slope = p * 0.26, p * 0.24
    pts = []
    for k in range(n):
        x = -length / 2 + k * p
        pts += [(x, 0.0), (x + flat, 0.0), (x + flat + slope, -depth), (x + p - slope, -depth)]
    pts.append((length / 2, 0.0))
    bm = bmesh.new()
    lo = [bm.verts.new((x, y, -height / 2)) for x, y in pts]
    hi = [bm.verts.new((x, y, height / 2)) for x, y in pts]
    for i in range(len(pts) - 1):
        bm.faces.new([lo[i], lo[i + 1], hi[i + 1], hi[i]])
    bm.transform(matrix)
    bm.normal_update()
    out = matrix.to_3x3() @ Vector((0, 1, 0))
    for face in bm.faces:
        if face.normal.dot(out) < 0:
            face.normal_flip()
    project_uv(bm, tile)
    return bm


LABELS = {"ship": (0, 0), "hazard": (1, 0), "plate": (0, 1), "caution": (1, 1)}


def label(kind, w, h, diamond=False):
    """A label facing +X, `w` across and `h` high, showing one cell of the
    label atlas; a `diamond` stands on its corner."""
    cx, cy = LABELS[kind]
    u0, u1 = cx * 0.5 + 0.005, cx * 0.5 + 0.495
    v0, v1 = 0.5 - cy * 0.5 + 0.005, 1.0 - cy * 0.5 - 0.005
    corners = [(0, -h / 2), (w / 2, 0), (0, h / 2), (-w / 2, 0)] if diamond else [(-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)]
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    face = bm.faces.new([bm.verts.new((0.0, y, z)) for y, z in corners])
    for loop, st in zip(face.loops, ((u0, v0), (u1, v0), (u1, v1), (u0, v1))):
        loop[uv].uv = st
    return bm


def lamp(b, m, at, up=(0, 0, 1)):
    """An obstruction light: a dark housing under a red lens."""
    b.put(aim(lathe([(0.0, 0.0), (0.08, 0.0), (0.08, 0.1), (0.0, 0.1)], 12, 0.5), at, up), m["dark"])
    b.put(aim(lathe([(0.0, 0.1), (0.07, 0.1), (0.07, 0.16), (0.04, 0.2), (0.0, 0.21)], 12, 0.5), at, up), m["red"])


# Cargo


def container(b, m, at, yaw, skin="container_orange", high=False):
    """A 20 ft ISO container: corrugated sides, front end and roof between
    corner posts and rails, castings with their apertures at the corners,
    fork pockets, and double doors at +X with four locking bars, cams,
    keepers, handles, hinges, a gasket and its data plate."""
    p = own(m)
    paint = p[f"{skin}_paint"] if skin in SKINS else m[skin]
    length, width, h = 6.058, 2.438, (2.896 if high else 2.591)
    hx, hy = length / 2, width / 2
    z0, z1 = 0.16, h - 0.1
    zc = (z0 + z1) / 2
    shell = bmesh.new()
    for s in (-1, 1):
        merge(shell, corrugated(length - 0.3, z1 - z0, 0.278, 0.036, Matrix.Translation((0, s * (hy - 0.012), zc)) @ Matrix.Rotation(0.0 if s > 0 else math.pi, 4, "Z")))
    merge(shell, corrugated(width - 0.3, z1 - z0, 0.278, 0.045, Matrix.Translation((-hx + 0.012, 0, zc)) @ Matrix.Rotation(math.pi / 2, 4, "Z")))
    merge(shell, corrugated(length - 0.3, width - 0.2, 0.25, 0.02, Matrix.Translation((0, 0, h - 0.012)) @ Matrix.Rotation(math.pi / 2, 4, "X")))
    # Door leaves, corrugated inside their frames.
    leaf = (width - 0.3) / 2 - 0.006
    d0, d1 = 0.2, h - 0.24
    for s in (-1, 1):
        yc = s * (leaf / 2 + 0.006)
        merge(shell, corrugated(leaf - 0.1, d1 - d0 - 0.1, 0.26, 0.022, Matrix.Translation((hx - 0.045, yc, (d0 + d1) / 2)) @ Matrix.Rotation(-math.pi / 2, 4, "Z")))
        for size, c in (((0.05, leaf, 0.06), (hx - 0.035, yc, d1 - 0.03)), ((0.05, leaf, 0.06), (hx - 0.035, yc, d0 + 0.03)),
                        ((0.05, 0.06, d1 - d0), (hx - 0.035, yc - leaf / 2 + 0.03, (d0 + d1) / 2)), ((0.05, 0.06, d1 - d0), (hx - 0.035, yc + leaf / 2 - 0.03, (d0 + d1) / 2))):
            merge(shell, box(size, c, tile=3.0))
    # Posts, rails, end rails and castings.
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(shell, box((0.15, 0.15, h - 0.236), (sx * (hx - 0.075), sy * (hy - 0.075), h / 2), tile=3.0))
    for sy in (-1, 1):
        merge(shell, box((length - 0.356, 0.1, 0.1), (0, sy * (hy - 0.05), h - 0.055), tile=3.0))
        merge(shell, box((length - 0.356, 0.06, 0.16), (0, sy * (hy - 0.03), 0.09), tile=3.0))
    merge(shell, box((0.1, width - 0.324, 0.1), (-hx + 0.05, 0, h - 0.055), tile=3.0))
    merge(shell, box((0.1, width - 0.324, 0.16), (-hx + 0.05, 0, 0.09), tile=3.0))
    merge(shell, box((0.1, width - 0.324, 0.24), (hx - 0.05, 0, h - 0.12), tile=3.0))
    merge(shell, box((0.1, width - 0.324, 0.2), (hx - 0.05, 0, 0.1), tile=3.0))
    dark = bmesh.new()
    for sx in (-1, 1):
        for sy in (-1, 1):
            for cz in (0.059, h - 0.059):
                cx, cy = sx * (hx - 0.089), sy * (hy - 0.081)
                merge(shell, box((0.178, 0.162, 0.118), (cx, cy, cz)))
                merge(dark, box((0.124, 0.006, 0.064), (cx, sy * (hy + 0.001), cz)))
                merge(dark, box((0.006, 0.064, 0.064), (sx * (hx + 0.001), cy, cz)))
                merge(dark, box((0.124, 0.064, 0.006), (cx, cy, h + 0.001 if cz > 1 else -0.001)))
    for sy in (-1, 1):
        for x in (-1.025, 1.025):
            merge(dark, box((0.36, 0.006, 0.1), (x, sy * (hy + 0.001), 0.09)))
    merge(dark, box((length - 0.3, width - 0.2, 0.02), (0, 0, 0.05)))
    # Door gear: locking bars in guides, cams and keepers, handles, hinges.
    gear = bmesh.new()
    for y in (-0.8, -0.27, 0.27, 0.8):
        x = hx + 0.012
        merge(gear, rod((x, y, 0.12), (x, y, h - 0.12), 0.014, 8))
        for z in (0.55, 1.25, 1.95):
            merge(gear, box((0.03, 0.06, 0.05), (hx - 0.002, y, z)))
        for z, kz in ((0.16, 0.1), (h - 0.16, h - 0.1)):
            merge(gear, box((0.05, 0.05, 0.08), (x + 0.01, y, z)))
            merge(gear, box((0.05, 0.1, 0.05), (hx + 0.012, y, kz)))
        inward = math.copysign(1.0, math.copysign(leaf / 2, y) - y)
        merge(gear, box((0.025, 0.22, 0.035), (x + 0.03, y + inward * 0.11, 1.05)))
        merge(gear, box((0.04, 0.05, 0.08), (x + 0.02, y + inward * 0.2, 1.05)))
    for sy in (-1, 1):
        for z in (0.45, 1.05, 1.65, h - 0.6):
            merge(gear, box((0.06, 0.1, 0.12), (hx + 0.01, sy * (hy - 0.2), z)))
            merge(gear, rod((hx + 0.03, sy * (hy - 0.14), z - 0.08), (hx + 0.03, sy * (hy - 0.14), z + 0.08), 0.015, 6))
    place = Matrix.Translation(Vector(at)) @ Matrix.Rotation(yaw, 4, "Z")
    for bm, mat in ((shell, paint), (dark, m["dark"]), (gear, p["galv"])):
        bm.transform(place)
        b.put(bm, mat, smooth=False)
    gasket = box((0.03, 0.025, d1 - d0), (hx - 0.03, 0, (d0 + d1) / 2))
    gasket.transform(place)
    b.put(gasket, m["rubber"], smooth=False)
    plate = moved(label("plate", 0.24, 0.16), (hx + 0.002, -0.55, 1.5))
    plate.transform(place)
    b.put(plate, p["label"], smooth=False)
    b.collider(moved(box((length, width, h), (0, 0, h / 2)), at, yaw))


def _case(p, m, w, d, h, rng):
    """A rotomoulded hard case: body and lid with a groove between, latches, end handles."""
    shell = p[rng.choice(tuple(CASES))]
    parts = [(box((w, d, h * 0.68), (0, 0, h * 0.34), tile=1.0, bevel=0.03), shell, True),
             (box((w, d, h * 0.3), (0, 0, h * 0.85), tile=1.0, bevel=0.03), shell, True),
             (box((w - 0.02, d - 0.02, h * 0.04), (0, 0, h * 0.69)), m["dark"], False)]
    gear = bmesh.new()
    for sy in (-1, 1):
        for x in (-w * 0.3, w * 0.3):
            merge(gear, box((0.07, 0.025, 0.1), (x, sy * (d / 2 + 0.008), h * 0.69)))
    for sx in (-1, 1):
        merge(gear, rod((sx * (w / 2 + 0.035), -0.12, h * 0.8), (sx * (w / 2 + 0.035), 0.12, h * 0.8), 0.015, 6))
        for sy in (-1, 1):
            merge(gear, box((0.04, 0.03, 0.05), (sx * (w / 2 + 0.015), sy * 0.13, h * 0.8)))
    parts.append((gear, m["dark"], False))
    parts.append((moved(label("ship", 0.2, 0.14), (0.0, d / 2 + 0.002, h * 0.85), math.pi / 2), p["label"], False))
    return parts


def _crate(p, m, w, d, h, rng):
    """A timber crate: plywood panels in a frame of battens, a diagonal
    brace each side, skids under it and two steel bands round it."""
    parts = [(box((w - 0.05, d - 0.05, h - 0.12), (0, 0, 0.1 + (h - 0.12) / 2), tile=1.2), p["ply"], False)]
    wood = bmesh.new()
    for sy in (-1, 1):
        merge(wood, box((w, 0.09, 0.1), (0, sy * (d / 2 - 0.12), 0.05), tile=1.2))
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(wood, box((0.07, 0.07, h - 0.1), (sx * (w / 2 - 0.035), sy * (d / 2 - 0.035), 0.1 + (h - 0.1) / 2), tile=1.2))
    rise = math.atan2(h - 0.27, w - 0.14)
    for z in (0.135, h - 0.055):
        for sy in (-1, 1):
            merge(wood, box((w - 0.14, 0.025, 0.07), (0, sy * (d / 2 - 0.0125), z), tile=1.2))
        for sx in (-1, 1):
            merge(wood, box((0.025, d - 0.14, 0.07), (sx * (w / 2 - 0.0125), 0, z), tile=1.2))
    for sy in (-1, 1):
        merge(wood, moved(box((math.hypot(w - 0.14, h - 0.27), 0.022, 0.07), tile=1.2), (0, sy * (d / 2 - 0.011), 0.1 + (h - 0.1) / 2), 0.0, -rise))
    for x in (-w * 0.3, 0.0, w * 0.3):
        merge(wood, box((0.09, d - 0.04, 0.025), (x, 0, h - 0.0075), tile=1.2))
    parts.append((wood, p["timber"], False))
    bands = bmesh.new()
    for x in (-w * 0.22, w * 0.22):
        merge(bands, box((0.032, d + 0.012, 0.004), (x, 0, h + 0.01)))
        for sy in (-1, 1):
            merge(bands, box((0.032, 0.004, h - 0.1), (x, sy * (d / 2 + 0.006), 0.1 + (h - 0.1) / 2)))
    parts.append((bands, p["galv"], False))
    parts.append((moved(label("ship", 0.22, 0.16), (w * 0.05, -d / 2 - 0.002, h * 0.6), -math.pi / 2), p["label"], False))
    if rng.random() < 0.5:
        parts.append((moved(label("caution", 0.16, 0.16), (w / 2 + 0.002, 0.0, h * 0.62)), p["label"], False))
    return parts


def _alu(p, m, w, d, h, rng):
    """A ribbed aluminium transport box: a deep lid on a seal, corner caps, latches, fold-down handles."""
    parts = [(box((w, d, h - 0.14), (0, 0, (h - 0.14) / 2), tile=1.0), p["alu"], False),
             (box((w + 0.01, d + 0.01, 0.13), (0, 0, h - 0.065), tile=1.0), p["alu"], False),
             (box((w - 0.004, d - 0.004, 0.012), (0, 0, h - 0.136)), m["rubber"], False)]
    ribs = bmesh.new()
    for z in (h * 0.28, h * 0.52):
        merge(ribs, box((w + 0.012, d + 0.012, 0.022), (0, 0, z), tile=1.0))
    merge(ribs, box((w + 0.022, d + 0.022, 0.022), (0, 0, h - 0.11), tile=1.0))
    parts.append((ribs, p["alu"], False))
    gear = bmesh.new()
    for sx in (-1, 1):
        for sy in (-1, 1):
            for z in (0.03, h - 0.03):
                merge(gear, box((0.06, 0.06, 0.06), (sx * (w / 2 - 0.02), sy * (d / 2 - 0.02), z)))
        merge(gear, rod((sx * (w / 2 + 0.02), -0.1, h * 0.7), (sx * (w / 2 + 0.02), 0.1, h * 0.7), 0.012, 6))
    for x in (-w * 0.28, w * 0.28):
        merge(gear, box((0.06, 0.02, 0.09), (x, -d / 2 - 0.012, h - 0.15)))
    parts.append((gear, m["dark"], False))
    parts.append((moved(label("plate", 0.16, 0.1), (0.0, -d / 2 - 0.004, h * 0.4), -math.pi / 2), p["label"], False))
    return parts


def _bag(p, m, w, d, h, rng):
    """A white cargo transfer bag: soft sides, two webbing straps over it, grab handles, a label."""
    parts = [(box((w, d, h), (0, 0, h / 2), tile=1.0, bevel=0.05), m["fabric"], True)]
    straps = bmesh.new()
    for x in (-w * 0.25, w * 0.25):
        merge(straps, box((0.05, d + 0.01, 0.006), (x, 0, h + 0.002)))
        for sy in (-1, 1):
            merge(straps, box((0.05, 0.006, h - 0.04), (x, sy * (d / 2 + 0.002), h / 2)))
        merge(straps, tube([(x, -0.12, h), (x, -0.08, h + 0.07), (x, 0.08, h + 0.07), (x, 0.12, h)], 0.012, 6))
    parts.append((straps, p["webbing"], False))
    parts.append((moved(label("ship", 0.18, 0.12), (w / 2 + 0.002, 0.0, h * 0.6)), p["label"], False))
    return parts


def crates(b, m, at, yaw, seed=0, count=6):
    """A stack of cargo, pallet-wide: timber crates, hard cases, ribbed
    aluminium boxes and cargo bags, a few stacked on the ones below."""
    rng = random.Random(seed)
    p = own(m)
    makers = (_crate, _case, _alu, _bag)
    base = Matrix.Translation(Vector(at)) @ Matrix.Rotation(yaw, 4, "Z")
    ground = []
    for k in range(count):
        if k < 5:
            w, d, h = rng.choice(((1.2, 1.0, 0.9), (1.2, 0.8, 0.6), (0.8, 0.6, 0.6), (1.2, 1.1, 1.0)))
            x, y, z = (k % 3) * 1.4, (k // 3) * 1.3, 0.0
            ground.append((x, y, h, w, d))
        else:
            x, y, under, uw, ud = ground[(k - 5) % len(ground)]
            w, d, h = min(uw, 0.8), min(ud, 0.6), rng.choice((0.4, 0.5, 0.6))
            z = under
        turn = rng.uniform(-0.1, 0.1)
        place = base @ Matrix.Translation((x, y, z)) @ Matrix.Rotation(turn, 4, "Z")
        for bm, mat, smooth in rng.choice(makers)(p, m, w, d, h, rng):
            bm.transform(place)
            b.put(bm, mat, smooth=smooth, sharp=40)
        col = box((w, d, h), (0, 0, h / 2))
        col.transform(place)
        b.collider(col)


DRUM = [(0.0, 0.012), (0.268, 0.012), (0.278, 0.0), (0.292, 0.0), (0.297, 0.012), (0.292, 0.026), (0.287, 0.034),
        (0.287, 0.27), (0.3, 0.284), (0.3, 0.298), (0.287, 0.312), (0.287, 0.568), (0.3, 0.582), (0.3, 0.596), (0.287, 0.61),
        (0.287, 0.846), (0.292, 0.854), (0.297, 0.868), (0.292, 0.882), (0.278, 0.882), (0.272, 0.866), (0.0, 0.866)]


def barrels(b, m, at, seed=0, count=8):
    """200 l steel drums, two rolling hoops and rolled chimes each, bungs
    in their lids, standing four to a grated spill pallet."""
    rng = random.Random(seed)
    p = own(m)
    deck = 0.28
    groups = {}
    for k in range(count):
        x, y = (k % 4) * 0.7, (k // 4) * 0.7
        groups.setdefault(((k % 4) // 2, (k // 4) // 2), []).append(k)
        c = Vector(at) + Vector((x, y, deck))
        spin = rng.uniform(0, TAU)
        b.put(moved(lathe(DRUM, 24, 1.0), tuple(c), spin), p[rng.choice(tuple(DRUMS))], sharp=30)
        bungs = moved(lathe([(0.0, 0.0), (0.038, 0.0), (0.038, 0.01), (0.03, 0.018), (0.0, 0.018)], 8, 0.2), (0.19, 0, 0.866))
        merge(bungs, moved(lathe([(0.0, 0.0), (0.022, 0.0), (0.022, 0.012), (0.0, 0.016)], 8, 0.2), (-0.2, 0.04, 0.866)))
        b.put(moved(bungs, tuple(c), spin), p["galv"], smooth=False)
        if rng.random() < 0.4:
            face = Vector((math.cos(spin), math.sin(spin), 0))
            b.put(moved(label("hazard", 0.15, 0.15, True), tuple(c + face * 0.302 + Vector((0, 0, 0.44))), spin), p["label"], smooth=False)
    for (gx, gy), _ in groups.items():
        c = Vector(at) + Vector((gx * 1.4 + 0.35, gy * 1.4 + 0.35, 0))
        b.put(moved(box((1.36, 1.36, deck - 0.02), (0, 0, (deck - 0.02) / 2), tile=1.0, bevel=0.02), tuple(c)), p["pallet"], smooth=False)
        b.put(moved(box((1.26, 1.26, 0.02), (0, 0, deck - 0.01), tile=0.6), tuple(c)), m["grate"], smooth=False)
        b.collider(moved(box((1.36, 1.36, deck + 0.88), (0, 0, (deck + 0.88) / 2)), tuple(c)))


def cable_reel(b, m, at, yaw):
    """A cable drum on its rim, chocked: flanges of spoked steel or bolted
    boards, a barrel wound with cable whose tail runs off along the ground,
    an arbor hole through each hub."""
    p = own(m)
    rng = random.Random(round(at[0] * 10) * 7919 + round(at[1] * 10))
    big, half, core, wound = 1.0, 0.55, 0.42, 0.8
    spin = Matrix.Translation(Vector(at) + Vector((0, 0, big))) @ Matrix.Rotation(yaw, 4, "Z") @ Matrix.Rotation(math.pi / 2, 4, "Y")
    steel = rng.random() < 0.5
    flange, hubs, dark = bmesh.new(), bmesh.new(), bmesh.new()
    for s in (-1, 1):
        z = s * half
        if steel:
            merge(flange, lathe([(big - 0.08, z - 0.03), (big, z - 0.03), (big, z + 0.03), (big - 0.08, z + 0.03), (big - 0.08, z - 0.03)], 40, 1.0))
            for k in range(8):
                merge(flange, moved(box((big - 0.1 - core, 0.08, 0.04), ((big - 0.08 + core) / 2 - 0.01, 0, z)), (0, 0, 0), TAU * k / 8))
            merge(flange, lathe([(0.0, z - 0.02), (core + 0.06, z - 0.02), (core + 0.06, z + 0.02), (0.0, z + 0.02)], 24, 1.0))
            merge(hubs, lathe([(wound, z - s * 0.035), (core, z - s * 0.035)][::s], 28, 1.0))
        else:
            merge(flange, lathe([(0.0, z - 0.04), (big, z - 0.04), (big, z + 0.04), (0.0, z + 0.04)], 40, 1.0))
            for k in range(10):
                a = TAU * k / 10
                merge(hubs, moved(lathe([(0.0, 0.0), (0.03, 0.0), (0.03, 0.012), (0.0, 0.015)], 6, 0.2), (0.32 * math.cos(a), 0.32 * math.sin(a), z + s * 0.04), 0.0, 0.0 if s > 0 else math.pi))
        merge(dark, moved(lathe([(0.065, 0.0), (0.0, 0.0)], 16, 0.2), (0, 0, z + s * (0.045 if not steel else 0.025)), 0.0, 0.0 if s > 0 else math.pi))
    merge(flange, lathe([(core, -half), (core, half)], 28, 1.0))
    wraps = []
    for k in range(15):
        z = -half + 0.03 + (2 * half - 0.06) * k / 14
        wraps += [(wound - 0.006, z), (wound + 0.01, z + (2 * half - 0.06) / 28)]
    cable = lathe(wraps[:-1], 28, 0.5)
    if not steel:
        project_uv(flange, 1.2)
    for bm, mat, sharp in ((flange, p["galv"] if steel else p["timber"], 40), (hubs, m["cable"] if steel else p["galv"], 40), (dark, m["dark"], 40), (cable, m["cable"], 80)):
        bm.transform(spin)
        b.put(bm, mat, sharp=sharp)
    side = Vector((-math.sin(yaw), math.cos(yaw), 0))
    axis = Vector((math.cos(yaw), math.sin(yaw), 0))
    root = Vector(at) + axis * 0.3
    tail = [root + Vector((0, 0, big - wound)) + side * 0.05, root + side * 0.45 + Vector((0, 0, 0.12)), root + side * 0.8 + Vector((0, 0, 0.04)),
            root + side * 1.6 + axis * 0.2 + Vector((0, 0, 0.04)), root + side * 2.1 + axis * 0.5 + Vector((0, 0, 0.04))]
    b.put(tube(tail, 0.03, 8), m["cable"])
    b.put(moved(lathe([(0.0, 0.0), (0.04, 0.0), (0.04, 0.12), (0.0, 0.12)], 8, 0.2), tuple(tail[-1]), math.atan2((tail[-1] - tail[-2]).y, (tail[-1] - tail[-2]).x), math.pi / 2), m["rubber"])
    chocks = bmesh.new()
    for s in (-1, 1):
        for t in (-1, 1):
            wedge = web([(0.0, 0.0), (0.32, 0.0)], [(0.0, 0.16), (0.32, 0.015)], 0.18)
            merge(chocks, moved(wedge, tuple(Vector(at) + axis * s * half + side * t * 0.62), yaw + (math.pi / 2 if t > 0 else -math.pi / 2)))
    b.put(chocks, p["timber"], smooth=False)


def marker_poles(b, m, path, every=12.0):
    """Route poles along a path, each side of it: orange fibreglass with two
    retroreflective bands and a cap, on a weighted rubber foot."""
    p = own(m)
    path = [Vector(q) for q in path]
    foot, pole, band, cap = bmesh.new(), bmesh.new(), bmesh.new(), bmesh.new()
    for a, c in zip(path, path[1:]):
        run = c - a
        side = Vector((-run.y, run.x, 0)).normalized() * 4.2
        for k in range(int(run.length / every) + 1):
            q = a + run * (k * every / run.length)
            for s in (-1, 1):
                at = q + side * s
                merge(foot, moved(lathe([(0.0, 0.0), (0.2, 0.0), (0.2, 0.035), (0.12, 0.06), (0.045, 0.1), (0.04, 0.14), (0.0, 0.14)], 12, 0.5), tuple(at)))
                merge(pole, rod(at + Vector((0, 0, 0.1)), at + Vector((0, 0, 1.6)), 0.022, 8))
                for z in (1.2, 1.42):
                    merge(band, rod(at + Vector((0, 0, z)), at + Vector((0, 0, z + 0.12)), 0.024, 8))
                merge(cap, moved(lathe([(0.0, 0.0), (0.03, 0.0), (0.03, 0.04), (0.0, 0.06)], 8, 0.2), tuple(at + Vector((0, 0, 1.58)))))
    for bm, mat in ((foot, m["rubber"]), (pole, p["pole"]), (band, p["retro"]), (cap, m["dark"])):
        b.put(bm, mat)


# Comms and science


def _lattice(z0, z1, w, bays, chord, brace, top):
    """A square lattice section: four chords, girts and zig-zag diagonals on each face."""
    bm = bmesh.new()
    corners = [Vector((sx * w / 2, sy * w / 2, 0)) for sx, sy in ((1, 1), (-1, 1), (-1, -1), (1, -1))]
    up = Vector((0, 0, 1))
    for c in corners:
        merge(bm, rod(c + up * z0, c + up * z1, chord, 6))
    for k in range(bays + 1):
        z = z0 + (z1 - z0) * k / bays
        if k < bays or top:
            for i in range(4):
                merge(bm, rod(corners[i] + up * z, corners[(i + 1) % 4] + up * z, brace, 4))
        if k < bays:
            z2 = z0 + (z1 - z0) * (k + 1) / bays
            for i in range(4):
                lo, hi = (z, z2) if (k + i) % 2 == 0 else (z2, z)
                merge(bm, rod(corners[i] + up * lo, corners[(i + 1) % 4] + up * hi, brace, 4))
    return bm


def lattice_mast(looks, name, at=(0, 0, 0), height=32.0, parent=None):
    """A guyed lattice comms mast banded orange and white: sector panels
    with their radios, whips, two microwave dishes, obstruction lights and a
    lightning rod; a caged ladder and a feeder ladder up its faces; guys to
    concrete anchors through turnbuckles; an equipment cabinet at its foot
    with a cable bridge across."""
    m = mats(looks)
    p = own(m)
    b = Building(name, parent, at, 0.0)
    w, base = 1.1, 0.4
    bands = 7
    sec = (height - base) / bands
    for k in range(bands):
        b.put(_lattice(base + k * sec, base + (k + 1) * sec, w, max(1, round(sec / w)), 0.045, 0.022, k == bands - 1), m["hull_orange"] if k % 2 == 0 else m["white_frame"])
    b.put(box((2.0, 2.0, 0.4), (0, 0, 0.2), tile=2.0, bevel=0.03), p["slab"], smooth=False)
    foot = box((1.3, 1.3, 0.03), (0, 0, base + 0.015))
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(foot, rod((sx * 0.55, sy * 0.55, base), (sx * 0.55, sy * 0.55, base + 0.12), 0.03, 6))
    b.put(foot, p["galv"], smooth=False)
    # Caged ladder up the +X face, with a fall-arrest wire.
    x = w / 2 + 0.2
    top = height - 0.6
    ladder = bmesh.new()
    for y in (-0.2, 0.2):
        merge(ladder, rod((x, y, base), (x, y, top), 0.016, 6))
    for k in range(int((top - base - 0.3) / 0.3)):
        z = base + 0.3 + k * 0.3
        merge(ladder, rod((x, -0.2, z), (x, 0.2, z), 0.011, 4))
    z = base + 1.0
    while z < top:
        for y in (-0.2, 0.2):
            merge(ladder, rod((x, y, z), (w / 2, y * 2.2, z), 0.012, 4))
        z += 1.5
    merge(ladder, rod((x, 0.0, base + 0.5), (x, 0.0, top), 0.005, 4))
    cage = [k * 0.9 + 2.4 for k in range(int((top - 2.4) / 0.9) + 1)]
    for z in cage:
        merge(ladder, tube([(x + 0.35 + 0.38 * math.cos(t), 0.38 * math.sin(t), z) for t in [math.radians(-125 + 250 * i / 12) for i in range(13)]], 0.012, 4))
    for t in (-80, -40, 0, 40, 80):
        a = math.radians(t)
        merge(ladder, rod((x + 0.35 + 0.38 * math.cos(a), 0.38 * math.sin(a), cage[0]), (x + 0.35 + 0.38 * math.cos(a), 0.38 * math.sin(a), cage[-1]), 0.01, 4))
    b.put(ladder, p["galv"])
    # Feeder ladder up the -Y face, its coax running down to the cabinet.
    fy = -(w / 2 + 0.12)
    feeder = bmesh.new()
    for fx in (-0.16, 0.16):
        merge(feeder, rod((fx, fy, 2.2), (fx, fy, height - 0.8), 0.012, 4))
    for k in range(int((height - 3.0) / 0.6)):
        merge(feeder, rod((-0.16, fy, 2.5 + k * 0.6), (0.16, fy, 2.5 + k * 0.6), 0.01, 4))
    b.put(feeder, p["galv"])
    cab = Vector((0.0, -2.6, 0.0))
    coax = bmesh.new()
    for fx in (-0.09, -0.03, 0.03, 0.09):
        merge(coax, tube([(fx, fy - 0.04, height - 0.9), (fx, fy - 0.04, 2.3), (fx, fy - 0.3, 2.05), (fx, cab.y + 0.3, 2.05), (fx, cab.y + 0.2, 1.9), (fx, cab.y + 0.2, 1.75)], 0.014, 6))
    b.put(coax, m["cable"])
    bridge = bmesh.new()
    for bx in (-0.2, 0.2):
        merge(bridge, rod((bx, fy, 1.98), (bx, cab.y + 0.35, 1.98), 0.015, 4))
        merge(bridge, rod((bx, cab.y + 0.35, 0.2), (bx, cab.y + 0.35, 1.98), 0.03, 6))
    for k in range(6):
        yk = fy + (cab.y + 0.35 - fy) * k / 5
        merge(bridge, rod((-0.2, yk, 1.98), (0.2, yk, 1.98), 0.012, 4))
    b.put(bridge, p["galv"])
    # Equipment cabinet on its plinth: doors, louvres, a cooler.
    b.put(box((1.2, 0.9, 0.15), tuple(cab + Vector((0, 0, 0.075))), tile=2.0), p["slab"], smooth=False)
    b.put(box((1.0, 0.7, 1.6), tuple(cab + Vector((0, 0, 0.95))), tile=1.0, bevel=0.015), p["cabinet"], smooth=False)
    trim = bmesh.new()
    merge(trim, box((0.008, 0.006, 1.4), tuple(cab + Vector((0, -0.352, 0.95)))))
    for sx in (-1, 1):
        merge(trim, box((0.03, 0.03, 0.14), tuple(cab + Vector((sx * 0.06, -0.36, 1.0)))))
        for k in range(5):
            merge(trim, box((0.006, 0.4, 0.03), tuple(cab + Vector((sx * 0.503, 0.0, 0.5 + 0.07 * k)))))
    merge(trim, box((0.4, 0.3, 0.5), tuple(cab + Vector((0.71, 0.0, 0.85)))))
    b.put(trim, m["dark"], smooth=False)
    b.put(moved(label("caution", 0.2, 0.2), tuple(cab + Vector((0.25, -0.352, 1.35))), -math.pi / 2), p["label"], smooth=False)
    # Sector panels on pipes with their radios, and two whips.
    panels, pipes, radios = bmesh.new(), bmesh.new(), bmesh.new()
    for k, az in enumerate((60.0, 180.0, 300.0)):
        a = math.radians(az)
        out = Vector((math.cos(a), math.sin(a), 0))
        at_pipe = out * (w / 2 + 0.5)
        merge(pipes, rod(at_pipe + Vector((0, 0, height - 2.0)), at_pipe + Vector((0, 0, height + 0.1)), 0.035, 8))
        for z in (height - 1.7, height - 0.3):
            merge(pipes, rod(out * (w / 2) + Vector((0, 0, z)), at_pipe + Vector((0, 0, z)), 0.03, 6))
        merge(panels, moved(box((0.12, 0.32, 1.4), (0, 0, 0), bevel=0.02), tuple(at_pipe + out * 0.14 + Vector((0, 0, height - 0.75))), a, -0.05))
        merge(radios, moved(box((0.16, 0.26, 0.42), (0, 0, 0), bevel=0.01), tuple(at_pipe - out * 0.12 + Vector((0, 0, height - 1.75))), a))
        merge(radios, moved(box((0.14, 0.34, 0.04), (0, 0, 0)), tuple(at_pipe + out * 0.14 + Vector((0, 0, height - 1.47))), a))
    b.put(panels, p["radome"], sharp=40)
    b.put(radios, m["hull_grey"], smooth=False)
    for az in (120.0, 240.0):
        a = math.radians(az)
        out = Vector((math.cos(a), math.sin(a), 0))
        merge(pipes, rod(out * (w / 2) + Vector((0, 0, height - 0.2)), out * (w / 2 + 0.7) + Vector((0, 0, height - 0.2)), 0.025, 6))
        b.put(moved(cone(0.025, 0.012, 2.2, segments=8), tuple(out * (w / 2 + 0.7) + Vector((0, 0, height - 0.3)))), p["radome"])
    b.put(pipes, p["galv"])
    # Microwave dishes on side arms.
    for az, z, dia in ((135.0, height * 0.72, 0.9), (220.0, height * 0.6, 0.6)):
        a = math.radians(az)
        out = Vector((math.cos(a), math.sin(a), 0))
        centre = out * (w / 2 + 0.55) + Vector((0, 0, z))
        b.put(rod(out * (w / 2) + Vector((0, 0, z)), centre - out * 0.2, 0.035, 6), p["galv"])
        r = dia / 2
        b.put(aim(lathe([(0.0, -0.22), (0.12, -0.22), (r * 0.8, -0.08), (r, -0.02), (r, 0.12), (0.0, 0.12)], 24, 0.5), centre, out), m["hull_grey"])
        b.put(aim(lathe([(r - 0.01, 0.12), (r * 0.6, 0.15), (0.0, 0.16)], 24, 0.5), centre, out), p["radome"])
    # Lights and the lightning rod.
    lamp(b, m, (0.3, 0.0, height + 0.02))
    lamp(b, m, (-0.3, 0.0, height + 0.02))
    for sx in (-1, 1):
        lamp(b, m, (sx * (w / 2 + 0.1), 0.0, height * 0.5), (sx, 0, 0))
    b.put(moved(cone(0.035, 0.008, 2.4, segments=8), (0, 0, height)), p["galv"])
    # Guys from two levels to an anchor each way, through turnbuckles.
    guys, buckles = bmesh.new(), bmesh.new()
    for k in range(3):
        a = TAU * k / 3
        c = Vector((math.cos(a), math.sin(a), 0))
        anchor = c * height * 0.55
        eye = anchor + Vector((0, 0, 0.45)) - c * 0.25
        for z in (height * 0.9, height * 0.5):
            top_end = c * (w / 2) + Vector((0, 0, z))
            run = (eye - top_end).normalized()
            buckle = eye - run * 0.5
            merge(guys, rod(top_end, buckle - run * 0.2, 0.008, 4))
            merge(buckles, rod(buckle - run * 0.2, buckle + run * 0.2, 0.022, 6))
            merge(guys, rod(buckle + run * 0.2, eye, 0.01, 4))
            merge(buckles, box((0.08, 0.08, 0.1), tuple(top_end)))
        b.put(moved(box((1.0, 0.7, 0.4), (0, 0, 0.2), tile=2.0, bevel=0.04), tuple(anchor), a), p["slab"], smooth=False)
        merge(buckles, rod(anchor + Vector((0, 0, 0.35)), eye, 0.03, 6))
        merge(buckles, moved(box((0.04, 0.22, 0.16), (0, 0, 0)), tuple(eye), a))
    b.put(guys, m["steel"])
    b.put(buckles, p["galv"], smooth=False)
    b.collider(box((1.2, 1.2, height), (0, 0, height / 2)))
    b.collider(box((1.2, 0.9, 1.75), tuple(cab + Vector((0, 0, 0.875)))))
    for k in range(3):
        a = TAU * k / 3
        b.collider(moved(box((1.0, 0.7, 0.4), (0, 0, 0.2)), tuple(Vector((math.cos(a), math.sin(a), 0)) * height * 0.55), a))
    return b.finish()


def dish(looks, name, diameter=8.0, at=(0, 0, 0), yaw=0.0, elevation=45.0, parent=None):
    """A steerable Cassegrain antenna, elevation over azimuth, its boresight
    `elevation` degrees above the horizon toward +X: a reflector of two rings
    of panels on a back structure of radial ribs and hoops, a corrugated
    feed horn at its vertex and a subreflector on a quadripod; a yoke with
    counterweights turning on an elevation housing, an azimuth bearing atop
    a pedestal, on a slab with anchor bolts and an equipment cabinet."""
    m = mats(looks)
    p = own(m)
    b = Building(name, parent, at, yaw)
    s = diameter / 5.0
    r = diameter / 2
    f = 0.35 * diameter
    vertex = 0.95 * s
    za = 2.95 * s
    e = math.radians(elevation)
    turn = Matrix.Translation((0, 0, za)) @ Matrix.Rotation(math.pi / 2 - e, 4, "Y")

    def sag(rho):
        return vertex + rho * rho / (4 * f)

    # Slab, anchor bolts, pedestal, gussets and its door.
    b.put(box((3.6 * s, 3.6 * s, 0.4), (0, 0, 0.0), tile=2.0, bevel=0.03), p["slab"], smooth=False)
    bolts = bmesh.new()
    for k in range(12):
        a = TAU * k / 12
        merge(bolts, moved(lathe([(0.0, 0.2), (0.045, 0.2), (0.045, 0.21), (0.032, 0.21), (0.032, 0.245), (0.018, 0.245), (0.018, 0.29), (0.0, 0.29)], 6, 0.2), (0.66 * s * math.cos(a), 0.66 * s * math.sin(a), 0), a))
    b.put(bolts, p["galv"], smooth=False)
    paint = p["dish_white"]
    b.put(lathe([(0.0, 0.2), (0.78 * s, 0.2), (0.78 * s, 0.24), (0.5 * s, 0.25), (0.47 * s, 0.32), (0.44 * s, 1.95 * s), (0.6 * s, 1.95 * s), (0.6 * s, 2.0 * s), (0.0, 2.0 * s)], 40, 1.0), paint, sharp=40)
    gussets = bmesh.new()
    for k in range(8):
        merge(gussets, moved(web([(0.46 * s, 0.25), (0.76 * s, 0.25)], [(0.46 * s, 0.75 * s), (0.76 * s, 0.29)], 0.025), (0, 0, 0), TAU * k / 8 + TAU / 16))
    b.put(gussets, paint, smooth=False)
    b.put(lathe([(0.452 * s, 0.6 * s), (0.448 * s, 1.5 * s)], 6, 1.0, angle=0.75, start=-0.375 + math.pi), p["cabinet"])
    b.put(moved(box((0.04, 0.03, 0.12), (0, 0, 0)), (-0.47 * s, 0.1 * s, 1.05 * s)), m["dark"], smooth=False)
    # Azimuth housing and bearing, its two drives.
    b.put(lathe([(0.0, 2.0 * s), (0.72 * s, 2.0 * s), (0.72 * s, 2.22 * s), (0.0, 2.22 * s)], 40, 1.0), paint, sharp=40)
    b.put(lathe([(0.72 * s, 2.22 * s), (0.76 * s, 2.24 * s), (0.76 * s, 2.3 * s), (0.72 * s, 2.32 * s)], 40, 0.5), m["dark"], sharp=40)
    b.put(lathe([(0.0, 2.32 * s), (0.72 * s, 2.32 * s), (0.7 * s, 2.42 * s), (0.0, 2.42 * s)], 40, 1.0), paint, sharp=40)
    drives, motors = bmesh.new(), bmesh.new()
    for sy in (-1, 1):
        merge(drives, box((0.26 * s, 0.22 * s, 0.2 * s), (0, sy * 0.86 * s, 2.3 * s)))
        merge(motors, moved(lathe([(0.0, 1.92 * s), (0.09 * s, 1.92 * s), (0.09 * s, 2.2 * s), (0.0, 2.2 * s)], 12, 0.5), (0, sy * 0.86 * s, 0)))
    b.put(drives, m["hull_grey"], smooth=False)
    b.put(motors, m["dark"])
    # Elevation housing, its bearings and drive.
    b.put(box((1.0 * s, 0.9 * s, 0.75 * s), (0, 0, 2.42 * s + 0.375 * s), tile=1.0, bevel=0.03), paint, smooth=False)
    for sy in (-1, 1):
        b.put(moved(lathe([(0.0, 0.0), (0.24 * s, 0.0), (0.24 * s, 0.13 * s), (0.0, 0.13 * s)], 24, 0.5), (0, sy * 0.45 * s, za), 0.0, 0.0, -sy * math.pi / 2), m["hull_grey"], sharp=40)
    b.put(box((0.32 * s, 0.3 * s, 0.3 * s), (-0.62 * s, 0.1 * s, 2.75 * s), bevel=0.01), m["hull_grey"], smooth=False)
    b.put(moved(lathe([(0.0, 0.0), (0.1 * s, 0.0), (0.1 * s, 0.3 * s), (0.0, 0.3 * s)], 12, 0.5), (-0.62 * s, 0.25 * s, 2.75 * s), 0.0, 0.0, -math.pi / 2), m["dark"])
    b.put(tube([(-0.5 * s, -0.25 * s, 2.6 * s), (-0.72 * s, -0.25 * s, 2.45 * s), (-0.66 * s, -0.25 * s, 2.1 * s), (-0.48 * s, -0.25 * s, 1.8 * s)], 0.035 * s, 8), m["cable"])
    # What turns in elevation, in the dish's frame: +Z its boresight.
    yoke, weights, ribs, skin, seams, front, feed = (bmesh.new() for _ in range(7))
    for sy in (-1, 1):
        merge(yoke, moved(web([(-0.18 * s, -1.0 * s), (-0.26 * s, -0.15 * s), (-0.5 * s, vertex - 0.45 * s)],
                              [(0.18 * s, -1.0 * s), (0.26 * s, -0.15 * s), (0.5 * s, vertex - 0.45 * s)], 0.04 * s), (0, sy * 0.62 * s, 0)))
        merge(weights, box((0.36 * s, 0.24 * s, 0.42 * s), (0, sy * 0.8 * s, -0.82 * s), bevel=0.01))
        merge(yoke, moved(lathe([(0.0, 0.0), (0.2 * s, 0.0), (0.2 * s, 0.05 * s), (0.0, 0.05 * s)], 20, 0.5), (0, sy * 0.64 * s, 0), 0.0, 0.0, -sy * math.pi / 2))
    merge(yoke, rod((0, -0.62 * s, -0.9 * s), (0, 0.62 * s, -0.9 * s), 0.05 * s, 8))
    merge(yoke, lathe([(0.0, vertex - 0.45 * s), (0.55 * s, vertex - 0.45 * s), (0.55 * s, vertex), (0.0, vertex)], 32, 1.0))
    merge(yoke, box((0.34 * s, 0.26 * s, 0.2 * s), (0.0, 0.0, vertex - 0.56 * s), bevel=0.01))
    inner, outer = 0.55 * s, r
    for k in range(16):
        rows = [inner + (outer - inner) * i / 6 for i in range(7)]
        back = [(q, vertex - 0.45 * s + (sag(outer) - 0.12 * s - vertex + 0.45 * s) * (q - inner) / (outer - inner)) for q in rows]
        merge(ribs, moved(web(back, [(q, sag(q) - 0.035 * s) for q in rows], 0.035 * s), (0, 0, 0), TAU * k / 16 + TAU / 32))
    for q, dz in ((1.45 * s, 0.1 * s), (outer - 0.03 * s, 0.07 * s)):
        merge(ribs, tube([(q * math.cos(t), q * math.sin(t), sag(q) - dz) for t in [TAU * i / 48 for i in range(49)]], 0.03 * s, 6))
    merge(skin, lathe([(q, sag(q) - 0.03 * s) for q in [inner + (outer - inner) * i / 10 for i in range(11)]], 64, 2.0))
    merge(skin, lathe([(outer, sag(outer) - 0.03 * s), (outer, sag(outer) + 0.004 * s)], 64, 1.0))
    merge(seams, lathe([(q, sag(q) - 0.012 * s) for q in [outer - (outer - inner) * i / 10 for i in range(11)]], 64, 2.0))
    for count, q0, q1, segs in ((8, inner, 1.44 * s, 6), (16, 1.46 * s, outer, 5)):
        gap = 0.02 * s / ((q0 + q1) / 2)
        for j in range(count):
            profile = [(q, sag(q)) for q in [q1 - (q1 - q0) * i / 5 for i in range(6)]]
            merge(front, lathe(profile, segs, 1.0, angle=TAU / count - gap, start=TAU * j / count + gap / 2))
    horn = [(0.0, vertex - 0.02 * s), (0.2 * s, vertex - 0.02 * s), (0.2 * s, vertex + 0.08 * s), (0.13 * s, vertex + 0.1 * s)]
    for i in range(7):
        z, q = vertex + (0.12 + 0.09 * i) * s, (0.13 + 0.014 * i) * s
        horn += [(q, z), (q + 0.025 * s, z + 0.01 * s), (q + 0.025 * s, z + 0.06 * s), (q, z + 0.07 * s)]
    horn += [(0.24 * s, vertex + 0.8 * s), (0.0, vertex + 0.82 * s)]
    merge(feed, lathe(horn, 24, 0.5))
    zs = vertex + 0.82 * f
    merge(feed, lathe([(0.0, zs - 0.07 * s), (0.16 * s, zs - 0.05 * s), (0.32 * s, zs), (0.33 * s, zs + 0.03 * s), (0.2 * s, zs + 0.08 * s), (0.0, zs + 0.1 * s)], 32, 0.5))
    for k in range(4):
        a = TAU * k / 4 + TAU / 8
        foot = Vector((0.96 * r * math.cos(a), 0.96 * r * math.sin(a), sag(0.96 * r)))
        merge(feed, rod(foot, (0.12 * s * math.cos(a), 0.12 * s * math.sin(a), zs + 0.07 * s), 0.035 * s, 8))
        merge(feed, box((0.12 * s, 0.12 * s, 0.06 * s), tuple(foot)))
    for bm, mat, smooth in ((yoke, paint, True), (weights, m["hull_grey"], False), (ribs, paint, True), (skin, paint, True),
                            (seams, m["dark"], True), (front, p["dish_white"], True), (feed, paint, True)):
        bm.transform(turn)
        b.put(bm, mat, smooth=smooth, sharp=40)
    # Equipment cabinet, its conduit to the pedestal and a warning sign.
    cab = Vector((-1.25 * s, 1.15 * s, 0.2))
    b.put(box((0.7, 0.9, 1.3), tuple(cab + Vector((0, 0, 0.65))), tile=1.0, bevel=0.015), p["cabinet"], smooth=False)
    trim = bmesh.new()
    merge(trim, box((0.006, 0.008, 1.1), tuple(cab + Vector((-0.352, 0, 0.65)))))
    for sy in (-1, 1):
        merge(trim, box((0.03, 0.03, 0.12), tuple(cab + Vector((-0.365, sy * 0.06, 0.7)))))
    for k in range(4):
        merge(trim, box((0.4, 0.006, 0.03), tuple(cab + Vector((0, 0.453, 0.3 + 0.07 * k)))))
    b.put(trim, m["dark"], smooth=False)
    b.put(moved(label("caution", 0.22, 0.22), tuple(cab + Vector((-0.353, -0.22, 0.95))), math.pi), p["label"], smooth=False)
    b.put(tube([tuple(cab + Vector((0.35, -0.2, 0.15))), (-0.6 * s, 0.55 * s, 0.35), (-0.45 * s, 0.25 * s, 0.4)], 0.04, 8), m["cable"])
    b.collider(lathe([(0.0, 0.0), (0.78 * s, 0.0), (0.6 * s, 2.4 * s), (0.0, 2.4 * s)], 8, 2.0))
    b.collider(box((3.6 * s, 3.6 * s, 0.4), (0, 0, 0.0)))
    b.collider(box((0.7, 0.9, 1.3), tuple(cab + Vector((0, 0, 0.65)))))
    return b.finish()


def weather_station(looks, name, at=(0, 0, 0), parent=None):
    """A guyed met mast on a footing: cup anemometer and wind vane on a
    cross arm at the top under a lightning rod, a radiation-shield stack
    for temperature and humidity, a pyranometer, a dust sampler, a solar
    panel, and a logger box with its cables run down the mast."""
    m = mats(looks)
    p = own(m)
    b = Building(name, parent, at, 0.0)
    top = 6.0
    b.put(box((0.8, 0.8, 0.2), (0, 0, 0.1), tile=1.0, bevel=0.02), p["slab"], smooth=False)
    metal = bmesh.new()
    merge(metal, box((0.42, 0.42, 0.02), (0, 0, 0.21)))
    for sx in (-1, 1):
        merge(metal, box((0.02, 0.14, 0.2), (sx * 0.06, 0, 0.32)))
    merge(metal, rod((0, 0, 0.22), (0, 0, top), 0.038, 12))
    merge(metal, rod((0, 0, 2.9), (0, 0, 3.15), 0.046, 12))
    merge(metal, rod((0, 0, top), (0, 0, top + 0.9), 0.008, 6))
    # Cross arm with the anemometer at +Y and the vane at -Y.
    merge(metal, box((0.04, 1.3, 0.04), (0, 0, top - 0.25)))
    merge(metal, box((0.1, 0.1, 0.12), (0, 0, top - 0.25)))
    for y in (-0.62, 0.62):
        merge(metal, rod((0, y, top - 0.25), (0, y, top - 0.05), 0.012, 8))
    b.put(metal, p["galv"])
    gear = bmesh.new()
    merge(gear, lathe([(0.0, top - 0.05), (0.03, top - 0.05), (0.03, top + 0.12), (0.02, top + 0.14), (0.0, top + 0.15)], 12, 0.2))
    for k in range(3):
        a = TAU * k / 3
        arm = Vector((math.cos(a), math.sin(a), 0))
        hub = Vector((0, 0.62, top + 0.13))
        merge(gear, rod(hub, hub + arm * 0.12, 0.004, 4))
        merge(gear, aim(lathe([(0.0, 0.0), (0.045, 0.0), (0.04, 0.025), (0.0, 0.05)], 10, 0.2), hub + arm * 0.12, Vector((-arm.y, arm.x, 0))))
    vane = Vector((0, -0.62, top + 0.1))
    merge(gear, moved(lathe([(0.0, -0.15), (0.02, -0.15), (0.02, 0.0), (0.0, 0.01)], 10, 0.2), tuple(vane)))
    merge(gear, rod(vane - Vector((0.2, 0, 0)), vane + Vector((0.32, 0, 0)), 0.006, 6))
    merge(gear, moved(web([(0.18, -0.08), (0.36, -0.1)], [(0.24, 0.08), (0.36, 0.1)], 0.004), tuple(vane)))
    merge(gear, moved(cone(0.025, 0.0, 0.08, segments=8), tuple(vane - Vector((0.2, 0, 0))), 0.0, -math.pi / 2))
    b.put(gear, m["white_frame"])
    # Radiation shield stack on an arm, the pyranometer and dust sampler higher up.
    shield = bmesh.new()
    sx = 0.45
    merge(shield, rod((0.04, 0, 2.0), (sx, 0, 2.0), 0.015, 6))
    for k in range(9):
        z = 1.82 + k * 0.03
        merge(shield, moved(lathe([(0.0, z), (0.1, z - 0.015), (0.1, z - 0.011), (0.0, z + 0.004)], 16, 0.2), (sx, 0, 0)))
    merge(shield, moved(cone(0.1, 0.06, 0.04, segments=16), (sx, 0, 2.08)))
    b.put(shield, m["white_frame"], sharp=50)
    pyr = bmesh.new()
    merge(pyr, box((0.5, 0.03, 0.03), (-0.25, 0, 2.6)))
    merge(pyr, moved(lathe([(0.0, 2.62), (0.08, 2.62), (0.08, 2.64), (0.0, 2.64)], 16, 0.2), (-0.5, 0, 0)))
    merge(pyr, moved(lathe([(0.0, 2.64), (0.05, 2.64), (0.05, 2.69), (0.0, 2.69)], 16, 0.2), (-0.5, 0, 0)))
    b.put(pyr, m["white_frame"])
    b.put(sphere(0.026, (-0.5, 0, 2.69), 12, 4, low=0.0), m["glass"])
    dust = bmesh.new()
    merge(dust, box((0.04, 0.4, 0.03), (0, 0.2, 3.3)))
    merge(dust, box((0.12, 0.18, 0.2), (0, 0.42, 3.3), bevel=0.01))
    merge(dust, tube([(0, 0.42, 3.4), (0, 0.42, 3.5), (0.05, 0.42, 3.55), (0.08, 0.42, 3.53)], 0.012, 8))
    b.put(dust, p["cabinet"])
    b.put(moved(box((0.002, 0.08, 0.06)), (0.061, 0.42, 3.33)), m["glass"], smooth=False)
    # Solar panel, logger box and the cables down the mast.
    b.put(moved(box((0.04, 0.6, 0.45), (0, 0, 0)), (-0.3, -0.3, 1.7), 0.0, -0.7), m["solar"], smooth=False)
    frame = bmesh.new()
    merge(frame, box((0.04, 0.45, 0.04), (-0.1, -0.3, 1.7)))
    merge(frame, rod((-0.1, -0.08, 1.7), (-0.04, 0.0, 1.7), 0.015, 6))
    merge(frame, box((0.03, 0.04, 0.3), (-0.2, -0.3, 1.6)))
    b.put(frame, p["galv"], smooth=False)
    b.put(box((0.22, 0.38, 0.48), (0, -0.25, 1.05), tile=1.0, bevel=0.01), p["cabinet"], smooth=False)
    trim = bmesh.new()
    for z in (0.92, 1.18):
        merge(trim, box((0.03, 0.02, 0.05), (0.115, -0.08, z)))
    for y in (-0.32, -0.25, -0.18):
        merge(trim, moved(lathe([(0.0, 0.0), (0.014, 0.0), (0.014, 0.03), (0.0, 0.03)], 8, 0.2), (0, y, 0.78)))
    b.put(trim, m["dark"], smooth=False)
    b.put(moved(label("plate", 0.12, 0.08), (0.111, -0.3, 1.18)), p["label"], smooth=False)
    cables = bmesh.new()
    for dx, top_z in ((0.045, top - 0.25), (-0.045, 3.3), (0.03, 2.6)):
        merge(cables, tube([(dx, 0.04, top_z), (dx, 0.045, 1.4), (dx * 0.5, -0.06, 1.3), (0.0, -0.25, 1.32)], 0.008, 6))
    b.put(cables, m["cable"])
    # Guys to ground anchors.
    guys = bmesh.new()
    for k in range(3):
        a = TAU * k / 3 + math.radians(30)
        c = Vector((math.cos(a), math.sin(a), 0))
        ground = c * 2.4 + Vector((0, 0, 0.05))
        mid = Vector((0, 0, 4.0)).lerp(ground, 0.9)
        merge(guys, rod((0, 0, 4.0), mid, 0.004, 4))
        merge(guys, rod(mid, ground, 0.012, 6))
        merge(guys, moved(box((0.16, 0.16, 0.06), (0, 0, 0.03)), tuple(c * 2.45), a))
    merge(guys, rod((0, 0, 3.95), (0, 0, 4.05), 0.05, 8))
    b.put(guys, p["galv"])
    b.collider(box((0.8, 0.8, 1.4), (0, 0, 0.7)))
    return b.finish()


def tracks(looks, name, path, width=7.0, parent=None):
    """Compacted regolith with tyre ruts, a ribbon laid along a path."""
    p = own(mats(looks))
    b = Building(name, parent, (0, 0, 0), 0.0)
    path = [Vector(q) for q in path]
    pts = []
    for a, c in zip(path, path[1:]):
        n = max(1, int((c - a).length / 6.0))
        pts += [a.lerp(c, k / n) for k in range(n)]
    pts.append(path[-1])
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    rows, run = [], 0.0
    for i, q in enumerate(pts):
        ahead = (pts[min(i + 1, len(pts) - 1)] - pts[max(i - 1, 0)]).normalized()
        side = Vector((-ahead.y, ahead.x, 0))
        if i:
            run += (q - pts[i - 1]).length
        rows.append(([bm.verts.new(q + side * (width / 2) * s + Vector((0, 0, 0.03))) for s in (-1, 1)], run))
    for (a, ra), (c, rc) in zip(rows, rows[1:]):
        face = bm.faces.new([a[0], a[1], c[1], c[0]])
        for loop, st in zip(face.loops, ((0.0, ra / width), (1.0, ra / width), (1.0, rc / width), (0.0, rc / width))):
            loop[uv].uv = st
    b.put(bm, p["road"], smooth=False)
    return b.finish()


# Parts of NASA's models drawn as coarse spheres with their light baked into
# their textures, by file and material: each is rounded onto its sphere and
# painted this sRGB colour instead.
DOMES = {("Radome_plain.glb", "Default"): (0.86, 0.86, 0.84), ("Radome.glb", "Default"): (0.86, 0.86, 0.84)}


def seam_map(bm, colour, path, size=1024):
    """A map of a part's panels: `colour`, its faces' edges drawn darker
    through its UVs as the seams between them, saved to `path`."""
    uv = bm.loops.layers.uv.active
    shade = np.ones((size, size), dtype=np.float32)
    for face in bm.faces:
        pts = [loop[uv].uv.copy() for loop in face.loops]
        for a, c in zip(pts, pts[1:] + pts[:1]):
            t = np.linspace(0.0, 1.0, int(max(abs(c.x - a.x), abs(c.y - a.y)) * size * 2) + 2)
            xs = ((a.x + (c.x - a.x) * t) * size).astype(int)
            ys = ((a.y + (c.y - a.y) * t) * size).astype(int)
            for dx in (0, 1):
                for dy in (0, 1):
                    shade[np.clip(ys + dy, 0, size - 1), np.clip(xs + dx, 0, size - 1)] = 0.8
    rgb = srgb_to_linear(np.array(colour, dtype=np.float32)) * shade[..., None]
    image = bpy.data.images.new(os.path.basename(path), size, size, alpha=False)
    image.pixels.foreach_set(np.concatenate([rgb, np.ones((size, size, 1), dtype=np.float32)], axis=-1).ravel())
    image.filepath_raw = path
    image.file_format = "PNG"
    image.save()
    return image


def round_dome(bm, mat, colour, path=None):
    """Subdivides a coarse sphere and puts every point back on it, its open
    rim kept level; paints
    its material `colour` in place of its texture, its panels' seams drawn
    in a map saved to `path` when given."""
    seams = seam_map(bm, colour, path) if path else None
    bmesh.ops.subdivide_edges(bm, edges=bm.edges[:], cuts=3, use_grid_fill=True)
    lo = Vector([min(v.co[i] for v in bm.verts) for i in range(3)])
    hi = Vector([max(v.co[i] for v in bm.verts) for i in range(3)])
    r = max(hi.x - lo.x, hi.y - lo.y) / 2
    c = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, hi.z - r))
    rim = min(v.co.z for v in bm.verts if v.is_boundary)
    for v in bm.verts:
        if v.is_boundary:
            flat = Vector((v.co.x - c.x, v.co.y - c.y, 0.0)).normalized() * math.sqrt(max(r * r - (rim - c.z) ** 2, 0.0))
            v.co = Vector((c.x + flat.x, c.y + flat.y, rim))
        else:
            v.co = c + (v.co - c).normalized() * r
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    for link in list(bsdf.inputs["Base Color"].links):
        mat.node_tree.links.remove(link)
    bsdf.inputs["Base Color"].default_value = (*(float(k) for k in srgb_to_linear(np.array(colour, dtype=np.float32))), 1.0)
    bsdf.inputs["Roughness"].default_value = 0.7
    if seams:
        node = mat.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = seams
        mat.node_tree.links.new(node.outputs["Color"], bsdf.inputs["Base Color"])


def nasa_model(path, name, parent, at, yaw=0.0, height=None, textures=None, solid=None):
    """Imports one of NASA's public-domain glTF models as a building, its
    parts joined per material, standing on the ground at `at`; `height`
    rescales a model authored at the wrong size. Each part is welded, its UV
    map named `st` as the kit's are, and shaded smooth, sharp only where its
    faces meet at more than 40 degrees;
    the parts in `DOMES` are rounded and repainted. Its packed images are
    written to `textures/<name>/`; `solid`, (radius, height), stands a
    cylinder for it in the physics world."""
    b = Building(name, parent, at, yaw)
    collection, root = b.collection, b.root
    before = set(bpy.data.objects)
    images = set(bpy.data.images)
    bpy.ops.import_scene.gltf(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in new if o.type == "MESH"]
    for o in new:
        for c in list(o.users_collection):
            c.objects.unlink(o)
        collection.objects.link(o)
    if textures:
        folder = os.path.join(textures, b.kind)
        os.makedirs(folder, exist_ok=True)
        for image in set(bpy.data.images) - images:
            if image.packed_file or not image.filepath:
                # Saved as the bytes they were packed in, under the matching extension.
                ext = {"WEBP": "webp", "JPEG": "jpg"}.get(image.file_format, "png")
                image.filepath_raw = os.path.join(folder, f"{bpy.path.clean_name(image.name)}.{ext}")
                image.save()
                if image.packed_file:
                    image.unpack(method="REMOVE")
    depsgraph = bpy.context.evaluated_depsgraph_get()
    by_mat = {}
    lo, hi = 1e9, -1e9
    for o in meshes:
        mesh = bpy.data.meshes.new_from_object(o.evaluated_get(depsgraph))
        mesh.transform(o.matrix_world)
        for v in mesh.vertices:
            lo, hi = min(lo, v.co.z), max(hi, v.co.z)
        for i, slot in enumerate(mesh.materials):
            by_mat.setdefault(slot.name if slot else "none", []).append((mesh, i))
        if not mesh.materials:
            by_mat.setdefault("none", []).append((mesh, 0))
    scale = height / max(hi - lo, 1e-6) if height else 1.0
    root.scale = (scale, scale, scale)
    for mat_name, parts in by_mat.items():
        bm = bmesh.new()
        for mesh, index in parts:
            sub = bmesh.new()
            sub.from_mesh(mesh)
            bmesh.ops.delete(sub, geom=[f for f in sub.faces if f.material_index != index], context="FACES")
            for f in sub.faces:
                f.material_index = 0
            tmp = bpy.data.meshes.new("tmp")
            sub.to_mesh(tmp)
            sub.free()
            bm.from_mesh(tmp)
            bpy.data.meshes.remove(tmp)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5 * (hi - lo))
        dome = DOMES.get((os.path.basename(path), mat_name.split(".")[0]))
        if dome and mat_name in bpy.data.materials:
            seams = os.path.join(textures, b.kind, f"{bpy.path.clean_name(mat_name)}_seams.png") if textures else None
            round_dome(bm, bpy.data.materials[mat_name], dome, seams)
        bmesh.ops.translate(bm, vec=(0, 0, -lo), verts=bm.verts)
        out = bpy.data.meshes.new(f"{name}_{mat_name}")
        bm.to_mesh(out)
        bm.free()
        if "custom_normal" in out.attributes:
            out.attributes.remove(out.attributes["custom_normal"])
        if out.uv_layers:
            out.uv_layers[0].name = "st"
        out.shade_smooth()
        out.set_sharp_from_angle(angle=math.radians(40))
        if mat_name in bpy.data.materials:
            out.materials.append(bpy.data.materials[mat_name])
        obj = bpy.data.objects.new(f"{name}_{mat_name}", out)
        collection.objects.link(obj)
        obj.parent = root
    for mesh in {p[0] for parts in by_mat.values() for p in parts}:
        bpy.data.meshes.remove(mesh)
    for o in new:
        bpy.data.objects.remove(o)
    if solid:
        radius, tall = solid
        b.collider(lathe([(0.0, 0.0), (radius / scale, 0.0), (radius / scale, tall / scale), (0.0, tall / scale)], 12, 2.0))
    return b.finish()
