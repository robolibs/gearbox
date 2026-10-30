"""The spaceport: sintered landing pads ringed by blast walls and lights, an
Ariane 6 on its launch table beside an umbilical mast on one, Honda's
reusable rocket, scaled up as the base's hopper, on the other, and their
propellant."""
import math
import random

import bmesh
import numpy as np
from mathutils import Matrix, Vector

import industry
import patterns
from buildings import light_mast, mats
from kit import (TAU, Building, box, cone, disc, lathe, merge, moved, prism, project_uv, rect, rod, sphere, srgb_to_linear, sweep, truss, tube)

# Top of a pad's sintered centre, where the landers' feet stand.
PAD_TOP = 0.1
# Ariane 6 (A64): core radius, booster radius and its axis's offset from the
# core's; heights up from the Vulcain's exit to the core's foot, the upper
# stage, the fairing, its ogive and the tip; a booster's foot, cone and tip.
A6_R, A6_BOOSTER_R, A6_BOOSTER_AT = 2.7, 1.7, 4.6
A6_CORE, A6_UPPER, A6_FAIRING, A6_OGIVE, A6_TIP = 2.2, 30.5, 42.0, 54.0, 62.0
A6_FOOT, A6_CONE, A6_NOSE = 0.6, 17.6, 22.6
# Its launch table: half the deck's width, the deck's top and underside.
TABLE_HALF, TABLE_TOP, TABLE_UNDER = 8.0, 5.0, 3.4
# Its umbilical mast toward -X: the mast's axis, half its width, its height.
MAST_X, MAST_HALF, MAST_TOP = -15.0, 3.5, 50.0
# The hopper, Honda's test rocket at four times its size: body radius, its
# skirt's fang tips and edge, the nose's foot and tip, its feet off the axis.
HOP_R, HOP_FANG, HOP_SKIRT, HOP_NOSE, HOP_TIP, HOP_FOOT = 1.7, 1.5, 3.0, 22.2, 25.2, 4.6


# Surface maps


def _paint(colour, rings=0, joints=0, soot=0.0, mottle=0.04, seed=61, n=2048):
    """Factory paint over one tile: faint seams `rings` up it and `joints`
    round it, mottling and orange peel; `soot` blackens it in streaks rising
    from the tile's foot."""
    def make():
        u, v = patterns.grid(n)
        up = 1.0 - v
        groove = np.zeros_like(u)
        for count, at in ((rings, up), (joints, u)):
            if count:
                f = at * count % 1.0
                groove = np.maximum(groove, patterns.seam(np.minimum(f, 1.0 - f) / count, 0.0014))
        dirt = patterns.noise(n, cells=6, seed=seed)
        peel = patterns.noise(n, cells=160, seed=seed + 1, octaves=2)
        shade = (1.0 - mottle * dirt - 0.02 * peel) * (1.0 - 0.3 * groove)
        diff = srgb_to_linear(np.array(colour, dtype=np.float32)) * shade[..., None]
        rough = 0.38 + 0.12 * dirt + 0.05 * peel + 0.25 * groove
        if soot:
            streak = patterns.noise(n, cells=10, seed=seed + 2, octaves=2)[:1, :].repeat(n, axis=0)
            smudge = patterns.noise(n, cells=12, seed=seed + 3, octaves=3)
            black = np.clip(soot * (np.exp(-up / (0.2 + 0.18 * streak)) * (0.7 + 0.35 * smudge) + 0.35 * np.exp(-up / 0.55) * streak * smudge), 0.0, 0.94)
            diff = diff * (1.0 - black)[..., None] + np.float32(0.01) * black[..., None]
            rough = rough + 0.35 * black
        height = 0.15 * peel - groove
        return np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 2.0), np.clip(rough, 0.0, 1.0).astype(np.float32)
    return make


def _carbon(n=1024, cells=64):
    """Black carbon fibre in a 2/2 twill: crowned tows, the over and under
    ones taking the light differently."""
    u, v = patterns.grid(n)
    iu, iv = np.floor(u * cells), np.floor(v * cells)
    fu, fv = u * cells % 1.0, v * cells % 1.0
    over = ((iu + iv) % 4) < 2
    crown = np.where(over, np.sin(np.pi * fv), np.sin(np.pi * fu))
    sheen = np.where(over, 1.0, 0.7) * (0.7 + 0.4 * crown)
    diff = np.float32(0.009) * sheen[..., None] * np.ones(3, dtype=np.float32)
    rough = 0.25 + 0.15 * (1.0 - crown) + 0.05 * patterns.noise(n, 16, 83, 2)
    return diff.astype(np.float32), patterns.normals_from_height((0.5 * crown).astype(np.float32), 1.5), rough.astype(np.float32)


def _flag(spec, x, y, c):
    """One flag's colours over (x, y) in [0, 1], y up: "v" or "h" stripes
    and their colours, a Nordic cross ("n"), or a named design."""
    kind, _, cols = spec.partition(" ")
    if kind in ("v", "h"):
        t = x if kind == "v" else 1.0 - y
        return np.stack([c[k] for k in cols])[np.minimum((t * len(cols)).astype(int), len(cols) - 1)]
    if kind == "n":
        out = np.where(((np.abs(x - 0.36) < 0.055) | (np.abs(y - 0.5) < 0.08))[..., None], c[cols[1]], c[cols[0]])
        if len(cols) > 2:
            out = np.where(((np.abs(x - 0.36) < 0.025) | (np.abs(y - 0.5) < 0.037))[..., None], c[cols[2]], out)
        return out
    if kind == "ch":
        sx, sy = np.abs(x - 0.5) * 1.5, np.abs(y - 0.5)
        cross = ((sx < 0.1) & (sy < 0.31)) | ((sy < 0.1) & (sx < 0.31))
        return np.where(cross[..., None], c["w"], c["r"])
    if kind == "gr":
        out = np.where(((((1.0 - y) * 9).astype(int) % 2) == 0)[..., None], c["b"], c["w"])
        canton = (x < 0.37) & (y > 4 / 9)
        cross = (np.abs(x - 0.185) < 0.037) | (np.abs(y - 13 / 18) < 1 / 18)
        return np.where(canton[..., None], np.where(cross[..., None], c["w"], c["b"]), out)
    if kind == "cz":
        out = np.where((y > 0.5)[..., None], c["w"], c["r"])
        return np.where((x < 0.5 * (1.0 - np.abs(2.0 * y - 1.0)))[..., None], c["b"], out)
    X = x * 1.5
    diag = np.minimum(np.abs(X - 1.5 * y), np.abs(X + 1.5 * y - 1.5)) / math.hypot(1.0, 1.5)
    cross = np.minimum(np.abs(X - 0.75), np.abs(y - 0.5))
    out = np.where((diag < 0.1)[..., None], c["w"], c["b"])
    out = np.where((diag < 0.035)[..., None], c["r"], out)
    out = np.where((cross < 0.167)[..., None], c["w"], out)
    return np.where((cross < 0.1)[..., None], c["r"], out)


def _flags(w=4096, h=512):
    """Ariane 6's band of member-state flags on white: two staggered rows of
    eleven round a tile that spans the band once."""
    palette = {"w": (0.94, 0.94, 0.93), "r": (0.80, 0.07, 0.13), "b": (0.0, 0.2, 0.55), "k": (0.03, 0.03, 0.03),
               "y": (1.0, 0.8, 0.0), "g": (0.0, 0.52, 0.27), "o": (1.0, 0.5, 0.1), "l": (0.0, 0.62, 0.86)}
    c = {k: srgb_to_linear(np.array(v, dtype=np.float32)).astype(np.float32) for k, v in palette.items()}
    flags = ("h rwr", "v kyr", "cz", "n rw", "h bkw", "n wb", "v bwr", "h kry", "gr", "h rwg", "v gwo",
             "v gwr", "h rwl", "h rwb", "n rwb", "h wr", "v ggrrr", "v byr", "h ryyr", "n by", "ch", "uk")
    img = np.empty((h, w, 3), dtype=np.float32)
    img[:] = c["w"]
    fw, fh = int(w * 1.05 / (TAU * A6_R)), int(h * 0.35)
    ys, xs = np.mgrid[0:fh, 0:fw]
    x, y = (xs + 0.5) / fw, 1.0 - (ys + 0.5) / fh
    for k, spec in enumerate(flags):
        row, col = divmod(k, 11)
        left = int((col + 0.5 * row + 0.5) * w / 11 - fw / 2)
        top = int(h * (0.1 + 0.45 * row))
        img[top:top + fh, (left + np.arange(fw)) % w] = _flag(spec, x, y, c)
    return img, None, np.full((h, w), 0.4, dtype=np.float32)


def _nozzle(n=1024, channels=180):
    """A regeneratively cooled bell round one tile: cooling channels up it,
    heat tint deepening from the exit (v = 0) toward the throat."""
    u, v = patterns.grid(n)
    t = np.clip((1.0 - v) / 0.45, 0.0, 1.0)
    rib = 0.5 + 0.5 * np.cos(TAU * u * channels)
    blotch = patterns.noise(n, cells=16, seed=31)
    cold = np.array([0.30, 0.30, 0.31], dtype=np.float32)
    straw = np.array([0.32, 0.22, 0.12], dtype=np.float32)
    blue = np.array([0.10, 0.11, 0.16], dtype=np.float32)
    w1 = np.clip(1.0 - np.abs(t - 0.45) / 0.3, 0.0, 1.0)[..., None]
    w2 = np.clip((t - 0.6) / 0.4, 0.0, 1.0)[..., None]
    diff = cold * (1.0 - w1 - w2) + straw * w1 + blue * w2
    diff = diff * (0.85 + 0.25 * blotch)[..., None] * (0.92 + 0.08 * rib)[..., None]
    rough = 0.32 + 0.15 * blotch + 0.1 * t
    return diff.astype(np.float32), patterns.normals_from_height((0.4 * rib).astype(np.float32), 2.0), rough.astype(np.float32)


def _ariane_mats(looks):
    g = looks.get
    white = (0.91, 0.91, 0.89)
    return {
        "core": g("a6_core", white, 0.45, 0.0, maps=_paint(white, rings=4, joints=3, seed=61)),
        "upper": g("a6_upper", white, 0.42, 0.0, maps=_paint(white, rings=6, joints=4, seed=63)),
        "booster": g("a6_booster", white, 0.4, 0.0, maps=_paint(white, rings=2, seed=65)),
        "fairing": g("a6_fairing", (0.93, 0.93, 0.92), 0.35, 0.0, maps=_paint((0.93, 0.93, 0.92), mottle=0.025, seed=67)),
        "white": g("a6_white", white, 0.45, 0.0, maps=_paint(white, seed=69, n=1024)),
        "flags": g("a6_flags", white, 0.4, 0.0, maps=_flags),
        "grey": g("a6_grey", (0.5, 0.51, 0.52), 0.5, 0.3),
        "seam": g("a6_seam", (0.6, 0.6, 0.6), 0.6, 0.0),
        "black": g("a6_marker", (0.025, 0.025, 0.025), 0.6, 0.0),
        "metal": g("a6_fittings", (0.6, 0.61, 0.62), 0.4, 0.9),
        "nozzle": g("vulcain_nozzle", (0.3, 0.3, 0.3), 0.35, 1.0, maps=_nozzle),
        "inside": g("rocket_nozzle_inside", (0.035, 0.03, 0.028), 0.75, 0.4),
        "ablative": g("p120_nozzle", (0.09, 0.085, 0.08), 0.85, 0.0, "rough_concrete", contrast=0.7),
        "shield": g("a6_heat_shield", (0.16, 0.15, 0.14), 0.8, 0.0, "rough_concrete", contrast=1.0),
        "boot": g("vulcain_boot", (0.78, 0.75, 0.68), 0.85, 0.0, "bi_stretch", contrast=0.8),
    }


def _mount_mats(looks):
    g = looks.get
    return {
        "deck": g("table_deck", (0.44, 0.45, 0.46), 0.55, 0.7, "metal_plate", contrast=0.6),
        "steel": g("table_steel", (0.38, 0.39, 0.4), 0.5, 0.6, maps=lambda: patterns.panels((0.15, 0.155, 0.16), cols=1, rows=2, seam_w=0.002, grime=0.15, seed=91)),
        "soot": g("table_sooted", (0.08, 0.075, 0.07), 0.8, 0.3, "metal_plate", contrast=0.9),
        "lattice": g("table_lattice", (0.05, 0.05, 0.055), 0.55, 0.5),
        "clad": g("mast_cladding", (0.86, 0.84, 0.78), 0.65, 0.0, maps=lambda: patterns.panels((0.68, 0.65, 0.58), cols=2, rows=2, seam_w=0.003, rivets=False, grime=0.1, seed=93)),
    }


def _hopper_mats(looks):
    g = looks.get
    white = (0.93, 0.93, 0.91)
    return {
        "white": g("hopper_white", white, 0.42, 0.0, maps=_paint(white, seed=71)),
        "sooted": g("hopper_white_sooted", white, 0.5, 0.0, maps=_paint(white, soot=1.0, seed=73, n=1024)),
        "trim": g("hopper_trim", white, 0.45, 0.0, maps=_paint(white, seed=75, n=512)),
        "carbon": g("hopper_carbon", (0.05, 0.05, 0.055), 0.3, 0.2, maps=_carbon),
        "alu": g("hopper_aluminium", (0.8, 0.81, 0.82), 0.28, 1.0),
        "sleeve": g("hopper_sleeve", (0.46, 0.47, 0.48), 0.55, 0.3),
        "fin": g("hopper_grid_fin", (0.62, 0.63, 0.65), 0.32, 1.0),
        "pocket": g("hopper_fin_pocket", (0.16, 0.16, 0.17), 0.55, 0.2, "metal_plate", contrast=0.4),
        "copper": g("hopper_copper", (0.80, 0.48, 0.30), 0.3, 1.0),
        "inside": g("rocket_nozzle_inside", (0.035, 0.03, 0.028), 0.75, 0.4),
        "seam": g("hopper_seam", (0.4, 0.4, 0.41), 0.6, 0.0),
        "black": g("hopper_black", (0.03, 0.03, 0.03), 0.45, 0.1),
        "blue": g("hopper_lidar_blue", (0.08, 0.32, 0.85), 0.35, 0.0),
        "lens": g("hopper_lens", (0.02, 0.022, 0.026), 0.05, 0.0),
        "yellow": g("hopper_yellow", (0.98, 0.82, 0.1), 0.5, 0.0),
        "rubber": g("hopper_foot", (0.06, 0.06, 0.06), 0.85, 0.0),
    }


# Shapes


def _ogive(radius, length, blunt=0.9, steps=28):
    """A tangent-ogive nose as (radius, height above its base) from base to
    tip, the tip rounded by a sphere of the nose's own slope."""
    rho = (radius * radius + length * length) / (2.0 * radius)

    def r_at(h):
        return math.sqrt(rho * rho - h * h) + radius - rho

    ht = math.sqrt(rho * rho - (blunt + rho - radius) ** 2)
    slope = -ht / math.sqrt(rho * rho - ht * ht)
    hc = ht + blunt * slope
    rc = blunt * math.sqrt(1.0 + slope * slope)
    out = [(r_at(ht * (1.0 - (1.0 - i / steps) ** 1.4)), ht * (1.0 - (1.0 - i / steps) ** 1.4)) for i in range(steps + 1)]
    a0 = math.atan2(ht - hc, blunt)
    out += [(rc * math.cos(a), hc + rc * math.sin(a)) for a in [a0 + (math.pi / 2 - a0) * k / 6 for k in range(1, 7)]]
    out[-1] = (0.0, hc + rc)
    return out


def _cone_nose(radius, length, blunt, steps=6):
    """A straight cone as (radius, height above its base) from base to tip,
    the tip rounded by a sphere of radius `blunt` tangent to it."""
    alpha = math.atan2(radius, length)
    for _ in range(4):
        apex = length + blunt / math.sin(alpha) - blunt
        alpha = math.atan2(radius, apex)
    centre = apex - blunt / math.sin(alpha)
    out = [(radius, 0.0)]
    out += [(blunt * math.cos(a), centre + blunt * math.sin(a)) for a in [alpha + (math.pi / 2 - alpha) * k / steps for k in range(steps)]]
    return out + [(0.0, length)]


def _offset(profile, d):
    """A lathe profile pushed `d` out along its normals."""
    out = []
    for i, (r, z) in enumerate(profile):
        a = profile[max(i - 1, 0)]
        c = profile[min(i + 1, len(profile) - 1)]
        tr, tz = c[0] - a[0], c[1] - a[1]
        k = math.hypot(tr, tz) or 1.0
        out.append((max(r + d * tz / k, 0.0), z - d * tr / k))
    return out


def _pod(z0, z1, depth, nose=0.8, segments=14, start=-math.pi / 2, angle=math.pi):
    """A half-round fairing up local Z, facing +X, tapered at both ends."""
    prof = [(0.0, z0), (depth * 0.75, z0 + nose * 0.45), (depth, z0 + nose), (depth, z1 - nose), (depth * 0.75, z1 - nose * 0.45), (0.0, z1)]
    return lathe(prof, segments, 1.0, angle=angle, start=start)


def _bell(throat, exit_r, length, chamber, chamber_len):
    """A rocket engine's chamber and bell nozzle hanging from z = 0: its
    outside, from the exit up to the injector, and the inside of the bell."""
    zt = -(chamber_len + 0.25)
    ze = zt - length
    curve = [(throat + (exit_r - throat) * t ** 0.6, zt - length * t) for t in [k / 14 for k in range(15)]]
    outside = [(exit_r - 0.02, ze), (exit_r + 0.015, ze + 0.02)] + [(r + 0.015, z) for r, z in curve[::-1][1:]]
    outside += [(throat + 0.05, zt + 0.05), (chamber, zt + 0.25), (chamber, -0.15), (chamber * 0.6, 0.0), (0.0, 0.0)]
    inside = [(0.0, zt + 0.1), (throat * 0.6, zt + 0.08)] + curve[:-1] + [(exit_r - 0.02, ze)]
    return outside, inside


def _band(r, z0, z1, segments=96, wraps=1, foot=None):
    """A ring of skin from z0 to z1, its texture wrapping `wraps` times round
    it and spanning it once up from `foot` (z0 when None)."""
    foot = z0 if foot is None else foot
    bm = lathe([(r, z0), (r, z1)], segments, TAU * r / wraps)
    uv = bm.loops.layers.uv["st"]
    for face in bm.faces:
        for loop in face.loops:
            loop[uv].uv.y = (loop.vert.co.z - foot) / (z1 - foot)
    return bm


def _rounded(points, radius, steps=5):
    """A convex outline of (x, y) points with its corners rounded to `radius`."""
    out = []
    for i, p in enumerate(points):
        p = Vector(p)
        u, w = (Vector(points[i - 1]) - p).normalized(), (Vector(points[(i + 1) % len(points)]) - p).normalized()
        half = u.angle(w) / 2
        centre = p + (u + w).normalized() * (radius / math.sin(half))
        s, e = p + u * (radius / math.tan(half)) - centre, p + w * (radius / math.tan(half)) - centre
        a0 = math.atan2(s.y, s.x)
        da = (math.atan2(e.y, e.x) - a0 + math.pi) % TAU - math.pi
        out += [(centre.x + radius * math.cos(a0 + da * k / steps), centre.y + radius * math.sin(a0 + da * k / steps)) for k in range(steps + 1)]
    return out


def _wrap(outline, R, a, z, t, step=0.12):
    """A flat shape of counter-clockwise (round, up) points laid on a drum of
    radius R round angle a and height z, standing t proud of it."""
    pts = []
    for (s0, h0), (s1, h1) in zip(outline, outline[1:] + outline[:1]):
        k = max(1, math.ceil(math.hypot(s1 - s0, h1 - h0) / step))
        pts += [(s0 + (s1 - s0) * i / k, h0 + (h1 - h0) * i / k) for i in range(k)]
    sc = sum(s for s, _ in pts) / len(pts)
    hc = sum(h for _, h in pts) / len(pts)
    rings = max(1, math.ceil(max(abs(s - sc) for s, _ in pts) / 0.2))
    bm = bmesh.new()

    def vert(s, h, r):
        return bm.verts.new((r * math.cos(a + s / R), r * math.sin(a + s / R), z + h))

    mid = vert(sc, hc, R + t)
    loops = [[vert(sc + (s - sc) * f, hc + (h - hc) * f, R + t) for s, h in pts] for f in [(k + 1) / rings for k in range(rings)]]
    loops.append([vert(s, h, R - 0.01) for s, h in pts])
    n = len(pts)
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((mid, loops[0][i], loops[0][j]))
        for lo, hi in zip(loops, loops[1:]):
            bm.faces.new((hi[i], hi[j], lo[j], lo[i]))
    project_uv(bm, 0.6)
    return bm


def _flats(R, z0, z1, angles, d, tile, steps=96):
    """A drum of radius R from z0 to z1 with a flat pocket milled `d` from
    the axis round each of `angles`, walled at both ends."""
    edge = math.acos(d / R)

    def depth(t):
        off = min(abs((t - a + math.pi) % TAU - math.pi) for a in angles)
        return d / math.cos(off) if off < edge else R

    ts = sorted({round(TAU * i / steps, 9) for i in range(steps)} | {round((a + s * edge) % TAU, 9) for a in angles for s in (-1, 1)})
    cols = [(t, depth(t)) for t in ts]
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")

    def ring(z, outer):
        return [bm.verts.new(((R if outer else r) * math.cos(t), (R if outer else r) * math.sin(t), z)) for t, r in cols]

    lo, hi, rim_lo, rim_hi = ring(z0, False), ring(z1, False), ring(z0, True), ring(z1, True)

    def face(verts, sts):
        keep = [(v, st) for k, (v, st) in enumerate(zip(verts, sts)) if all((v.co - w.co).length > 1e-6 for w in verts[:k])]
        if len(keep) > 2:
            f = bm.faces.new([v for v, _ in keep])
            for loop, (_, st) in zip(f.loops, keep):
                loop[uv].uv = st

    for i in range(len(cols)):
        j = (i + 1) % len(cols)
        (ti, ri), (tj, rj) = cols[i], cols[j]
        tj = tj + TAU if j == 0 else tj
        ui, uj = ti * R / tile, tj * R / tile
        face((lo[i], lo[j], hi[j], hi[i]), ((ui, z0 / tile), (uj, z0 / tile), (uj, z1 / tile), (ui, z1 / tile)))
        if min(ri, rj) < R - 1e-6:
            wi, wj = (R - ri) / tile, (R - rj) / tile
            face((rim_lo[i], rim_lo[j], lo[j], lo[i]), ((ui, 0.0), (uj, 0.0), (uj, wj), (ui, wi)))
            face((hi[i], hi[j], rim_hi[j], rim_hi[i]), ((ui, wi), (uj, wj), (uj, 0.0), (ui, 0.0)))
    for v in bm.verts[:]:
        if not v.link_faces:
            bm.verts.remove(v)
    return bm


def _fangs(R, top, tip, angles, half, wraps, z1, steps=8, t=0.05):
    """Skirt panels under a drum's edge at `top`: a downward triangle round
    each of `angles`, `half` wide either way, its point at `tip`; textured
    as a `_band` of `wraps` spanning `tip` to z1."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    for a in angles:
        cols = []
        for i in range(-steps, steps + 1):
            th = a + half * i / steps
            low = tip + (top - tip) * abs(i) / steps
            cols.append([(bm.verts.new((r * math.cos(th), r * math.sin(th), z)), (wraps * th / TAU, (z - tip) / (z1 - tip)))
                         for r, z in ((R - t, low), (R, low), (R, top + 0.02), (R - t, top + 0.02))])
        for i in range(len(cols) - 1):
            for k in (0, 1, 3):
                corners = (cols[i][k], cols[i + 1][k], cols[i + 1][(k + 1) % 4], cols[i][(k + 1) % 4])
                f = bm.faces.new([v for v, _ in corners])
                for loop, (_, st) in zip(f.loops, corners):
                    loop[uv].uv = st
    return bm


def _clip(p, d, poly):
    """The ends of the part of the line p + t d inside a convex,
    counter-clockwise polygon, or None."""
    lo, hi = -1e9, 1e9
    for (ax, az), (bx, bz) in zip(poly, poly[1:] + poly[:1]):
        nx, nz = bz - az, ax - bx
        num = nx * (ax - p[0]) + nz * (az - p[1])
        den = nx * d[0] + nz * d[1]
        if abs(den) < 1e-12:
            if num < 0:
                return None
        elif den > 0:
            hi = min(hi, num / den)
        else:
            lo = max(lo, num / den)
    if hi - lo < 1e-6:
        return None
    return (p[0] + d[0] * lo, p[1] + d[1] * lo), (p[0] + d[0] * hi, p[1] + d[1] * hi)


def _slat(bm, a, c, thick, y0, y1):
    """Appends a thin plate from a to c in the XZ plane, `thick` across and
    from y0 to y1 deep, to bm."""
    k = math.hypot(c[0] - a[0], c[1] - a[1])
    nx, nz = -(c[1] - a[1]) / k * thick / 2, (c[0] - a[0]) / k * thick / 2
    quad = [(a[0] - nx, a[1] - nz), (c[0] - nx, c[1] - nz), (c[0] + nx, c[1] + nz), (a[0] + nx, a[1] + nz)]
    v = [bm.verts.new((x, y, z)) for y in (y0, y1) for x, z in quad]
    for f in ((0, 1, 2, 3), (7, 6, 5, 4), (0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0)):
        bm.faces.new([v[i] for i in f])


def _grid_fin(w, h, depth, chamfer, cells, y0=0.0, frame=0.07, slat=0.022):
    """An octagonal grid fin in the XZ plane, from y0 out `depth` along +Y:
    a chamfered frame round a diamond lattice `cells` across."""
    def octagon(w, h, c):
        return [(-w / 2 + c, -h / 2), (w / 2 - c, -h / 2), (w / 2, -h / 2 + c), (w / 2, h / 2 - c),
                (w / 2 - c, h / 2), (-w / 2 + c, h / 2), (-w / 2, h / 2 - c), (-w / 2, -h / 2 + c)]

    outer = octagon(w, h, chamfer)
    inner = octagon(w - 2 * frame, h - 2 * frame, chamfer - frame * (2 - math.sqrt(2)))
    bm = bmesh.new()
    rings = [[bm.verts.new((x, y, z)) for x, z in loop] for loop, y in ((outer, y0), (outer, y0 + depth), (inner, y0 + depth), (inner, y0))]
    for k in range(4):
        a, c = rings[k], rings[(k + 1) % 4]
        for i in range(8):
            bm.faces.new((a[i], a[(i + 1) % 8], c[(i + 1) % 8], c[i]))
    step = (w - 2 * frame) / cells
    reach = int((w + h) / step) + 2
    for sign in (-1, 1):
        for k in range(-reach, reach + 1):
            seg = _clip(((k + 0.5) * step, 0.0), (1.0, -sign), inner)
            if seg:
                _slat(bm, seg[0], seg[1], slat, y0 + 0.01, y0 + depth - 0.01)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    project_uv(bm, 0.5)
    return bm


def _bulkhead(R, z, off, hole, steps=48):
    """A flat bulkhead of radius R at height z cut by two round holes, `off`
    either side of its centre along Y: a ring of quads from each hole out
    to its half of the rim."""
    bm = bmesh.new()
    for e in (-1, 1):
        cy = e * off
        corners = {math.atan2(-cy, R) % TAU, math.atan2(-cy, -R) % TAU}
        phis = sorted({TAU * i / steps for i in range(steps)} | corners)
        rim, edge = [], []
        for phi in phis:
            dx, dy = math.cos(phi), math.sin(phi)
            t = -cy * dy + math.sqrt((cy * dy) ** 2 - cy * cy + R * R)
            if dy * e < -1e-9:
                t = min(t, -cy / dy)
            rim.append(bm.verts.new((hole * dx, cy + hole * dy, z)))
            edge.append(bm.verts.new((t * dx, cy + t * dy, z)))
        for i in range(len(phis)):
            j = (i + 1) % len(phis)
            bm.faces.new((rim[i], rim[j], edge[j], edge[i]))
    project_uv(bm, 1.0)
    return bm


def _bellows(r0, r1, z0, z1, pitch):
    """A corrugated sleeve about Z from z0 to z1, its ridges `pitch` apart."""
    n = max(2, 2 * int((z1 - z0) / pitch))
    prof = [(r0 if k % 2 == 0 else r1, z0 + (z1 - z0) * k / n) for k in range(n + 1)]
    return lathe([(0.0, z0)] + prof + [(0.0, z1)], 20, 0.5)


def _pad_mats(looks):
    g = looks.get
    return {
        "sintered": g("pad_sintered", (0.36, 0.24, 0.18), 0.8, 0.0, "rough_concrete", contrast=1.3),
        "scorched": g("pad_sintered_scorched", (0.16, 0.11, 0.09), 0.75, 0.0, "rough_concrete", contrast=1.4),
        "pavers": g("pad_pavers", (0.58, 0.42, 0.32), 0.85, 0.0, "hexagonal_concrete_paving", contrast=1.1),
        "pavers_burnt": g("pad_pavers_scorched", (0.36, 0.26, 0.2), 0.85, 0.0, "hexagonal_concrete_paving", contrast=1.2),
        "apron": g("pad_apron", (0.52, 0.32, 0.21), 0.95, 0.0, "rough_concrete", contrast=1.5),
        "joint": g("pad_joint", (0.08, 0.07, 0.065), 0.9, 0.0),
        "curb": g("pad_curb", (0.55, 0.40, 0.31), 0.85, 0.0, "precast_concrete_wall", contrast=0.7),
        "blocks": [g(f"pad_blocks_{k}", c, 0.9, 0.0, "rough_concrete", contrast=1.6)
                   for k, c in enumerate(((0.46, 0.27, 0.18), (0.40, 0.24, 0.17), (0.52, 0.31, 0.21)))],
        "white": g("pad_line_white", (0.86, 0.85, 0.80), 0.75, 0.0, "rough_concrete", contrast=0.45),
        "yellow": g("pad_line_yellow", (0.92, 0.66, 0.10), 0.7, 0.0, "rough_concrete", contrast=0.45),
        "amber": g("pad_light_amber", (1.0, 0.55, 0.1), 0.3, 0.0, emit=((1.0, 0.5, 0.08), 5.0)),
        "blue": g("pad_light_blue", (0.15, 0.3, 1.0), 0.3, 0.0, emit=((0.1, 0.25, 1.0), 4.0)),
        "white_light": g("pad_light_white", (1.0, 0.97, 0.9), 0.3, 0.0, emit=((1.0, 0.95, 0.85), 8.0)),
    }


def _arc_solid(profile, radius, a0, a1, steps=4, tile=2.0):
    """A closed solid swept round an arc of `radius` from angle a0 to a1;
    its profile is (height, outward offset) points."""
    path = [Vector((radius * math.cos(a), radius * math.sin(a), 0.0)) for a in [a0 + (a1 - a0) * k / steps for k in range(steps + 1)]]
    return sweep(profile, path, tile)


def _berm(radius, a0, a1, height, width, steps=18):
    """Regolith piled round an arc behind a wall: up to `height` against
    it, sloping out over `width`, slumping into the ground at both ends."""
    section = [(-0.3, -0.15), (-0.3, 1.0), (0.4, 0.98), (1.4, 0.86), (2.8, 0.6), (4.2, 0.32), (5.4, 0.12), (width, -0.15)]
    bm = bmesh.new()
    rows = []
    for k in range(steps + 1):
        t = k / steps
        a = a0 + (a1 - a0) * t
        f = min(1.0, min(t, 1.0 - t) / 0.16) ** 0.6
        c = Vector((math.cos(a), math.sin(a), 0.0))
        rows.append([bm.verts.new(c * (radius + dr * (0.55 + 0.45 * f)) + Vector((0, 0, max(z * height * f, -0.15)))) for dr, z in section])
    for lo, hi in zip(rows, rows[1:]):
        for i in range(len(section) - 1):
            bm.faces.new([lo[i], lo[i + 1], hi[i + 1], hi[i]])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    project_uv(bm, 4.0)
    return bm


def _blast_wall(b, p, radius, a0, a1, rng, courses=5, block=1.6, course_h=0.62, depth=0.8):
    """Sintered blocks stacked in running bond round an arc, each course
    set back a little and stepped in half a block at both ends."""
    for c in range(courses):
        r = radius + depth / 2 + 0.06 * c
        step = block / r
        lo, hi = a0 + c * step / 2, a1 - c * step / 2
        n = int((hi - lo) / step)
        start = (lo + hi) / 2 - n * step / 2
        for k in range(n):
            a = start + (k + 0.5) * step + rng.uniform(-0.002, 0.002)
            size = (block - 0.04, depth + rng.uniform(-0.03, 0.03), course_h - 0.02)
            at = (r * math.cos(a), r * math.sin(a), c * course_h + course_h / 2 + rng.uniform(-0.01, 0.01))
            b.put(moved(box(size, (0, 0, 0), tile=1.6), at, a + math.pi / 2 + rng.uniform(-0.012, 0.012)), p["blocks"][rng.randrange(3)], smooth=False)


def landing_pad(looks, name, radius=32.0, at=(0, 0, 0), parent=None, walls=10):
    """A landing pad on the regolith: a scorched centre of sintered slabs,
    an apron of interlocking sintered pavers crossed by grated vent
    trenches, worn paint, a precast curb with flush edge lights, floodlight
    masts, and a ring of block-faced regolith blast berms broken by lanes
    for the crawlers, lit at their edges, with approach lights down one."""
    m = mats(looks)
    p = _pad_mats(looks)
    b = Building(name, parent, at, 0.0)
    rng = random.Random(name)
    inner = radius * 0.4
    top = PAD_TOP

    # Apron of rolled regolith round the pad, and the pavers.
    b.put(lathe([(radius + 7.5, -0.12), (radius + 3.0, 0.0), (radius + 0.6, 0.03)], 128, 3.0), p["apron"], sharp=40)
    for r0, r1, skin in ((radius, inner + 5.0, "pavers"), (inner + 5.0, inner, "pavers_burnt")):
        paving = lathe([(r0, 0.07), (r1, 0.07)], 128, 4.0)
        paving.normal_update()
        project_uv(paving, 4.0)
        b.put(paving, p[skin], smooth=False)

    # Sintered slabs in rings at the centre, over dark joints.
    b.put(disc(inner + 0.05, 0.04, 96, 2.0), p["joint"], smooth=False)
    edges = [0.0, 2.4] + [2.4 + (inner - 2.4) * k / 4 for k in range(1, 5)]
    for ring, (r0, r1) in enumerate(zip(edges, edges[1:])):
        section = [(-0.05, 0.03), (top - 0.025, 0.03), (top, 0.06), (top, r1 - r0 - 0.06), (top - 0.025, r1 - r0 - 0.03), (-0.05, r1 - r0 - 0.03)]
        skin = p["scorched"] if ring < 3 else p["sintered"]
        if r0 == 0.0:
            b.put(lathe([(r1 - 0.03, -0.05), (r1 - 0.03, top - 0.025), (r1 - 0.06, top), (0.0, top)], 48, 2.0), skin, sharp=40)
            continue
        count = max(6, int(TAU * (r0 + r1) / 2 / 3.4))
        for k in range(count):
            a0 = TAU * k / count + (ring % 2) * TAU / count / 2
            a1 = a0 + TAU / count - 0.05 / r0
            b.put(_arc_solid(section, r0, a0, a1, 3, 2.0), skin, sharp=40)

    # Steel edge rings between the zones and vent trenches across the pavers.
    for r in (inner + 0.05, radius - 0.05):
        b.put(lathe([(r + 0.08, 0.06), (r + 0.08, 0.085), (r - 0.08, 0.085), (r - 0.08, 0.06)], 128, 1.0), m["steel"], sharp=40)
    for k in range(12):
        a = TAU * (k + 0.5) / 12
        mid = (inner + radius) / 2
        length = radius - inner - 0.4
        b.put(moved(box((length, 0.7, 0.04), (0, 0, 0.055)), (mid * math.cos(a), mid * math.sin(a), 0), a), p["joint"], smooth=False)
        b.put(moved(box((length - 0.1, 0.5, 0.02), (0, 0, 0.072), tile=1.0), (mid * math.cos(a), mid * math.sin(a), 0), a), m["grate"], smooth=False)

    # Worn paint: touchdown ring, aiming bars, a dashed outer ring and the edge line.
    b.put(disc(radius * 0.28 + 0.35, top + 0.003, 128, 2.0, radius * 0.28 - 0.35), p["yellow"], smooth=False)
    for k in range(4):
        a = TAU * k / 4
        b.put(moved(box((radius * 0.1, 0.6, 0.004), (radius * 0.12, 0, top + 0.003)), (0, 0, 0), a), p["white"], smooth=False)
    dashes = 36
    for k in range(dashes):
        a = TAU * k / dashes
        b.put(lathe([(radius * 0.86 + 0.3, 0.075), (radius * 0.86 - 0.3, 0.075)], 6, 2.0, angle=TAU / dashes * 0.55, start=a), p["white"], smooth=False)
    b.put(disc(radius - 0.5, 0.074, 128, 2.0, radius - 0.85), p["white"], smooth=False)

    # Precast curb with flush amber edge lights.
    segments = int(TAU * radius / 2.4)
    curb = [(-0.05, 0.0), (0.13, 0.0), (0.16, 0.05), (0.16, 0.55), (0.12, 0.6), (-0.05, 0.6)]
    for k in range(segments):
        a0 = TAU * k / segments
        b.put(_arc_solid(curb, radius, a0, a0 + TAU / segments - 0.03 / radius, 2, 1.2), p["curb"], sharp=40)
    for k in range(segments // 2):
        a = TAU * (k + 0.25) / (segments // 2)
        c = (radius + 0.3) * Vector((math.cos(a), math.sin(a), 0.0))
        b.put(moved(lathe([(0.0, 0.155), (0.15, 0.155), (0.15, 0.18), (0.0, 0.18)], 12, 1.0), tuple(c)), m["dark"], smooth=False)
        b.put(moved(lathe([(0.0, 0.18), (0.09, 0.18), (0.06, 0.21), (0.0, 0.215)], 10, 1.0), tuple(c)), p["amber"])

    # Blast berms: block walls faced toward the pad, regolith piled behind,
    # broken by lanes whose edges carry blue lights.
    wall_r = radius + 9.0
    lane_w = 10.0
    lane = lane_w / wall_r
    for k in range(walls):
        a0 = TAU * k / walls + lane / 2
        a1 = TAU * (k + 1) / walls - lane / 2
        _blast_wall(b, p, wall_r, a0, a1, rng)
        b.put(_berm(wall_r + 0.75, a0 - 0.6 / wall_r, a1 + 0.6 / wall_r, 3.0, 7.5), m["regolith"], sharp=50)
        mid = TAU * k / walls
        for e in (-1, 1):
            side = Vector((-math.sin(mid), math.cos(mid), 0.0)) * (e * (lane_w / 2 - 0.8))
            for d in range(4):
                c = Vector((math.cos(mid), math.sin(mid), 0.0)) * (radius + 2.0 + d * 3.5) + side
                b.put(moved(lathe([(0.0, 0.0), (0.12, 0.0), (0.1, 0.5), (0.0, 0.5)], 10, 1.0), tuple(c)), m["dark"], sharp=40)
                b.put(moved(lathe([(0.0, 0.5), (0.09, 0.5), (0.07, 0.62), (0.0, 0.64)], 10, 1.0), tuple(c)), p["blue"])

    # Approach lights out along the first lane: bars of three on frangible posts.
    for d in range(9):
        x = wall_r + 3.0 + d * 3.0
        b.put(rod((x, 0, 0), (x, 0, 0.9), 0.04, 8), m["yellow"])
        b.put(box((0.1, 1.8, 0.08), (x, 0, 0.92)), m["steel"], smooth=False)
        for y in (-0.75, 0.0, 0.75):
            b.put(box((0.16, 0.22, 0.14), (x, y, 1.02)), m["dark"], smooth=False)
            b.put(box((0.17, 0.16, 0.02), (x, y, 1.1)), p["white_light"], smooth=False)
        b.put(box((0.6, 0.6, 0.12), (x, 0, 0.06)), m["concrete"], smooth=False)

    # Floodlight masts inside four of the walls, each with its power cabinet.
    for i in range(4):
        a = TAU * (round(i * walls / 4) + 0.5) / walls
        c = Vector((math.cos(a), math.sin(a), 0)) * (radius + 4.0)
        light_mast(b, m, tuple(c), 12.0, 3, a + math.pi)
        cab = c + Vector((math.cos(a), math.sin(a), 0)) * 0.9
        b.put(moved(box((0.5, 0.9, 1.3), (0, 0, 0.65), tile=1.0), tuple(cab), a), m["hull_grey"], smooth=False)
        b.put(moved(box((0.04, 0.8, 1.1), (0.26, 0, 0.7), tile=1.0), tuple(cab), a), m["frame"], smooth=False)

    for k in range(walls):
        a0 = TAU * k / walls + lane / 2
        a1 = TAU * (k + 1) / walls - lane / 2
        b.collider(_arc_solid([(-0.1, 0.0), (3.1, 0.0), (3.1, 1.6), (-0.1, 8.0)], wall_r, a0, a1, 2, 4.0))
    return b.finish()


def ariane6(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """An Ariane 6 in its four-booster A64 form, standing on its launch
    table (`launch_mount`) with the mast toward -X: the white core with its
    cable tunnel, raceway, feed lines and aft heat shield over the Vulcain
    2.1; four P120C boosters in an X on fore and aft attachments, each with
    a raceway and black tracking squares; the upper stage under its band of
    member-state flags with the umbilical plates; and the long fairing's
    two half-shells over a grey joint ring."""
    a = _ariane_mats(looks)
    b = Building(name, parent, at, yaw)
    R, Rb = A6_R, A6_BOOSTER_R
    z0 = TABLE_TOP - A6_FOOT
    core, upper, fair, ogive, tip = (z0 + h for h in (A6_CORE, A6_UPPER, A6_FAIRING, A6_OGIVE, A6_TIP))
    tile = TAU * R
    nose = [(r, ogive + h) for r, h in _ogive(R, A6_TIP - A6_OGIVE, 0.4)]

    # Skins: core, upper stage, fairing; the core's aft heat shield.
    b.put(lathe([(R, core), (R, upper)], 144, tile), a["core"], sharp=40)
    b.put(lathe([(R, upper), (R, fair)], 144, tile), a["upper"], sharp=40)
    b.put(lathe([(R, fair), (R, ogive)] + nose[1:], 144, tile), a["fairing"], sharp=40)
    b.put(lathe([(0.63, core + 0.05), (R - 0.01, core + 0.05)], 96, 2.0), a["shield"], smooth=False)

    # Joints between the stages' sections, the fairing's grey ring and
    # half-shell seams, the band of flags.
    for z in (core + 2.6, core + 20.8, core + 26.4, upper, upper + 3.0):
        b.put(lathe([(R + 0.006, z - 0.03), (R + 0.006, z + 0.03)], 144, 1.0), a["seam"])
    b.put(lathe([(R + 0.002, fair - 0.25), (R + 0.025, fair - 0.22), (R + 0.025, fair + 0.18), (R + 0.002, fair + 0.21)], 144, 1.0), a["grey"])
    shell = _offset([(R, fair + 0.21), (R, ogive)] + nose[1:-2], 0.008)
    for s in (math.pi / 2, -math.pi / 2):
        b.put(lathe(shell, 1, 1.0, angle=0.035 / R, start=s - 0.0175 / R), a["seam"], smooth=False)
    b.put(_band(R + 0.004, z0 + 35.4, z0 + 37.4, 144), a["flags"], sharp=40)

    # Cable tunnel up the core and upper stage, a raceway, the feed lines and their clamps.
    for ang, z1, depth in ((math.pi - 0.45, fair - 0.4, 0.24), (math.pi / 2, upper - 0.5, 0.13)):
        b.put(moved(_pod(core + 0.3, z1, depth, 0.6, 10), ((R - 0.05) * math.cos(ang), (R - 0.05) * math.sin(ang), 0), ang), a["white"], sharp=40)
    clamps = bmesh.new()
    for y, r in ((-0.36, 0.18), (0.32, 0.11)):
        x = R + 0.16 + r
        b.put(tube([(R - 0.05, y, core + 0.7), (x, y, core + 1.5), (x, y, core + 19.4), (R - 0.05, y, core + 20.2)], r, 16), a["white"])
        for k in range(5):
            merge(clamps, box((x - R + 0.05, 0.1, 0.3), ((R + x) / 2, y, core + 3.0 + 4.0 * k)))
            merge(clamps, moved(lathe([(r + 0.02, -0.1), (r + 0.02, 0.1)], 16, 1.0), (x, y, core + 3.0 + 4.0 * k)))
    b.put(clamps, a["white"])

    # Umbilical plates: the tail one over the fuelling unit, two under the mast's arms.
    for z, w, h in ((z0 + 3.2, 1.0, 0.7), (z0 + 33.0, 1.1, 0.9), (z0 + 39.5, 1.1, 0.9)):
        b.put(_wrap(rect(w, h), R, math.pi, z, 0.06), a["grey"], smooth=False)
        for dy in (-0.25, 0.25):
            b.put(moved(lathe([(0.0, 0.0), (0.12, 0.0), (0.12, 0.08), (0.0, 0.08)], 12, 1.0), (-(R + 0.05), dy, z), 0.0, -math.pi / 2), a["black"])

    # The Vulcain 2.1, and its boot through the heat shield.
    outside, inside = _bell(0.15, 1.15, 2.6, 0.34, 0.5)
    b.put(moved(lathe(outside, 48, TAU * 1.15), (0, 0, z0 + 3.35)), a["nozzle"], sharp=35)
    b.put(moved(lathe(inside, 48, 2.0), (0, 0, z0 + 3.35)), a["inside"], sharp=50)
    b.put(lathe([(0.48, core + 0.02), (0.64, core + 0.06)], 48, 1.0), a["boot"], smooth=False)

    # Boosters: skin and cone, skirt and aft dome, nozzle, raceway, markers, attachments.
    zb, zc, zt = z0 + A6_FOOT, z0 + A6_CONE, z0 + A6_NOSE
    btile = TAU * Rb
    for k in range(4):
        th = math.pi / 4 + k * math.pi / 2
        e = Vector((math.cos(th), math.sin(th), 0))
        s = Vector((-e.y, e.x, 0))
        c = tuple(e * A6_BOOSTER_AT)
        b.put(moved(lathe([(Rb, zb), (Rb, zc)] + [(r, zc + h) for r, h in _cone_nose(Rb, zt - zc, 0.22)[1:]], 72, btile), c), a["booster"], sharp=40)
        for z in (zb + 2.0, zc):
            b.put(moved(lathe([(Rb + 0.006, z - 0.03), (Rb + 0.006, z + 0.03)], 72, 1.0), c), a["seam"])
        b.put(moved(lathe([(1.05, zb + 1.75), (Rb - 0.03, zb + 1.6), (Rb - 0.03, zb), (Rb, zb)], 72, 2.0), c), a["shield"], sharp=40)
        b.put(moved(lathe([(1.1, zb - 0.32), (1.14, zb - 0.29), (1.06, zb + 0.3), (0.86, zb + 1.1), (0.72, zb + 1.8)], 48, 2.0), c), a["ablative"], sharp=40)
        b.put(moved(lathe([(0.0, zb + 1.2), (0.45, zb + 1.15), (0.75, zb + 0.6), (0.98, zb + 0.0), (1.09, zb - 0.3)], 48, 2.0), c), a["inside"], sharp=50)
        b.put(moved(lathe([(0.72, zb + 1.8), (1.05, zb + 1.75)], 48, 1.0), c), a["boot"], smooth=False)
        b.put(moved(_pod(zb + 0.4, zc - 0.3, 0.11, 0.5, 8), tuple(e * (A6_BOOSTER_AT + Rb - 0.03)), th), a["white"], sharp=40)
        for z in (zc - 0.85, zb + 2.8):
            for da in (-2.2, -0.9, 0.9, 2.2):
                b.put(moved(_wrap(rect(0.6, 0.6), Rb, th + da, z, 0.004), c), a["black"], smooth=False)
        fwd = e * (R + 0.1) + Vector((0, 0, zc - 0.4))
        b.put(moved(box((0.5, 0.7, 0.9), bevel=0.04), tuple(fwd), th), a["white"], smooth=False)
        for side in (-1, 1):
            p0 = e * (A6_BOOSTER_AT - Rb + 0.02) + s * (side * 0.45) + Vector((0, 0, zb + 2.5))
            p1 = e * (R - 0.02) + s * (side * 0.95) + Vector((0, 0, zb + 2.9))
            b.put(rod(p0, p1, 0.07, 8), a["metal"])

    b.collider(lathe([(0.0, core), (R, core), (R, ogive), (R * 0.72, ogive + 4.5), (0.0, tip)], 12, 4.0))
    for k in range(4):
        th = math.pi / 4 + k * math.pi / 2
        c = (A6_BOOSTER_AT * math.cos(th), A6_BOOSTER_AT * math.sin(th), 0.0)
        b.collider(moved(lathe([(0.0, zb), (Rb, zb), (Rb, zc), (0.0, zt)], 8, 4.0), c))
    return b.finish()


def launch_mount(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """What an Ariane 6 stands on (`ariane6`): a raised steel launch table
    on box columns over a two-way sintered flame deflector, its deck cut
    for the Vulcain and the four boosters, a black lattice frame clasping
    each booster's skirt, and a squat fuelling unit by the core's tail; at
    -X the umbilical mast, a steel tower clad on three sides with a floor
    every bay, a lift, railings on its open face, two yellow arms reaching
    the upper stage's plates, and a lightning rod over it all."""
    m = mats(looks)
    q = industry.plant_mats(looks)
    p = _pad_mats(looks)
    t = _mount_mats(looks)
    b = Building(name, parent, at, yaw)
    H, top, under = TABLE_HALF, TABLE_TOP, TABLE_UNDER
    off = A6_BOOSTER_AT / math.sqrt(2)
    hole, well = 1.3, 1.5
    z0 = TABLE_TOP - A6_FOOT

    # Deck: steel cells round the Vulcain's well and the boosters' holes,
    # sooted liners in them, stiffeners round its edge.
    cuts = [-H, -off - hole, -off + hole, -well, well, off - hole, off + hole, H]
    deck, liners, ribs = bmesh.new(), bmesh.new(), bmesh.new()
    for x0, x1 in zip(cuts, cuts[1:]):
        for y0, y1 in zip(cuts, cuts[1:]):
            cx, cy = abs(x0 + x1) / 2, abs(y0 + y1) / 2
            if (cx < well and cy < well) or (off - hole < cx < off + hole and off - hole < cy < off + hole):
                continue
            merge(deck, box((x1 - x0, y1 - y0, top - under), ((x0 + x1) / 2, (y0 + y1) / 2, (top + under) / 2)))
    for cx, cy, half in [(0.0, 0.0, well)] + [(sx * off, sy * off, hole) for sx in (-1, 1) for sy in (-1, 1)]:
        for k in range(4):
            merge(liners, moved(box((2 * half - 0.04, 0.03, top - under - 0.02), (0, half - 0.03, 0)), (cx, cy, (top + under) / 2), k * math.pi / 2))
    for k in range(4):
        for j in range(int(2 * H)):
            y = -H + 0.5 + j
            if abs(y) > 0.7:
                merge(ribs, moved(box((0.16, 0.05, top - under - 0.12), (H + 0.08, y, (top + under) / 2)), (0, 0, 0), k * math.pi / 2))
        merge(ribs, moved(box((0.2, 2 * H + 0.4, 0.12), (H + 0.1, 0, top - 0.06)), (0, 0, 0), k * math.pi / 2))
        merge(ribs, moved(box((0.02, 2 * H, top - under - 0.12), (H + 0.01, 0, (top + under - 0.12) / 2)), (0, 0, 0), k * math.pi / 2))
    b.put(deck, t["deck"], smooth=False)
    b.put(liners, t["soot"], smooth=False)
    b.put(ribs, t["steel"], smooth=False)

    # Columns on footings, edge girders under the deck, the flame deflector between.
    cols = [(sx * (H - 1.1), sy * (H - 1.1)) for sx in (-1, 1) for sy in (-1, 0, 1)]
    steel = bmesh.new()
    for x, y in cols:
        merge(steel, box((1.6, 1.6, under - 0.25), (x, y, (under + 0.25) / 2)))
        b.put(box((2.2, 2.2, 0.25), (x, y, 0.125)), m["cast"], smooth=False)
    for k in range(4):
        merge(steel, moved(box((0.8, 2 * H, 1.0), (H - 0.4, 0, under - 0.5)), (0, 0, 0), k * math.pi / 2))
    b.put(steel, t["steel"], smooth=False)
    ridge = [(y, 0.1 + 3.0 * (1.0 - abs(y) / 7.8) ** 2) for y in np.linspace(7.8, -7.8, 27)]
    defl = prism([(-7.8, 0.0), (7.8, 0.0)] + ridge, -(H - 1.9), H - 1.9)
    defl.transform(Matrix(((0, 0, 1, 0), (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 0, 1))))
    project_uv(defl, 3.0)
    b.put(defl, p["scorched"], sharp=30)

    # Booster frames: black lattice round each skirt's outer side, hold-down arms to it.
    lattice = bmesh.new()
    for k in range(4):
        th = math.pi / 4 + k * math.pi / 2
        e, s = Vector((math.cos(th), math.sin(th), 0)), Vector((-math.sin(th), math.cos(th), 0))
        c = e * A6_BOOSTER_AT + Vector((0, 0, top))
        posts = [c + e * de + s * ds for de, ds in ((-0.9, -2.1), (2.1, -2.1), (2.1, 2.1), (-0.9, 2.1))]
        for pt in posts:
            merge(lattice, moved(box((0.3, 0.3, 2.6), (0, 0, 1.3)), tuple(pt), th))
            foot = c + (pt - c).normalized() * (A6_BOOSTER_R + 0.12) + Vector((0, 0, 1.5))
            merge(lattice, industry.member(pt + Vector((0, 0, 2.35)), foot, rect(0.24, 0.2)))
            merge(lattice, moved(box((0.22, 0.4, 0.5)), tuple(foot), math.atan2(*(pt - c).yx)))
        for pa, pc in zip(posts, posts[1:]):
            for h in (0.15, 1.3, 2.45):
                merge(lattice, industry.member(pa + Vector((0, 0, h)), pc + Vector((0, 0, h)), rect(0.2, 0.16)))
            for z_lo, z_hi in ((0.15, 1.3), (1.3, 2.45)):
                merge(lattice, rod(pa + Vector((0, 0, z_lo)), pc + Vector((0, 0, z_hi)), 0.05, 6))
                merge(lattice, rod(pc + Vector((0, 0, z_lo)), pa + Vector((0, 0, z_hi)), 0.05, 6))
    b.put(lattice, t["lattice"], sharp=30)

    # The fuelling unit at the core's tail, its louvres and hoses to the tail plate.
    fu = Vector((-4.9, 0.0, top))
    b.put(box((2.4, 1.6, 1.9), tuple(fu + Vector((0, 0, 0.95))), bevel=0.04), q["paint_grey"], smooth=False)
    for dy in (-0.45, 0.45):
        b.put(box((1.6, 0.02, 1.1), tuple(fu + Vector((-0.2, dy * 1.8, 0.9)))), q["louvre"], smooth=False)
        b.put(tube([fu + Vector((1.2, dy * 0.6, 1.5)), fu + Vector((1.6, dy * 0.6, 2.1)), Vector((-(A6_R + 0.1), dy * 0.55, z0 + 3.2))], 0.09, 12), q["jacket"])

    # The mast: corner columns, a ring of beams and a grating floor every
    # bay, cladding and trims on three sides, bracing on the open face.
    x0, x1, hw = MAST_X - MAST_HALF, MAST_X + MAST_HALF, MAST_HALF
    bay = 5.0
    arms = (z0 + 33.0, z0 + 39.5)
    frame, braces, floors, rails, clad = bmesh.new(), bmesh.new(), bmesh.new(), bmesh.new(), bmesh.new()
    for cx in (x0 + 0.3, x1 - 0.3):
        for cy in (-hw + 0.3, hw - 0.3):
            merge(frame, box((0.6, 0.6, MAST_TOP), (cx, cy, MAST_TOP / 2)))
    for k in range(1, int(MAST_TOP / bay) + 1):
        z = k * bay
        for cx in (x0 + 0.3, x1 - 0.3):
            merge(frame, box((0.45, 2 * hw, 0.55), (cx, 0, z - 0.275)))
        for cy in (-hw + 0.3, hw - 0.3):
            merge(frame, box((2 * hw, 0.45, 0.55), (MAST_X, cy, z - 0.275)))
        merge(frame, box((0.16, 2 * hw + 0.3, 0.3), (x0 - 0.13, 0, z - 0.3)))
        for e in (-1, 1):
            merge(frame, box((2 * hw + 0.3, 0.16, 0.3), (MAST_X, e * (hw + 0.13), z - 0.3)))
        merge(floors, box((2 * hw - 0.7, 2 * hw - 0.7, 0.06), (MAST_X, 0, z - 0.03), tile=1.0))
        if not any(z - bay < arm + 1.4 and arm - 1.4 < z for arm in arms):
            for e in (-1, 1):
                merge(braces, rod((x1 - 0.3, -e * (hw - 0.4), z - bay + 0.3), (x1 - 0.3, e * (hw - 0.4), z - 0.6), 0.08, 8))
        if k < MAST_TOP / bay:
            for y in np.linspace(-hw + 0.4, hw - 0.4, 5):
                merge(rails, rod((x1 + 0.05, y, z), (x1 + 0.05, y, z + 1.1), 0.03, 6))
            for zz in (0.55, 1.1):
                merge(rails, rod((x1 + 0.05, -hw + 0.4, z + zz), (x1 + 0.05, hw - 0.4, z + zz), 0.025, 6))
    merge(clad, box((0.1, 2 * hw + 0.2, MAST_TOP - 2.6), (x0 - 0.05, 0, (MAST_TOP + 2.6) / 2), tile=5.0))
    for e in (-1, 1):
        merge(clad, box((2 * hw, 0.1, MAST_TOP - 2.6), (MAST_X, e * (hw + 0.05), (MAST_TOP + 2.6) / 2), tile=5.0))
    for dx in (-0.9, 0.9):
        for dy in (-1.0, 1.0):
            merge(frame, box((0.15, 0.15, MAST_TOP), (x0 + 1.7 + dx, dy, MAST_TOP / 2)))
    b.put(frame, q["paint_grey"], smooth=False)
    b.put(braces, q["paint_grey"])
    b.put(floors, m["grate"], smooth=False)
    b.put(rails, q["paint_yellow"])
    b.put(clad, t["clad"], smooth=False)
    b.put(box((1.6, 1.8, 2.5), (x0 + 1.7, 0, arms[0] - 1.25)), q["paint_yellow"], smooth=False)
    b.put(box((2 * hw + 1.0, 2 * hw + 1.0, 0.3), (MAST_X, 0, 0.15)), m["cast"], smooth=False)
    b.put(box((0.08, 1.4, 2.3), (x0 - 0.12, 0, 1.45)), m["hull_grey"], smooth=False)

    # The arms: yellow trusses on hinge housings, carriers on the plates, hoses along them.
    for z in arms:
        root, tip = Vector((x1 + 0.4, 0, z)), Vector((-(A6_R + 0.42), 0, z))
        b.put(truss(root, tip, 1.3, 1.4, 6, 0.08, 0.045), q["paint_yellow"])
        b.put(box((0.6, 1.9, 2.2), (x1 + 0.3, 0, z), bevel=0.03), q["paint_yellow"], smooth=False)
        b.put(box((0.36, 1.2, 1.0), (-(A6_R + 0.24), 0, z), bevel=0.02), q["paint_grey"], smooth=False)
        for dy in (-0.3, 0.3):
            b.put(tube([root + Vector((0.2, dy, -0.75)), root.lerp(tip, 0.5) + Vector((0, dy, -1.05)), tip + Vector((0.05, dy, -0.5)), tip + Vector((0.18, dy * 0.6, -0.15))], 0.08, 12), q["jacket"])

    # Propellant lines up the open face into the arms, a cable tray beside them.
    for z, ys in ((arms[0], (-1.95, -1.7)), (arms[1], (-1.4, -1.15))):
        for y, e in zip(ys, (-1, 1)):
            b.put(tube([(x1 + 0.35, y, 0.3), (x1 + 0.35, y, z - 1.6), (x1 + 0.6, 0.3 * e, z - 0.75)], 0.08, 12), q["jacket"])
    b.put(box((0.12, 0.45, arms[1] - 0.5), (x1 + 0.3, 1.7, arms[1] / 2)), q["paint_dark"], smooth=False)

    # The roof, its obstruction lights and the lightning rod.
    b.put(box((2 * hw + 0.3, 2 * hw + 0.3, 0.3), (MAST_X, 0, MAST_TOP + 0.15)), q["paint_grey"], smooth=False)
    b.put(cone(0.35, 0.05, 22.0, (MAST_X, 0, MAST_TOP + 0.3), 12), q["galv"], sharp=30)
    for cx in (x0, x1):
        for cy in (-hw, hw):
            b.put(sphere(0.15, (cx, cy, MAST_TOP + 0.45), 10, 5), m["red"])

    b.collider(box((2 * H, 2 * H, top - under), (0, 0, (top + under) / 2)))
    for x, y in cols:
        b.collider(box((1.6, 1.6, under), (x, y, under / 2)))
    solid = prism([(-7.8, 0.0), (7.8, 0.0), (0.0, 3.1)], -(H - 1.9), H - 1.9)
    solid.transform(Matrix(((0, 0, 1, 0), (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 0, 1))))
    b.collider(solid)
    b.collider(box((2.4, 1.6, 1.9), tuple(fu + Vector((0, 0, 0.95)))))
    b.collider(box((2 * hw + 0.2, 2 * hw + 0.2, MAST_TOP), (MAST_X, 0, MAST_TOP / 2)))
    for z in arms:
        b.collider(box((x1 + A6_R + 0.6, 1.4, 1.4), ((x1 - A6_R) / 2, 0, z)))
    return b.finish()


def honda_hopper(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """Honda's reusable test rocket at four times its size, the base's
    hopper, on its legs with a gap between two of them toward +X: a white
    body under an ogive nose, four octagonal grid fins stowed in flat
    pockets under it with LiDARs and a GPS antenna beside them, black carbon
    strips and rounded-triangle hatches below, ring seams, an aft skirt cut
    into four white fangs over the black carbon engine bay and two recessed
    copper-rimmed nozzles, and four telescoping aluminium legs in corrugated
    sleeves on carbon braces and round feet."""
    h = _hopper_mats(looks)
    b = Building(name, parent, at, yaw)
    R = HOP_R
    tile = TAU * R / 3
    fins = [math.pi / 4 + k * math.pi / 2 for k in range(4)]
    gaps = [k * math.pi / 2 for k in range(4)]
    flat, zp0, zp1 = 1.38, 19.4, 21.9
    zf = (zp0 + zp1) / 2 - 0.1
    soot = HOP_SKIRT + 3.0
    half = math.radians(28)

    # Skin: the sooted aft barrel and fangs, the barrel, the fin pockets, the nose, ring seams.
    b.put(_band(R, HOP_SKIRT, soot, 96, 3, HOP_FANG), h["sooted"], sharp=40)
    b.put(_fangs(R, HOP_SKIRT, HOP_FANG, fins, half, 3, soot), h["sooted"], sharp=40)
    b.put(lathe([(R, soot), (R, zp0)], 96, tile), h["white"], sharp=40)
    b.put(_flats(R, zp0, zp1, fins, flat, tile), h["white"], sharp=40)
    b.put(lathe([(R, zp1), (R, HOP_NOSE)] + [(r, HOP_NOSE + z) for r, z in _ogive(R, HOP_TIP - HOP_NOSE, 0.25)][1:], 96, tile), h["white"], sharp=40)
    for z in (8.8, 15.8, zp0 - 0.25, HOP_NOSE):
        b.put(lathe([(R + 0.004, z - 0.02), (R + 0.004, z + 0.02)], 96, 1.0), h["seam"])

    # Engine bay: carbon barrel, stringers and frame between the fangs, the
    # sooted bulkhead, two nozzles with copper rims.
    bay = bmesh.new()
    merge(bay, lathe([(1.56, 1.95), (1.56, HOP_SKIRT + 0.2)], 64, TAU * 1.56 / 16))
    merge(bay, lathe([(1.62, 2.0), (1.62, 2.16)], 64, TAU * 1.62 / 16))
    for g in gaps:
        for da in (-0.16, 0.16):
            merge(bay, moved(box((0.12, 0.1, HOP_SKIRT - 1.9)), (1.6 * math.cos(g + da), 1.6 * math.sin(g + da), (HOP_SKIRT + 1.9) / 2), g + da))
    b.put(bay, h["carbon"], sharp=30)
    b.put(_bulkhead(1.56, 2.3, 0.72, 0.4), h["inside"], smooth=False)
    outside, inside = _bell(0.12, 0.5, 1.0, 0.2, 0.3)
    for y in (-0.72, 0.72):
        b.put(moved(lathe(outside, 32, 1.0), (0, y, 3.45)), h["black"], sharp=35)
        b.put(moved(lathe(inside, 32, 1.0), (0, y, 3.45)), h["inside"], sharp=50)
        b.put(moved(industry.torus(0.5, 0.04, 32, 8), (0, y, 1.92)), h["copper"])

    # Grid fins stowed in their dark pockets, hinge brackets and pins over them.
    for a in fins:
        floor = box((2.0 * math.sqrt(R * R - flat * flat) - 0.03, 0.01, zp1 - zp0 - 0.02), (0, flat + 0.005, (zp0 + zp1) / 2), tile=0.6)
        b.put(moved(floor, (0, 0, 0), a - math.pi / 2), h["pocket"], smooth=False)
        fin = _grid_fin(1.8, 2.0, 0.4, 0.42, 7, flat)
        merge(fin, box((0.7, 0.32, 0.24), (0, flat + 0.16, 1.13)))
        merge(fin, rod((-0.42, flat + 0.3, 1.02), (0.42, flat + 0.3, 1.02), 0.05, 10))
        b.put(moved(fin, (0, 0, zf), a - math.pi / 2), h["fin"], sharp=30)

    # Sensors on the ridges between the pockets: two LiDAR pucks, a cube LiDAR, a GPS antenna.
    for g, z, kind in ((gaps[0], 21.0, "puck"), (gaps[1], 19.0, "cube"), (gaps[2], 21.4, "gps"), (gaps[3], 20.4, "puck")):
        c = Vector((math.cos(g), math.sin(g), 0))
        b.put(moved(box((0.5, 0.18, 0.12), (R + 0.2, 0, z - 0.06)), (0, 0, 0), g), h["black"], smooth=False)
        p = c * (R + 0.48) + Vector((0, 0, z))
        if kind == "puck":
            b.put(moved(lathe([(0.0, 0.0), (0.21, 0.0), (0.21, 0.32), (0.0, 0.32)], 24, 1.0), tuple(p)), h["black"], sharp=30)
            b.put(moved(lathe([(0.215, 0.07), (0.215, 0.19)], 24, 1.0), tuple(p)), h["lens"])
            b.put(moved(lathe([(0.215, 0.23), (0.215, 0.28)], 24, 1.0), tuple(p)), h["blue"])
        elif kind == "cube":
            b.put(moved(box((0.42, 0.42, 0.42), (0, 0, 0.21), bevel=0.02), tuple(p), g), h["black"], smooth=False)
            b.put(moved(box((0.02, 0.3, 0.2), (0.215, 0, 0.24)), tuple(p), g), h["lens"], smooth=False)
        else:
            b.put(rod(p, p + Vector((0, 0, 0.25)), 0.03, 8), h["alu"])
            b.put(sphere(0.16, tuple(p + Vector((0, 0, 0.25))), 16, 6, low=0.0), h["trim"])

    # Carbon strips, hatches with their gaps, and yellow marks under the fins.
    for g in gaps:
        b.put(_wrap(rect(0.55, 6.8), R, g, 15.6, 0.006), h["carbon"], smooth=False)
    hatch = [(-0.8, 0.5), (0.0, -0.8), (0.8, 0.5)]
    for a in fins:
        b.put(_wrap(_rounded([(1.07 * x, 1.07 * y) for x, y in hatch], 0.25), R, a, 17.9, 0.006), h["seam"], smooth=False)
        b.put(_wrap(_rounded(hatch, 0.22), R, a, 17.9, 0.012), h["trim"], smooth=False)
        for e in (-1, 1):
            b.put(_wrap(rect(0.36, 0.36), R, a + e * 0.62 / R, 18.95, 0.005), h["yellow"], smooth=False)

    # Aft hatches and ports.
    for g in (gaps[1], gaps[3]):
        b.put(_wrap(rect(0.66, 0.5), R, g, 4.8, 0.006), h["seam"], smooth=False)
        b.put(_wrap(rect(0.6, 0.44), R, g, 4.8, 0.012), h["trim"], smooth=False)
    for a in fins:
        for e in (-1, 1):
            c = Vector((math.cos(a + e * 0.42), math.sin(a + e * 0.42), 0))
            b.put(moved(lathe([(0.0, 0.0), (0.15, 0.0), (0.15, 0.03), (0.09, 0.035), (0.0, 0.01)], 16, 1.0), tuple(c * (R - 0.005) + Vector((0, 0, 3.9))), a + e * 0.42, math.pi / 2), h["black"], smooth=False)

    # Legs: hinge fittings under fairings, telescoping struts in corrugated
    # sleeves, carbon braces from the fangs, ball joints and round feet.
    for a in fins:
        c = Vector((math.cos(a), math.sin(a), 0))
        side = Vector((-c.y, c.x, 0))
        hinge = c * (R + 0.32) + Vector((0, 0, 7.1))
        foot = c * HOP_FOOT + Vector((0, 0, PAD_TOP + 0.45))
        axis = foot - hinge
        L = axis.length
        b.put(moved(_pod(6.2, 8.4, 0.3, 0.6, 10), tuple(c * (R - 0.05)), a), h["trim"], sharp=40)
        b.put(moved(box((0.5, 0.7, 0.55)), tuple(c * (R + 0.2) + Vector((0, 0, 7.1))), a), h["fin"], smooth=False)
        b.put(rod(hinge - side * 0.42, hinge + side * 0.42, 0.08, 10), h["alu"])
        b.put(industry.aim(lathe([(0.0, 0.0), (0.2, 0.0), (0.2, L * 0.58), (0.0, L * 0.58)], 20, 1.0), hinge, axis), h["alu"])
        b.put(industry.aim(_bellows(0.25, 0.28, L * 0.04, L * 0.5, 0.08), hinge, axis), h["sleeve"], sharp=60)
        b.put(industry.aim(lathe([(0.2, L * 0.55), (0.24, L * 0.56), (0.24, L * 0.6), (0.14, L * 0.61)], 20, 1.0), hinge, axis), h["alu"])
        b.put(industry.aim(lathe([(0.0, L * 0.5), (0.14, L * 0.5), (0.14, L), (0.0, L)], 20, 1.0), hinge, axis), h["alu"])
        knee = hinge.lerp(foot, 0.84)
        for e in (-1, 1):
            base = Vector((math.cos(a + e * 0.33), math.sin(a + e * 0.33), 0)) * (R + 0.04) + Vector((0, 0, 2.62))
            b.put(rod(base, knee, 0.075, 10), h["carbon"])
            b.put(sphere(0.11, tuple(base), 10, 5), h["alu"])
        b.put(sphere(0.17, tuple(knee), 12, 6), h["alu"])
        b.put(sphere(0.2, tuple(foot), 14, 7), h["alu"])
        b.put(moved(lathe([(0.0, 0.0), (0.6, 0.0), (0.62, 0.06), (0.5, 0.2), (0.22, 0.32), (0.0, 0.34)], 28, 1.0), (foot.x, foot.y, PAD_TOP)), h["alu"], sharp=40)
        b.put(moved(lathe([(0.0, -0.002), (0.61, -0.002), (0.61, 0.02), (0.0, 0.02)], 28, 1.0), (foot.x, foot.y, PAD_TOP)), h["rubber"], smooth=False)

    b.collider(lathe([(0.0, HOP_FANG), (R, HOP_FANG), (R, HOP_NOSE), (R * 0.55, HOP_NOSE + 2.0), (0.0, HOP_TIP)], 10, 4.0))
    for a in fins:
        foot = Vector((math.cos(a), math.sin(a), 0)) * HOP_FOOT
        b.collider(box((1.2, 1.2, 0.5), (foot.x, foot.y, PAD_TOP + 0.25)))
    return b.finish()


def pad_services(looks, name, at=(0, 0, 0), yaw=0.0, parent=None, pad_radius=32.0, skirt=4.6, plate_z=1.2, walls=10):
    """What a vehicle on its pad is served by, along the blast-wall lane at
    `yaw` (plans/003-mars-base-research.md §8.4): an armoured umbilical vault
    beside the skirt with two propellant hoses (LOX, methane) up to a
    quick-disconnect plate; covered service trenches out to the pad's edge;
    in the lane, a valve skid with its purge bottles and control cabinet, a
    power terminal and a cable cart whose cable runs to the vehicle; a nav
    beacon with a radar corner reflector; marker posts over the buried
    lines toward the tank farm."""
    import industry
    m = industry.plant_mats(looks)
    p = _pad_mats(looks)
    b = Building(name, parent, at, yaw)
    x_v = skirt + 1.4
    # The umbilical vault: a low armoured box, its lid swung open, hoses up to the plate on the skirt.
    b.put(box((1.3, 1.0, 0.4), (x_v, 0, 0.2), bevel=0.03), m["paint_dark"], smooth=False)
    b.put(moved(box((1.3, 0.06, 1.0), (0, 0, 0.5)), (x_v, 0.5, 0.4), 0.0, 0.0, -1.2), m["tread"], smooth=False)
    for dy, key in ((-0.22, "jacket"), (0.22, "paint_orange")):
        b.put(tube([Vector((x_v - 0.2, dy, 0.35)), Vector((x_v - 0.6, dy, 0.9)), Vector((skirt + 0.35, dy * 0.8, plate_z - 0.1)), Vector((skirt + 0.08, dy * 0.6, plate_z))], 0.07, 12), m[key])
    b.put(box((0.12, 0.8, 0.5), (skirt + 0.02, 0, plate_z), bevel=0.02), m["steel"], smooth=False)
    # Covered trenches from the vault out to the pad's edge.
    covers = bmesh.new()
    x = x_v + 0.8
    while x < pad_radius - 0.6:
        merge(covers, box((1.18, 0.9, 0.04), (x + 0.6, 0, 0.02)))
        x += 1.2
    b.put(covers, m["tread"], smooth=False)
    # The valve skid, purge bottles and the control cabinet in the lane.
    xs = pad_radius + 3.0
    b.put(box((2.6, 1.6, 0.12), (xs, 0, 0.06)), m["galv"], smooth=False)
    for dy, key in ((-0.4, "jacket"), (0.4, "paint_orange")):
        b.put(rod((xs - 1.2, dy, 0.4), (xs + 1.2, dy, 0.4), 0.09, 14), m[key])
        b.put(rod((xs + 1.2, dy, 0.4), (xs + 1.2, dy, -0.3), 0.09, 14), m[key])
        industry.valve(b, m, (xs - 0.2, dy, 0.4), (1, 0, 0), 0.09)
        industry.gauge(b, m, (xs + 0.5, dy, 0.55), (0, 0, 1), (0, -1 if dy < 0 else 1, 0))
    industry.bottles(b, m, (xs - 0.2, -1.6, 0.0), 0.0, 6)
    industry.cabinet(b, m, (xs + 0.4, 1.4, 0.0), math.pi, (0.8, 0.5, 1.6), "paint_grey", 1, 2)
    # The power terminal and its cable cart; the cable laid over the trench covers to the vehicle.
    term = (xs - 1.6, 2.6, 0.0)
    b.put(box((0.6, 0.6, 1.4), (term[0], term[1], 0.7), bevel=0.03), m["paint_yellow"], smooth=False)
    b.put(box((0.02, 0.4, 0.3), (term[0] - 0.31, term[1], 1.05)), m["vision"], smooth=False)
    cart = (xs - 3.2, 1.6, 0.0)
    b.put(box((1.4, 1.0, 0.12), (cart[0], cart[1], 0.5)), m["paint_dark"], smooth=False)
    for dx in (-0.55, 0.55):
        for dy in (-0.45, 0.45):
            b.put(moved(lathe([(0.0, -0.08), (0.3, -0.08), (0.3, 0.08), (0.0, 0.08)], 16, 0.5), (cart[0] + dx, cart[1] + dy, 0.3), roll=math.pi / 2), m["hose"], smooth=False)
    b.put(moved(lathe([(0.0, -0.35), (0.6, -0.35), (0.6, -0.3), (0.25, -0.3), (0.25, 0.3), (0.6, 0.3), (0.6, 0.35), (0.0, 0.35)], 24, 0.5), (cart[0], cart[1], 1.15), roll=math.pi / 2), m["paint_yellow"], smooth=False)
    b.put(moved(lathe([(0.25, -0.3), (0.42, -0.3), (0.42, 0.3), (0.25, 0.3)], 24, 0.5), (cart[0], cart[1], 1.15), roll=math.pi / 2), m["cable"], smooth=False)
    b.put(tube([Vector((cart[0], cart[1], 0.75)), Vector((cart[0] - 0.5, 0.6, 0.05)), Vector((x_v + 1.0, 0.6, 0.05)), Vector((skirt + 0.3, 0.6, 0.05)), Vector((skirt + 0.05, 0.6, plate_z - 0.4))], 0.03, 8), m["cable"])
    # The nav beacon: a mast with a strobe and a trihedral corner reflector.
    nb = Vector((pad_radius + 4.5, -3.2, 0.0))
    b.put(rod(nb, nb + Vector((0, 0, 6.0)), 0.06, 10), m["galv"])
    b.put(box((0.5, 0.5, 0.3), tuple(nb + Vector((0, 0, 0.15)))), m["footing"], smooth=False)
    corner = bmesh.new()
    for rot in ((0, 0, 0), (math.pi / 2, 0, 0), (0, -math.pi / 2, 0)):
        merge(corner, moved(box((0.6, 0.6, 0.01), (0.3, 0.3, 0.0)), (0, 0, 0), rot[2], rot[1], rot[0]))
    b.put(moved(corner, tuple(nb + Vector((0.05, 0.05, 5.2))), math.pi / 4), m["alu"], smooth=False)
    b.put(sphere(0.1, tuple(nb + Vector((0, 0, 6.1))), 10, 5), p["white_light"])
    # Marker posts over the buried lines, out toward the tank farm.
    posts = bmesh.new()
    for k in range(6):
        x = pad_radius + 14.0 + 8.0 * k
        merge(posts, rod((x, 0, 0), (x, 0, 1.2), 0.03, 6))
        merge(posts, box((0.12, 0.02, 0.2), (x, 0, 1.1)))
    b.put(posts, m["paint_yellow"], smooth=False)
    b.collider(box((2.6, 1.6, 1.0), (xs, 0, 0.5)))
    b.collider(box((1.4, 1.0, 1.4), (cart[0], cart[1], 0.7)))
    return b.finish()
