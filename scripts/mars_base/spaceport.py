"""The spaceport: sintered landing pads ringed by blast walls and lights, a
Starship on one and a crew lander on the other, and their propellant."""
import math
import random

import bmesh
import numpy as np
from mathutils import Matrix, Vector

import patterns
from buildings import light_mast, mats
from kit import (TAU, Building, box, disc, lathe, merge, moved, project_uv, rod, sphere, sweep, tube)

# Starship: barrel radius, skirt height above the ground on its legs, barrel
# and nose lengths, and the height of one welded steel ring.
SHIP_R, SHIP_Z, SHIP_BARREL, SHIP_NOSE = 4.5, 3.2, 36.0, 12.0
SHIP_RINGS = 15
SHIP_TILE = TAU * SHIP_R
# Top of a pad's sintered centre, where the landers' feet stand.
PAD_TOP = 0.1


# Surface maps


def _hex_cells(n, cols, rows):
    """Hexagonal cells over a tile: each texel's cell index and its distance
    to the nearest cell border, in cell pitches."""
    u, v = patterns.grid(n)
    x, y = u * cols, v * rows
    aspect = cols / rows
    best = np.full(x.shape, 1e9, dtype=np.float32)
    second = np.full(x.shape, 1e9, dtype=np.float32)
    cell = np.zeros(x.shape, dtype=np.int64)
    j0 = np.floor(y).astype(np.int64)
    for dj in (-1, 0, 1):
        j = j0 + dj
        off = 0.5 * (j % 2)
        i0 = np.floor(x - off).astype(np.int64)
        for di in (-1, 0, 1):
            i = i0 + di
            d = np.hypot(x - (i + off + 0.5), (y - (j + 0.5)) * aspect).astype(np.float32)
            closer = d < best
            second = np.where(closer, best, np.minimum(second, d))
            best = np.where(closer, d, best)
            cell = np.where(closer, (i % cols) + cols * (j % rows), cell)
    return cell, (second - best) * 0.5


def _ring_steel(n=2048, soot=False, seed=21):
    """Starship's skin round one tile: welded rings up it, three rolled
    plates round each ring, each plate its own tone, grain and waviness;
    `soot` adds engine soot and thrown dust rising from the tile's foot."""
    u, v = patterns.grid(n)
    up = 1.0 - v
    rng = np.random.default_rng(seed)
    rings, sheets = SHIP_RINGS, 3
    row = np.minimum((up * rings).astype(np.int64), rings - 1)
    across = u * sheets + rng.random(rings)[row]
    plate = row * sheets + (np.floor(across).astype(np.int64) % sheets)
    pu, pv = across % 1.0, up * rings % 1.0
    dv, du = np.minimum(pv, 1.0 - pv) / rings, np.minimum(pu, 1.0 - pu) / sheets
    weld = np.maximum(patterns.seam(dv, 0.0006), 0.6 * patterns.seam(du, 0.0005))
    tint = np.exp(-dv / 0.0025)
    ring_tone = (0.93 + 0.1 * rng.random(rings))[row]
    tone = ring_tone * (0.98 + 0.04 * rng.random(rings * sheets))[plate]
    warm = (rng.random(rings) - 0.5)[row] * 0.04
    mottle = patterns.noise(n, cells=40, seed=seed + 1, octaves=3)
    grain = patterns.noise(n, cells=256, seed=seed + 2, octaves=1)[:1, :].repeat(n, axis=0)
    wave = patterns.noise(n, cells=16, seed=seed + 3, octaves=2)
    shade = tone * (0.93 + 0.1 * mottle + 0.04 * grain) * (1.0 - 0.3 * weld) * (1.0 - 0.12 * tint)
    diff = np.stack([shade * (0.62 + warm + 0.015 * tint), shade * (0.62 + 0.005 * tint), shade * (0.64 - warm - 0.02 * tint)], axis=-1)
    rough = 0.18 + 0.08 * mottle + 0.08 * (rng.random(rings)[row]) + 0.3 * weld
    if soot:
        h = up * SHIP_TILE
        streak = patterns.noise(n, cells=96, seed=seed + 4, octaves=2)[:1, :].repeat(n, axis=0)
        black = np.clip(np.exp(-h / 1.1) * (0.55 + 0.6 * streak), 0.0, 1.0)
        dust = np.clip((1.0 - h / 3.6) * (0.5 + 0.6 * mottle), 0.0, 1.0) * 0.55
        diff = diff * (1.0 - dust)[..., None] + np.array([0.42, 0.22, 0.12], dtype=np.float32) * dust[..., None]
        diff = diff * (1.0 - 0.9 * black)[..., None]
        rough = rough + 0.5 * np.maximum(black, dust)
    height = 0.25 * weld + 2.5 * wave
    return np.clip(diff, 0.0, 1.0).astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 3.0), np.clip(rough, 0.0, 1.0).astype(np.float32)


def _ship_tiles(n=2048, seed=9):
    """Starship's hexagonal heat-shield tiles, about 0.31 m across on a
    10 m tile: glossy black, a few greyer replacements, felt in the gaps."""
    cell, edge = _hex_cells(n, 32, 36)
    rng = np.random.default_rng(seed)
    pick = rng.random(32 * 36)
    tone = (0.018 + 0.014 * rng.random(32 * 36))[cell]
    tone = np.where(pick[cell] > 0.994, 0.07, tone)
    gap = patterns.seam(edge, 0.04)
    grime = patterns.noise(n, cells=24, seed=seed + 1)
    diff = (tone * (0.85 + 0.3 * grime)) * (1.0 - gap) + 0.035 * gap
    rough = 0.4 + 0.15 * grime + 0.35 * gap
    height = np.clip(edge / 0.12, 0.0, 1.0) ** 0.5
    return np.repeat(diff[..., None], 3, axis=-1).astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 2.5), rough.astype(np.float32)


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


def _quilt(colour, cells=6, crinkle=1.0, seed=51, n=1024):
    """Multi-layer insulation blankets: pillowed between stitched tacks,
    `cells` across a tile, the foil crinkled by `crinkle`."""
    u, v = patterns.grid(n)
    fu, fv = u * cells % 1.0, v * cells % 1.0
    pillow = (np.sin(np.pi * fu) * np.sin(np.pi * fv)) ** 0.5
    stitch = np.maximum(patterns.seam(np.minimum(fu, 1.0 - fu) / cells, 0.004), patterns.seam(np.minimum(fv, 1.0 - fv) / cells, 0.004))
    folds = patterns.noise(n, cells=48, seed=seed, octaves=4)
    fine = patterns.noise(n, cells=160, seed=seed + 1, octaves=2)
    shade = (0.8 + 0.25 * folds + 0.1 * pillow) * (1.0 - 0.25 * stitch)
    diff = np.clip(np.array(colour, dtype=np.float32) * shade[..., None], 0.0, 1.0)
    height = 0.6 * pillow + crinkle * (0.5 * folds + 0.25 * fine) - 0.5 * stitch
    rough = np.clip(0.22 + 0.25 * fine * crinkle + 0.2 * stitch + 0.1 * (1.0 - crinkle), 0.0, 1.0)
    return diff.astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 6.0), rough.astype(np.float32)


def _ship_mats(looks):
    g = looks.get
    return {
        "steel": g("ship_steel", (0.8, 0.8, 0.82), 0.25, 1.0, maps=_ring_steel),
        "soot": g("ship_steel_sooted", (0.6, 0.6, 0.6), 0.4, 1.0, maps=lambda: _ring_steel(1024, soot=True)),
        "tiles": g("ship_heat_tiles", (0.04, 0.04, 0.04), 0.5, 0.0, maps=_ship_tiles),
        "nozzle": g("ship_nozzle", (0.3, 0.3, 0.3), 0.35, 1.0, maps=_nozzle),
        "throat": g("ship_nozzle_inside", (0.035, 0.03, 0.028), 0.75, 0.4),
        "engine": g("ship_engine", (0.42, 0.42, 0.43), 0.38, 1.0),
        "plumbing": g("ship_plumbing", (0.72, 0.62, 0.45), 0.35, 1.0),
        "shield": g("ship_aft_shield", (0.06, 0.055, 0.05), 0.8, 0.0, "rough_concrete", contrast=1.2),
        "chrome": g("ship_strut", (0.66, 0.66, 0.67), 0.24, 1.0),
        "pad": g("ship_footpad", (0.42, 0.31, 0.24), 0.75, 0.5, "metal_plate", contrast=0.8),
        "flap": g("ship_flap_steel", (0.75, 0.75, 0.77), 0.3, 1.0, maps=lambda: patterns.panels((0.6, 0.6, 0.62), cols=1, rows=3, seam_w=0.002, rivets=False, grime=0.08, seed=23)),
        "window": g("ship_window", (0.015, 0.018, 0.022), 0.04, 0.0, emit=((1.0, 0.82, 0.6), 0.12)),
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


def _ship_profile():
    """Starship's outer line as (radius, z), skirt foot to nose tip."""
    top = SHIP_Z + SHIP_BARREL
    return [(SHIP_R, SHIP_Z), (SHIP_R, top)] + [(r, top + h) for r, h in _ogive(SHIP_R, SHIP_NOSE)[1:]]


def _hull_at(z):
    """Starship's radius at height z and how far its outward normal tilts up."""
    prof = _ship_profile()
    for (r0, z0), (r1, z1) in zip(prof, prof[1:]):
        if z0 <= z <= z1 and z1 > z0:
            t = (z - z0) / (z1 - z0)
            return r0 + (r1 - r0) * t, math.atan2(r0 - r1, z1 - z0)
    return 0.0, 0.0


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


def _plate(outline, root, tip, s0):
    """A tapered fin over an outline of (s, z) points, counter-clockwise, in
    the plane X = 0: `root` thick at s = s0, `tip` thick at its farthest s.
    Returns its -X face, and its +X face with the rim."""
    reach = max(s for s, _ in outline) - s0

    def half(s):
        return 0.5 * (root + (tip - root) * min(max((s - s0) / reach, 0.0), 1.0))

    front, back = bmesh.new(), bmesh.new()
    fv = [front.verts.new((-half(s), s, z)) for s, z in outline]
    front.faces.new(fv[::-1])
    bv = [back.verts.new((half(s), s, z)) for s, z in outline]
    lo = [back.verts.new((-half(s), s, z)) for s, z in outline]
    back.faces.new(bv)
    for i in range(len(outline)):
        j = (i + 1) % len(outline)
        back.faces.new([bv[i], lo[i], lo[j], bv[j]])
    front.normal_update()
    back.normal_update()
    project_uv(front, 9.9)
    project_uv(back, SHIP_TILE / 3)
    return front, back


def _hull_strip(r, z0, z1, a0, a1, t=0.03, segments=6, tile=1.0):
    """A raised strip on a round hull: radius r, from z0 to z1, angles a0 to a1."""
    return lathe([(r - 0.01, z0), (r + t, z0), (r + t, z1), (r - 0.01, z1)], segments, tile, angle=a1 - a0, start=a0)


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


def _raptor(b, s, at, vacuum):
    """A Raptor hanging from the aft dome at `at`: powerhead, turbopumps,
    plumbing, and the sea-level bell or the long vacuum one."""
    at = Vector(at)
    if vacuum:
        outside, inside = _bell(0.17, 1.15, 3.35, 0.3, 0.75)
    else:
        outside, inside = _bell(0.17, 0.65, 1.45, 0.3, 0.75)
    exit_r = max(r for r, _ in outside)
    b.put(moved(lathe(outside, 40, TAU * exit_r), tuple(at)), s["nozzle"], sharp=35)
    b.put(moved(lathe(inside, 40, 2.0), tuple(at)), s["throat"], sharp=50)
    head = bmesh.new()
    merge(head, box((0.5, 0.5, 0.35), (0, 0, -0.17)))
    for dx, dy, r, h in ((0.5, 0.0, 0.19, 0.75), (-0.46, 0.14, 0.16, 0.65)):
        merge(head, moved(lathe([(0.0, -0.95), (r, -0.95), (r, -0.95 + h), (r * 0.5, -0.9 + h), (0.0, -0.9 + h)], 14, 1.0), (dx, dy, 0)))
    b.put(moved(head, tuple(at)), s["engine"])
    pipes = bmesh.new()
    for k in range(4):
        a = TAU * k / 4 + 0.4
        c = Vector((math.cos(a), math.sin(a), 0))
        merge(pipes, tube([at + c * 0.32 + Vector((0, 0, -0.35)), at + c * 0.48 + Vector((0, 0, -0.7)), at + c * 0.36 + Vector((0, 0, -1.05))], 0.05, 8))
    merge(pipes, moved(lathe(_torus(0.36, 0.05), 24, 1.0), tuple(at + Vector((0, 0, -1.05)))))
    b.put(pipes, s["plumbing"])


def _torus(radius, tube_r, steps=8):
    """A torus's profile for `lathe`: a small circle at `radius`."""
    pts = [(radius + tube_r * math.cos(TAU * k / steps), tube_r * math.sin(TAU * k / steps)) for k in range(steps + 1)]
    return pts[::-1]


def _ogive_r(h):
    """Starship's nose radius at height h above the barrel."""
    r, _ = _hull_at(SHIP_Z + SHIP_BARREL + h)
    return r


def _skin_patch(z0, z1, a0, a1, t, segments=8):
    """A panel standing `t` proud of Starship's skin between heights z0 and
    z1 and angles a0 to a1, following the nose where it curves."""
    line = [(_hull_at(z)[0], z) for z in [z0 + (z1 - z0) * k / 4 for k in range(5)]]
    profile = [(line[0][0] - 0.01, z0)] + _offset(line, t) + [(line[-1][0] - 0.01, z1)]
    return lathe(profile, segments, 1.0, angle=a1 - a0, start=a0)


def _flap(b, s, outline, root, tip, s0, angle, deflect):
    """A flap in the radial plane at `angle`, hinged at radius s0, tiled on
    its windward (-X) face and swung `deflect` toward it."""
    front, back = _plate(outline, root, tip, s0)
    hinge = Matrix.Translation((0, s0, 0)) @ Matrix.Rotation(deflect, 4, "Z") @ Matrix.Translation((0, -s0, 0))
    place = Matrix.Rotation(abs(angle) - math.pi / 2, 4, "Z")
    for bm, mat in ((front, s["tiles"]), (back, s["flap"])):
        bm.transform(place @ hinge)
        if angle < 0:
            bm.transform(Matrix.Diagonal((1.0, -1.0, 1.0, 1.0)))
            bmesh.ops.reverse_faces(bm, faces=bm.faces[:])
        b.put(bm, mat, sharp=30)


def _airlock(b, m, s, a, z):
    """The crew airlock door at angle `a`, its guide rails down the hull,
    a davit over it and the elevator cage on cables to the ground."""
    R, Z0 = SHIP_R, SHIP_Z
    c = Vector((math.cos(a), math.sin(a), 0))
    side = Vector((-c.y, c.x, 0))
    w = 1.9 / R
    b.put(_hull_strip(R, z - 0.15, z + 2.75, a - w / 2 - 0.05, a + w / 2 + 0.05, 0.05, 8), m["dark"], sharp=30)
    b.put(_hull_strip(R, z, z + 2.6, a - w / 2, a + w / 2, 0.07, 8), m["hull_grey"], sharp=30)
    b.put(_hull_strip(R, z + 1.6, z + 2.0, a - 0.06, a + 0.06, 0.09, 2), s["window"], smooth=False)
    for e in (-1, 1):
        ra = a + e * (w / 2 + 0.16)
        b.put(_hull_strip(R, Z0 + 0.4, z, ra - 0.012, ra + 0.012, 0.12, 1), s["chrome"], smooth=False)
    dav = c * (R + 0.1) + Vector((0, 0, z + 3.3))
    arm = c * (R + 2.0) + Vector((0, 0, z + 3.4))
    b.put(rod(dav, arm, 0.12, 10), m["yellow"])
    b.put(rod(dav - Vector((0, 0, 1.0)), dav.lerp(arm, 0.6), 0.07, 8), m["yellow"])
    b.put(moved(box((0.5, 1.2, 0.3)), tuple(arm), a), m["dark"], smooth=False)
    cage_at = c * (R + 2.0) + Vector((0, 0, PAD_TOP))
    cage, cables = bmesh.new(), bmesh.new()
    for ex in (-1, 1):
        for ey in (-1, 1):
            p = cage_at + c * (ex * 1.05) + side * (ey * 1.05)
            merge(cage, rod(p, p + Vector((0, 0, 2.5)), 0.05, 6))
            merge(cables, rod(p + Vector((0, 0, 2.5)), arm + side * (ey * 0.45), 0.012, 4))
    b.put(cables, s["engine"])
    for z1 in (1.1, 2.5):
        ring = [cage_at + c * (ex * 1.05) + side * (ey * 1.05) + Vector((0, 0, z1)) for ex, ey in ((-1, -1), (1, -1), (1, 1), (-1, 1), (-1, -1))]
        merge(cage, tube(ring, 0.035, 6, caps=False))
    b.put(cage, m["yellow"])
    b.put(moved(box((2.2, 2.2, 0.12), (0, 0, 0.06)), tuple(cage_at), a), m["grate"], smooth=False)
    for e in (-1, 1):
        b.put(moved(box((0.12, 2.2, 0.16), (0, e * 0.45, 2.55)), tuple(cage_at), a + math.pi / 2), m["frame"], smooth=False)
    for x, y, w in ((-1.05, 0.0, (0.03, 2.1, 0.95)), (0.0, 1.05, (2.1, 0.03, 0.95)), (0.0, -1.05, (2.1, 0.03, 0.95))):
        b.put(moved(box(w, (x, y, 0.65), tile=1.0), tuple(cage_at), a), m["grate"], smooth=False)
    b.put(moved(box((0.04, 2.1, 0.12), (1.05, 0, 1.1), tile=0.5), tuple(cage_at), a), m["hazard"], smooth=False)
    b.collider(moved(box((2.2, 2.2, 2.6), (0, 0, 1.3)), tuple(cage_at), a))


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


def starship(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """A stainless Starship standing on six legs, its windward side toward
    -X: ring-welded barrel and ogive nose, hexagonal heat tiles over the
    windward half, two forward and two aft flaps under aerocovers, three
    sea-level and three vacuum Raptors in the skirt, crew windows, an
    airlock with a cable elevator down to the ground, and a cargo door."""
    m = mats(looks)
    s = _ship_mats(looks)
    b = Building(name, parent, at, yaw)
    R, Z0 = SHIP_R, SHIP_Z
    top = Z0 + SHIP_BARREL
    prof = _ship_profile()
    tip = prof[-1][1]
    band = Z0 + 2 * SHIP_TILE / SHIP_RINGS
    aft = Z0 + 4.5

    # Skin: sooted lowest rings, welded barrel and nose, skirt inside and aft dome.
    b.put(lathe([(R - 0.03, Z0), (R, Z0 + 0.03), (R, band)], 144, SHIP_TILE), s["soot"], sharp=40)
    b.put(lathe([(R, band)] + prof[1:], 144, SHIP_TILE), s["steel"], sharp=40)
    b.put(lathe([(R - 0.03, aft), (R - 0.03, Z0)], 144, 3.0), s["shield"], sharp=40)
    b.put(lathe([(0.0, aft), (R - 0.03, aft)], 96, 2.0), s["shield"])

    # Heat tiles over the windward half, nose tip included.
    shell = _offset([(R, Z0 + 0.35)] + prof[1:], 0.04)
    b.put(lathe(shell, 96, 9.9, angle=math.pi + 0.14, start=math.pi / 2 - 0.07), s["tiles"], sharp=40)

    # Aft flaps, their aerocovers half tiled, and the forward flaps on the nose.
    for side in (1, -1):
        a = side * math.pi / 2
        s0 = R + 0.3
        outline = [(s0, Z0 + 0.9), (s0 + 3.7, Z0 + 1.5), (s0 + 3.7, Z0 + 6.6), (s0 + 1.5, Z0 + 10.4), (s0, Z0 + 10.9)]
        _flap(b, s, outline, 0.55, 0.16, s0, a, math.radians(6))
        for start, tiled in ((a, side > 0), (a - math.pi / 2, side < 0)):
            pod = _pod(Z0 + 0.4, Z0 + 11.6, 0.55, 1.6, 10, start - a, math.pi / 2)
            b.put(moved(pod, tuple(Vector((math.cos(a), math.sin(a), 0)) * (R - 0.05)), a), s["tiles"] if tiled else s["steel"], sharp=40)
        fa = side * math.radians(68)
        root = [(h, _ogive_r(h) + 0.18) for h in (6.9, 5.6, 4.2, 2.8, 1.5, 0.5)]
        outline = [(root[-1][1], top + 0.5), (root[-1][1] + 2.7, top + 1.1), (_ogive_r(3.4) + 1.7, top + 4.6)] + [(r, top + h) for h, r in root[:-1]]
        _flap(b, s, outline, 0.42, 0.12, root[-1][1], fa, 0.0)
        r0, r1 = _ogive_r(0.2), _ogive_r(7.4)
        lean = math.atan2(r0 - r1, 7.2)
        for start, tiled in ((fa, side > 0), (fa - math.pi / 2, side < 0)):
            pod = _pod(0.0, 7.4, 0.42, 1.2, 10, start - fa, math.pi / 2)
            b.put(moved(pod, tuple(Vector((math.cos(fa) * (r0 - 0.08), math.sin(fa) * (r0 - 0.08), top + 0.1))), fa, -lean), s["tiles"] if tiled else s["steel"], sharp=40)

    # Raptors: three sea-level inside, three vacuum round them.
    for k in range(6):
        vac = k % 2 == 0
        a = TAU * k / 6
        rr = 3.0 if vac else 1.15
        _raptor(b, s, (rr * math.cos(a), rr * math.sin(a), aft), vac)

    # Legs: housings on the skirt, telescoping struts, braces and footpads.
    for k in range(6):
        a = TAU * k / 6
        c = Vector((math.cos(a), math.sin(a), 0))
        side = Vector((-c.y, c.x, 0))
        b.put(moved(_pod(Z0 - 0.1, Z0 + 5.2, 0.42, 0.9, 12), tuple(c * (R - 0.06)), a), s["steel"], sharp=40)
        pivot = c * (R + 0.3) + Vector((0, 0, Z0 + 4.2))
        foot = c * (R + 2.4) + Vector((0, 0, PAD_TOP + 0.55))
        mid = pivot.lerp(foot, 0.55)
        b.put(rod(pivot, mid, 0.27, 16), s["engine"])
        b.put(rod(pivot.lerp(foot, 0.5), foot, 0.18, 16), s["chrome"])
        for e in (-1, 1):
            b.put(rod(c * (R + 0.15) + side * (e * 0.7) + Vector((0, 0, Z0 + 0.4)), foot.lerp(pivot, 0.3), 0.1, 10), s["engine"])
        b.put(sphere(0.28, tuple(foot), 12, 6), s["chrome"])
        b.put(moved(lathe([(0.0, 0.0), (0.9, 0.0), (0.95, 0.08), (0.85, 0.2), (0.35, 0.38), (0.0, 0.4)], 24, 1.0), tuple(foot - Vector((0, 0, 0.55)))), s["pad"], sharp=40)

    # Crew windows round the leeward nose, an airlock, its elevator and the cargo door.
    for k in range(9):
        a = math.radians(-44 + 11 * k)
        if abs(math.degrees(a) - 30) < 8:
            continue
        z = top + 1.4
        r, lift = _hull_at(z)
        b.put(moved(box((0.08, 0.62, 0.95)), (r * math.cos(a), r * math.sin(a), z), a, -lift), m["dark"], smooth=False)
        b.put(moved(box((0.04, 0.48, 0.8), (0.03, 0, 0)), (r * math.cos(a), r * math.sin(a), z), a, -lift), s["window"], smooth=False)
    _airlock(b, m, s, math.radians(30), top - 4.2)
    da = math.radians(-32)
    for z0, z1, a0, a1 in ((top - 7.5, top - 7.4, da - 0.42, da + 0.42), (top - 0.7, top - 0.6, da - 0.42, da + 0.42),
                           (top - 7.5, top - 0.6, da - 0.43, da - 0.41), (top - 7.5, top - 0.6, da + 0.41, da + 0.43)):
        b.put(_hull_strip(R, z0, z1, a0, a1, 0.035, 12), s["engine"], sharp=30)
    for z in (top - 7.2, top - 4.0, top - 0.9):
        b.put(_hull_strip(R, z - 0.15, z + 0.15, da - 0.47, da - 0.43, 0.06, 2), m["dark"], smooth=False)

    # Header-tank service panels on the leeward nose.
    for h, a0, a1 in ((3.2, -0.28, -0.08), (3.2, 0.08, 0.28), (5.6, -0.12, 0.12)):
        b.put(_skin_patch(top + h, top + h + 0.9, a0, a1, 0.025), s["flap"], sharp=30)

    # Aft quick-disconnect plate, RCS thrusters and vents.
    qa = math.radians(-60)
    b.put(_hull_strip(R, Z0 + 5.6, Z0 + 7.2, qa - 0.16, qa + 0.16, 0.05, 6), m["hull_grey"], sharp=30)
    port = [(0.0, 0.0), (0.16, 0.0), (0.16, 0.08), (0.1, 0.1), (0.0, 0.1)]
    for k in range(3):
        a = qa - 0.09 + 0.09 * k
        b.put(moved(lathe(port, 12, 1.0), ((R + 0.04) * math.cos(a), (R + 0.04) * math.sin(a), Z0 + 6.4), a, math.pi / 2), m["dark"])
    vent = [(0.0, 0.0), (0.12, 0.0), (0.12, 0.03), (0.08, 0.035), (0.06, 0.0), (0.0, 0.0)]
    for z, angles in ((Z0 + 12.0, (1.4, -1.4)), (top + 8.0, (1.05, -1.05)), (top + 3.2, (0.0,))):
        for a in angles:
            r, lift = _hull_at(z)
            b.put(moved(lathe(vent, 12, 1.0), (r * math.cos(a), r * math.sin(a), z), a, math.pi / 2 - lift), s["engine"], sharp=30)

    b.collider(lathe([(0.0, Z0), (R, Z0), (R, top), (R * 0.8, top + 5.0), (R * 0.45, top + 9.0), (0.0, tip)], 16, 4.0))
    for k in range(6):
        foot = Vector((math.cos(TAU * k / 6), math.sin(TAU * k / 6), 0)) * (R + 2.4)
        b.collider(box((1.0, 1.0, Z0), (foot.x, foot.y, Z0 / 2)))
    for side in (1, -1):
        b.collider(box((0.6, 3.9, 10.0), (0, side * (R + 2.2), Z0 + 5.9)))
    return b.finish()


def _lander_mats(looks):
    g = looks.get
    return {
        "gold": g("lander_mli_gold", (0.9, 0.7, 0.35), 0.3, 1.0, maps=lambda: _quilt((0.88, 0.6, 0.24))),
        "silver": g("lander_mli_silver", (0.75, 0.75, 0.76), 0.3, 1.0, maps=lambda: _quilt((0.72, 0.72, 0.74), seed=53)),
        "beta": g("lander_beta_cloth", (0.86, 0.85, 0.81), 0.8, 0.0, maps=lambda: _quilt((0.74, 0.73, 0.68), 4, 0.25, 55)),
        "copv": g("lander_copv", (0.2, 0.2, 0.21), 0.45, 0.1, "bi_stretch", contrast=0.7),
        "deck": g("lander_deck", (0.34, 0.35, 0.36), 0.5, 0.8, maps=lambda: patterns.panels((0.24, 0.25, 0.26), cols=2, rows=2, seed=41)),
    }


def _rcs_quad(b, m, s, at, yaw):
    """A reaction-control quad: a housing with four thrusters, up, down and
    to either side, its back to the hull at `at`, facing yaw."""
    b.put(moved(box((0.3, 0.34, 0.34), (0.15, 0, 0), tile=1.0), at, yaw), m["hull_grey"], smooth=False)
    cup = [(0.0, 0.0), (0.035, 0.0), (0.06, 0.16), (0.055, 0.165), (0.0, 0.03)]
    for (dy, dz), pitch, roll in (((0, 1), 0.0, 0.0), ((0, -1), math.pi, 0.0), ((-1, 0), 0.0, math.pi / 2), ((1, 0), 0.0, -math.pi / 2)):
        thruster = moved(lathe(cup, 10, 1.0), (0.15, dy * 0.17, dz * 0.17), 0.0, pitch, roll)
        b.put(moved(thruster, at, yaw), s["nozzle"], sharp=40)


def crew_lander(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """A two-stage crew lander, its hatch toward +X: an octagonal descent
    stage under gold and silver insulation with four throttled engines,
    tank domes and helium bottles on its deck, four legs on shock struts
    and footpads; on it the ascent stage, a cabin between two insulated
    tanks with a hatch onto a porch and a ladder down, windows, RCS quads,
    radiators, a docking ring, a high-gain dish and whip antennas."""
    m = mats(looks)
    s = _ship_mats(looks)
    q = _lander_mats(looks)
    b = Building(name, parent, at, yaw)
    z0, z1 = 2.0, 3.6
    corner = 3.5
    flat = corner * math.cos(math.pi / 8)

    # Descent stage: frame, blast shield, deck, and a blanket on every face.
    b.put(lathe([(0.0, z0), (corner, z0), (corner, z1), (0.0, z1)], 8, 2.0, start=math.pi / 8), m["frame"], smooth=False)
    b.put(lathe([(0.0, z0 - 0.06), (corner - 0.2, z0 - 0.06)], 8, 2.0, start=math.pi / 8), s["shield"], smooth=False)
    b.put(lathe([(corner - 0.1, z1 + 0.06), (0.0, z1 + 0.06)], 8, 2.0, start=math.pi / 8), q["deck"], smooth=False)
    for k in range(8):
        a = TAU * k / 8
        skin = q["beta"] if k % 2 else (q["silver"] if k == 0 else q["gold"])
        pillow = box((0.1, 2 * flat * math.tan(math.pi / 8) - 0.3, z1 - z0 - 0.24), (flat + 0.03, 0, (z0 + z1) / 2), tile=1.2)
        b.put(moved(pillow, (0, 0, 0), a), skin, smooth=False)
        post = Vector((math.cos(a + math.pi / 8), math.sin(a + math.pi / 8), 0)) * (corner - 0.02)
        b.put(moved(box((0.16, 0.16, z1 - z0 + 0.1), (0, 0, (z0 + z1) / 2)), tuple(post), a + math.pi / 8), m["dark"], smooth=False)

    # Four descent engines under the shield.
    for k in range(4):
        a = TAU * k / 4
        outside, inside = _bell(0.1, 0.52, 1.0, 0.17, 0.3)
        c = (1.45 * math.cos(a), 1.45 * math.sin(a), z0 - 0.06)
        b.put(moved(lathe(outside, 32, TAU * 0.53), c), s["nozzle"], sharp=35)
        b.put(moved(lathe(inside, 32, 2.0), c), s["throat"], sharp=50)

    # Tank domes and helium bottles on the deck, their feed lines.
    feed = bmesh.new()
    for k in range(4):
        a = TAU * k / 4 + TAU / 8
        c = Vector((math.cos(a), math.sin(a), 0))
        b.put(sphere(0.85, tuple(c * 2.3 + Vector((0, 0, z1 + 0.06))), 24, 10, tile=1.2, low=0.0), q["gold"])
        b.put(moved(lathe([(0.97, 0.0), (0.97, 0.05), (0.84, 0.08)], 24, 1.0), tuple(c * 2.3 + Vector((0, 0, z1 + 0.06)))), m["frame"])
        merge(feed, tube([c * 1.5 + Vector((0, 0, z1 + 0.1)), c * 1.2 + Vector((0, 0, z1 + 0.35)), c * 0.9 + Vector((0, 0, z1 + 0.15))], 0.05, 8))
    for a in (math.radians(160), math.radians(200)):
        c = Vector((math.cos(a), math.sin(a), 0))
        b.put(sphere(0.34, tuple(c * 2.55 + Vector((0, 0, z1 + 0.4))), 16, 8), q["copv"])
        merge(feed, tube([c * 2.55 + Vector((0, 0, z1 + 0.06)), c * 2.2 + Vector((0, 0, z1 + 0.2)), c * 1.6 + Vector((0, 0, z1 + 0.2))], 0.03, 6))
    b.put(feed, s["plumbing"])

    # Legs: shock struts from the deck corners, braces from below, footpads.
    for k in range(4):
        a = TAU * k / 4 + TAU / 8
        c = Vector((math.cos(a), math.sin(a), 0))
        side = Vector((-c.y, c.x, 0))
        top = c * (flat + 0.1) + Vector((0, 0, z1 - 0.2))
        foot = c * 5.4 + Vector((0, 0, PAD_TOP + 0.45))
        b.put(rod(top, top.lerp(foot, 0.55), 0.14, 14), q["gold"])
        b.put(rod(top.lerp(foot, 0.5), foot, 0.095, 14), s["chrome"])
        knee = top.lerp(foot, 0.72)
        for e in (-1, 1):
            b.put(rod(c * (flat - 0.1) + side * (e * 0.95) + Vector((0, 0, z0 + 0.1)), knee, 0.06, 10), m["steel"])
        b.put(rod(c * (flat + 0.05) + Vector((0, 0, z0 + 0.05)), knee, 0.05, 8), m["dark"])
        b.put(sphere(0.15, tuple(foot), 10, 5), s["chrome"])
        b.put(moved(lathe([(0.0, 0.0), (0.62, 0.0), (0.66, 0.06), (0.58, 0.16), (0.2, 0.3), (0.0, 0.32)], 20, 1.0), tuple(foot - Vector((0, 0, 0.45)))), s["pad"], sharp=40)

    # Ascent stage: interstage struts, its engine, and the two side tanks.
    ring = 4.3
    for k in range(8):
        a = TAU * k / 8 + TAU / 16
        a1 = a + TAU / 16
        b.put(rod((1.55 * math.cos(a), 1.55 * math.sin(a), z1 + 0.06), (1.3 * math.cos(a1), 1.3 * math.sin(a1), ring), 0.05, 8), m["steel"])
    outside, inside = _bell(0.08, 0.42, 0.45, 0.14, 0.1)
    b.put(moved(lathe(outside, 28, TAU * 0.43), (0, 0, ring)), s["nozzle"], sharp=35)
    b.put(moved(lathe(inside, 28, 2.0), (0, 0, ring)), s["throat"], sharp=50)
    for e in (-1, 1):
        c = Vector((0, e * 2.35, 0))
        b.put(moved(lathe([(0.0, 0.0), (0.45, 0.06), (0.68, 0.3), (0.75, 0.7), (0.75, 2.1), (0.68, 2.5), (0.45, 2.74), (0.0, 2.8)], 28, 2.0), tuple(c + Vector((0, 0, z1 + 0.2)))), q["beta"], sharp=40)
        for zz in (z1 + 0.9, z1 + 2.2):
            b.put(rod(c * 0.68 + Vector((0, 0, zz)), Vector((0, e * 1.45, zz + 0.2)), 0.05, 8), m["steel"])
            b.put(moved(lathe([(0.77, -0.05), (0.77, 0.05)], 28, 1.0), tuple(c + Vector((0, 0, zz)))), m["frame"])
        b.put(tube([c + Vector((0, -e * 0.4, z1 + 0.25)), Vector((0, e * 1.0, z1 + 0.35)), Vector((0, e * 0.35, ring - 0.05))], 0.05, 8), s["plumbing"])

    # Cabin: insulated barrel, panelled cone, dome and docking ring.
    cr = 1.5
    b.put(lathe([(0.0, ring), (cr, ring), (cr, 7.4)], 48, 2.0), q["beta"], sharp=40)
    b.put(lathe([(cr, 7.4), (cr - 0.05, 7.5), (0.95, 8.8), (0.8, 9.05), (0.7, 9.15)], 48, 2.0), m["hull"], sharp=40)
    b.put(lathe([(0.7, 9.15), (0.7, 9.75), (0.85, 9.78), (0.85, 9.95), (0.6, 9.98), (0.0, 9.98)], 32, 1.0), m["hull_grey"], sharp=40)
    for k in range(3):
        a = TAU * k / 3
        b.put(moved(box((0.08, 0.32, 0.32), (0.72, 0, 10.05), (Matrix.Rotation(-0.35, 3, "Y"))), (0, 0, 0), a), m["hull_grey"], smooth=False)
    for z in (5.2, 6.3, 7.3):
        b.put(lathe([(cr + 0.03, z - 0.04), (cr + 0.03, z + 0.04)], 48, 1.0), m["frame"], sharp=40)

    # Hatch, porch and ladder at +X.
    hw = 1.05 / cr
    b.put(_hull_strip(cr, 4.55, 6.0, -hw / 2 - 0.08, hw / 2 + 0.08, 0.05, 8), m["dark"], sharp=30)
    b.put(_hull_strip(cr, 4.65, 5.9, -hw / 2, hw / 2, 0.08, 8), m["hull_grey"], sharp=30)
    b.put(moved(lathe([(0.0, 0.0), (0.13, 0.0), (0.13, 0.03), (0.0, 0.03)], 14, 1.0), (cr + 0.08, 0, 5.5), 0.0, math.pi / 2), s["window"])
    for e in (-1, 1):
        b.put(rod((cr + 0.12, e * 0.6, 4.8), (cr + 0.12, e * 0.6, 5.8), 0.025, 6), m["yellow"])
    deck = 4.3
    b.put(box((3.7 - cr, 1.5, 0.08), ((3.7 + cr) / 2, 0, deck - 0.04), tile=1.0), m["grate"], smooth=False)
    b.put(box((3.7 - cr, 0.1, 0.16), ((3.7 + cr) / 2, 0.75, deck - 0.08)), m["frame"], smooth=False)
    b.put(box((3.7 - cr, 0.1, 0.16), ((3.7 + cr) / 2, -0.75, deck - 0.08)), m["frame"], smooth=False)
    for x in (2.2, 3.2):
        b.put(rod((x, 0, deck - 0.1), (x, 0, z1 + 0.06), 0.05, 8), m["steel"])
    for e in (-1, 1):
        y = e * 0.72
        for x in (cr + 0.15, 2.6, 3.65):
            b.put(rod((x, y, deck), (x, y, deck + 1.0), 0.025, 6), m["yellow"])
        for zz in (deck + 0.5, deck + 1.0):
            b.put(rod((cr + 0.15, y, zz), (3.65, y, zz), 0.025, 6), m["yellow"])
    lad_top, lad_foot = Vector((3.7, 0, deck)), Vector((5.0, 0, PAD_TOP + 0.05))
    for e in (-1, 1):
        off = Vector((0, e * 0.26, 0))
        b.put(rod(lad_top + off, lad_foot + off, 0.035, 8), m["yellow"])
        b.put(rod(lad_top + off + Vector((0, 0, 1.0)), lad_top.lerp(lad_foot, 0.25) + off + Vector((0, 0, 1.0)), 0.025, 6), m["yellow"])
        b.put(rod(lad_top.lerp(lad_foot, 0.25) + off, lad_top.lerp(lad_foot, 0.25) + off + Vector((0, 0, 1.0)), 0.025, 6), m["yellow"])
    rungs = bmesh.new()
    count = 15
    for k in range(1, count):
        p = lad_top.lerp(lad_foot, k / count)
        merge(rungs, box((0.06, 0.5, 0.03), tuple(p), tile=0.5))
    b.put(rungs, m["frame"], smooth=False)
    b.put(box((0.5, 0.8, 0.06), (5.0, 0, PAD_TOP + 0.03)), m["frame"], smooth=False)

    # Windows on the cone, RCS quads, radiators and antennas.
    lift = math.atan2(cr - 0.95, 8.8 - 7.5)
    for a in (-0.32, 0.32):
        r = cr - 0.05 - (8.0 - 7.5) * math.tan(lift)
        at_w = (r * math.cos(a), r * math.sin(a), 8.0)
        b.put(moved(box((0.08, 0.5, 0.6)), at_w, a, -lift), m["dark"], smooth=False)
        b.put(moved(box((0.04, 0.38, 0.48), (0.03, 0, 0)), at_w, a, -lift), s["window"], smooth=False)
    for k in range(4):
        a = TAU * k / 4 + TAU / 8
        _rcs_quad(b, m, s, (cr * math.cos(a), cr * math.sin(a), 7.15), a)
    for a in (math.radians(140), math.radians(220)):
        c = Vector((math.cos(a), math.sin(a), 0))
        b.put(moved(box((0.06, 1.3, 2.2), (cr + 0.25, 0, 5.85), tile=1.0), (0, 0, 0), a), m["radiator"], smooth=False)
        for zz in (5.0, 6.7):
            b.put(rod(c * cr + Vector((0, 0, zz)), c * (cr + 0.22) + Vector((0, 0, zz)), 0.03, 6), m["steel"])
    boom0, boom1 = Vector((-1.05, 0, 8.6)), Vector((-2.1, 0, 9.4))
    b.put(rod(boom0, boom1, 0.05, 8), m["white_frame"])
    b.put(moved(box((0.22, 0.22, 0.22)), tuple(boom1)), m["hull_grey"], smooth=False)
    f = 0.32
    bowl = [(0.5 * t, (0.5 * t) ** 2 / (4 * f)) for t in [k / 8 for k in range(9)]]
    bowl[0] = (0.0, 0.0)
    dish_p = [(r, z - 0.04) for r, z in bowl] + bowl[::-1]
    b.put(moved(lathe(dish_p, 28, 1.0), tuple(boom1 + Vector((-0.1, 0, 0.12))), 0.0, -0.9), m["white_frame"], sharp=40)
    for at_a, h in (((2.6, 1.3, z1 + 0.06), 1.3), ((-1.0, -2.9, z1 + 0.06), 1.1), ((0.35, 0.3, 9.98), 0.9)):
        b.put(rod(at_a, (at_a[0], at_a[1], at_a[2] + h), 0.012, 4), m["steel"])
        b.put(moved(lathe([(0.0, 0.0), (0.05, 0.0), (0.05, 0.06), (0.0, 0.06)], 8, 1.0), at_a), m["dark"], smooth=False)

    b.collider(lathe([(0.0, z0), (corner, z0), (corner, z1), (0.0, z1)], 8, 4.0, start=math.pi / 8))
    b.collider(lathe([(0.0, z1), (cr, z1), (cr, 7.4), (0.95, 8.8), (0.0, 9.98)], 12, 4.0))
    for e in (-1, 1):
        b.collider(box((1.5, 1.5, 2.8), (0, e * 2.35, z1 + 1.6)))
    for k in range(4):
        a = TAU * k / 4 + TAU / 8
        foot = Vector((math.cos(a), math.sin(a), 0)) * 5.4
        b.collider(box((0.8, 0.8, 2.4), (foot.x, foot.y, 1.2)))
    return b.finish()
