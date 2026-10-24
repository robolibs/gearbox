"""Where the crew lives and works: habitat towers round a hub, an inflatable
hab, printed regolith towers, corridors, a rover hangar and the EDEN module's
skin.

Corridors meet their modules flange to flange: a port's active berthing ring
ends at its face and the corridor's passive ring starts there, both reaching
FLANGE past the corridor's bore, each RING deep, bolted through their backs.
"""
import math

import bmesh
import numpy as np
from mathutils import Vector

import patterns
from buildings import door, light_mast, mats, railing, stairs, unit
from kit import (TAU, Building, box, cone, lathe, merge, moved, prism, project_uv, rect, rod, sphere, sweep, truss, tube, vault)

FLANGE = 0.26
RING = 0.14


# Materials


def _rougher(maps, by):
    """Procedural maps with their roughness raised by `by`."""
    diff, nor, rough = maps
    return diff, nor, np.clip(rough + by, 0.0, 1.0)


def _quilt(colour=(0.66, 0.65, 0.61), cells=4, seed=31):
    """Quilted beta-cloth blanket: stitched squares that puff between their
    seams over a fine weave, `cells` across a tile."""
    u, v = patterns.grid()
    fu, fv = u * cells % 1.0, v * cells % 1.0
    puff = np.sqrt(np.sin(np.pi * fu) * np.sin(np.pi * fv))
    stitch = np.maximum(patterns.seam(np.minimum(fu, 1.0 - fu) / cells, 0.004), patterns.seam(np.minimum(fv, 1.0 - fv) / cells, 0.004))
    weave = patterns.noise(cells=256, seed=seed, octaves=1)
    dirt = patterns.noise(cells=8, seed=seed + 1)
    height = 0.6 * puff - 0.5 * stitch + 0.08 * weave
    shade = 0.9 + 0.1 * puff - 0.3 * stitch - 0.12 * dirt
    diff = np.clip(np.array(colour, dtype=np.float32) * shade[..., None], 0.0, 1.0)
    rough = np.clip(0.7 + 0.12 * dirt + 0.1 * stitch, 0.0, 1.0)
    return diff.astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 24.0), rough.astype(np.float32)


def _print_layers(layers=12, seed=41):
    """Extruded regolith beads stacked `layers` to a tile: rounded, a little
    uneven in width and tone, over the mix's grain."""
    u, v = patterns.grid()
    wobble = 0.15 * (patterns.noise(cells=6, seed=seed) - 0.5)
    f = (v * layers + wobble) % 1.0
    row = np.floor(v * layers + wobble).astype(int) % layers
    bead = np.sin(np.pi * f) ** 0.5
    grain = patterns.noise(cells=128, seed=seed + 1, octaves=3)
    tone = np.random.default_rng(seed).random(layers).astype(np.float32)[row]
    height = bead + 0.3 * grain
    shade = 0.55 + 0.38 * bead + 0.12 * grain - 0.08 * tone
    diff = np.clip(np.array((0.42, 0.27, 0.17), dtype=np.float32) * shade[..., None], 0.0, 1.0)
    rough = np.clip(0.82 + 0.1 * grain, 0.0, 1.0)
    return diff.astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 14.0), rough.astype(np.float32)


def _mats(looks):
    """The base's materials and the habitat's own: MMOD panels, quilted
    blankets, print layers, ring aluminium, bellows, webbing and glazing."""
    g = looks.get
    m = dict(mats(looks))
    m.update({
        "mmod": g("mmod_panels", (0.85, 0.85, 0.83), 0.6, 0.1, maps=lambda: _rougher(patterns.panels((0.68, 0.68, 0.66), cols=2, rows=2, seam_w=0.003, grime=0.08, seed=21), 0.14)),
        "beta": g("beta_cloth", (0.85, 0.84, 0.80), 0.75, 0.0, maps=_quilt),
        "layers": g("print_layers", (0.68, 0.42, 0.27), 0.9, 0.0, maps=_print_layers),
        "alu": g("ring_alu", (0.72, 0.73, 0.75), 0.42, 0.85),
        "bellows": g("bellows", (0.24, 0.24, 0.25), 0.8, 0.0, "bi_stretch", contrast=0.9),
        "webbing": g("webbing", (0.72, 0.71, 0.67), 0.8, 0.0, "hessian_230", contrast=0.5),
        "glazing": g("hab_glazing", (0.04, 0.05, 0.06), 0.14, 0.0, emit=((1.0, 0.84, 0.62), 0.3)),
        "sinter": g("sintered_regolith", (0.44, 0.33, 0.26), 0.9, 0.0, "rough_concrete", contrast=0.9),
        "amber": g("marker_amber", (1.0, 0.55, 0.12), 0.4, 0.0, emit=((1.0, 0.55, 0.12), 6.0)),
    })
    return m


# Shapes


def _put(b, bm, mat, at=(0, 0, 0), yaw=0.0, smooth=True, sharp=40):
    """Puts a part built in a local frame at `at`, turned by `yaw`."""
    b.put(moved(bm, at, yaw), mat, smooth, sharp)


def _turn(yaw, x, y=0.0, z=0.0):
    """The point (x, y, z) of a frame turned by `yaw` about Z."""
    return (x * math.cos(yaw) - y * math.sin(yaw), x * math.sin(yaw) + y * math.cos(yaw), z)


def _clear(a, angles):
    """Angular distance from a to the nearest of `angles`."""
    return min((abs((a - p + math.pi) % TAU - math.pi) for p in angles), default=math.pi)


def _ports(ports, z, bore=1.3):
    """Ports as (angle, axis height, bore); a bare angle takes `z` and `bore`."""
    return [tuple(p) if isinstance(p, (tuple, list)) else (p, z, bore) for p in ports]


def _arcs(cuts):
    """(start, span) arcs round the circle leaving out each (angle, half) cut."""
    if not cuts:
        return [(0.0, TAU)]
    cuts = sorted(((a - h) % TAU, 2 * h) for a, h in cuts)
    out = []
    for k, (a, w) in enumerate(cuts):
        nxt = cuts[(k + 1) % len(cuts)][0] + (TAU if k + 1 == len(cuts) else 0.0)
        if nxt - a - w > 0.05:
            out.append((a + w, nxt - a - w))
    return out


def _ring_band(r, z, w=0.08, d=0.06):
    """A raised band's (radius, z) profile on a wall of radius r at height z."""
    return [(r - 0.02, z - w), (r + d - 0.02, z - w + 0.01), (r + d, z - w + 0.03), (r + d, z + w - 0.03), (r + d - 0.02, z + w - 0.01), (r - 0.02, z + w)]


def _bands(b, mat, r, heights, cuts=(), segments=72):
    """Raised bands round a wall, broken where each (angle, half, z0, z1) cut
    spans their height."""
    bm = bmesh.new()
    for z in heights:
        for start, span in _arcs([(a, h) for a, h, z0, z1 in cuts if z0 < z < z1]):
            merge(bm, lathe(_ring_band(r, z), max(2, int(segments * span / TAU)), 0.5, span, start))
    b.put(bm, mat, sharp=30)


def _axial(profile, segments=48, tile=1.0):
    """A lathe about X from (radius, x) points."""
    return moved(lathe(profile, segments, tile), pitch=math.pi / 2)


def _pad(r0, r1, top, segments=64, tile=2.0):
    """A round footing of radius r1 with a chamfered edge, its top at `top`,
    open inside r0 under what stands on it, mapped flat."""
    bm = lathe([(r1, -0.1), (r1, top - 0.08), (r1 - 0.2, top), (r0, top)], segments, tile)
    bm.normal_update()
    project_uv(bm, tile)
    return bm


def _panel(points, x0, x1, tile=1.0):
    """A plate over a convex (y, z) outline, from x0 to x1 along X."""
    bm = moved(prism([(-z, y) for y, z in points], x0, x1, tile), pitch=math.pi / 2)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    bm.normal_update()
    project_uv(bm, tile)
    return bm


def _fin(points, t, tile=1.0):
    """A plate over a convex (x, z) outline, t thick across Y."""
    bm = moved(prism(points, -t / 2, t / 2, tile), roll=math.pi / 2)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


def _frame(outer, inner, x0, x1, tile=1.0, closed=True):
    """A plate between an outer and an inner (y, z) outline of as many
    points, from x0 to x1 along X; open outlines make a horseshoe."""
    bm = bmesh.new()
    loops = [[bm.verts.new((x, y, z)) for y, z in line] for x in (x0, x1) for line in (outer, inner)]
    n = len(outer)
    for i in range(n if closed else n - 1):
        j = (i + 1) % n
        for a, c in ((0, 1), (3, 2), (2, 0), (1, 3)):
            bm.faces.new([loops[a][i], loops[a][j], loops[c][j], loops[c][i]])
    if not closed:
        for i in (0, n - 1):
            bm.faces.new([loops[0][i], loops[1][i], loops[3][i], loops[2][i]])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    bm.normal_update()
    project_uv(bm, tile)
    return bm


def _strap(path, out, w, t):
    """A flat strap w wide and t thick along a path, lying on a surface whose
    outward normals along it are `out`."""
    bm = bmesh.new()
    rings = []
    for k, p in enumerate(path):
        ahead = path[min(k + 1, len(path) - 1)] - path[max(k - 1, 0)]
        side = ahead.cross(out[k]).normalized() * (w / 2)
        n = out[k].normalized() * t
        rings.append([bm.verts.new(p - side), bm.verts.new(p + side), bm.verts.new(p + side + n), bm.verts.new(p - side + n)])
    for a, c in zip(rings, rings[1:]):
        for i in range(4):
            j = (i + 1) % 4
            bm.faces.new([a[i], c[i], c[j], a[j]])
    bm.normal_update()
    project_uv(bm, 1.0)
    return bm


def _rounded(w, h, r, n=4):
    """A rounded rectangle's (y, z) outline about its centre, counter-clockwise."""
    pts = []
    for cy, cz, a0 in ((w / 2 - r, -h / 2 + r, -math.pi / 2), (w / 2 - r, h / 2 - r, 0.0), (-w / 2 + r, h / 2 - r, math.pi / 2), (-w / 2 + r, -h / 2 + r, math.pi)):
        pts += [(cy + r * math.cos(a0 + math.pi / 2 * k / n), cz + r * math.sin(a0 + math.pi / 2 * k / n)) for k in range(n + 1)]
    return pts


def _bolts(count, radius, x, length, size=0.032, phase=0.5):
    """Hex bolt heads round the X axis at `radius`, standing `length` along X from x."""
    bm = bmesh.new()
    for k in range(count):
        a = TAU * (k + phase) / count
        p = Vector((x, radius * math.cos(a), radius * math.sin(a)))
        merge(bm, tube([p, p + Vector((length, 0, 0))], size, 6))
    return bm


def _corrugated(x0, x1, z0, z1, y, side=1, depth=0.036, pitch=0.278):
    """A corrugated sheet from x0 to x1 and z0 to z1 on the plane Y = side * y,
    its trapezoid ribs `depth` proud toward side * Y."""
    n = max(1, round((x1 - x0) / pitch))
    p = (x1 - x0) / n
    line = []
    for k in range(n):
        x = x0 + k * p
        line += [(x, depth), (x + 0.36 * p, depth), (x + 0.5 * p, 0.0), (x + 0.86 * p, 0.0)]
    line.append((x1, depth))
    bm = bmesh.new()
    lo = [bm.verts.new((x, side * (y + d), z0)) for x, d in line]
    hi = [bm.verts.new((x, side * (y + d), z1)) for x, d in line]
    for i in range(len(line) - 1):
        face = [lo[i], hi[i], hi[i + 1], lo[i + 1]]
        bm.faces.new(face if side > 0 else face[::-1])
    bm.normal_update()
    project_uv(bm, 2.4)
    return bm


# Shared hardware


def _flange(bore, face, facing, segments=48):
    """One berthing ring of a bore: its mating face at x = face looking along
    `facing` (±1), and the bolt heads on its back; (ring, bolts)."""
    rf, back, s = bore + FLANGE, face - facing * RING, facing
    pts = [(bore - 0.05, back), (rf - 0.03, back), (rf, back + s * 0.03), (rf, face - s * 0.025), (rf - 0.025, face), (bore - 0.05, face), (bore - 0.05, back)]
    return _axial(pts if s > 0 else pts[::-1], segments, 0.5), _bolts(16, bore + 0.18, back, -s * 0.05)


def _berth(b, m, at, yaw, hull, face, bore=1.3, doubler=True):
    """A berthing port along +X of `yaw` from the point `at` on its axis: a
    blanketed collar from inside the hull at `hull` out to its active ring,
    whose face lies `face` out, with bolts, latch housings and alignment
    guides; a hatch behind it, a doubler ring where the collar leaves the
    hull, a lamp and handrails."""
    rb, rf = bore + 0.08, bore + FLANGE
    back = face - RING
    reach = face - hull
    _put(b, _axial([(rb, hull - 0.45), (rb, back)], 32, 1.0), m["beta"], at, yaw)
    if reach > 0.5:
        xa = (hull + back) / 2
        _put(b, _axial([(rb - 0.01, xa - 0.08), (rb + 0.035, xa - 0.06), (rb + 0.035, xa + 0.06), (rb - 0.01, xa + 0.08)], 32, 0.5), m["alu"], at, yaw)
    ring, bolts = _flange(bore, face, 1)
    _put(b, ring, m["alu"], at, yaw, sharp=30)
    _put(b, bolts, m["steel"], at, yaw, False)
    hatch = _axial([(bore - 0.05, face - 0.1), (0.0, face - 0.1)], 32)
    merge(hatch, _panel(_rounded(1.2, 1.2, 0.26), face - 0.1, face - 0.05))
    _put(b, hatch, m["hull_grey"], at, yaw, False)
    fittings = bmesh.new()
    for k in range(4):
        merge(fittings, moved(box((0.36, 0.22, 0.12), (back - 0.2, 0, rb + 0.06), bevel=0.02), roll=TAU * (k + 0.5) / 4))
    _put(b, fittings, m["hull_grey"], at, yaw, False)
    guides = bmesh.new()
    for k in range(4):
        merge(guides, moved(_fin([(face - 0.12, rf - 0.01), (face + 0.1, rf - 0.01), (face + 0.1, rf + 0.03), (face - 0.02, rf + 0.07), (face - 0.12, rf + 0.07)], 0.06), roll=TAU * k / 4))
    _put(b, guides, m["alu"], at, yaw, False)
    if doubler:
        _put(b, _axial([(rb - 0.01, hull - 0.55), (rb + 0.2, hull - 0.5), (rb + 0.22, hull + 0.04), (rb + 0.16, hull + 0.1), (rb - 0.01, hull + 0.12)], 48, 1.0), m["hull_grey"], at, yaw, sharp=35)
        _put(b, _bolts(24, rb + 0.12, hull + 0.09, 0.035, 0.02), m["steel"], at, yaw, False)
    if reach > 0.6:
        xl = back - 0.5
        _put(b, box((0.2, 0.28, 0.14), (xl, 0, rb + 0.08), bevel=0.02), m["dark"], at, yaw, False)
        _put(b, box((0.02, 0.22, 0.09), (xl + 0.105, 0, rb + 0.08)), m["lamp"], at, yaw, False)
        rails = bmesh.new()
        x0, x1 = hull + 0.5, back - 0.12
        for s in (-1, 1):
            merge(rails, tube([Vector((x0, s * rb, 0.0)), Vector((x0, s * (rb + 0.13), 0.0)), Vector((x1, s * (rb + 0.13), 0.0)), Vector((x1, s * rb, 0.0))], 0.022, 6))
        _put(b, rails, m["yellow"], at, yaw)


def _porthole(b, m, at, yaw, r=0.3, shutter=1.9):
    """A round window facing +X of `yaw` at `at`: a raised frame with bolts
    round a recessed pane, and an external shutter on an arm, swung
    `shutter` radians aside about its pivot beside the frame (0 is shut)."""
    frame = _axial([(r, -0.08), (r + 0.14, -0.08), (r + 0.16, 0.04), (r + 0.12, 0.1), (r + 0.02, 0.11), (r, 0.06), (r, -0.08)], 24, 0.5)
    _put(b, frame, m["hull_grey"], at, yaw, sharp=35)
    _put(b, _axial([(r + 0.01, 0.03), (0.0, 0.03)], 24), m["glazing"], at, yaw, False)
    _put(b, _bolts(12, r + 0.075, 0.09, 0.035, 0.016), m["steel"], at, yaw, False)
    pivot = r + 0.3
    leaf = _axial([(0.0, 0.0), (r + 0.13, 0.0), (r + 0.13, 0.03), (r + 0.09, 0.045), (0.0, 0.045)], 24, 0.5)
    merge(leaf, box((0.03, 0.06, 2 * r), (0.055, 0, 0)))
    merge(leaf, box((0.03, pivot, 0.08), (0.03, -pivot / 2, 0)))
    moved(leaf, (0, pivot, 0))
    moved(leaf, roll=shutter)
    _put(b, moved(leaf, (0.12, -pivot, 0)), m["mmod"], at, yaw, False)
    _put(b, rod((-0.05, -pivot, 0.0), (0.16, -pivot, 0.0), 0.05, 10), m["steel"], at, yaw)


def _hatch(b, m, at, yaw, w=1.0, h=1.6):
    """An EVA hatch facing +X of `yaw`, its sill at `at`: a raised frame round
    a rounded leaf with a porthole, dogging handle and hinges, a striped sill
    and a lamp over it."""
    lift = Vector((0, 0, h / 2))
    outer = [(y, z + h / 2) for y, z in _rounded(w + 0.36, h + 0.36, 0.32)]
    inner = [(y, z + h / 2) for y, z in _rounded(w, h, 0.2)]
    _put(b, _frame(outer, inner, -0.12, 0.12), m["hull_grey"], at, yaw, False)
    _put(b, _panel([(y, z + h / 2) for y, z in _rounded(w - 0.02, h - 0.02, 0.19)], -0.04, 0.05), m["mmod"], at, yaw, False)
    zw = h * 0.74
    _put(b, moved(_axial([(0.12, 0.04), (0.18, 0.04), (0.18, 0.08), (0.12, 0.09), (0.12, 0.04)], 20), (0, 0, zw)), m["hull_grey"], at, yaw)
    _put(b, moved(_axial([(0.125, 0.07), (0.0, 0.07)], 20), (0, 0, zw)), m["glazing"], at, yaw, False)
    hardware = tube([lift * 0.9 + Vector((0.05, -0.14, 0)), lift * 0.9 + Vector((0.14, -0.14, 0)), lift * 0.9 + Vector((0.14, 0.14, 0)), lift * 0.9 + Vector((0.05, 0.14, 0))], 0.02, 6)
    for z in (h * 0.2, h * 0.8):
        merge(hardware, rod((0.12, w / 2 + 0.08, z - 0.12), (0.12, w / 2 + 0.08, z + 0.12), 0.04, 8))
        merge(hardware, box((0.06, 0.2, 0.08), (0.08, w / 2 - 0.02, z)))
    _put(b, hardware, m["steel"], at, yaw)
    _put(b, box((0.02, w + 0.1, 0.12), (0.125, 0, -0.09), tile=0.5), m["hazard"], at, yaw, False)
    _put(b, box((0.22, 0.42, 0.12), (0.12, 0, h + 0.34), bevel=0.02), m["dark"], at, yaw, False)
    _put(b, box((0.02, 0.36, 0.06), (0.235, 0, h + 0.32)), m["lamp"], at, yaw, False)
    b.places.append(("light", {"at": tuple(Vector(at) + Vector((math.cos(yaw), math.sin(yaw), 0)) * 0.5 + Vector((0, 0, h + 0.3))), "kind": "door"}))


def _ladder(b, m, at, yaw, z0, z1, cage=2.4):
    """A caged steel ladder at `at` from z0 to z1, its climbing side toward
    +X of `yaw`, bracketed to a wall 0.22 behind it, hooped from `cage` up."""
    frame = bmesh.new()
    for s in (-1, 1):
        merge(frame, rod((0, s * 0.23, z0), (0, s * 0.23, z1 + 1.0), 0.025, 6))
    hoops = [cage + 0.9 * k for k in range(int((z1 + 1.0 - cage) / 0.9) + 1)] if z1 - z0 > cage else []
    arc = [(0.33 + 0.36 * math.cos(t), 0.36 * math.sin(t)) for t in [-(math.pi - 0.69) + 2 * (math.pi - 0.69) * k / 10 for k in range(11)]]
    for z in hoops:
        merge(frame, tube([Vector((0, -0.23, z))] + [Vector((x, y, z)) for x, y in arc] + [Vector((0, 0.23, z))], 0.018, 6))
    if hoops:
        for t in (-1.9, -0.95, 0.0, 0.95, 1.9):
            p = Vector((0.33 + 0.36 * math.cos(t), 0.36 * math.sin(t), 0))
            merge(frame, rod(p + Vector((0, 0, hoops[0])), p + Vector((0, 0, hoops[-1])), 0.014, 4))
    _put(b, frame, m["yellow"], at, yaw)
    steps = bmesh.new()
    for k in range(int((z1 - z0) / 0.3)):
        z = z0 + 0.3 * (k + 1)
        merge(steps, rod((0, -0.23, z), (0, 0.23, z), 0.016, 6))
    for k in range(int((z1 - z0) / 1.5) + 1):
        for s in (-1, 1):
            merge(steps, box((0.22, 0.03, 0.06), (-0.11, s * 0.23, z0 + 0.5 + 1.5 * k)))
    _put(b, steps, m["steel"], at, yaw, False)


def _saddle(b, m, x, r, mid):
    """A corridor saddle at x: a cradle round a frame's underside on a rubber
    pad, splayed legs and a tie beam down to plates on sintered footings, or
    a sleeper block where the corridor runs low."""
    rc = r + 0.16
    arc = [math.radians(-150 + 120 * k / 12) for k in range(13)]
    b.put(sweep(rect(0.05, 0.32), [(x, rc * math.cos(t), mid + rc * math.sin(t)) for t in arc], 1.0), m["steel"], sharp=30)
    b.put(sweep(rect(0.02, 0.3), [(x, (rc - 0.03) * math.cos(t), mid + (rc - 0.03) * math.sin(t)) for t in arc], 1.0), m["rubber"], sharp=30)
    spread = rc * math.cos(math.radians(30))
    low = mid - rc * 0.5
    foot = 0.15
    steel, footing = bmesh.new(), bmesh.new()
    if mid - rc > 0.45:
        for s in (-1, 1):
            merge(steel, rod((x, s * spread, low), (x, s * (spread + 0.22), foot), 0.07, 8))
            merge(steel, box((0.36, 0.36, 0.04), (x, s * (spread + 0.22), foot + 0.02)))
            merge(footing, box((0.7, 0.7, 0.3), (x, s * (spread + 0.22), 0.0), tile=2.0))
        merge(steel, box((0.2, 2 * spread + 0.3, 0.16), (x, 0, mid - rc - 0.08)))
        if low - foot > 0.8:
            merge(steel, rod((x, -spread - 0.15, foot + 0.3), (x, spread + 0.15, foot + 0.3), 0.04, 6))
    else:
        merge(steel, box((0.4, 2 * spread * 0.8, mid - rc * 0.7), (x, 0, (mid - rc * 0.7) / 2)))
        merge(footing, box((0.8, 2 * spread + 0.4, 0.3), (x, 0, 0.0), tile=2.0))
    b.put(steel, m["steel"], smooth=False)
    b.put(footing, m["sinter"], smooth=False)


# Corridors and nodes


def corridor(looks, name, a, c, radius=1.4, parent=None, z=None):
    """A pressurised corridor between two points: a passive berthing ring at
    each end, its face where the run starts and stops, closed by a hatch and
    backed by a bellows; then MMOD-clad rigid segments between bolted ring
    frames, handrails, a cable tray and a pipe run on brackets, marker lamps,
    and steel saddles on sintered footings."""
    m = _mats(looks)
    a, c = Vector(a), Vector(c)
    run = c - a
    b = Building(name, parent, tuple(a), math.atan2(run.y, run.x))
    length = run.length
    mid = radius + 0.5 if z is None else z
    axis = (0, 0, mid)
    r = radius
    shell = r + 0.05
    bell = min(0.6, length * 0.08)
    x0, x1 = RING + bell, length - RING - bell
    for face, s in ((0.0, -1), (length, 1)):
        ring, bolts = _flange(r, face, s)
        _put(b, ring, m["alu"], axis, sharp=30)
        _put(b, bolts, m["steel"], axis, smooth=False)
        xh = face - s * 0.04
        _put(b, _axial([(r - 0.05, xh), (0.0, xh)] if s > 0 else [(0.0, xh), (r - 0.05, xh)], 32), m["hull_grey"], axis, smooth=False)
        start = face - s * RING
        folds = [(r + 0.04, start)]
        for k in range(7):
            folds += [(r + 0.13, start - s * bell * (k + 0.5) / 7), (r + 0.04, start - s * bell * (k + 1) / 7)]
        _put(b, _axial(folds if s < 0 else folds[::-1], 32, 0.5), m["bellows"], axis, sharp=60)
    segs = max(1, round((x1 - x0) / 2.6))
    frames = [x0 + (x1 - x0) * k / segs for k in range(segs + 1)]
    _put(b, _axial([(shell, x0), (shell, x1)], 32, TAU * shell / 4), m["mmod"], axis)
    rings = bmesh.new()
    bolts = bmesh.new()
    for x in frames:
        merge(rings, _axial([(shell - 0.02, x - 0.09), (r + 0.11, x - 0.08), (r + 0.13, x - 0.06), (r + 0.13, x + 0.06), (r + 0.11, x + 0.08), (shell - 0.02, x + 0.09)], 32, 0.5))
        merge(bolts, _bolts(12, r + 0.1, x - 0.085, -0.03, 0.022))
    _put(b, rings, m["alu"], axis, sharp=30)
    _put(b, bolts, m["steel"], axis, smooth=False)
    # Handrails on stand-offs along the upper flanks.
    xs0, xs1 = x0 + 0.3, x1 - 0.3
    rails = bmesh.new()
    for t in (math.radians(55), math.radians(125)):
        er = Vector((0, math.cos(t), math.sin(t)))
        merge(rails, tube([Vector((xs0, 0, 0)) + er * (r + 0.28), Vector((xs1, 0, 0)) + er * (r + 0.28)], 0.022, 6))
        n = max(2, int((xs1 - xs0) / 1.2) + 1)
        for k in range(n):
            p = Vector((xs0 + (xs1 - xs0) * k / (n - 1), 0, 0))
            merge(rails, rod(p + er * shell, p + er * (r + 0.28), 0.014, 6))
    _put(b, rails, m["yellow"], axis)
    # A ladder tray of cables on one flank, coolant and gas lines on the other.
    tt = -0.25
    ty, tz = (r + 0.34) * math.cos(tt), (r + 0.34) * math.sin(tt)
    span = xs1 - xs0
    tray = bmesh.new()
    for s in (-1, 1):
        merge(tray, box((span, 0.03, 0.09), ((xs0 + xs1) / 2, ty + s * 0.16, tz)))
    for k in range(int(span / 0.3) + 1):
        merge(tray, box((0.04, 0.32, 0.02), (xs0 + 0.3 * k, ty, tz - 0.035)))
    brackets = bmesh.new()
    for k in range(int(span / 1.2) + 1):
        x = xs0 + 1.2 * k
        merge(brackets, box((0.05, 0.22, 0.05), (x, ty - 0.24, tz - 0.05)))
        merge(brackets, box((0.05, 0.3, 0.3), (x, -ty + 0.17, tz - 0.06)))
    _put(b, tray, m["steel"], axis, smooth=False)
    _put(b, brackets, m["frame"], axis, smooth=False)
    cables = bmesh.new()
    for k in range(3):
        merge(cables, tube([(xs0 - 0.1, ty - 0.08 + 0.08 * k, tz), (xs1 + 0.1, ty - 0.08 + 0.08 * k, tz)], 0.028, 6))
    _put(b, cables, m["cable"], axis)
    _put(b, tube([(xs0 - 0.1, -ty + 0.02, tz + 0.05), (xs1 + 0.1, -ty + 0.02, tz + 0.05)], 0.07, 10), m["pipe_white"], axis)
    _put(b, tube([(xs0 - 0.1, -ty - 0.02, tz - 0.14), (xs1 + 0.1, -ty - 0.02, tz - 0.14)], 0.04, 8), m["pipe_yellow"], axis)
    boxes = bmesh.new()
    for x in (xs0 - 0.25, xs1 + 0.25):
        merge(boxes, box((0.3, 0.34, 0.3), (x, ty - 0.05, tz), bevel=0.02))
        merge(boxes, box((0.3, 0.3, 0.36), (x, -ty + 0.02, tz - 0.05), bevel=0.02))
    _put(b, boxes, m["hull_grey"], axis, smooth=False)
    # Marker lamps on top of every other frame, saddles under them.
    for x in frames[1::2] or frames[:1]:
        _put(b, box((0.14, 0.14, 0.07), (x, 0, r + 0.165), bevel=0.01), m["dark"], axis, smooth=False)
        _put(b, box((0.1, 0.1, 0.03), (x, 0, r + 0.21)), m["amber"], axis, smooth=False)
    for x in sorted(set(frames[::2] + frames[-1:])):
        _saddle(b, m, x, r, mid)
    b.collider(moved(lathe([(radius * 1.03, 0.0), (radius * 1.03, length)], 12, 4.0), (0, 0, mid), 0, math.pi / 2))
    b.collider(box((length, radius * 1.7, mid - radius), (length / 2, 0, (mid - radius) / 2)))
    return b.finish()


def _cupola(b, m, z, rb=1.5, rt=0.85, h=0.75):
    """An ISS-style cupola from height z on a berthing ring: a hexagonal
    frustum of six trapezoid windows round a round top one, each with its
    shutter swung open, and a handrail round it."""
    b.put(lathe([(rb + 0.22, z - 0.08), (rb + 0.22, z + 0.22), (rb + 0.1, z + 0.3), (rb, z + 0.3)], 48, 0.5), m["alu"], sharp=30)
    zb, zt = z + 0.3, z + 0.3 + h
    ab, at_ = rb * math.cos(math.pi / 6), rt * math.cos(math.pi / 6)
    s = math.hypot(ab - at_, h)
    lean = math.atan2(ab - at_, h)
    outer = [(-rb / 2, 0.0), (rb / 2, 0.0), (rt / 2, s), (-rt / 2, s)]
    inner = [(-rb / 2 + 0.17, 0.11), (rb / 2 - 0.17, 0.11), (rt / 2 - 0.1, s - 0.09), (-rt / 2 + 0.1, s - 0.09)]
    lid = [(-rb / 2 + 0.12, 0.0), (rb / 2 - 0.12, 0.0), (rt / 2 - 0.05, s - 0.05), (-rt / 2 + 0.05, s - 0.05)]
    for k in range(6):
        phi = math.pi / 6 + k * math.pi / 3
        foot = (ab * math.cos(phi), ab * math.sin(phi), zb)
        b.put(moved(_frame(outer, inner, -0.06, 0.05), foot, phi, -lean), m["white_frame"], smooth=False)
        b.put(moved(_panel(inner, -0.05, -0.02), foot, phi, -lean), m["glazing"], smooth=False)
        shutter = moved(_panel(lid, 0.0, 0.03), pitch=2.05)
        b.put(moved(moved(shutter, (0.06, 0, 0)), foot, phi, -lean), m["mmod"], smooth=False)
    b.put(lathe([(rt + 0.04, zt - 0.06), (rt + 0.04, zt + 0.04), (0.5, zt + 0.06)], 6, 0.5), m["white_frame"], smooth=False)
    b.put(lathe([(0.5, zt + 0.06), (0.52, zt + 0.12), (0.4, zt + 0.14), (0.36, zt + 0.08)], 24, 0.5), m["white_frame"], sharp=40)
    b.put(lathe([(0.37, zt + 0.08), (0.0, zt + 0.08)], 24), m["glazing"], smooth=False)
    lid_top = moved(lathe([(0.0, 0.0), (0.42, 0.0), (0.42, 0.03), (0.0, 0.03)], 24, 0.5), (0.45, 0, 0))
    b.put(moved(moved(lid_top, pitch=-2.5), (-0.5, 0, zt + 0.14), math.pi / 6), m["mmod"], smooth=False)
    ring = [Vector(((rb + 0.5) * math.cos(TAU * k / 24), (rb + 0.5) * math.sin(TAU * k / 24), z + 0.3)) for k in range(25)]
    rail = tube(ring, 0.022, 6)
    for k in range(6):
        p = ring[k * 4]
        merge(rail, rod(p - Vector((0, 0, 0.34)), p, 0.016, 6))
    b.put(rail, m["yellow"])


def hub_node(looks, name, radius=3.8, at=(0, 0, 0), parent=None, ports=(0.0,)):
    """The node corridors meet: an MMOD-clad drum on levelling jacks over a
    sintered pad, a berthing port toward each corridor, an ISS-style cupola
    on a blanketed roof, a guard rail, a caged ladder, antennas, lamps and a
    cable tray round its shoulder. A port is an angle about Z, or (angle,
    axis height, bore) for a corridor off the node's own height; the drum
    reaches down to the pad when one runs low."""
    m = _mats(looks)
    b = Building(name, parent, at, 0.0)
    z0, h = 0.6, 4.6
    top = z0 + h
    ports = _ports(ports, z0 + 1.9)
    angles = [a for a, _, _ in ports]
    zb = min(z0, max(0.2, min((pz - bore - 0.2 for _, pz, bore in ports), default=z0)))
    free = max((TAU * k / 36 for k in range(36)), key=lambda a: _clear(a, angles))
    b.put(_pad(radius - 0.8, radius + 1.6, 0.2), m["sinter"], sharp=40)
    if zb > 0.4:
        jacks = bmesh.new()
        for k in range(8):
            c = Vector((math.cos(TAU * (k + 0.5) / 8), math.sin(TAU * (k + 0.5) / 8), 0)) * (radius - 0.35)
            merge(jacks, box((0.55, 0.55, 0.05), (c.x, c.y, 0.225)))
            merge(jacks, box((0.24, 0.24, zb - 0.33), (c.x, c.y, 0.25 + (zb - 0.33) / 2)))
            merge(jacks, box((0.4, 0.4, 0.06), (c.x, c.y, zb - 0.05)))
        b.put(jacks, m["steel"], smooth=False)
    b.put(lathe([(0.0, zb - 0.15), (radius - 0.1, zb - 0.02), (radius, zb)], 64, 2.0), m["hull_grey"], sharp=30)
    b.put(lathe([(radius, zb), (radius, top)], 64, TAU * radius / 12), m["mmod"])
    b.put(lathe([(radius, top), (radius - 0.12, top + 0.12), (radius * 0.5, top + 0.5), (1.75, top + 0.58), (0.0, top + 0.6)], 64, 2.0), m["beta"], sharp=30)
    cuts = [(a, math.asin(min(0.95, (bore + FLANGE + 0.12) / radius)), pz - bore - FLANGE - 0.2, pz + bore + FLANGE + 0.2) for a, pz, bore in ports]
    _bands(b, m["alu"], radius, (zb + 0.1, top - 0.1), cuts, 64)
    for a, pz, bore in ports:
        _berth(b, m, (0, 0, pz), a, radius, radius + 1.2, bore)
    _cupola(b, m, top + 0.58)
    # Guard rail round the roof, broken at the ladder.
    rr = radius - 0.3
    rz = top + 0.12 + (radius - 0.12 - rr) / (radius * 0.5 - 0.12) * 0.38
    railing(b, m, [(rr * math.cos(free + 0.15 + (TAU - 0.3) * k / 30), rr * math.sin(free + 0.15 + (TAU - 0.3) * k / 30), rz) for k in range(31)], 1.0, 1.6)
    _ladder(b, m, _turn(free, radius + 0.22, 0, 0.0), free, 0.2, top + 0.12)
    # Antennas, a beacon and floodlamps on the roof edge.
    gear = bmesh.new()
    for a in (free + 2.2, free + 4.0):
        p = Vector(_turn(a, radius - 0.6, 0, top + 0.2))
        merge(gear, rod(p, p + Vector((0, 0, 1.6)), 0.02, 6))
        merge(gear, box((0.2, 0.2, 0.12), tuple(p + Vector((0, 0, 0.06)))))
    p = Vector(_turn(free + math.pi, radius - 0.7, 0, top + 0.25))
    merge(gear, rod(p, p + Vector((0, 0, 0.7)), 0.03, 6))
    b.put(gear, m["steel"], smooth=False)
    b.put(sphere(0.1, tuple(p + Vector((0, 0, 0.78))), 10, 5), m["red"])
    for k in range(4):
        a = free + TAU * (k + 0.5) / 4
        _put(b, box((0.18, 0.4, 0.2), (radius - 0.2, 0, top + 0.38), bevel=0.02), m["dark"], (0, 0, 0), a, False)
        _put(b, box((0.02, 0.34, 0.12), (radius - 0.1, 0, top + 0.36)), m["lamp"], (0, 0, 0), a, False)
        _put(b, rod((radius - 0.25, 0, top + 0.1), (radius - 0.25, 0, top + 0.3), 0.03, 6), m["steel"], (0, 0, 0), a)
    # A cable tray round the shoulder.
    zt = top - 0.55
    b.put(lathe([(radius + 0.03, zt), (radius + 0.3, zt), (radius + 0.3, zt + 0.1)], 64, 0.5), m["tray"], sharp=60)
    b.put(lathe([(radius + 0.17 + 0.045 * math.cos(TAU * k / 8), zt + 0.06 + 0.045 * math.sin(TAU * k / 8)) for k in range(9)], 64, 0.5), m["cable"])
    brackets = bmesh.new()
    for k in range(16):
        merge(brackets, moved(box((0.3, 0.05, 0.12), (radius + 0.15, 0, zt - 0.06)), yaw=TAU * k / 16))
    b.put(brackets, m["steel"], smooth=False)
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, z0 + h / 2), "size": (radius * 1.4, radius * 1.4, h), "volume": math.pi * radius ** 2 * h, "glazing": 0.0}))
    b.collider(lathe([(radius + 1.6, 0.0), (radius + 1.6, 0.2), (radius, 0.2), (radius, top), (1.75, top + 0.6), (1.5, top + 1.65), (0.0, top + 1.75)], 16, 4.0))
    return b.finish()


def junction(looks, name, at=(0, 0, 0), parent=None, ports=(0.0,), radius=2.0, z=2.5):
    """Where corridors bend or branch: an upright MMOD-clad drum on jacks
    over a footing, a short collar out of it toward each corridor ending in a
    berthing ring at `radius`, a blanketed dome cap with vent valves, an
    antenna, a beacon and a lamp. A port is an angle about Z, or (angle,
    axis height, bore) for a corridor off `z`; the drum reaches down to the
    footing when one runs low."""
    m = _mats(looks)
    b = Building(name, parent, at, 0.0)
    top = z + 1.9
    drum = radius - 0.3
    ports = _ports(ports, z)
    angles = [a for a, _, _ in ports]
    zb = min(0.45, max(0.2, min((pz - bore - 0.2 for _, pz, bore in ports), default=0.45)))
    free = max((TAU * k / 24 for k in range(24)), key=lambda a: _clear(a, angles))
    b.put(_pad(drum - 0.7, radius + 0.7, 0.2, 48), m["sinter"], sharp=40)
    if zb > 0.4:
        jacks = bmesh.new()
        for k in range(6):
            c = Vector((math.cos(TAU * (k + 0.5) / 6), math.sin(TAU * (k + 0.5) / 6), 0)) * (drum - 0.3)
            merge(jacks, box((0.4, 0.4, 0.04), (c.x, c.y, 0.22)))
            merge(jacks, box((0.18, 0.18, zb - 0.22), (c.x, c.y, 0.22 + (zb - 0.22) / 2)))
        b.put(jacks, m["steel"], smooth=False)
    b.put(lathe([(0.0, zb - 0.1), (drum - 0.08, zb - 0.01), (drum, zb)], 48, 2.0), m["hull_grey"], sharp=30)
    b.put(lathe([(drum, zb), (drum, top)], 48, TAU * drum / 6), m["mmod"])
    cap = [(drum * (0.03 + 0.97 * math.cos(t)), top + 0.75 * math.sin(t)) for t in [math.pi / 2 * k / 8 for k in range(9)]]
    cap[-1] = (0.0, top + 0.75)
    b.put(lathe(cap, 48, 2.0), m["beta"], sharp=35)
    cuts = [(a, math.asin(min(0.95, (bore + 0.1) / drum)), pz - bore - FLANGE - 0.1, pz + bore + FLANGE + 0.1) for a, pz, bore in ports]
    _bands(b, m["alu"], drum, (zb + 0.12, top - 0.12), cuts, 48)
    for a, pz, bore in ports:
        _berth(b, m, (0, 0, pz), a, drum, radius, bore, doubler=False)
    gear = bmesh.new()
    for k in range(3):
        p = Vector(_turn(free + 2.1 * k, 0.9, 0, top + 0.6))
        merge(gear, rod(p, p + Vector((0, 0, 0.3)), 0.06, 8))
        merge(gear, rod(p + Vector((0, 0, 0.3)), p + Vector((0, 0, 0.36)), 0.09, 8))
    merge(gear, rod((0, 0, top + 0.72), (0, 0, top + 1.2), 0.04, 6))
    p = Vector(_turn(free + 1.0, 1.2, 0, top + 0.45))
    merge(gear, rod(p, p + Vector((0, 0, 1.3)), 0.018, 6))
    b.put(gear, m["steel"])
    b.put(sphere(0.12, (0, 0, top + 1.28), 10, 5), m["red"])
    _put(b, box((0.2, 0.36, 0.2), (drum + 0.1, 0, top - 0.4), bevel=0.02), m["dark"], (0, 0, 0), free, False)
    _put(b, box((0.02, 0.3, 0.12), (drum + 0.21, 0, top - 0.42)), m["lamp"], (0, 0, 0), free, False)
    b.collider(lathe([(radius + 0.7, 0.0), (radius + 0.7, 0.2), (radius, 0.2), (radius, top), (0.0, top + 0.75)], 12, 4.0))
    return b.finish()


# Habitats


def _radiator(b, m, at, yaw, hull, panels=4, w=3.0, h=1.4):
    """A ground radiator wing along +X of `yaw` from `at`: panels stacked
    accordion-wise between two truss masts on a skid, a manifold and flex
    hoses down each side, diagonal braces behind, and an insulated supply and
    return run `hull` back along -X to a valve box on the shell."""
    pitch = h + 0.14
    high = 0.7 + panels * pitch
    x1 = w + 0.5
    cx = x1 / 2
    skid = bmesh.new()
    for s in (-1, 1):
        merge(skid, box((x1 + 0.6, 0.18, 0.22), (cx, s * 0.45, 0.26)))
    for x in (0.0, cx, x1):
        merge(skid, box((0.18, 1.08, 0.18), (x, 0, 0.24)))
    b.put(moved(skid, at, yaw), m["steel"], smooth=False)
    footing = bmesh.new()
    for x in (0.0, cx, x1):
        merge(footing, box((0.8, 1.4, 0.3), (x, 0, 0.0), tile=1.0))
    b.put(moved(footing, at, yaw), m["sinter"], smooth=False)
    for x in (0.1, x1 - 0.1):
        b.put(moved(truss((x, 0, 0.37), (x, 0, high + 0.3), 0.28, 0.28, panels * 2, 0.035, 0.018), at, yaw), m["steel"], smooth=False)
    edges = bmesh.new()
    for k in range(panels):
        zc = 0.7 + k * pitch + h / 2
        tilt = 0.1 if k % 2 else -0.1
        b.put(moved(moved(box((w, 0.05, h), (0, 0, 0), tile=h), (cx, 0, zc), roll=tilt), at, yaw), m["radiator"], smooth=False)
        rims = box((w + 0.04, 0.07, 0.05), (0, 0, h / 2))
        merge(rims, box((w + 0.04, 0.07, 0.05), (0, 0, -h / 2)))
        merge(edges, moved(rims, (cx, 0, zc), roll=tilt))
    b.put(moved(edges, at, yaw), m["alu"], smooth=False)
    lines = bmesh.new()
    for xm, d in ((0.32, 1), (x1 - 0.32, -1)):
        merge(lines, tube([(xm, 0.14, 0.5), (xm, 0.14, high)], 0.05, 8))
        for k in range(panels):
            zc = 0.7 + k * pitch + h / 2
            merge(lines, tube([(xm, 0.14, zc - 0.12), (xm + d * 0.12, 0.12, zc - 0.02), (xm + d * 0.22, 0.03, zc)], 0.025, 6))
    for s in (-1, 1):
        merge(lines, tube([(0.32, s * 0.15, 0.55), (-hull + 0.35, s * 0.15, 0.55), (-hull + 0.35, s * 0.15, 1.2), (-hull + 0.1, s * 0.15, 1.2)], 0.06, 10))
    b.put(moved(lines, at, yaw), m["pipe_white"])
    braces = bmesh.new()
    for k in range(panels):
        z_lo, z_hi = 0.7 + k * pitch, 0.7 + (k + 1) * pitch
        merge(braces, rod((0.1, -0.14, z_lo), (x1 - 0.1, -0.14, z_hi), 0.02, 6))
        merge(braces, rod((0.1, -0.14, z_hi), (x1 - 0.1, -0.14, z_lo), 0.02, 6))
    for x in (-hull * 0.33, -hull * 0.66):
        merge(braces, box((0.1, 0.5, 0.06), (x, 0, 0.47)))
        merge(braces, box((0.08, 0.08, 0.45), (x, 0, 0.22)))
    b.put(moved(braces, at, yaw), m["steel"], smooth=False)
    b.put(moved(box((0.3, 0.7, 0.55), (-hull + 0.12, 0, 1.2), bevel=0.02), at, yaw), m["hull_grey"], smooth=False)
    b.put(sphere(0.1, tuple(Vector(at) + Vector(_turn(yaw, x1 - 0.1, 0, high + 0.4))), 10, 5), m["red"])
    b.collider(moved(box((x1 + 0.6, 0.7, high + 0.3), (cx, 0, (high + 0.3) / 2)), at, yaw))


FEED_OUT = 1.4


def _feed(b, m, yaw, hull):
    """A utility cabinet on a plinth beside the shell along +X of `yaw`,
    where the base's pipes plug into its outer face: a bulkhead plate with
    boot seals, side doors, vents and a status lamp, two flanged spools
    back into a doubler on the equipment bay, and bollards before it."""
    x0, x1 = hull + 0.6, hull + FEED_OUT
    cx = (x0 + x1) / 2
    b.put(moved(box((x1 - x0 + 0.4, 1.8, 0.15), (cx, 0, 0.075), tile=1.0), (0, 0, 0), yaw), m["sinter"], smooth=False)
    b.put(moved(box((x1 - x0, 1.4, 1.3), (cx, 0, 0.8), bevel=0.02), (0, 0, 0), yaw), m["hull_grey"], smooth=False)
    trim = bmesh.new()
    merge(trim, box((0.04, 1.0, 0.6), (x1 + 0.02, 0, 0.95)))
    for s in (-1, 1):
        merge(trim, box((x1 - x0 - 0.16, 0.03, 1.1), (cx, s * 0.715, 0.8)))
        merge(trim, box((0.06, 0.05, 0.22), (cx + 0.22, s * 0.74, 0.85)))
    merge(trim, box((x1 - x0 + 0.04, 1.44, 0.06), (cx, 0, 1.48)))
    b.put(moved(trim, (0, 0, 0), yaw), m["steel"], smooth=False)
    seals, vents = bmesh.new(), bmesh.new()
    for y, z in ((0.2, 1.06), (-0.2, 0.92)):
        merge(seals, moved(lathe([(0.0, 0.0), (0.2, 0.0), (0.2, 0.05), (0.14, 0.12), (0.0, 0.12)], 20, 1.0), (x1 + 0.04, y, z), pitch=math.pi / 2))
    for k in range(4):
        for s in (-1, 1):
            merge(vents, box((x1 - x0 - 0.3, 0.02, 0.04), (cx, s * 0.735, 0.3 + 0.07 * k)))
    b.put(moved(seals, (0, 0, 0), yaw), m["rubber"])
    b.put(moved(vents, (0, 0, 0), yaw), m["dark"], smooth=False)
    b.put(moved(box((0.03, 0.5, 0.12), (x1 + 0.02, 0, 1.36)), (0, 0, 0), yaw), m["hazard"], smooth=False)
    b.put(moved(box((0.08, 0.08, 0.05), (x1 - 0.1, 0.55, 1.535)), (0, 0, 0), yaw), m["amber"], smooth=False)
    spools, flanges = bmesh.new(), bmesh.new()
    for y in (0.25, -0.25):
        merge(spools, rod((hull - 0.02, y, 1.3), (x0, y, 1.3), 0.08, 12))
        for x in (hull + 0.05, x0 - 0.04):
            merge(flanges, rod((x - 0.02, y, 1.3), (x + 0.02, y, 1.3), 0.14, 16))
    b.put(moved(spools, (0, 0, 0), yaw), m["pipe_white"])
    b.put(moved(flanges, (0, 0, 0), yaw), m["steel"])
    b.put(moved(box((0.07, 0.9, 0.5), (hull, 0, 1.3)), (0, 0, 0), yaw), m["hull_grey"], smooth=False)
    posts = bmesh.new()
    for s in (-1, 1):
        merge(posts, rod((x1 + 0.9, s * 0.85, 0.0), (x1 + 0.9, s * 0.85, 0.9), 0.07, 12))
    b.put(moved(posts, (0, 0, 0), yaw), m["yellow"])
    b.collider(moved(box((x1 - x0, 1.4, 1.45), (cx, 0, 0.725)), (0, 0, 0), yaw))


def tower_feed(at, yaw, radius=4.5, ports=(math.pi,)):
    """Where the base's pipes meet a hab tower's utility cabinet, in its
    parent's frame: the middle of the boot seals' faces, on the ground, and
    the unit vector pointing out of the cabinet."""
    a = yaw + _hatch_angle(ports) - 2 * math.pi / 3
    out = Vector((math.cos(a), math.sin(a), 0.0))
    return Vector(at) + out * (radius + FEED_OUT + 0.16), out


def _hatch_angle(ports):
    return max((TAU * k / 24 for k in range(24)), key=lambda a: _clear(a, ports))


def hab_tower(looks, name, radius=4.5, height=8.0, at=(0, 0, 0), yaw=0.0, parent=None, ports=(math.pi,), decks=2):
    """A vertical habitat on six lander legs, after NASA's surface hab
    concepts: an MMOD-clad pressure shell with ring frames over a louvred
    equipment bay, a quilted blanket dome, round windows with shutters on
    swing arms, a berthing port toward each corridor, an EVA hatch over
    a railed platform and stairs, a caged ladder up the shell and over the
    dome to a railed roof deck with a high-gain dish, antennas and lamps, a
    cable trunk, and a ground radiator wing piped to the shell."""
    m = _mats(looks)
    b = Building(name, parent, at, yaw)
    zb, z0 = 0.95, 1.8
    top = z0 + height
    dome = radius * 0.42
    zp = 2.5
    rf = 1.3 + FLANGE
    hatch = _hatch_angle(ports)
    climb = hatch + 1.0 / radius
    trunk = hatch - 0.35
    wing = hatch - math.pi / 3
    feed = hatch - 2 * math.pi / 3
    tile = TAU * radius / 14
    b.put(lathe([(0.0, zb - 0.4), (radius * 0.6, zb - 0.3), (radius - 0.12, zb - 0.04), (radius, zb)], 72, 2.0), m["hull_grey"], sharp=30)
    b.put(lathe([(radius, zb), (radius, top)], 72, tile), m["mmod"])
    shell = [(radius * (0.02 + 0.98 * math.cos(t)), top + dome * math.sin(t)) for t in [math.pi / 2 * k / 10 for k in range(11)]]
    shell[-1] = (0.0, top + dome)
    b.put(lathe(shell, 72, tile), m["beta"], sharp=35)
    port_cut = math.asin(min(0.95, (rf + 0.12) / radius))
    cuts = [(p, port_cut, zp - rf - 0.2, zp + rf + 0.2) for p in ports] + [(hatch, 0.2, z0 - 0.3, z0 + 2.0), (feed, 0.11, zb, z0 - 0.2)]
    bay = bmesh.new()
    louvres = bmesh.new()
    for start, span in _arcs([(a, h) for a, h, lo, hi in cuts if lo < z0 - 0.4 < hi]):
        merge(bay, lathe([(radius + 0.015, zb + 0.12), (radius + 0.015, z0 - 0.12)], max(2, int(72 * span / TAU)), 1.0, span, start))
        for k in range(int(span * radius / 1.6)):
            a = start + (k + 0.5) * span / int(span * radius / 1.6)
            merge(louvres, moved(box((0.04, 0.7, 0.32), (radius + 0.03, 0, (zb + z0) / 2)), yaw=a))
            for j in range(4):
                merge(louvres, moved(box((0.03, 0.64, 0.025), (radius + 0.055, 0, (zb + z0) / 2 - 0.11 + 0.075 * j)), yaw=a))
    b.put(bay, m["hull_grey"])
    b.put(louvres, m["dark"], smooth=False)
    _bands(b, m["alu"], radius, [zb + 0.06] + [z0 + height * d / decks for d in range(decks)] + [top - 0.06], cuts)
    for p in ports:
        _berth(b, m, (0, 0, zp), p, radius, radius + 1.1)
    # Windows round each deck, shutters mostly open.
    k = 0
    for deck in range(decks):
        z = z0 + deck * height / decks + 1.5
        for i in range(6):
            a = hatch + math.pi / 6 * (1 - deck) + i * math.pi / 3
            if _clear(a, [climb]) < 0.3 or (deck == 0 and _clear(a, ports) < 0.62):
                continue
            _porthole(b, m, _turn(a, radius, 0, z), a, 0.3, 0.0 if k % 4 == 3 else 1.9)
            k += 1
    # EVA hatch over a grated platform, with stairs down and a ladder up.
    _hatch(b, m, _turn(hatch, radius, 0, z0 + 0.05), hatch, 1.0, 1.6)
    deck0 = z0 - 0.08
    _put(b, box((1.5, 2.4, 0.08), (radius + 0.75, 0, deck0 - 0.04), tile=1.0), m["grate"], (0, 0, 0), hatch, False)
    frame = bmesh.new()
    merge(frame, box((1.5, 0.12, 0.16), (radius + 0.75, -1.14, deck0 - 0.16)))
    merge(frame, box((1.5, 0.12, 0.16), (radius + 0.75, 1.14, deck0 - 0.16)))
    merge(frame, box((0.12, 2.4, 0.16), (radius + 1.44, 0, deck0 - 0.16)))
    for s in (-1, 1):
        merge(frame, rod((radius + 1.4, s * 1.1, deck0 - 0.2), (radius + 1.4, s * 1.1, 0.0), 0.05, 8))
        merge(frame, rod((radius + 0.1, s * 1.1, deck0 - 0.9), (radius + 1.3, s * 1.1, deck0 - 0.2), 0.035, 6))
    _put(b, frame, m["steel"], (0, 0, 0), hatch, False)
    railing(b, m, [_turn(hatch, radius + 0.05, -1.15, deck0), _turn(hatch, radius + 1.45, -1.15, deck0), _turn(hatch, radius + 1.45, -0.55, deck0)], 1.0, 0.8)
    railing(b, m, [_turn(hatch, radius + 1.45, 0.55, deck0), _turn(hatch, radius + 1.45, 1.15, deck0), _turn(hatch, radius + 1.0, 1.15, deck0)], 1.0, 0.8)
    steps = max(1, int(round(deck0 / 0.2)))
    stairs(b, m, _turn(hatch, radius + 1.5 + steps * 0.28, 0, 0.0), hatch + math.pi, deck0, 1.0)
    _ladder(b, m, _turn(climb, radius + 0.22, 0, 0.0), climb, deck0, top)
    # Ladder over the dome to the roof deck.
    zd = top + dome + 0.12
    rd = radius * 0.6
    t_hi = math.acos(((rd + 0.1) / radius - 0.02) / 0.98)
    path = []
    for i in range(9):
        t = t_hi * i / 8
        n = Vector((dome * math.cos(t), radius * 0.98 * math.sin(t))).normalized()
        path.append((radius * (0.02 + 0.98 * math.cos(t)) + n.x * 0.25, top + dome * math.sin(t) + n.y * 0.25))
    over = bmesh.new()
    for s in (-1, 1):
        merge(over, tube([Vector(_turn(climb, r_, s * 0.23, z_)) for r_, z_ in path], 0.025, 6))
    for r_, z_ in path[1:]:
        merge(over, rod(_turn(climb, r_, -0.23, z_), _turn(climb, r_, 0.23, z_), 0.016, 6))
    b.put(over, m["yellow"])
    # Roof deck on posts over the dome.
    b.put(lathe([(0.0, zd - 0.08), (rd, zd - 0.08), (rd, zd), (0.0, zd)], 48, 1.0), m["grate"], sharp=40)
    b.put(lathe([(rd - 0.25, zd - 0.28), (rd - 0.02, zd - 0.28), (rd - 0.02, zd - 0.08), (rd - 0.25, zd - 0.08), (rd - 0.25, zd - 0.28)], 48, 1.0), m["hull_grey"], sharp=40)
    posts = bmesh.new()
    for i in range(8):
        a = TAU * (i + 0.5) / 8
        rp = rd - 0.14
        zs = top + dome * math.sqrt(max(0.0, 1.0 - ((rp / radius - 0.02) / 0.98) ** 2))
        merge(posts, rod(_turn(a, rp, 0, zs - 0.05), _turn(a, rp, 0, zd - 0.28), 0.06, 8))
    b.put(posts, m["steel"])
    railing(b, m, [_turn(climb + 0.16 + (TAU - 0.32) * i / 28, rd - 0.05, 0, zd) for i in range(29)], 1.0, 1.4)
    mast = bmesh.new()
    merge(mast, rod((0, 0, zd), (0, 0, zd + 3.2), 0.06, 8))
    for k in range(2):
        merge(mast, rod(_turn(k * math.pi / 2, -0.5, 0, zd + 2.6), _turn(k * math.pi / 2, 0.5, 0, zd + 2.6), 0.015, 6))
    b.put(mast, m["steel"])
    _put(b, box((0.9, 0.6, 0.5), (1.5, 0, zd + 0.25), bevel=0.02), m["mmod"], (0, 0, 0), climb + math.pi / 2, False)
    b.put(sphere(0.12, (0, 0, zd + 3.3), 10, 5), m["red"])
    fins = bmesh.new()
    for k in range(5):
        merge(fins, moved(box((0.04, 0.5, 0.25), (-0.36 + 0.18 * k, 0, 0.62)), _turn(climb + math.pi / 2, 1.5, 0, zd), climb + math.pi / 2))
    b.put(fins, m["dark"], smooth=False)
    # A high-gain dish on a two-axis mount.
    da = climb + math.pi
    base = Vector(_turn(da, 1.3, 0, zd))
    b.put(moved(cone(0.2, 0.13, 0.7, segments=12), tuple(base)), m["hull_grey"])
    _put(b, box((0.12, 0.7, 0.12), (0, 0, 0.75)), m["hull_grey"], tuple(base), da, False)
    for s in (-1, 1):
        _put(b, box((0.1, 0.06, 0.45), (0, s * 0.32, 0.95)), m["hull_grey"], tuple(base), da, False)
    dish = lathe([(0.0, 0.0), (0.3, 0.03), (0.55, 0.1), (0.75, 0.2), (0.73, 0.23), (0.53, 0.13), (0.29, 0.06), (0.0, 0.03)], 32, 1.0)
    for k in range(3):
        a = TAU * k / 3
        merge(dish, rod((0.72 * math.cos(a), 0.72 * math.sin(a), 0.2), (0, 0, 0.62), 0.012, 4))
    merge(dish, cone(0.06, 0.03, 0.14, (0, 0, 0.56), 10))
    b.put(moved(moved(dish, (0, 0, 0), 0, math.pi / 2 - 0.6), tuple(base + Vector((0, 0, 1.05))), da), m["white_frame"])
    for k in range(3):
        a = climb + 2.0 + 1.5 * k
        _put(b, box((0.2, 0.36, 0.18), (rd + 0.05, 0, zd + 1.05), bevel=0.02), m["dark"], (0, 0, 0), a, False)
        _put(b, box((0.02, 0.3, 0.1), (rd + 0.16, 0, zd + 1.03)), m["lamp"], (0, 0, 0), a, False)
    # Cable trunk up the shell.
    _put(b, box((0.12, 0.24, top - zb - 0.3), (radius + 0.06, 0, (zb + top) / 2), tile=1.0), m["tray"], (0, 0, 0), trunk, False)
    clamps = bmesh.new()
    for k in range(int((top - zb) / 1.0)):
        merge(clamps, box((0.16, 0.3, 0.05), (radius + 0.07, 0, zb + 0.6 + k)))
    _put(b, clamps, m["steel"], (0, 0, 0), trunk, False)
    # Lander legs: a telescopic strut to a hardpoint, an A-frame to the lower ring, a footpad.
    for i in range(6):
        a = hatch + math.pi / 6 + i * math.pi / 3
        c = Vector((math.cos(a), math.sin(a), 0))
        t = Vector((-math.sin(a), math.cos(a), 0))
        foot = c * (radius + 1.15) + Vector((0, 0, 0.3))
        hard = c * (radius + 0.12) + Vector((0, 0, z0 + 0.35))
        knee = hard.lerp(foot, 0.55)
        along = (foot - hard).normalized()
        b.put(rod(hard, knee, 0.13, 12), m["hull_grey"])
        b.put(rod(knee - along * 0.25, foot, 0.08, 10), m["alu"])
        b.put(rod(knee - along * 0.03, knee + along * 0.06, 0.155, 12), m["dark"])
        for s in (-1, 1):
            b.put(rod(c * (radius - 0.02) + t * (s * 0.75) + Vector((0, 0, zb + 0.12)), foot + Vector((0, 0, 0.05)), 0.055, 8), m["steel"])
        _put(b, box((0.3, 0.36, 0.42), (radius + 0.02, 0, z0 + 0.35), bevel=0.02), m["hull_grey"], (0, 0, 0), a, False)
        b.put(sphere(0.13, tuple(foot), 10, 5), m["dark"])
        b.put(moved(lathe([(0.0, 0.0), (0.62, 0.0), (0.62, 0.06), (0.5, 0.14), (0.16, 0.22), (0.0, 0.24)], 20, 1.0), (foot.x, foot.y, 0.0)), m["alu"], sharp=40)
    _radiator(b, m, _turn(wing, radius + 2.6, 0, 0.0), wing, 2.6)
    _feed(b, m, feed, radius)
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, z0 + height / 2), "size": (radius * 1.5, radius * 1.5, height), "volume": math.pi * radius ** 2 * height, "glazing": 0.0}))
    b.collider(lathe([(0.0, zb - 0.4), (radius, zb), (radius, top), (0.0, top + dome)], 16, 4.0))
    for i in range(6):
        a = hatch + math.pi / 6 + i * math.pi / 3
        b.collider(box((0.9, 0.9, 0.3), _turn(a, radius + 1.15, 0, 0.15)))
    return b.finish()


def _lobed(radius, height, lobes, a, lat, lift=0.0):
    """A point on the inflatable's lobed skin at azimuth a and latitude lat,
    `lift` proud of it, and the skin's outward normal there."""
    r = radius * math.cos(lat) * (1.0 + 0.035 * math.cos(lobes * a) * math.cos(lat))
    n = Vector((height * math.cos(lat) * math.cos(a), height * math.cos(lat) * math.sin(a), radius * math.sin(lat))).normalized()
    return Vector((r * math.cos(a), r * math.sin(a), 0.5 + height * math.sin(lat) ** 0.85)) + n * lift, n


def inflatable_hab(looks, name, radius=7.5, height=4.6, at=(0, 0, 0), yaw=0.0, parent=None, lobes=18):
    """A soft-goods hab, as in The Martian: a lobed dome of quilted beta
    cloth under a restraint layer of meridian straps and hoop bands, a rigid
    crown with vent valves, a clamp ring on a sintered footing with the straps
    staked to ground anchors, and two rigid airlocks with EVA hatches,
    platforms and steps."""
    m = _mats(looks)
    b = Building(name, parent, at, yaw)
    locks = (0.0, math.pi)
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    seg, rows = lobes * 6, 14
    grid = []
    for j in range(rows + 1):
        if j == rows:
            grid.append([bm.verts.new((0.0, 0.0, 0.5 + height))] * seg)
        else:
            grid.append([bm.verts.new(_lobed(radius, height, lobes, TAU * i / seg, math.pi / 2 * j / rows)[0]) for i in range(seg)])
    for j in range(rows):
        for i in range(seg):
            seen, keep = set(), []
            for v, col in ((grid[j][i], i), (grid[j][(i + 1) % seg], i + 1), (grid[j + 1][(i + 1) % seg], i + 1), (grid[j + 1][i], i)):
                if v not in seen:
                    seen.add(v)
                    keep.append((v, col))
            face = bm.faces.new([v for v, _ in keep])
            for loop, (v, col) in zip(face.loops, keep):
                loop[uv].uv = (TAU * radius * col / seg / 3.0, v.co.z / 3.0)
    b.put(bm, m["beta"])
    # Restraint layer: meridian straps in the lobes' valleys, hoop bands over them.
    webbing = bmesh.new()
    for i in range(lobes):
        a = TAU * (i + 0.5) / lobes
        lat0 = 0.0 if _clear(a, locks) > 0.22 else 0.52
        pts = [_lobed(radius, height, lobes, a, lat0 + (math.radians(84) - lat0) * k / 18, 0.015) for k in range(19)]
        merge(webbing, _strap([p for p, _ in pts], [n for _, n in pts], 0.14, 0.03))
    for lat in (0.3, 0.62, 0.95):
        for start, span in _arcs([(a, 0.24) for a in locks] if lat < 0.5 else []):
            count = max(2, int(seg * 2 * span / TAU))
            pts = [_lobed(radius, height, lobes, start + span * k / count, lat, 0.035) for k in range(count + 1)]
            merge(webbing, _strap([p for p, _ in pts], [n for _, n in pts], 0.1, 0.025))
    anchors = bmesh.new()
    for i in range(lobes):
        a = TAU * (i + 0.5) / lobes
        if _clear(a, locks) <= 0.22:
            continue
        c = Vector((math.cos(a), math.sin(a), 0))
        p = _lobed(radius, height, lobes, a, 0.0, 0.015)[0]
        over = c * (radius + 0.4) + Vector((0, 0, 0.69))
        eye = c * (radius + 0.75) + Vector((0, 0, 0.5))
        up = Vector((0, 0, 1))
        merge(webbing, _strap([p, over, eye], [c * 0.3 + up, up, up + c], 0.14, 0.03))
        merge(anchors, moved(box((0.08, 0.22, 0.14), (0, 0, 0), bevel=0.01), tuple(over + Vector((0, 0, 0.04))), a))
        merge(anchors, moved(box((0.18, 0.18, 0.22), (0, 0, 0)), tuple(c * (radius + 0.78) + Vector((0, 0, 0.41))), a))
    b.put(webbing, m["webbing"])
    b.put(anchors, m["steel"], smooth=False)
    # Sintered footing, the clamp ring the skin is sealed in, and the crown.
    b.put(_pad(radius - 0.4, radius + 1.0, 0.3, 96), m["sinter"], sharp=40)
    clamp = bmesh.new()
    for start, span in _arcs([(a, 0.2) for a in locks]):
        merge(clamp, lathe([(radius + 0.35, 0.28), (radius + 0.35, 0.58), (radius + 0.2, 0.64), (radius - 0.25, 0.64)], max(2, int(96 * span / TAU)), 1.0, span, start))
    b.put(clamp, m["hull_grey"], sharp=40)
    zc = 0.5 + height * math.sin(math.acos(1.4 / radius)) ** 0.85
    b.put(lathe([(1.5, zc - 0.14), (1.5, zc + 0.1), (1.32, zc + 0.2), (0.7, zc + 0.26), (0.0, zc + 0.28)], 48, 1.0), m["alu"], sharp=40)
    valves = bmesh.new()
    for k in range(3):
        p = Vector(_turn(TAU * k / 3 + 0.5, 0.9, 0, zc + 0.18))
        merge(valves, rod(p, p + Vector((0, 0, 0.25)), 0.08, 10))
        merge(valves, rod(p + Vector((0, 0, 0.25)), p + Vector((0, 0, 0.31)), 0.11, 10))
    merge(valves, rod((0, 0, zc + 0.28), (0, 0, zc + 1.4), 0.03, 6))
    b.put(valves, m["hull_grey"])
    b.put(sphere(0.1, (0, 0, zc + 1.48), 10, 5), m["red"])
    # Rigid airlocks through the skin, each to an EVA hatch over a platform and steps.
    for a in locks:
        axis = (0, 0, 1.6)
        xe = radius + 2.2
        _put(b, _axial([(1.3, radius - 1.0), (1.3, xe)], 32, TAU * 1.3 / 4), m["mmod"], axis, a)
        bands = bmesh.new()
        for x in (radius + 0.05, radius + 1.1, xe - 0.1):
            merge(bands, _axial([(1.28, x - 0.09), (1.36, x - 0.08), (1.38, x - 0.06), (1.38, x + 0.06), (1.36, x + 0.08), (1.28, x + 0.09)], 32, 0.5))
        _put(b, bands, m["alu"], axis, a, sharp=30)
        _put(b, _axial([(1.3, xe), (1.3, xe + 0.06), (0.0, xe + 0.06)], 32), m["hull_grey"], axis, a, False)
        rails = bmesh.new()
        for s in (-1, 1):
            merge(rails, tube([(radius + 0.4, s * 1.3, 1.6), (radius + 0.4, s * 1.45, 1.6), (xe - 0.3, s * 1.45, 1.6), (xe - 0.3, s * 1.3, 1.6)], 0.022, 6))
        _put(b, rails, m["yellow"], (0, 0, 0), a)
        _hatch(b, m, _turn(a, xe + 0.06, 0, 0.85), a, 0.9, 1.5)
        px0, px1 = xe + 0.06, xe + 1.36
        _put(b, box((1.3, 2.2, 0.08), ((px0 + px1) / 2, 0, 0.76), tile=1.0), m["grate"], (0, 0, 0), a, False)
        frame = bmesh.new()
        for s in (-1, 1):
            merge(frame, box((1.3, 0.1, 0.14), ((px0 + px1) / 2, s * 1.05, 0.65)))
            merge(frame, rod((px1 - 0.08, s * 1.0, 0.6), (px1 - 0.08, s * 1.0, 0.0), 0.045, 8))
        _put(b, frame, m["steel"], (0, 0, 0), a, False)
        for s in (-1, 1):
            railing(b, m, [_turn(a, px0 + 0.05, s * 1.08, 0.8), _turn(a, px1 - 0.05, s * 1.08, 0.8), _turn(a, px1 - 0.05, s * 0.55, 0.8)], 1.0, 0.7)
        stairs(b, m, _turn(a, px1 + 4 * 0.28, 0, 0.0), a + math.pi, 0.8, 1.0)
        _put(b, box((2.8, 1.6, 0.36), (radius + 0.9, 0, 0.18), tile=1.0), m["sinter"], (0, 0, 0), a, False)
        b.collider(moved(lathe([(0.0, 0.0), (1.3, 0.0), (1.3, 3.5), (0.0, 3.5)], 12, 2.0), tuple(Vector((math.cos(a), math.sin(a), 0)) * (radius - 1.0) + Vector((0, 0, 1.6))), a, math.pi / 2))
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, 2.5), "size": (radius * 1.4, radius * 1.4, height), "volume": 2.0 / 3.0 * math.pi * radius ** 2 * height, "glazing": 0.0}))
    b.collider(lathe([(radius + 0.4, 0.0), (radius + 0.4, 0.55)] + [(radius * math.cos(t), 0.5 + height * math.sin(t) ** 0.85) for t in [math.pi / 2 * i / 5 for i in range(1, 5)]] + [(0.0, 0.5 + height)], 16, 4.0))
    return b.finish()


def _shell(rows, segments, holes, depth, tile):
    """A shell of revolution through (radius, z) rows, open over the cells
    each (column, row, columns, rows) hole lists, every opening lined with a
    reveal `depth` deep; returns the shell and the reveals' back outlines."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    reach = max(r for r, _ in rows)
    grid = [[bm.verts.new((r * math.cos(TAU * i / segments), r * math.sin(TAU * i / segments), z)) for i in range(segments)] for r, z in rows]
    gone = {((i0 + i) % segments, j0 + j) for i0, j0, ni, nj in holes for i in range(ni) for j in range(nj)}
    for j in range(len(rows) - 1):
        for i in range(segments):
            if (i, j) in gone:
                continue
            k = (i + 1) % segments
            face = bm.faces.new([grid[j][i], grid[j][k], grid[j + 1][k], grid[j + 1][i]])
            for loop, (col, row) in zip(face.loops, ((i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1))):
                loop[uv].uv = (TAU * reach * col / segments / tile, rows[row][1] / tile)
    backs = []
    for i0, j0, ni, nj in holes:
        ring = [(i0 + i, j0) for i in range(ni)] + [(i0 + ni, j0 + j) for j in range(nj)] + [(i0 + ni - i, j0 + nj) for i in range(ni)] + [(i0, j0 + nj - j) for j in range(nj)]
        outer = [grid[j][i % segments] for i, j in ring]
        inner = [bm.verts.new(v.co - Vector((v.co.x, v.co.y, 0.0)).normalized() * depth) for v in outer]
        for p in range(len(ring)):
            q = (p + 1) % len(ring)
            face = bm.faces.new([outer[p], outer[q], inner[q], inner[p]])
            for loop, (idx, d) in zip(face.loops, ((p, 0.0), (q, 0.0), (q, depth), (p, depth))):
                i, j = ring[idx]
                loop[uv].uv = ((TAU * reach * i / segments + d) / tile, rows[j][1] / tile)
        backs.append([v.co.copy() for v in inner])
    bm.normal_update()
    return bm, backs


def printed_tower(looks, name, radius=5.0, height=15.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """A habitat printed from regolith, after AI SpaceFactory's MARSHA: an
    upright egg of print layers on a printed footing, a spiral of slot
    windows in deep reveals, a glazed skylight at its tip, an entry airlock
    at +X whose berthing ring meets its corridor, and a printed porch over
    a door at -X."""
    m = _mats(looks)
    b = Building(name, parent, at, yaw)
    n = 60
    rows = []
    for k in range(n + 1):
        t = k / n
        rows.append((radius * (0.92 + 0.18 * math.sin(math.pi * min(t * 1.6, 1.0))) * (1.0 - 0.55 * t ** 2.2) * (1.0 + 0.002 * (-1) ** k), height * t))
    holes = []
    for k in range(12):
        t = 0.12 + 0.68 * k / 12
        holes.append((round((2.0 + 2.4 * k) / TAU * 72) % 72, round(t * n), 2, 6))
    shell, backs = _shell(rows, 72, holes, 0.35, 1.0)
    b.put(shell, m["layers"], sharp=60)
    panes, frames = bmesh.new(), bmesh.new()
    for back in backs:
        panes.faces.new([panes.verts.new(p) for p in back])
        merge(frames, sweep(rect(0.05, 0.05), back + [back[0]], 1.0))
    panes.normal_update()
    project_uv(panes, 1.0)
    b.put(panes, m["glazing"], smooth=False)
    b.put(frames, m["dark"], smooth=False)
    # Skylight: a printed lip round a glazed cap with mullions.
    r1, z1 = rows[-1]
    b.put(lathe([(r1, z1), (r1 + 0.03, z1 + 0.12), (r1 - 0.12, z1 + 0.2), (r1 - 0.3, z1 + 0.18)], 72, 1.0), m["layers"], sharp=50)
    rs = r1 - 0.32
    b.put(lathe([(rs + 0.06, z1 + 0.1), (rs + 0.06, z1 + 0.26), (rs - 0.04, z1 + 0.3)], 48, 0.5), m["alu"], sharp=40)
    cap = [(rs * math.cos(t), z1 + 0.28 + 0.5 * math.sin(t)) for t in [math.pi / 2 * k / 6 for k in range(7)]]
    cap[-1] = (0.0, z1 + 0.78)
    b.put(lathe(cap, 48, 1.0), m["glazing"])
    mullions = bmesh.new()
    for k in range(8):
        merge(mullions, sweep(rect(0.05, 0.06), [_turn(TAU * k / 8, r_ + 0.02, 0, z_ + 0.02) for r_, z_ in cap[:-1]] + [(0, 0, z1 + 0.81)], 1.0))
    b.put(mullions, m["alu"], smooth=False)
    # A conduit and cable up the shell to a mast at the lip.
    ac = -0.5
    for dy, r_c, mat in ((0.0, 0.06, "pipe_white"), (0.16, 0.035, "cable")):
        b.put(tube([Vector(_turn(ac, r_ + 0.12, dy, z_)) for r_, z_ in rows[1::3]] + [Vector(_turn(ac, r1 - 0.1, dy, z1 + 0.25))], r_c, 8), m[mat])
    clamps = bmesh.new()
    for r_, z_ in rows[2::6]:
        merge(clamps, moved(box((0.16, 0.32, 0.06), (r_ + 0.08, 0.08, z_)), yaw=ac))
    b.put(clamps, m["steel"], smooth=False)
    tip = Vector(_turn(ac + 0.35, r1 - 0.2, 0, z1 + 0.15))
    b.put(rod(tip, tip + Vector((0, 0, 1.8)), 0.04, 6), m["steel"])
    b.put(rod(tip + Vector((0.15, 0, 0.1)), tip + Vector((0.15, 0, 1.3)), 0.015, 4), m["steel"])
    b.put(sphere(0.09, tuple(tip + Vector((0, 0, 1.88))), 10, 5), m["red"])
    b.put(_pad(radius * 0.8, radius * 1.25, 0.3, 72), m["sinter"], sharp=40)
    # Entry airlock: a printed buttress round a steel collar whose ring meets the corridor at 5.4 m.
    face = radius + 0.4
    outline = []
    for k in range(32):
        p = TAU * k / 32
        reach = 1.8 if math.sin(p) >= 0 else min(1.8 / max(abs(math.cos(p)), 1e-6), 1.4 / max(abs(math.sin(p)), 1e-6))
        outline.append((reach * math.cos(p), 1.4 + reach * math.sin(p)))
    b.put(_panel(outline, radius - 1.0, face - 0.28), m["layers"], smooth=False)
    _berth(b, m, (0, 0, 1.4), 0.0, face - 0.45, face, 1.2, doubler=False)
    b.put(box((0.2, 0.4, 0.14), (face - 0.18, 0, 3.15), bevel=0.02), m["dark"], smooth=False)
    b.put(box((0.02, 0.34, 0.06), (face - 0.07, 0, 3.12)), m["lamp"], smooth=False)
    # Porch: a thick printed arch over the door at -X.
    x0, x1 = -(radius * 0.92 + 1.9), -(radius * 0.92 - 0.4)
    arch = [math.pi * i / 16 for i in range(17)]
    outer = [(-2.2 * math.cos(t), 0.3 + 2.95 * math.sin(t)) for t in arch]
    inner = [(-1.75 * math.cos(t), 0.3 + 2.6 * math.sin(t)) for t in arch]
    b.put(_frame(outer, inner, x0, x0 + 0.3, 1.0, closed=False), m["layers"], smooth=False)
    b.put(vault(x0, x1, 2.2, 0.3, 16, 1.0, 2.95 / 2.2), m["layers"])
    porch = vault(x0 + 0.3, x1, 1.75, 0.3, 16, 1.0, 2.6 / 1.75)
    bmesh.ops.reverse_faces(porch, faces=porch.faces[:])
    b.put(porch, m["layers"])
    door(b, m, (-(radius * 1.03 + 0.05), 0, 0.3), math.pi, 1.1, 2.1)
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, height * 0.4), "size": (radius * 1.4, radius * 1.4, height * 0.8), "volume": 0.6 * math.pi * radius ** 2 * height, "glazing": 0.0}))
    b.collider(lathe(rows[::5] + [(0.0, height + 0.8)], 16, 4.0))
    b.collider(box((face - radius + 0.6, 3.6, 3.2), ((face + radius - 1.4) / 2, 0, 1.6)))
    b.collider(box((x1 - x0, 4.4, 3.25), ((x0 + x1) / 2, 0, 1.62)))
    return b.finish()


def _mound(radius, squash, cover, spread, a):
    """A point (y, z) on a regolith berm's section heaped over an arch, at arch angle a."""
    s = math.sin(a)
    return (-(radius + cover + spread * (1.0 - s) ** 2) * math.cos(a), (radius * squash + cover) * s)


def hangar(looks, name, length=22.0, radius=7.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """A rover hangar under regolith: a corrugated steel vault on box ribs,
    buried in a heaped regolith berm between printed portal walls; at +X a
    steel portal round a raised roller door, at -X a bulkhead with a
    personnel airlock; lit bays, a crane rail, racks, a workbench and a
    charging post inside, an apron with bollards and a light mast."""
    m = _mats(looks)
    b = Building(name, parent, at, yaw)
    sq = 0.82
    crown = radius * sq
    cover, spread = 1.3, 4.2
    rear, front = -0.5, length + 0.5
    dw, dh = 3.5, 4.6
    arc = [math.pi * i / 36 for i in range(37)]
    section = [_mound(radius, sq, cover, spread, t) for t in arc]
    run = [0.0]
    for (ya, za), (yb, zb) in zip(section, section[1:]):
        run.append(run[-1] + math.hypot(yb - ya, zb - za))
    berm = bmesh.new()
    uv = berm.loops.layers.uv.new("st")
    xs = [rear + (front - rear) * k / 30 for k in range(31)]
    grid = []
    for x in xs:
        fade = min(1.0, (x - rear) / 2.5, (front - x) / 2.5)
        row = []
        for t, (y, z) in zip(arc, section):
            lump = fade * (0.09 * math.sin(0.9 * x + 3.1 * t) + 0.07 * math.sin(2.3 * x - 7.3 * t + 1.0) + 0.05 * math.sin(4.7 * x + 13.0 * t + 2.0) + 0.03 * math.sin(9.1 * x - 23.0 * t))
            d = Vector((y, z)).normalized() * lump
            row.append(berm.verts.new((x, y + d.x, max(0.0, z + d.y))))
        grid.append(row)
    for k in range(len(xs) - 1):
        for i in range(len(arc) - 1):
            face = berm.faces.new([grid[k][i], grid[k + 1][i], grid[k + 1][i + 1], grid[k][i + 1]])
            for loop, (kk, ii) in zip(face.loops, ((k, i), (k + 1, i), (k + 1, i + 1), (k, i + 1))):
                loop[uv].uv = (xs[kk] / 3.0, run[ii] / 3.0)
    b.put(berm, m["regolith"], sharp=70)
    # The vault: corrugated liner facing in, box ribs inside it, printed portal walls at both ends.
    liner = vault(0.0, length, radius, 0.0, 36, 2.0, sq)
    bmesh.ops.reverse_faces(liner, faces=liner.faces[:])
    b.put(liner, m["corrugated"], sharp=60)
    ribs = bmesh.new()
    feet = bmesh.new()
    for k in range(9):
        x = 0.4 + k * (length - 0.8) / 8
        merge(ribs, sweep(rect(0.3, 0.22), [(x, -(radius - 0.16) * math.cos(t), (radius - 0.16) * sq * math.sin(t)) for t in arc], 1.0))
        for s in (-1, 1):
            merge(feet, box((0.5, 0.5, 0.2), (x, s * (radius - 0.3), 0.25)))
    b.put(ribs, m["hull_grey"], sharp=50)
    b.put(feet, m["steel"], smooth=False)
    inner = [(-radius * math.cos(t), crown * math.sin(t)) for t in arc]
    b.put(_frame(section, inner, length, front, 1.0, closed=False), m["layers"], smooth=False)
    b.put(_frame(section, inner, rear, 0.0, 1.0, closed=False), m["layers"], smooth=False)
    # Front bulkhead round the door opening, the portal frame and the raised roller door.
    ta = math.acos(dw / radius)
    jamb = [(-radius * math.cos(t), crown * math.sin(t)) for t in [ta * (1 - k / 8) for k in range(9)]]
    pieces = [[(-dw, 0.0)] + jamb, [(dw, 0.0)] + [(-y, z) for y, z in jamb], [(-dw, dh), (dw, dh)] + [(-radius * math.cos(t), crown * math.sin(t)) for t in [math.pi - ta - (math.pi - 2 * ta) * k / 16 for k in range(17)]]]
    for piece in pieces:
        b.put(_panel(piece, length - 0.15, length, 2.0), m["mmod"], smooth=False)
        b.collider(_panel(piece, length - 0.15, length, 4.0))
    b.put(_panel([(-radius * math.cos(t), crown * math.sin(t)) for t in arc], 0.0, 0.15, 2.0), m["mmod"], smooth=False)
    stiff = bmesh.new()
    for y in (-4.5, -3.0, -1.5, 1.5, 3.0, 4.5):
        zt = crown * math.sqrt(1.0 - (y / radius) ** 2) - 0.1
        merge(stiff, box((0.14, 0.12, zt), (-0.07, y, zt / 2)))
    for y in (-6.3, -4.3, 4.3, 5.3, 6.3):
        zt = crown * math.sqrt(1.0 - (y / radius) ** 2) - 0.1
        merge(stiff, box((0.14, 0.12, zt), (length + 0.07, y, zt / 2)))
    merge(stiff, box((0.1, 2 * radius * 0.93, 0.16), (-0.06, 0, 2.6)))
    b.put(stiff, m["hull_grey"], smooth=False)
    services = bmesh.new()
    for y, z, r in ((-2.2, 1.0, 0.09), (-2.5, 1.0, 0.09), (2.4, 0.8, 0.06)):
        merge(services, tube([(-0.05, y, z), (rear - 0.3, y, z), (rear - 0.3, y, 0.25), (rear - 1.6, y, 0.25)], r, 10))
    b.put(services, m["pipe_white"])
    b.put(box((0.3, 0.8, 0.9), (-0.2, 3.6, 1.2), bevel=0.02), m["hull_grey"], smooth=False)
    b.put(tube([(-0.1, 0.0, 4.6), (rear - 0.4, 0.0, 4.6), (rear - 0.4, 0.0, 5.6)], 0.16, 12), m["steel"])
    b.put(moved(cone(0.26, 0.2, 0.2, segments=12), (rear - 0.4, 0.0, 5.6)), m["dark"])
    portal = bmesh.new()
    for s in (-1, 1):
        merge(portal, box((0.45, 0.5, dh + 0.5), (length + 0.22, s * (dw + 0.25), (dh + 0.5) / 2), tile=1.0))
        merge(portal, box((0.15, 0.14, dh), (length + 0.52, s * (dw + 0.02), dh / 2)))
    merge(portal, box((0.45, 2 * dw + 1.0, 0.5), (length + 0.22, 0, dh + 0.25), tile=1.0))
    merge(portal, box((0.55, 2 * dw + 0.6, 0.55), (length + 0.62, 0, dh + 0.33), tile=1.0, bevel=0.03))
    b.put(portal, m["hull_grey"], smooth=False)
    b.put(box((0.06, 2 * dw - 0.05, 0.9), (length + 0.52, 0, dh - 0.45), tile=0.5), m["corrugated"], smooth=False)
    b.put(box((0.12, 2 * dw - 0.05, 0.1), (length + 0.52, 0, dh - 0.95), tile=0.5), m["hazard"], smooth=False)
    door(b, m, (length + 0.01, -(dw + 1.8), 0.15), 0.0, 0.9, 2.0)
    stripes = bmesh.new()
    for s in (-1, 1):
        merge(stripes, box((0.47, 0.52, 1.2), (length + 0.22, s * (dw + 0.25), 0.6), tile=0.5))
        merge(stripes, box((0.3, 0.3, 0.9), (front + 0.7, s * (dw + 0.7), 0.45), tile=0.5))
    merge(stripes, box((0.5, 2 * dw, 0.02), (length + 0.25, 0, 0.16), tile=0.5))
    b.put(stripes, m["hazard"], smooth=False)
    for s in (-1, 1):
        b.put(box((0.22, 0.42, 0.2), (length + 1.0, s * (dw - 0.6), dh + 0.42), bevel=0.02), m["dark"], smooth=False)
        b.put(box((0.02, 0.36, 0.12), (length + 1.12, s * (dw - 0.6), dh + 0.4)), m["lamp"], smooth=False)
    b.put(box((0.16, 0.16, 0.14), (length + 0.62, 0, dh + 0.68)), m["dark"], smooth=False)
    b.put(sphere(0.1, (length + 0.62, 0, dh + 0.82), 10, 5), m["amber"])
    # Rear personnel airlock through the bulkhead.
    b.put(box((2.4, 2.4, 2.8), (rear - 1.2, 0, 1.4), tile=2.0, bevel=0.04), m["mmod"], smooth=False)
    b.put(box((2.6, 2.6, 0.12), (rear - 1.2, 0, 2.86), tile=1.0), m["hull_grey"], smooth=False)
    door(b, m, (rear - 2.42, 0, 0.0), math.pi, 1.0, 2.1)
    door(b, m, (0.16, 0, 0.15), 0.0, 1.0, 2.1)
    # Inside: floor and lanes, crane rail and hoist, bay lamps, racks, bench, charging post, conduits.
    b.put(box((front, radius * 1.9, 0.15), (front / 2, 0, 0.075), tile=2.0), m["floor"], smooth=False)
    lanes = bmesh.new()
    for s in (-1, 1):
        merge(lanes, box((length - 2.0, 0.12, 0.01), (length / 2 + 0.5, s * dw, 0.155)))
    merge(lanes, box((0.12, 2 * dw, 0.01), (2.0, 0, 0.155)))
    b.put(lanes, m["yellow"], smooth=False)
    b.put(box((length - 1.0, 0.3, 0.45), (length / 2, 0, crown - 0.65)), m["yellow"], smooth=False)
    hang = bmesh.new()
    for k in range(5):
        x = 1.0 + k * (length - 2.0) / 4
        merge(hang, rod((x, 0, crown - 0.45), (x, 0, crown - 0.05), 0.04, 6))
    merge(hang, rod((length * 0.62, 0, crown - 1.0), (length * 0.62, 0, crown - 2.2), 0.012, 4))
    b.put(hang, m["steel"])
    b.put(box((0.6, 0.5, 0.35), (length * 0.62, 0, crown - 1.05), bevel=0.02), m["dark"], smooth=False)
    b.put(box((0.18, 0.12, 0.2), (length * 0.62, 0, crown - 2.3)), m["yellow"], smooth=False)
    for k in range(6):
        x, y = 4.0 + 6.0 * (k // 2), 2.2 * (1 if (k + k // 2) % 2 == 0 else -1)
        zl = crown * math.sqrt(1.0 - (y / radius) ** 2) - 0.6
        b.put(rod((x, y, zl + 0.08), (x, y, zl + 0.6), 0.015, 4), m["steel"])
        b.put(box((1.2, 0.3, 0.1), (x, y, zl + 0.05)), m["dark"], smooth=False)
        b.put(box((1.1, 0.24, 0.02), (x, y, zl - 0.005)), m["lamp"], smooth=False)
        if k % 2 == 0:
            b.places.append(("light", {"at": (x, y, zl - 0.15), "kind": "bay"}))
    rack = bmesh.new()
    for x0 in (3.0, 10.0):
        for dx in (0.0, 2.0, 4.0):
            for dy in (-0.4, 0.4):
                merge(rack, box((0.08, 0.08, 2.4), (x0 + dx, -(radius - 1.6) + dy, 1.35)))
        for z in (0.5, 1.3, 2.1):
            merge(rack, box((4.1, 0.9, 0.05), (x0 + 2.0, -(radius - 1.6), z)))
    b.put(rack, m["yellow"], smooth=False)
    crates = bmesh.new()
    for k, (x, z) in enumerate(((3.6, 0.52), (5.1, 0.52), (6.4, 1.32), (4.3, 1.32), (10.8, 0.52), (12.9, 1.32), (11.5, 2.12))):
        merge(crates, box((0.9 + 0.2 * (k % 2), 0.7, 0.55 - 0.1 * (k % 3)), (x, -(radius - 1.6), z + (0.55 - 0.1 * (k % 3)) / 2), tile=1.0))
    b.put(crates, m["container_orange"], smooth=False)
    b.put(box((3.0, 0.9, 0.95), (4.0, radius - 1.6, 0.47), tile=1.0, bevel=0.03), m["hull_grey"], smooth=False)
    b.put(box((3.0, 0.06, 1.2), (4.0, radius - 1.2, 1.75), tile=1.0), m["frame"], smooth=False)
    b.put(box((0.5, 0.4, 1.4), (length - 3.0, radius - 2.2, 0.85), tile=1.0, bevel=0.03), m["hull"], smooth=False)
    b.put(tube([(length - 3.0, radius - 2.0, 1.3), (length - 3.0, radius - 1.6, 1.0), (length - 3.6, radius - 1.6, 0.2)], 0.035, 8), m["cable"])
    b.put(box((0.1, 0.1, 0.06), (length - 3.0, radius - 2.42, 1.4)), m["amber"], smooth=False)
    for s in (-1, 1):
        b.put(tube([(0.4, s * (radius - 0.4), 1.2), (length - 0.4, s * (radius - 0.4), 1.2)], 0.06, 8), m["pipe_white"])
        b.put(tube([(0.4, s * (radius - 0.42), 1.45), (length - 0.4, s * (radius - 0.42), 1.45)], 0.04, 8), m["cable"])
    b.put(box((9.0, 2 * dw + 6.0, 0.06), (front + 4.5, 0, 0.03), tile=2.0), m["pad"], smooth=False)
    light_mast(b, m, (front + 6.5, dw + 4.5, 0), 7.0, 2, math.pi)
    # What stops a rover: the berm, its portal walls, the bulkheads and the floor.
    low = [math.pi * i / 8 for i in range(9)]
    lo_section = [_mound(radius, sq, cover, spread, t) for t in low]
    shell = bmesh.new()
    rows = [[shell.verts.new((x, y, z)) for y, z in lo_section] for x in (rear, front)]
    for i in range(len(low) - 1):
        shell.faces.new([rows[0][i], rows[1][i], rows[1][i + 1], rows[0][i + 1]])
    b.collider(shell)
    lo_inner = [(-radius * math.cos(t), crown * math.sin(t)) for t in low]
    b.collider(_frame(lo_section, lo_inner, length, front, 4.0, closed=False))
    b.collider(_panel(lo_section, rear - 0.01, rear, 4.0))
    b.collider(box((2.4, 2.4, 2.8), (rear - 1.2, 0, 1.4)))
    b.collider(box((front, radius * 1.9, 0.15), (front / 2, 0, 0.075)))
    return b.finish()


def eden_skin(looks, name, length, width, height, door_w, at=(0, 0, 0), yaw=0.0, parent=None):
    """The outside of the EDEN growth module: two high-cube containers end to
    end, corrugated walls between corner posts, side rails and castings; the
    porch door at -X standing open under a canopy and lamp, cargo doors with
    locking bars at +X, chillers piped through the wall, a roof cable tray,
    wall lamps, louvres, and a sandbag berm."""
    m = _mats(looks)
    b = Building(name, parent, at, yaw)
    half = length / 2
    hw = width / 2
    cx = half
    top = height + 0.03
    jamb = door_w / 2 + 0.08
    # Corrugated side walls, the porch end round its door, and the roof.
    walls = bmesh.new()
    for k in range(2):
        for s in (-1, 1):
            merge(walls, _corrugated(k * half + 0.13, (k + 1) * half - 0.13, 0.16, height - 0.14, hw + 0.005, s))
    for y0, y1, z0, z1 in ((-hw + 0.13, -jamb, 0.16, height - 0.2), (jamb, hw - 0.13, 0.16, height - 0.2), (-jamb, jamb, 2.32, height - 0.2)):
        merge(walls, moved(_corrugated(y0, y1, z0, z1, 0.005, 1), yaw=math.pi / 2))
    for y0, y1 in ((-hw + 0.13, -0.02), (0.02, hw - 0.13)):
        merge(walls, moved(_corrugated(-y1, -y0, 0.16, height - 0.14, length + 0.005, 1, 0.02, 0.2), yaw=-math.pi / 2))
    merge(walls, box((length, width - 0.16, 0.03), (cx, 0, height + 0.018), tile=2.4))
    b.put(walls, m["container_white"], smooth=False)
    # Corner posts, side and end rails, castings and fork pockets.
    steel = bmesh.new()
    dark = bmesh.new()
    for k in range(2):
        xa, xb = k * half, (k + 1) * half
        for end, d in ((xa, 1), (xb, -1)):
            lip = 0.045 if (k == 0) == (d > 0) else 0.0
            for s in (-1, 1):
                merge(steel, box((0.13 + lip, 0.14, top - 0.24), (end + d * (0.13 - lip) / 2, s * (hw - 0.02), top / 2)))
                for z in (0.06, top - 0.06):
                    merge(steel, box((0.178 + lip, 0.162, 0.118), (end + d * (0.178 - lip) / 2, s * (hw - 0.01), z)))
                    merge(dark, box((0.1, 0.02, 0.05), (end + d * 0.089, s * (hw + 0.07), z)))
                    if lip:
                        merge(dark, box((0.02, 0.05, 0.1), (end - d * (lip + 0.005), s * (hw - 0.01), z)))
        for s in (-1, 1):
            merge(steel, box((half - 0.26, 0.13, 0.16), ((xa + xb) / 2, s * (hw - 0.015), 0.08)))
            merge(steel, box((half - 0.26, 0.12, 0.14), ((xa + xb) / 2, s * (hw - 0.01), top - 0.07)))
            for f in (-0.3, 0.3):
                merge(dark, box((0.36, 0.02, 0.1), ((xa + xb) / 2 + f * half, s * (hw + 0.055), 0.08)))
    for x in (0.02, length - 0.02):
        merge(steel, box((0.13, width - 0.3, 0.16), (x, 0, 0.08)))
        merge(steel, box((0.13, width - 0.3, 0.2), (x, 0, top - 0.1)))
    b.put(steel, m["container_white"], smooth=False)
    b.put(dark, m["dark"], smooth=False)
    b.put(box((length + 0.02, 0.012, 0.3), (cx, -(hw + 0.045), height - 0.55), tile=1.0), m["container_blue"], smooth=False)
    b.put(box((length + 0.02, 0.012, 0.3), (cx, hw + 0.045, height - 0.55), tile=1.0), m["container_blue"], smooth=False)
    # Cargo doors at +X: locking bars with cam keepers and handles, hinges, a seal.
    gear = bmesh.new()
    for s in (-1, 1):
        for y in (s * 0.28, s * 0.86):
            merge(gear, rod((length + 0.07, y, 0.2), (length + 0.07, y, height - 0.2), 0.02, 6))
            for z in (0.24, height - 0.24):
                merge(gear, box((0.05, 0.1, 0.09), (length + 0.05, y, z)))
            for z in (0.8, 1.6, 2.3):
                merge(gear, box((0.04, 0.08, 0.04), (length + 0.05, y, z)))
            merge(gear, box((0.04, 0.05, 0.42), (length + 0.1, y + s * 0.08, 1.15)))
        for z in (0.45, 1.1, 1.8, 2.45):
            merge(gear, box((0.08, 0.07, 0.14), (length + 0.04, s * (hw - 0.09), z)))
    b.put(gear, m["steel"], smooth=False)
    b.put(box((0.03, 0.04, height - 0.36), (length + 0.03, 0, height / 2)), m["rubber"], smooth=False)
    # Porch door: frame, the leaf swung open, sill step, canopy and lamp.
    frame = bmesh.new()
    for s in (-1, 1):
        merge(frame, box((0.1, 0.1, 2.3), (-0.05, s * (door_w / 2 + 0.05), 1.15)))
    merge(frame, box((0.1, door_w + 0.2, 0.12), (-0.05, 0, 2.26)))
    b.put(frame, m["frame"], smooth=False)
    leaf = box((0.06, door_w - 0.02, 2.12), (0.0, door_w / 2, 1.1), tile=1.0)
    merge(leaf, box((0.03, 0.16, 0.05), (-0.045, door_w - 0.12, 1.05)))
    merge(leaf, box((0.02, door_w - 0.2, 0.3), (-0.035, door_w / 2, 0.2), tile=1.0))
    b.put(moved(leaf, (-0.12, -door_w / 2, 0.0), math.radians(100)), m["hull"], smooth=False)
    b.put(rod((-0.11, -door_w / 2 - 0.02, 0.15), (-0.11, -door_w / 2 - 0.02, 2.1), 0.025, 6), m["steel"])
    b.put(box((0.9, door_w + 0.6, 0.12), (-0.5, 0, 0.06), tile=1.0), m["grate"], smooth=False)
    b.put(box((0.2, door_w + 0.6, 0.03), (-0.85, 0, 0.13), tile=0.5), m["hazard"], smooth=False)
    b.put(box((0.9, door_w + 0.7, 0.05), (-0.45, 0, 2.62)), m["hull_grey"], smooth=False)
    for s in (-1, 1):
        b.put(rod((-0.02, s * (door_w / 2 + 0.25), 2.3), (-0.8, s * (door_w / 2 + 0.25), 2.6), 0.02, 6), m["steel"])
    b.put(box((0.12, 0.3, 0.08), (-0.2, 0, 2.55)), m["lamp"], smooth=False)
    b.places.append(("light", {"at": (-0.5, 0, 2.4), "kind": "door"}))
    # Chillers on the -Y side, piped through glands in the wall.
    pipes = bmesh.new()
    glands = bmesh.new()
    for k in range(3):
        x = 2.0 + k * 3.6
        unit(b, m, (x, -(hw + 0.9), 0), 0.0, (2.2, 1.0, 1.2), 2)
        for d in (-0.35, 0.35):
            merge(pipes, tube([(x + d, -(hw + 0.42), 0.9), (x + d, -(hw + 0.3), 1.45), (x + d, -(hw + 0.07), 1.45)], 0.05, 8))
        merge(glands, box((0.95, 0.06, 0.24), (x, -(hw + 0.06), 1.45), bevel=0.01))
    b.put(pipes, m["pipe_white"])
    b.put(glands, m["hull_grey"], smooth=False)
    # Louvres and wall lamps.
    vents, slats = bmesh.new(), bmesh.new()
    for x in (0.9, length - 0.9):
        merge(vents, box((0.5, 0.04, 0.5), (x, -(hw + 0.06), 2.25)))
        for k in range(5):
            merge(slats, box((0.44, 0.03, 0.03), (x, -(hw + 0.09), 2.07 + 0.09 * k)))
    b.put(vents, m["hull_grey"], smooth=False)
    b.put(slats, m["dark"], smooth=False)
    for x in (3.8, 9.2):
        b.put(box((0.3, 0.14, 0.2), (x, -(hw + 0.12), 2.35), bevel=0.02), m["dark"], smooth=False)
        b.put(box((0.26, 0.02, 0.08), (x, -(hw + 0.2), 2.3)), m["lamp"], smooth=False)
    # Roof: a ladder tray of cables and lines on feet, dropping down the porch end.
    tray = bmesh.new()
    for s in (-1, 1):
        merge(tray, box((length - 1.0, 0.03, 0.08), (cx, 0.6 + s * 0.2, height + 0.2)))
    for k in range(int((length - 1.0) / 0.3) + 1):
        merge(tray, box((0.04, 0.4, 0.02), (0.5 + 0.3 * k, 0.6, height + 0.17)))
    for k in range(9):
        merge(tray, box((0.06, 0.5, 0.12), (0.6 + k * (length - 1.2) / 8, 0.6, height + 0.09)))
    merge(tray, box((0.03, 0.36, height - 0.4), (-0.1, 0.85, height / 2 + 0.1)))
    b.put(tray, m["tray"], smooth=False)
    for k in range(3):
        b.put(tube([(length - 0.5, 0.48 + 0.12 * k, height + 0.22), (0.5, 0.48 + 0.12 * k, height + 0.22), (-0.06, 0.48 + 0.12 * k, height + 0.22), (-0.06, 0.48 + 0.12 * k, 1.0)], 0.03, 6), m["cable"])
    b.put(tube([(0.5, -0.6, height + 0.2), (length - 0.5, -0.6, height + 0.2)], 0.07, 8), m["cable"])
    b.put(tube([(0.5, -0.3, height + 0.45), (length - 0.5, -0.3, height + 0.45)], 0.07, 8), m["pipe_white"])
    b.put(box((0.12, 0.4, 0.5), (-0.1, 0.85, 1.0), bevel=0.02), m["hull_grey"], smooth=False)
    b.put(box((length + 1.6, width + 3.6, 0.15), (cx, 0, 0.0), tile=2.0), m["pad"], smooth=False)
    berm = bmesh.new()
    for k in range(int(length / 0.8)):
        for row in range(3):
            merge(berm, box((0.78, 0.55, 0.34), (0.4 + k * 0.8 + (row % 2) * 0.4, hw + 0.45, 0.17 + row * 0.33), tile=0.6, bevel=0.1))
    b.put(berm, m["bags"])
    b.collider(box((length, 0.6, 1.0), (cx, hw + 0.45, 0.5)))
    for k in range(3):
        b.collider(box((2.2, 1.0, 1.2), (2.0 + k * 3.6, -(hw + 0.9), 0.6)))
    return b.finish()
