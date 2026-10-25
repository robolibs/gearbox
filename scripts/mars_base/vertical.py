"""The base's vertical farm: a plant factory of leafy greens grown in
stacked trays under their own LEDs, after the automated farms that work
their racks with stacker cranes (Spread's Techno Farm, Jones Food, the IGS
growth towers) and sized to NASA's BVAD crop areas.

The hall is a catenary vault printed from regolith along +X, its shell
0.8 m thick for shielding, ribbed outside on a sintered footing and
windowless. Inside, a white pressure liner on steel ribs; a rigid lock at -X
meets the corridor flange to flange as the arch greenhouses' do. Between
two rows of racks runs one aisle where a twin-mast stacker crane (`crane`)
travels on a floor rail and a top guide rail hung from the aisle ties.

    x  0.6 .. 2.7   entry: the inner hatch, germination chamber, seeding bench
       2.7          the crane aisle's fence and gate
       4.0 .. 24.8  racks: one nursery bay of nine seedling levels, seven
                    main bays of seven levels, each level a 2.4 x 0.6 m tray
                    under its own LED fixture, air blown across it from a
                    plenum at the rack's back
      24.8 .. 27.6  the workstation (conveyor stub and packing bench) and
                    the fertigation skid, both in the crane's reach
      27.6 .. 29.9  the air handler and the power distribution

Everything is in the hall's frame: the -X end wall's outer face at x = -0.1,
the aisle on y = 0, z up from the ground; the floor's top at FLOOR.
"""
import math

import bmesh
import numpy as np
from mathutils import Vector

import patterns
from buildings import _drum, _hash, _hatch, _lock, _rounded, _uv
from habitat import _mats as habitat_mats, _panel, _strap
from industry import angle, cabinet, channel, gauge, member, pipe, plant_mats, sign, valve
from kit import Building, box, cone, disc, lathe, merge, moved, rod, sphere, sweep, tube
from mechanics import finish

FLOOR = 0.12
SPAN = 7.0
CROWN = 8.6
GAP = 0.12
SHELL = 0.8
X0, X1 = 0.6, 29.9
END = 0.7
LOCK_FACE = -3.2
FOOT_X = (-0.3, 30.65)
OPENING = (2.9, 3.08)

AISLE = 1.2
FRONT = AISLE / 2
POST = 0.08
TRAY = (2.4, 0.6)
TRAY_Y = FRONT + 0.03 + TRAY[1] / 2
BACK = FRONT + 0.66
PLENUM = (BACK + 0.04, BACK + 0.18)
BAY = 2.6
X_RACK = 4.0
BAYS = ("nursery",) + ("main",) * 7
# Each kind of bay: levels, pitch between tray tops, lamp height over a tray.
LEVELS = {"main": (7, 0.62, 0.42), "nursery": (9, 0.48, 0.28)}
Z_TRAY = FLOOR + 0.48
BEAM = (0.05, 0.08)
POST_TOP = FLOOR + 5.73
TIE = (FLOOR + 5.58, FLOOR + 5.68)
PORTALS = (2.7, 27.6)

# Where the crane's machine frame stands at travel 0, and how far it travels.
CRANE_X0 = X_RACK + 0.2
CRANE_TRAVEL = X_RACK + len(BAYS) * BAY + 1.3 - CRANE_X0
# Outside: where a ground cable ends at the feed cabinet's gland.
FEED = Vector((27.0, 6.25, 0.0))
BERM = (0.75, 1.1)


def frames():
    """The X of each rack frame, -X first."""
    return [X_RACK + BAY * k for k in range(len(BAYS) + 1)]


def tops(kind):
    """The tray tops of a kind of bay, lowest first."""
    n, pitch, _ = LEVELS[kind]
    return [Z_TRAY + pitch * i for i in range(n)]


def ties():
    """The X of every aisle tie the crane's top rail hangs from."""
    return [PORTALS[0], *frames(), PORTALS[1]]


def crane_place(at=(0, 0, 0), yaw=0.0):
    """Where the crane's machine frame stands in the hall's parent frame, and its yaw."""
    c, s = math.cos(yaw), math.sin(yaw)
    return (at[0] + c * CRANE_X0, at[1] + s * CRANE_X0, at[2] + FLOOR), yaw


def power_tie(at=(0, 0, 0), yaw=0.0):
    """Where a ground cable ends at the feed cabinet's gland, in the hall's
    parent frame, on the ground, and the unit vector pointing away from it."""
    c, s = math.cos(yaw), math.sin(yaw)
    p = Vector(at) + Vector((c * FEED.x - s * FEED.y, s * FEED.x + c * FEED.y, 0.0))
    return p, Vector((-s, c, 0.0))


# Section


def _sag():
    """The catenary's parameter c: c (cosh(SPAN / 2c) - 1) = CROWN."""
    lo, hi = 0.1, 50.0
    for _ in range(80):
        c = (lo + hi) / 2
        if c * (math.cosh(SPAN / 2 / c) - 1.0) > CROWN:
            lo = c
        else:
            hi = c
    return (lo + hi) / 2


C = _sag()


def arch_z(y):
    """The liner's height over |y| from the aisle."""
    return FLOOR + CROWN - C * (math.cosh(y / C) - 1.0)


def arch_y(z):
    """|y| of the liner at height z."""
    return C * math.acosh(1.0 + max(FLOOR + CROWN - z, 0.0) / C)


def area():
    """The liner's section above the floor, m²."""
    a = SPAN / 2
    return 2.0 * ((CROWN + C) * a - C * C * math.sinh(a / C))


def section(off=0.0, n=48, ground=None):
    """The liner's section, or the curve `off` outside it, from -Y over the
    crown to +Y: (y, z) points evenly spaced along it, their outward normals
    and the length along it to each; with `ground`, its ends run on down to
    that height."""
    fine = []
    for i in range(1601):
        y = -SPAN / 2 + SPAN * i / 1600
        out = Vector((math.sinh(y / C), 1.0)).normalized()
        fine.append((Vector((y, arch_z(y))) + out * off, out))
    if ground is not None:
        fine = [(Vector((fine[0][0].x, ground)), fine[0][1])] + fine + [(Vector((fine[-1][0].x, ground)), fine[-1][1])]
    run = [0.0]
    for (a, _), (c, _) in zip(fine, fine[1:]):
        run.append(run[-1] + (c - a).length)
    pts, normals, arcs = [], [], []
    j = 0
    for k in range(n + 1):
        s = run[-1] * k / n
        while j < len(run) - 2 and run[j + 1] < s:
            j += 1
        t = (s - run[j]) / max(run[j + 1] - run[j], 1e-9)
        p = fine[j][0].lerp(fine[j + 1][0], t)
        q = fine[j][1].lerp(fine[j + 1][1], t).normalized()
        pts.append((p.x, p.y))
        normals.append((q.x, q.y))
        arcs.append(s)
    return pts, normals, arcs


def _skin(pts, arcs, x0, x1, tile, inward=False):
    """The surface a section sweeps from x0 to x1, mapped along X and along the section."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    ends = [[bm.verts.new((x, y, z)) for y, z in pts] for x in (x0, x1)]
    for i in range(len(pts) - 1):
        quad = [(0, i), (1, i), (1, i + 1), (0, i + 1)]
        if inward:
            quad = quad[::-1]
        face = bm.faces.new([ends[a][b] for a, b in quad])
        for loop, (a, b) in zip(face.loops, quad):
            loop[uv].uv = ((x0, x1)[a] / tile, arcs[b] / tile)
    bm.normal_update()
    return bm


def _above(pts, top):
    """The convex outline of a section above the line z = top."""
    out = []
    for (ya, za), (yb, zb) in zip(pts, pts[1:]):
        if za >= top:
            out.append((ya, za))
        if (za - top) * (zb - top) < 0:
            t = (top - za) / (zb - za)
            out.append((ya + (yb - ya) * t, top))
    if pts[-1][1] >= top:
        out.append(pts[-1])
    return out


def _side(pts, s, w, top, ground):
    """The convex outline under a section on the side s beyond |y| = w,
    below z = top and down to `ground`."""
    half = pts if s < 0 else [(-y, z) for y, z in reversed(pts)]
    out = []
    for (ya, za), (yb, zb) in zip(half, half[1:]):
        if za <= top and ya <= -w:
            out.append((ya, max(za, ground)))
        if za < top <= zb:
            t = (top - za) / (zb - za)
            out.append((ya + (yb - ya) * t, top))
            break
    out += [(-w, top), (-w, ground)]
    return out if s < 0 else [(-y, z) for y, z in out]


def _perforated():
    """Perforated sheet: a staggered grid of round holes, 40 to a tile, dark through."""
    u, v = patterns.grid()
    n = 40
    row = np.floor(v * n)
    pu = (u * n + 0.5 * (row % 2)) % 1.0 - 0.5
    pv = (v * n) % 1.0 - 0.5
    hole = (np.hypot(pu, pv) < 0.3).astype(np.float32)
    grime = patterns.noise(cells=8, seed=71)
    sheet = np.array([0.62, 0.63, 0.63], np.float32) * (0.92 + 0.1 * grime)[..., None]
    diff = sheet * (1.0 - 0.85 * hole)[..., None]
    rough = np.clip(0.42 + 0.2 * grime + 0.4 * hole, 0.0, 1.0)
    return diff.astype(np.float32), patterns.normals_from_height(-hole, 2.0), rough.astype(np.float32)


def _wire_mesh():
    """Welded wire mesh of a machine guard: 4 mm wires on a 50 mm pitch, 20 to a tile, open between."""
    u, v = patterns.grid()
    n = 20
    du, dv = np.abs((u * n) % 1.0 - 0.5), np.abs((v * n) % 1.0 - 0.5)
    wire = ((du > 0.42) | (dv > 0.42)).astype(np.float32)
    colour = np.array([0.08, 0.08, 0.085], np.float32) * (0.9 + 0.2 * patterns.noise(cells=8, seed=73))[..., None]
    diff = np.concatenate([colour, wire[..., None]], axis=-1)
    return diff.astype(np.float32), patterns.normals_from_height(wire, 1.5), np.full(wire.shape, 0.45, np.float32)


def _mats(looks):
    """The plant's materials, the habitat's printed regolith and panels, and the farm's own."""
    m = dict(plant_mats(looks))
    h = habitat_mats(looks)
    m.update({k: h[k] for k in ("layers", "sinter", "mmod")})
    g = looks.get
    m.update({
        "wire": g("guard_mesh", (0.08, 0.08, 0.085), 0.45, 0.6, maps=_wire_mesh),
        "perforated": g("perforated_sheet", (0.62, 0.63, 0.63), 0.5, 0.8, maps=_perforated),
        "rack": g("rack_white", (0.88, 0.88, 0.86), 0.5, 0.0,
                  maps=lambda: patterns.panels((0.86, 0.86, 0.84), cols=1, rows=1, seam_w=0.0, rivets=False, grime=0.05, seed=72)),
        "epoxy": g("epoxy_floor", (0.55, 0.57, 0.56), 0.45, 0.0, "hangar_concrete_floor", contrast=0.35),
        "worktop": g("worktop", (0.78, 0.79, 0.80), 0.3, 1.0, maps=finish((0.78, 0.79, 0.80), 0.3, 74, brush=0.06, grime=0.03)),
    })
    return m


# Shell


def _berm(s, base, x0, x1):
    """Loose regolith heaped against the flank on the side s, from inside the
    shell's foot out to the ground at its angle of repose, lumpy, its ends
    run down to nothing."""
    h, spread = BERM
    bm = bmesh.new()
    nx, cols = int((x1 - x0) / 0.5), 7
    grid = []
    for i in range(nx + 1):
        x = x0 + (x1 - x0) * i / nx
        fade = min(1.0, (x - x0) / 1.8, (x1 - x) / 1.8)
        row = []
        for j in range(cols + 1):
            t = j / cols
            lump = (_hash(i, j, 3.0 + s) - 0.5) * 0.14 if 0 < j < cols else 0.0
            z = max(fade * (h * (1.0 - t) ** 1.25 + lump), 0.0)
            row.append(bm.verts.new((x, s * (base - 0.15 + (spread + 0.15) * t + 0.4 * lump), z)))
        grid.append(row)
    for i in range(nx):
        for j in range(cols):
            quad = [grid[i][j], grid[i + 1][j], grid[i + 1][j + 1], grid[i][j + 1]]
            bm.faces.new(quad if s > 0 else quad[::-1])
    return _uv(bm, 1.5)


def _shell(b, m):
    """The printed vault and its ribs, the end walls round the lock's
    opening, the liner on its steel ribs, the inner bulkhead and hatch, the
    floor, the footings and the lock."""
    out_pts, out_n, out_arc = section(GAP + SHELL, 48, ground=-0.1)
    b.put(_skin(out_pts, out_arc, X0 - 0.02, X1 + 0.02, 1.0), m["layers"], sharp=60)
    for k in range(1, 8):
        x = X0 + (X1 - X0) * k / 8
        path = [Vector((x, y, z)) for y, z in out_pts]
        b.put(_strap(path, [Vector((0.0, ny, nz)) for ny, nz in out_n], 0.55, 0.3), m["layers"], sharp=50)
    lip, _, _ = section(GAP + SHELL + 0.14, 40, ground=-0.1)
    w, top = OPENING[0] / 2, OPENING[1]
    for s in (-1, 1):
        b.put(_panel(_side(lip, s, w, top, -0.1), -0.1, X0, 1.0), m["layers"], smooth=False)
    b.put(_panel(_above(lip, top), -0.1, X0, 1.0), m["layers"], smooth=False)
    b.put(_panel(lip, X1, X1 + END, 1.0), m["layers"], smooth=False)
    # The lock's way through the wall and its bolted collar.
    hz, zc = 2.95, 0.05 + 2.95 / 2
    lining = sweep(_rounded(2.8, hz, 0.6), [(-0.1, 0, zc), (X0 + 0.02, 0, zc)], 2.0)
    bmesh.ops.reverse_faces(lining, faces=lining.faces[:])
    b.put(lining, m["hull_grey"], sharp=50)
    b.put(sweep(_rounded(3.3, hz + 0.4, 0.75), [(-0.26, 0, zc), (-0.1, 0, zc)], 1.0), m["hull_grey"], sharp=50)
    bolts = bmesh.new()
    ring = _rounded(3.12, hz + 0.24, 0.68)
    for i in range(0, len(ring), 2):
        up, across = ring[i]
        merge(bolts, rod((-0.28, -across, zc + up), (-0.26, -across, zc + up), 0.022, 6))
    b.put(bolts, m["steel"])
    # The liner at both ends, the bulkhead round the inner hatch, the barrel on its ribs.
    liner_pts, liner_n, liner_arc = section(0.0, 48)
    dw, dh = 0.8, FLOOR + 2.42
    for s in (-1, 1):
        b.put(_panel(_side(liner_pts, s, dw, dh, FLOOR), X0, X0 + 0.02, 2.6), m["mmod"], smooth=False)
    b.put(_panel(_above(liner_pts, dh), X0, X0 + 0.02, 2.6), m["mmod"], smooth=False)
    b.put(_panel(liner_pts, X1 - 0.02, X1, 2.6), m["mmod"], smooth=False)
    _hatch(b, m, (X0 + 0.1, 0, FLOOR), 0.0, 1.2, 2.2)
    b.put(_skin(liner_pts, liner_arc, X0, X1, 2.6, inward=True), m["mmod"], sharp=60)
    ribs, plates = bmesh.new(), bmesh.new()
    inner = [Vector((0.0, -ny, -nz)) for ny, nz in liner_n]
    for k in range(12):
        x = X0 + 1.0 + (X1 - X0 - 2.0) * k / 11
        path = [Vector((x, y, z)) for y, z in liner_pts]
        merge(ribs, _strap(path, inner, 0.012, 0.18))
        merge(ribs, _strap([p + n * 0.18 for p, n in zip(path, inner)], inner, 0.12, 0.014))
        for s in (-1, 1):
            merge(plates, box((0.3, 0.3, 0.02), (x, s * (SPAN / 2 - 0.2), FLOOR + 0.01)))
    b.put(ribs, m["rack"], sharp=50)
    b.put(plates, m["steel"], smooth=False)
    b.put(box((X1 - X0, SPAN + 0.4, FLOOR + 0.1), ((X0 + X1) / 2, 0, (FLOOR - 0.1) / 2), tile=2.0), m["epoxy"], smooth=False)
    base = abs(out_pts[0][0])
    for s in (-1, 1):
        b.put(box((FOOT_X[1] - FOOT_X[0], 1.3, 0.3), ((FOOT_X[0] + FOOT_X[1]) / 2, s * (base + 0.25), 0.05), tile=2.0, bevel=0.04), m["sinter"], smooth=False)
    for x in (FOOT_X[0] + 0.45, FOOT_X[1] - 0.45):
        b.put(box((0.9, 2 * base + 0.6, 0.28), (x, 0, 0.04), tile=2.0, bevel=0.04), m["sinter"], smooth=False)
    for s in (-1, 1):
        b.put(_berm(s, base, FOOT_X[0] + 0.6, FOOT_X[1] - 0.6), m["regolith"], sharp=70)
        b.collider(_panel([(s * (base - 0.15), 0.0), (s * (base - 0.15), BERM[0]), (s * (base + BERM[1]), 0.0)], FOOT_X[0] + 0.6, FOOT_X[1] - 0.6, 4.0))
    _lock(b, m, (-0.1, 0, 0), math.pi, -0.1 - LOCK_FACE, 2.8, 0.05, 3.0, 1.2, 1.4)
    return out_pts


# Racks


def _upright(bm, x, y, z0, z1):
    """A rack upright, a C section 80 mm square, on its base plate."""
    merge(bm, member((x, y - POST / 2, z0 + 0.012), (x, y - POST / 2, z1), channel(POST, POST, 0.003), up=(1, 0, 0)))
    merge(bm, box((0.16, 0.14, 0.012), (x, y, z0 + 0.006)))


def _rack_row(b, m, s):
    """One row of racks on the side s of the aisle: upright frames braced
    front to back, step beams and tray supports at every level, each tray's
    LED fixture clipped under the level above with its driver, the feed and
    drain lines, the plenums at the back, the supply duct and cable tray
    over the top."""
    xs = frames()
    yf, yb = s * (FRONT + POST / 2), s * (BACK - POST / 2)
    yr = s * (BACK + 0.02)
    posts, braces, beams, supports, rails, drivers, feeds, drains, cables, plates = (bmesh.new() for _ in range(10))
    for x in xs:
        _upright(posts, x, yf, FLOOR, POST_TOP)
        _upright(posts, x, yb, FLOOR, POST_TOP)
        z, k = FLOOR + 0.25, 0
        while z < POST_TOP - 0.4:
            z_next = min(z + 0.9, POST_TOP - 0.15)
            merge(braces, rod((x, yf, z), (x, yb, z), 0.014, 6))
            a, c = ((x, yf, z), (x, yb, z_next)) if k % 2 == 0 else ((x, yb, z), (x, yf, z_next))
            merge(braces, rod(a, c, 0.012, 6))
            z, k = z_next, k + 1
        merge(braces, rod((x, yf, POST_TOP - 0.15), (x, yb, POST_TOP - 0.15), 0.014, 6))
        merge(feeds, tube([(x - 0.06, s * (BACK + 0.11), FLOOR + 0.15), (x - 0.06, yr, FLOOR + 0.15), (x - 0.06, yr, POST_TOP - 0.7)], 0.018, 8))
        merge(drains, tube([(x + 0.06, s * (BACK + 0.18), FLOOR + 0.12), (x + 0.06, yr, FLOOR + 0.12), (x + 0.06, yr, POST_TOP - 0.7)], 0.026, 10))
        merge(plates, box((0.04, 0.64, 0.04), (x, s * (BACK + 0.32), POST_TOP + 0.02)))
    for k, kind in enumerate(BAYS):
        xa, xc = xs[k] + POST / 2, xs[k + 1] - POST / 2
        xm = (xa + xc) / 2
        _, _, lamp = LEVELS[kind]
        levels = tops(kind)
        decks = levels + [levels[-1] + lamp + 0.2]
        for n, z in enumerate(decks):
            under = z - 0.08
            for y in (s * (FRONT + BEAM[0] / 2), s * (BACK - BEAM[0] / 2)):
                merge(beams, box((xc - xa, BEAM[0], BEAM[1]), (xm, y, under - BEAM[1] / 2), tile=1.0))
                for xe in (xa + 0.02, xc - 0.02):
                    merge(plates, box((0.04, BEAM[0] + 0.01, BEAM[1] + 0.03), (xe, y, under - BEAM[1] / 2), tile=0.3))
            for f in (0.08, 0.36, 0.64, 0.92):
                x = xa + (xc - xa) * f
                merge(supports, member((x, s * (FRONT + BEAM[0]), under - 0.025), (x, s * (BACK - BEAM[0]), under - 0.025), angle(0.025, 0.003)))
            if n == len(levels):
                continue
            z_rail = z + lamp + 0.04
            hang = decks[n + 1] - 0.105 - (z_rail + 0.03)
            for y in (s * (TRAY_Y - 0.18), s * (TRAY_Y + 0.18)):
                merge(rails, box((TRAY[0] - 0.1, 0.03, 0.03), (xm, y, z_rail + 0.015), tile=0.5))
                for f in (0.08, 0.92):
                    merge(rails, box((0.02, 0.025, hang), (xa + (xc - xa) * f, y, z_rail + 0.03 + hang / 2), tile=0.3))
            merge(drivers, box((0.32, 0.09, 0.045), (xm + 0.6, s * (TRAY_Y + 0.18), z_rail + 0.0525), tile=0.3, bevel=0.004))
            merge(cables, tube([(xm + 0.44, s * (TRAY_Y + 0.18), z_rail + 0.05), (xm + 0.3, s * (BACK - 0.03), z_rail + 0.07),
                                (xa + 0.1, s * (BACK - 0.03), z_rail + 0.07), (xs[k] + 0.03, yr, z_rail + 0.07)], 0.006, 5))
            # The tray's feed spigot at its -X end, its drain from the +X end into the riser.
            merge(feeds, tube([(xs[k] - 0.06, yr, z + 0.05), (xa + 0.08, yr, z + 0.05), (xa + 0.08, s * (BACK - 0.08), z + 0.05),
                               (xa + 0.08, s * (BACK - 0.08), z + 0.0)], 0.008, 6))
            merge(drains, tube([(xc - 0.12, s * (BACK - 0.05), z - 0.05), (xc - 0.12, yr, z - 0.05), (xs[k + 1] + 0.06, yr, z - 0.07)], 0.013, 6))
        # The plenum blowing across every level, its neck up into the duct.
        z0, z1 = FLOOR + 0.3, POST_TOP - 0.3
        b.put(box((xc - xa - 0.1, 0.012, z1 - z0), (xm, s * PLENUM[0], (z0 + z1) / 2), tile=1.0), m["perforated"], smooth=False)
        b.put(box((xc - xa - 0.1, PLENUM[1] - PLENUM[0] - 0.012, z1 - z0), (xm, s * ((PLENUM[0] + PLENUM[1]) / 2 + 0.006), (z0 + z1) / 2), tile=1.0),
              m["galv"], smooth=False)
        b.put(box((0.5, PLENUM[1] - PLENUM[0], POST_TOP + 0.13 - z1), (xm, s * (PLENUM[0] + PLENUM[1]) / 2, (z1 + POST_TOP + 0.13) / 2), tile=0.5),
              m["galv"], smooth=False)
        sign(b, m, (xa + 0.14, s * (FRONT - 0.002), Z_TRAY - 0.28), -s * math.pi / 2, 0.16, 0.12, (k + (s > 0)) % 4)
    b.put(posts, m["rack"], sharp=30)
    b.put(braces, m["rack"])
    b.put(beams, m["paint_orange"], smooth=False)
    b.put(plates, m["paint_dark"], smooth=False)
    b.put(supports, m["alu"], smooth=False)
    b.put(rails, m["alu"], smooth=False)
    b.put(drivers, m["paint_dark"], smooth=False)
    b.put(feeds, m["pipe_blue"])
    b.put(drains, m["pipe_white"])
    b.put(cables, m["cable"])
    # The headers under the plenums from the skid's trench, the supply duct and cable tray over the top.
    b.put(tube([(xs[0] - 0.2, s * (BACK + 0.11), FLOOR + 0.15), (PORTALS[1] - 2.6, s * (BACK + 0.11), FLOOR + 0.15)], 0.03, 10), m["pipe_blue"])
    b.put(tube([(xs[0] - 0.2, s * (BACK + 0.18), FLOOR + 0.12), (PORTALS[1] - 2.6, s * (BACK + 0.18), FLOOR + 0.12)], 0.04, 10), m["pipe_white"])
    y0, y1 = s * (FRONT + 0.15), s * (BACK + 0.12)
    yc, w = (y0 + y1) / 2, abs(y1 - y0)
    dx0, dx1 = xs[0] - 0.2, 28.6
    zd = POST_TOP + 0.33
    b.put(box((dx1 - dx0, w, 0.42), ((dx0 + dx1) / 2, yc, zd), tile=1.5), m["galv"], smooth=False)
    seams, hangers = bmesh.new(), bmesh.new()
    x = dx0 + 1.2
    while x < dx1 - 0.1:
        merge(seams, box((0.04, w + 0.04, 0.46), (x, yc, zd)))
        x += 1.2
    for x in (26.2, 28.0):
        for y in (y0, y1):
            merge(hangers, rod((x, y, zd + 0.21), (x, y, arch_z(abs(y)) - 0.18), 0.01, 6))
        merge(hangers, box((0.05, w + 0.1, 0.03), (x, yc, zd - 0.225)))
        merge(hangers, rod((x, y0, zd - 0.24), (x, y0, zd + 0.21), 0.01, 6))
        merge(hangers, rod((x, y1, zd - 0.24), (x, y1, zd + 0.21), 0.01, 6))
    b.put(seams, m["galv"], smooth=False)
    b.put(hangers, m["steel"])
    trays = bmesh.new()
    for y in (s * (BACK + 0.22), s * (BACK + 0.62)):
        merge(trays, box((xs[-1] - xs[0], 0.02, 0.08), ((xs[0] + xs[-1]) / 2, y, POST_TOP + 0.08)))
    for x in [xs[0] + 0.15 * k for k in range(int((xs[-1] - xs[0]) / 0.15) + 1)]:
        merge(trays, box((0.02, 0.4, 0.012), (x, s * (BACK + 0.42), POST_TOP + 0.046)))
    b.put(trays, m["galv"], smooth=False)
    b.put(box((xs[-1] - xs[0], 0.24, 0.05), ((xs[0] + xs[-1]) / 2, s * (BACK + 0.42), POST_TOP + 0.077)), m["cable"], smooth=False)


def _portals(b, m):
    """The aisle ties over every rack frame and the portals at the rail's
    ends, which the crane's top rail hangs from; the fences and gates."""
    steel, posts = bmesh.new(), bmesh.new()
    zt = (TIE[0] + TIE[1]) / 2
    for x in ties():
        merge(steel, member((x - 0.025, -FRONT - POST, zt), (x - 0.025, FRONT + POST, zt), channel(0.05, TIE[1] - TIE[0], 0.006)))
        if x in PORTALS:
            for s in (-1, 1):
                _upright(posts, x, s * (FRONT + POST / 2), FLOOR, POST_TOP)
                merge(steel, rod((x, s * (FRONT + POST / 2), TIE[0] - 0.6), (x, s * 0.25, TIE[0] - 0.01), 0.02, 6))
    b.put(steel, m["galv"], smooth=False)
    b.put(posts, m["rack"], sharp=30)
    frame, mesh, stripes = bmesh.new(), bmesh.new(), bmesh.new()
    for x in PORTALS:
        face = -1.0 if x < 10 else 1.0
        xf = x + face * 0.12
        z0, z1 = FLOOR + 0.1, FLOOR + 2.3
        for y in (-FRONT, -0.05, 0.05, FRONT):
            merge(frame, box((0.04, 0.04, z1 - z0 + 0.1), (xf, y, (z0 + z1) / 2), tile=0.5))
        for z in (z0, (z0 + z1) / 2, z1):
            merge(frame, box((0.04, 2 * FRONT, 0.04), (xf, 0, z), tile=0.5))
        for sy in (-1, 1):
            merge(mesh, box((0.004, FRONT - 0.09, z1 - z0 - 0.04), (xf, sy * (FRONT + 0.05) / 2, (z0 + z1) / 2), tile=1.0))
        b.put(box((0.06, 0.05, 0.08), (xf + face * 0.04, 0.12, FLOOR + 1.2), tile=0.2), m["paint_red"], smooth=False)
        b.put(box((0.03, 0.03, 0.12), (xf + face * 0.04, 0.06, FLOOR + 1.1), tile=0.2), m["steel"], smooth=False)
        merge(stripes, box((0.3, 2 * FRONT, 0.004), (xf + face * 0.17, 0, FLOOR + 0.002), tile=0.5))
    b.put(frame, m["paint_yellow"], smooth=False)
    b.put(mesh, m["wire"], smooth=False)
    b.put(stripes, m["hazard"], smooth=False)
    lines = bmesh.new()
    for s in (-1, 1):
        merge(lines, box((PORTALS[1] - PORTALS[0], 0.05, 0.004), ((PORTALS[0] + PORTALS[1]) / 2, s * (FRONT - 0.04), FLOOR + 0.002)))
    b.put(lines, m["yellow"], smooth=False)
    b.put(box((0.4, 2 * BACK + 0.4, 0.01), (PORTALS[1] - 2.6, 0, FLOOR + 0.005), tile=0.5), m["tread"], smooth=False)


# Equipment


def _air_handler(b, m, x0, x1, y0, y1):
    """An air handler on its plinth: filter, coil and fan sections behind
    hinged access doors, the return grille, condensate drain and controls,
    its supply riser up to the crossover feeding both rack ducts."""
    h = 2.0
    xm, ym = (x0 + x1) / 2, (y0 + y1) / 2
    b.put(box((x1 - x0 + 0.1, y1 - y0 + 0.1, 0.12), (xm, ym, FLOOR + 0.06)), m["paint_dark"], smooth=False)
    b.put(box((x1 - x0, y1 - y0, h), (xm, ym, FLOOR + 0.12 + h / 2), tile=1.0, bevel=0.01), m["paint_white"], smooth=False)
    doors, hw = bmesh.new(), bmesh.new()
    for k in range(3):
        yc = y0 + (y1 - y0) * (k + 0.5) / 3
        wd = (y1 - y0) / 3 - 0.08
        merge(doors, box((0.02, wd, h - 0.2), (x0 - 0.01, yc, FLOOR + 0.12 + h / 2), tile=1.0))
        for z in (0.5, h - 0.4):
            merge(hw, box((0.05, 0.04, 0.1), (x0 - 0.035, yc + wd / 2 - 0.08, FLOOR + 0.12 + z)))
        merge(hw, box((0.03, 0.08, 0.16), (x0 - 0.025, yc - wd / 2 + 0.06, FLOOR + 1.1)))
    b.put(doors, m["paint_white"], smooth=False)
    b.put(hw, m["steel"], smooth=False)
    b.put(box((0.03, (y1 - y0) / 3 - 0.2, 0.9), (x0 - 0.035, y0 + (y1 - y0) / 6, FLOOR + 1.2), tile=0.5), m["louvre"], smooth=False)
    cabinet(b, m, (xm, y1 + 0.14, FLOOR), math.pi / 2, (0.25, 0.6, 0.8), "paint_grey", 1, 1)
    b.put(tube([(xm - 0.3, y0 + 0.2, FLOOR + 0.2), (x0 - 0.2, y0 + 0.2, FLOOR + 0.12), (x0 - 0.2, y0 + 0.2, FLOOR + 0.02)], 0.02, 8), m["pipe_white"])
    top, zd = FLOOR + 0.12 + h, POST_TOP + 0.33
    b.put(box((0.6, 0.6, zd - top), (xm, ym, (top + zd) / 2), tile=1.0), m["galv"], smooth=False)
    ya, yb = ym - 0.3, BACK + 0.15
    b.put(box((0.6, yb - ya, 0.42), (xm, (ya + yb) / 2, zd), tile=1.0), m["galv"], smooth=False)
    b.collider(box((x1 - x0, y1 - y0, h + 0.12), (xm, ym, FLOOR + (h + 0.12) / 2)))
    return (x0 - 0.1, ym, FLOOR + 1.2)


def _fertigation(b, m, x0, x1, y):
    """The fertigation skid off the aisle on its -Y side: stock tanks A and B
    and the acid dose, the mixing tank, the feed pump on its base with its
    valve and gauge, the filter, the dosing pumps and their controller."""
    yc = y - 0.42
    b.put(box((x1 - x0, 0.84, 0.1), ((x0 + x1) / 2, yc, FLOOR + 0.05), tile=1.0), m["galv"], smooth=False)
    tanks = bmesh.new()
    for dx, r, h in ((0.25, 0.2, 0.9), (0.7, 0.2, 0.9), (1.1, 0.14, 0.7), (2.22, 0.26, 1.15)):
        merge(tanks, moved(lathe([(0.0, 0.0), (r, 0.0), (r, h - 0.04), (r - 0.04, h), (0.0, h)], 24, 1.0), (x0 + dx, yc, FLOOR + 0.1)))
        merge(tanks, moved(lathe([(0.0, h), (0.07, h), (0.07, h + 0.05), (0.0, h + 0.05)], 12, 1.0), (x0 + dx, yc, FLOOR + 0.1)))
    b.put(tanks, m["paint_white"])
    px, pz = x0 + 1.5, FLOOR + 0.27
    b.put(box((0.46, 0.3, 0.04), (px, yc, FLOOR + 0.12)), m["paint_dark"], smooth=False)
    b.put(moved(cone(0.1, 0.1, 0.24, segments=16), (px - 0.2, yc, pz), 0, math.pi / 2), m["paint_orange"])
    b.put(moved(cone(0.09, 0.09, 0.05, segments=16), (px + 0.04, yc, pz), 0, math.pi / 2), m["paint_dark"])
    b.put(moved(lathe([(0.0, -0.05), (0.11, -0.05), (0.11, 0.05), (0.0, 0.05)], 20, 1.0), (px + 0.14, yc, pz), 0, math.pi / 2), m["steel"])
    b.put(moved(lathe([(0.0, 0.0), (0.08, 0.0), (0.08, 0.5), (0.0, 0.5)], 16, 1.0), (x0 + 1.86, y - 0.16, FLOOR + 0.12)), m["worktop"])
    doses = bmesh.new()
    for k in range(3):
        merge(doses, box((0.14, 0.12, 0.18), (x0 + 0.25 + 0.42 * k, yc - 0.3, FLOOR + 1.02), tile=0.3, bevel=0.01))
    merge(doses, box((1.3, 0.04, 0.06), (x0 + 0.67, yc - 0.37, FLOOR + 0.9)))
    for dx in (0.05, 1.29):
        merge(doses, box((0.04, 0.04, 0.85), (x0 + dx, yc - 0.37, FLOOR + 0.52)))
    merge(doses, box((0.5, 0.1, 0.3), (x0 + 0.67, yc - 0.34, FLOOR + 1.33), tile=0.3, bevel=0.01))
    for dx in (0.05, 1.29):
        merge(doses, box((0.04, 0.04, 0.4), (x0 + dx, yc - 0.37, FLOOR + 1.12)))
    merge(doses, box((1.3, 0.04, 0.04), (x0 + 0.67, yc - 0.37, FLOOR + 1.32)))
    b.put(doses, m["paint_grey"], smooth=False)
    b.put(box((0.36, 0.01, 0.2), (x0 + 0.67, yc - 0.285, FLOOR + 1.35)), m["vision"], smooth=False)
    lines = bmesh.new()
    merge(lines, pipe([(x0 + 2.22, yc, FLOOR + 0.3), (px + 0.14, yc, FLOOR + 0.3), (px + 0.14, yc, pz)], 0.025))
    merge(lines, pipe([(px + 0.14, yc + 0.12, pz), (px + 0.14, yc + 0.18, pz), (px + 0.14, yc + 0.18, FLOOR + 0.7), (x0 + 1.86, yc + 0.18, FLOOR + 0.7),
                       (x0 + 1.86, y - 0.16, FLOOR + 0.62)], 0.02))
    merge(lines, pipe([(x0 + 1.86, y - 0.24, FLOOR + 0.2), (x0 + 1.86, -BACK - 0.11, FLOOR + 0.2), (x0 + 0.05, -BACK - 0.11, FLOOR + 0.2),
                       (x0 + 0.05, -BACK - 0.11, FLOOR + 0.15)], 0.025))
    b.put(lines, m["pipe_blue"])
    valve(b, m, (px + 0.14, yc + 0.18, FLOOR + 0.5), (0, 0, 1), 0.02, (1, 0, 0), "paint_red", "paint_dark")
    gauge(b, m, (px + 0.14, yc + 0.18, FLOOR + 0.7), (0, 0, 1), (0, 1, 0))
    b.collider(box((x1 - x0, 0.84, 1.3), ((x0 + x1) / 2, yc, FLOOR + 0.65)))
    return (px, y - 0.12, FLOOR + 0.4)


def _workstation(b, m, x0, x1, y):
    """Where the crane hands trays over, off the aisle on its +Y side: a
    powered roller conveyor stub with a tray waiting on it, and a stainless
    packing bench with crates and a scale."""
    cx, length, w = x0 + 0.6, 1.8, 0.68
    ya, yb = y, y + length
    top = FLOOR + 0.86
    steel, rollers, legs = bmesh.new(), bmesh.new(), bmesh.new()
    for sx in (-1, 1):
        merge(steel, member((cx + sx * w / 2 - 0.025, ya, top - 0.05), (cx + sx * w / 2 - 0.025, yb, top - 0.05), channel(0.05, 0.12, 0.004)))
        merge(steel, box((0.02, length, 0.05), (cx + sx * (w / 2 + 0.02), (ya + yb) / 2, top + 0.04)))
        for t in (0.1, 0.9):
            merge(legs, box((0.05, 0.05, top - 0.11 - FLOOR), (cx + sx * w / 2, ya + length * t, (top - 0.11 + FLOOR) / 2)))
            merge(legs, box((0.1, 0.1, 0.012), (cx + sx * w / 2, ya + length * t, FLOOR + 0.006)))
    n = int(length / 0.09)
    for k in range(n):
        yy = ya + 0.05 + k * (length - 0.1) / (n - 1)
        merge(rollers, rod((cx - w / 2 + 0.01, yy, top), (cx + w / 2 - 0.01, yy, top), 0.024, 10))
    b.put(steel, m["galv"], smooth=False)
    b.put(rollers, m["steel"])
    b.put(legs, m["galv"], smooth=False)
    b.put(moved(cone(0.07, 0.07, 0.22, segments=14), (cx + w / 2 + 0.02, yb - 0.2, top - 0.08), 0, math.pi / 2), m["paint_orange"])
    b.put(box((0.1, 0.12, 0.12), (cx + w / 2 + 0.06, yb - 0.2, top - 0.08)), m["paint_dark"], smooth=False)
    b.put(box((0.04, 0.04, 0.3), (cx, yb + 0.02, top + 0.12)), m["paint_yellow"], smooth=False)
    b.put(box((w - 0.08, 0.6, 0.06), (cx, ya + 0.42, top + 0.055), tile=0.5), m["tray"], smooth=False)
    bx0, bx1 = cx + w / 2 + 0.3, x1 - 0.05
    bm_ = (bx0 + bx1) / 2
    b.put(box((bx1 - bx0, 0.75, 0.04), (bm_, y + 0.4, FLOOR + 0.9), tile=1.0), m["worktop"], smooth=False)
    frame = bmesh.new()
    for xx in (bx0 + 0.05, bx1 - 0.05):
        for yy in (y + 0.07, y + 0.73):
            merge(frame, box((0.04, 0.04, 0.876), (xx, yy, FLOOR + 0.44)))
    merge(frame, box((bx1 - bx0 - 0.1, 0.65, 0.02), (bm_, y + 0.4, FLOOR + 0.25)))
    b.put(frame, m["worktop"], smooth=False)
    crates = bmesh.new()
    for xx, zz in ((bx0 + 0.28, FLOOR + 1.03), (bx0 + 0.28, FLOOR + 0.37), (bx0 + 0.72, FLOOR + 0.37)):
        merge(crates, box((0.4, 0.3, 0.22), (xx, y + 0.45, zz), tile=0.5))
    b.put(crates, m["container_blue"], smooth=False)
    b.put(box((0.32, 0.3, 0.06), (bx1 - 0.3, y + 0.4, FLOOR + 0.95), tile=0.3, bevel=0.005), m["paint_grey"], smooth=False)
    b.collider(box((w, length, top - FLOOR), (cx, (ya + yb) / 2, (top + FLOOR) / 2)))
    b.collider(box((bx1 - bx0, 0.75, 0.92), (bm_, y + 0.4, FLOOR + 0.46)))


def _entry(b, m):
    """The entry inside the hatch: a germination chamber with its trolley
    racks behind glass, a seeding bench with a sink and a plug-tray seeder,
    a fire extinguisher and the crane's pendant station by the gate."""
    x0, x1 = X0 + 0.3, PORTALS[0] - 0.3
    y0, y1 = -2.8, -1.4
    h = 2.2
    b.put(box((x1 - x0, y1 - y0, h), ((x0 + x1) / 2, (y0 + y1) / 2, FLOOR + h / 2), tile=1.0, bevel=0.02), m["paint_white"], smooth=False)
    b.put(box((x1 - x0 - 0.3, 0.02, h - 0.4), ((x0 + x1) / 2, y1 + 0.01, FLOOR + h / 2), tile=1.0), m["vision"], smooth=False)
    trim = bmesh.new()
    for x in (x0 + 0.13, (x0 + x1) / 2, x1 - 0.13):
        merge(trim, box((0.04, 0.03, h - 0.36), (x, y1 + 0.02, FLOOR + h / 2)))
    merge(trim, box((0.04, 0.05, 0.4), (x0 + 0.24, y1 + 0.04, FLOOR + 1.1)))
    b.put(trim, m["steel"], smooth=False)
    b.put(box((0.18, 0.02, 0.12), (x1 - 0.3, y1 + 0.02, FLOOR + 1.6)), m["andon"], smooth=False)
    b.collider(box((x1 - x0, y1 - y0, h), ((x0 + x1) / 2, (y0 + y1) / 2, FLOOR + h / 2)))
    by0, by1 = 1.5, 2.25
    ym = (by0 + by1) / 2
    b.put(box((x1 - x0, by1 - by0, 0.04), ((x0 + x1) / 2, ym, FLOOR + 0.9), tile=1.0), m["worktop"], smooth=False)
    b.put(box((x1 - x0, by1 - by0 - 0.04, 0.8), ((x0 + x1) / 2, ym + 0.02, FLOOR + 0.48), tile=1.0), m["paint_white"], smooth=False)
    b.put(box((0.5, 0.4, 0.2), (x0 + 0.4, ym, FLOOR + 0.83), tile=0.5), m["steel"], smooth=False)
    b.put(tube([(x0 + 0.4, by1 - 0.06, FLOOR + 0.92), (x0 + 0.4, by1 - 0.06, FLOOR + 1.2), (x0 + 0.4, by1 - 0.2, FLOOR + 1.2)], 0.012, 6), m["steel"])
    b.put(box((0.6, 0.45, 0.35), (x1 - 0.45, ym, FLOOR + 1.1), tile=0.5, bevel=0.01), m["paint_grey"], smooth=False)
    b.put(box((0.55, 0.3, 0.04), (x1 - 0.45, ym, FLOOR + 0.94), tile=0.5), m["tray"], smooth=False)
    b.collider(box((x1 - x0, by1 - by0, 0.92), ((x0 + x1) / 2, ym, FLOOR + 0.46)))
    b.put(moved(lathe([(0.0, 0.0), (0.08, 0.0), (0.08, 0.5), (0.05, 0.56), (0.0, 0.56)], 12, 1.0), (X0 + 0.14, 1.15, FLOOR + 0.3)), m["paint_red"])
    sign(b, m, (X0 + 0.022, -1.1, FLOOR + 1.7), 0.0, 0.3, 0.22, 1)
    px, py = PORTALS[0] - 0.45, -0.95
    b.put(box((0.08, 0.08, 1.1), (px, py, FLOOR + 0.55)), m["paint_yellow"], smooth=False)
    b.put(box((0.12, 0.22, 0.3), (px, py, FLOOR + 1.2), tile=0.3, bevel=0.01), m["paint_grey"], smooth=False)
    b.put(moved(cone(0.03, 0.03, 0.03, segments=12), (px + 0.06, py, FLOOR + 1.28), 0, math.pi / 2), m["paint_red"])


def _outside(b, m, out_pts):
    """Outside: the heat-rejection radiator wing on the -Y side and its
    lines into the shell, the power feed cabinet on the +Y side whose gland
    takes the base's cable and whose conduit runs into the shell, and the
    vent stack with its obstruction light on the crown."""
    base = abs(out_pts[0][0])
    ry, rx0, rx1 = -(base + 1.6), 15.5, 23.5
    skid = bmesh.new()
    for x in (rx0, (rx0 + rx1) / 2, rx1):
        merge(skid, box((0.18, 1.0, 0.18), (x, ry, 0.3)))
        merge(skid, box((0.6, 1.2, 0.22), (x, ry, 0.11)))
    merge(skid, box((rx1 - rx0 + 0.18, 0.14, 0.16), ((rx0 + rx1) / 2, ry, 0.47)))
    b.put(skid, m["galv"], smooth=False)
    edges = bmesh.new()
    for k in range(3):
        x = rx0 + 0.15 + (rx1 - rx0 - 0.3) * (k + 0.5) / 3
        w = (rx1 - rx0 - 0.3) / 3 - 0.1
        b.put(box((w, 0.05, 1.9), (x, ry, 1.5), tile=1.9), m["radiator"], smooth=False)
        for z in (0.55, 2.45):
            merge(edges, box((w + 0.04, 0.08, 0.06), (x, ry, z)))
    b.put(edges, m["alu"], smooth=False)
    posts = bmesh.new()
    for x in (rx0 + 0.07, rx1 - 0.07):
        merge(posts, box((0.12, 0.12, 2.3), (x, ry, 1.65)))
        merge(posts, rod((x, ry - 0.55, 0.22), (x, ry, 2.2), 0.025, 6))
    b.put(posts, m["galv"], smooth=False)
    lines = bmesh.new()
    for k, z in enumerate((0.75, 1.05)):
        xl = rx1 - 0.3 - 0.25 * k
        merge(lines, pipe([(xl, ry, 2.5 - 0.12 * k), (xl, ry + 0.25, 2.5 - 0.12 * k), (xl, ry + 0.25, z), (xl, -base + 0.25, z)], 0.05))
    b.put(lines, m["pipe_white"])
    b.collider(box((rx1 - rx0 + 0.6, 1.2, 2.6), ((rx0 + rx1) / 2, ry, 1.3)))
    fx, fy = FEED.x, FEED.y - 0.42
    b.put(box((1.5, 1.0, 0.15), (fx, fy, 0.075), tile=1.0), m["sinter"], smooth=False)
    cabinet(b, m, (fx, fy, 0.05), math.pi / 2, (0.5, 1.1, 1.4), "paint_grey", 2, 0)
    b.put(pipe([(fx - 0.3, fy - 0.25, 0.9), (fx - 0.3, base - 0.3, 0.9)], 0.06), m["paint_dark"])
    b.put(moved(lathe([(0.0, 0.0), (0.09, 0.0), (0.09, 0.08), (0.07, 0.16), (0.0, 0.16)], 14, 1.0), (fx, FEED.y - 0.17, 0.18), 0, 0, -math.pi / 2), m["paint_dark"])
    bollards = bmesh.new()
    for dx in (-0.9, 0.9):
        merge(bollards, rod((fx + dx, FEED.y + 0.3, 0.0), (fx + dx, FEED.y + 0.3, 0.9), 0.07, 12))
    b.put(bollards, m["yellow"])
    b.collider(box((1.1, 0.5, 1.5), (fx, fy, 0.75)))
    top_z = out_pts[len(out_pts) // 2][1]
    b.put(tube([(X1 - 2.0, 0, top_z - 0.3), (X1 - 2.0, 0, top_z + 1.2)], 0.12, 12), m["steel"])
    b.put(moved(cone(0.22, 0.14, 0.25, segments=12), (X1 - 2.0, 0, top_z + 1.2)), m["paint_dark"])
    b.put(rod((X1 - 1.6, 0, top_z - 0.2), (X1 - 1.6, 0, top_z + 2.2), 0.03, 6), m["steel"])
    b.put(sphere(0.08, (X1 - 1.6, 0, top_z + 2.28), 10, 5), m["red"])


def _lights_and_fans(b, m):
    """Work lights hung from the ribs over the walkways and both ends; two
    circulation fans hung from the crown over the aisle."""
    spots = [(x, s * 2.3) for x in (6.6, 13.0, 19.4, 25.8) for s in (-1, 1)] + [(1.7, 0.0), (28.8, 0.0)]
    housings, faces, rods = bmesh.new(), bmesh.new(), bmesh.new()
    for x, y in spots:
        z = FLOOR + (3.4 if y else 3.0)
        merge(housings, box((1.2, 0.16, 0.08), (x, y, z + 0.04), tile=0.5))
        merge(faces, box((1.1, 0.12, 0.01), (x, y, z - 0.002)))
        for dx in (-0.5, 0.5):
            merge(rods, rod((x + dx, y, z + 0.08), (x + dx, y, arch_z(abs(y)) - 0.15), 0.006, 4))
        b.places.append(("light", {"at": (x, y, z - 0.1), "kind": "bay"}))
    b.put(housings, m["paint_dark"], smooth=False)
    b.put(faces, m["lamp"], smooth=False)
    b.put(rods, m["steel"])
    fans = []
    for x in (11.8, 17.4):
        z = FLOOR + 6.3
        drum = bmesh.new()
        _drum(drum, (x, 0, z), (1, 0, 0), 0.32, 0.34, 18)
        b.put(_uv(drum, 0.5), m["paint_grey"])
        for dx in (-0.18, 0.18):
            b.put(moved(disc(0.3, 0.0, 18, 0.5), (x + dx, 0, z), 0, math.pi / 2), m["grate"])
        b.put(rod((x, 0, z + 0.32), (x, 0, arch_z(0.0) - 0.2), 0.015, 6), m["steel"])
        fans.append((x, 0.0, z))
    return fans


# The hall


def _rack_colliders(b):
    """The racks as blocks, open over every tray up to its lamp so the
    crane's arm can reach in, their plenums and uprights."""
    xs = frames()
    for s in (-1, 1):
        yc = s * (FRONT + BACK) / 2
        for k, kind in enumerate(BAYS):
            xa, xc = xs[k], xs[k + 1]
            _, _, lamp = LEVELS[kind]
            levels = tops(kind)
            bands = [(FLOOR, levels[0])] + [(za + lamp, zb) for za, zb in zip(levels, levels[1:])] + [(levels[-1] + lamp, POST_TOP)]
            for z0, z1 in bands:
                b.collider(box((xc - xa, BACK - FRONT, z1 - z0), ((xa + xc) / 2, yc, (z0 + z1) / 2)))
            b.collider(box((xc - xa - 0.1, PLENUM[1] - PLENUM[0], POST_TOP - FLOOR - 0.6), ((xa + xc) / 2, s * (PLENUM[0] + PLENUM[1]) / 2, (POST_TOP + FLOOR) / 2)))
        for x in xs:
            for y in (FRONT + POST / 2, BACK - POST / 2):
                b.collider(box((POST, POST, POST_TOP - FLOOR), (x, s * y, (POST_TOP + FLOOR) / 2)))


def vertical_farm(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """The vertical farm hall along +X from its lock at -X: see the module's docstring."""
    m = _mats(looks)
    b = Building(name, parent, at, yaw)
    b.places = []
    out_pts = _shell(b, m)
    for s in (-1, 1):
        _rack_row(b, m, s)
    _portals(b, m)
    xs = frames()
    for s in (-1, 1):
        for k, kind in enumerate(BAYS):
            for z in tops(kind):
                b.places.append(("tray", {"at": ((xs[k] + xs[k + 1]) / 2, s * TRAY_Y, z), "yaw": 0.0, "size": TRAY, "lamp": LEVELS[kind][2],
                                          "ring": 0 if kind == "main" else 1}))
    hvac = _air_handler(b, m, 28.2, 29.5, -2.7, -0.8)
    cabinet(b, m, (29.62, 1.6, FLOOR), math.pi, (0.4, 1.4, 1.9), "paint_grey", 2, 0)
    cabinet(b, m, (29.62, 0.55, FLOOR), math.pi, (0.4, 0.6, 1.9), "paint_grey", 1, 2)
    b.collider(box((0.4, 2.0, 2.0), (29.62, 1.25, FLOOR + 1.0)))
    wall = arch_y(FLOOR + 2.4)
    b.put(pipe([(29.62, 2.1, FLOOR + 2.05), (29.62, 2.1, FLOOR + 2.4), (26.7, 2.1, FLOOR + 2.4), (26.7, wall - 0.02, FLOOR + 2.4)], 0.05), m["paint_dark"])
    pump = _fertigation(b, m, PORTALS[1] - 2.65, PORTALS[1] - 0.15, -(FRONT + 0.05))
    _workstation(b, m, PORTALS[1] - 2.65, PORTALS[1] - 0.15, FRONT + 0.05)
    _entry(b, m)
    fans = _lights_and_fans(b, m)
    _outside(b, m, out_pts)
    b.places.append(("room", {"name": name.lower(), "centre": ((X0 + X1) / 2, 0, FLOOR + CROWN / 2), "size": (X1 - X0, SPAN, CROWN),
                              "volume": area() * (X1 - X0), "glazing": 0.0}))
    b.places.append(("service", {"fault": "hvac", "at": hvac}))
    b.places.append(("service", {"fault": "pump", "at": pump}))
    b.places.append(("service", {"fault": "fans", "at": fans[1]}))
    # What stops a rover or an arm: the vault, end walls, floor, bulkhead and the racks.
    lo_pts, _, _ = section(GAP + SHELL, 12, ground=-0.1)
    b.collider(_skin(lo_pts, [0.0] * len(lo_pts), -0.1, X1 + END, 4.0))
    lip, _, _ = section(GAP + SHELL + 0.14, 12, ground=-0.1)
    b.collider(_panel(lip, X1, X1 + END, 4.0))
    for s in (-1, 1):
        b.collider(_panel(_side(lip, s, OPENING[0] / 2, OPENING[1], -0.1), -0.1, X0, 4.0))
    b.collider(_panel(_above(lip, OPENING[1]), -0.1, X0, 4.0))
    b.collider(box((X1 - X0, SPAN + 0.4, FLOOR + 0.1), ((X0 + X1) / 2, 0, (FLOOR - 0.1) / 2)))
    b.collider(box((0.2, 3.0, 2.6), (X0 + 0.1, 0, FLOOR + 1.3)))
    _rack_colliders(b)
    return b.finish()
