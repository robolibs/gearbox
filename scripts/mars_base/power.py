"""Power for the base: a field of fixed-tilt arrays, its power station, and
fission units after NASA's Kilopower behind a regolith berm, joined by a
ladder cable tray and armoured cables laid on the ground."""
import math
import random

import bmesh
import numpy as np
from mathutils import Matrix, Vector, noise

import patterns
from buildings import light_mast
from industry import (REPOSE, aim, cabinet, channel, clast, container, cooler, fillet, grid, lengths, louvres, member, offset, plant_mats,
                      point_at, ring, sign)
from kit import (TAU, Building, box, lathe, merge, moved, project_uv, rect, rod, sphere, srgb_to_linear, sweep, truss, tube)

_DUSTY = {}

# Where cables meet modules, in a module's frame: a Kilopower's lead ends on
# the ground heading -Y, a feeder taking it on 4 cm further; the power
# station's pull pits (PIT across) for the reactor feed, the solar feed and
# the tray to the base; a junction box's size along a cable, across it and up.
KILOPOWER_LEAD = Vector((0.5, -4.04, 0.0))
STATION_PITS = {"reactor": Vector((-4.0, 7.75, 0.0)), "solar": Vector((-6.6, 7.75, 0.0)), "tray": Vector((-6.0, -7.75, 0.0))}
PIT = 0.9
JUNCTION = (0.5, 0.8, 0.6)


def solar_pit(rows, panels, pitch):
    """The pull pit a solar field's trench ends in, in the field's frame."""
    return Vector(((rows - 1) * pitch + 2.95, -panels * 1.14 / 2 - 1.6, 0.0))


def pull_pit(b, m, at):
    """A precast cable pull pit, PIT across, under a tread-plate lid."""
    b.put(box((PIT, PIT, 0.3), tuple(Vector(at) + Vector((0, 0, 0.1)))), m["footing"], smooth=False)
    b.put(box((PIT - 0.1, PIT - 0.1, 0.02), tuple(Vector(at) + Vector((0, 0, 0.26))), tile=1.0), m["tread"], smooth=False)


def _dusty_solar():
    """The cell maps under a film of dust, heavier toward a module's lower edge."""
    d, n, r = patterns.solar()
    _, v = patterns.grid()
    lower = np.clip((0.16 - (1.0 - v)) / 0.16, 0.0, 1.0) ** 1.5
    film = (0.12 + 0.16 * patterns.noise(cells=6, seed=51) + 0.55 * lower) * (0.85 + 0.3 * patterns.noise(cells=40, seed=52, octaves=2))
    film = np.clip(film, 0.0, 0.8)[..., None]
    dust = np.asarray(srgb_to_linear(np.array((0.62, 0.45, 0.33), dtype=np.float32)), dtype=np.float32)
    return (d * (1.0 - film) + dust * film).astype(np.float32), n, (r * (1.0 - film[..., 0]) + 0.8 * film[..., 0]).astype(np.float32)


def power_mats(looks):
    """The plant's materials and the dusty cells, made once."""
    if _DUSTY.get("looks") is not looks:
        m = dict(plant_mats(looks))
        m["solar_dusty"] = looks.get("solar_cells_dusty", (0.05, 0.08, 0.20), 0.2, 0.3, maps=_dusty_solar)
        m["backsheet"] = looks.get("pv_backsheet", (0.80, 0.79, 0.76), 0.65, 0.0)
        m["gold_mli"] = looks.get("gold_mli", (0.86, 0.64, 0.34), 0.35, 1.0, "bi_stretch", contrast=1.2)
        _DUSTY.update(looks=looks, set=m)
    return _DUSTY["set"]


def _panel(w, h, t=0.04):
    """A PV module, w along its row and h up its slope: its glass mapped
    whole to the cell texture, a backsheet, an aluminium frame lip and a
    junction box behind. Returns (glass, backsheet, frame, box)."""
    glass = bmesh.new()
    uv = glass.loops.layers.uv.new("st")
    face = glass.faces.new([glass.verts.new((x * w / 2, y * h / 2, t / 2)) for x, y in ((-1, -1), (1, -1), (1, 1), (-1, 1))])
    for loop in face.loops:
        loop[uv].uv = (loop.vert.co.x / w + 0.5, loop.vert.co.y / h + 0.5)
    back = box((w - 0.01, h - 0.01, t - 0.004), (0, 0, -0.002), tile=1.0)
    lip = bmesh.new()
    for s in (-1, 1):
        merge(lip, box((w, 0.035, 0.046), (0, s * (h / 2 - 0.0175), 0.003)))
        merge(lip, box((0.035, h - 0.07, 0.046), (s * (w / 2 - 0.0175), 0, 0.003)))
    jbox = box((0.14, 0.1, 0.03), (0, h / 2 - 0.22, -t / 2 - 0.015))
    return glass, back, lip, jbox


def ladder_tray(line, width=0.5, depth=0.1, every=0.3):
    """A ladder cable tray along a level line at its floor: side rails and
    rungs. Returns (rails, rungs)."""
    rails = bmesh.new()
    for s in (-1, 1):
        merge(rails, sweep(rect(depth, 0.004), [p + Vector((0, 0, depth / 2)) for p in offset(line, s * width / 2)], 1.0))
        merge(rails, sweep(rect(0.004, 0.03), [p + Vector((0, 0, 0.002)) for p in offset(line, s * (width / 2 - 0.015))], 1.0))
    rungs = bmesh.new()
    cum = lengths(line)
    for k in range(int(cum[-1] / every) + 1):
        p, d = point_at(line, cum, k * every + 0.1)
        merge(rungs, moved(box((0.03, width, 0.015), (0, 0, 0.0075)), tuple(p), math.atan2(d.y, d.x)))
    return rails, rungs


def _cables(line, specs, floor=0.0):
    """Cables laid side by side along a level line: [(offset, radius)]."""
    bm = bmesh.new()
    for off, r in specs:
        merge(bm, tube([p + Vector((0, 0, floor + r)) for p in offset(line, off)], r, 8))
    return bm


def solar_field(looks, name, rows=10, panels=36, pitch=9.0, tilt=22.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """Rows of fixed-tilt tables facing -X (the equator from the base), each
    two dusty modules high in portrait on purlins and rafters over front and
    rear posts on ballast footings, clamped, string-wired to a combiner box
    at the row's end, the combiners feeding a covered cable trench."""
    m = power_mats(looks)
    b = Building(name, parent, at, yaw)
    w, h = 1.1, 2.2
    slope = math.radians(tilt)
    depth = 2 * h + 0.03
    length = panels * (w + 0.04)
    zc = 1.6
    us, nrm = Vector((math.cos(slope), 0, math.sin(slope))), Vector((-math.sin(slope), 0, math.cos(slope)))
    area = 0.0
    # Module frame: x along the row, y up the slope, which rises toward +X.
    tilt_frame = Matrix.Rotation(-slope, 4, "Y") @ Matrix.Rotation(-math.pi / 2, 4, "Z")
    frames = int(length / 4.0)
    purlins = [sc + q for sc in (-(h / 2 + 0.015), h / 2 + 0.015) for q in (-0.3 * h, 0.3 * h)]
    trench_y = -length / 2 - 1.6
    for r in range(rows):
        x = r * pitch
        c = Vector((x, 0, zc))

        def at_slope(s, t, y=0.0):
            return c + us * s + nrm * t + Vector((0, y, 0))

        faces, backs, lips, jboxes = bmesh.new(), bmesh.new(), bmesh.new(), bmesh.new()
        for k in range(panels):
            y = -length / 2 + (k + 0.5) * (w + 0.04)
            for s in (0, 1):
                glass, back, lip, jbox = _panel(w, h)
                place = Matrix.Translation((x, y, zc)) @ tilt_frame @ Matrix.Translation((0, (s - 0.5) * (h + 0.03), 0))
                merge(faces, glass, place)
                merge(backs, back, place)
                merge(lips, lip, place)
                merge(jboxes, jbox, place)
                area += w * h
        b.put(faces, m["solar_dusty"], smooth=False)
        b.put(backs, m["backsheet"], smooth=False)
        b.put(lips, m["alu"], smooth=False)
        b.put(jboxes, m["paint_dark"], smooth=False)
        steel, feet, clamps = bmesh.new(), bmesh.new(), bmesh.new()
        for s in purlins:
            merge(steel, member(at_slope(s, -0.075, -length / 2 - 0.15), at_slope(s, -0.075, length / 2 + 0.15), channel(0.05, 0.1, 0.004), up=nrm))
            for k in range(panels + 1):
                y = -length / 2 + k * (w + 0.04)
                merge(clamps, moved(box((0.05, 0.04, 0.025), (0, 0, 0)), tuple(at_slope(s, 0.035, y)), 0.0, slope))
        for f in range(frames + 1):
            y = -length / 2 + 0.3 + (length - 0.6) * f / frames
            merge(steel, member(at_slope(-depth / 2 + 0.1, -0.19, y), at_slope(depth / 2 - 0.1, -0.19, y), channel(0.06, 0.12, 0.005), up=nrm))
            for s in (-0.36 * depth, 0.36 * depth):
                top = at_slope(s, -0.25, y)
                merge(steel, member((top.x, y, 0.3), top, channel(0.06, 0.12, 0.005, True), up=(1, 0, 0)))
                merge(feet, box((0.45, 0.45, 0.35), (top.x, y, 0.12)))
            rear = at_slope(0.36 * depth, -0.25, y)
            merge(steel, rod((rear.x, y, 0.45), at_slope(-0.05, -0.25, y), 0.025, 6))
        b.put(steel, m["galv"])
        b.put(feet, m["footing"], smooth=False)
        b.put(clamps, m["alu"], smooth=False)
        # String cables under the upper purlins, down to a combiner at the row's -Y end.
        strings = bmesh.new()
        for s in purlins[1::2]:
            merge(strings, tube([at_slope(s, -0.15, length / 2 - 0.2), at_slope(s, -0.15, -length / 2 - 0.1)], 0.012, 6))
            end = at_slope(s, -0.15, -length / 2 - 0.1)
            merge(strings, tube(fillet([end, end - Vector((0.0, 0.25, 0.0)), Vector((x + 0.3, -length / 2 - 0.6, 1.25))], 0.2), 0.012, 6))
        cb = Vector((x + 0.3, -length / 2 - 0.6, 0.0))
        merge(strings, tube(fillet([cb + Vector((0, 0, 0.62)), cb + Vector((0, 0, 0.2)), Vector((cb.x, trench_y, 0.05))], 0.3), 0.03, 8))
        b.put(strings, m["cable"])
        b.put(member(cb, cb + Vector((0, 0, 1.4)), rect(0.08, 0.08)), m["galv"], smooth=False)
        cabinet(b, m, cb + Vector((0.08, 0, 0.55)), 0.0, (0.2, 0.55, 0.7), doors=1, cell=0)
        b.put(moved(box((0.45, 0.75, 0.03), (0, 0, 0)), tuple(cb + Vector((0.1, 0, 1.45))), 0.0, -0.2), m["paint_grey"], smooth=False)

    # The covered cable trench along the row ends to a pull pit, and an irradiance sensor.
    pit = solar_pit(rows, panels, pitch)
    x0, x1 = -1.5, pit.x - PIT / 2
    b.put(box((x1 - x0, 0.6, 0.14), ((x0 + x1) / 2, trench_y, 0.04)), m["footing"], smooth=False)
    lids = bmesh.new()
    for k in range(int(x1 - x0)):
        merge(lids, box((0.97, 0.48, 0.02), (x0 + k + 0.5, trench_y, 0.12), tile=1.0))
    b.put(lids, m["tread"], smooth=False)
    pull_pit(b, m, pit)
    sx, sy = -1.5, length / 2 + 1.0
    b.put(member((sx, sy, 0.0), (sx, sy, 1.9), rect(0.06, 0.06)), m["galv"], smooth=False)
    b.put(moved(box((0.04, 0.4, 0.3), (0, 0, 0)), (sx - 0.1, sy, 1.8), 0.0, slope + math.pi / 2), m["solar_dusty"], smooth=False)
    b.put(moved(lathe([(0.0, 0.0), (0.07, 0.0), (0.07, 0.04), (0.04, 0.06), (0.0, 0.08)], 12, 1.0), (sx, sy, 1.95)), m["paint_white"])
    b.places.append(("solar", {"area": area}))
    for r in range(rows):
        b.collider(box((depth * math.cos(slope), length, 2.2), (r * pitch, 0, 1.1)))
    return b.finish()


def _rack_doors(b, m, x0, y, yaw, count, skin, pitch=1.3):
    """Battery rack doors along a container's long side, facing +X of `yaw`: leaves, handles, vents and signs."""
    turn = Matrix.Rotation(yaw, 3, "Z")
    for k in range(count):
        p = Vector((x0 + k * pitch, y, 0.0))
        b.put(moved(box((0.03, 1.15, 2.05), (0.015, 0, 1.17), tile=2.4), tuple(p), yaw), m[skin], smooth=False)
        b.put(moved(box((0.04, 0.04, 0.3), (0.045, 0.45, 1.2)), tuple(p), yaw), m["steel"], smooth=False)
        louvres(b, m, p + turn @ Vector((0.03, 0, 0.45)), yaw, 0.8, 0.3)
        sign(b, m, p + turn @ Vector((0.031, -0.2, 1.65)), yaw, 0.3, 0.22, 0)


def power_station(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """Where the arrays and the fission units meet the base's bus: two
    battery containers with rack doors, wall-hung air conditioners and roof
    coolers; back-to-back inverter cabinets on a skid; a transformer with
    finned radiators and bushings on a plinth; the switchgear; ladder trays
    overhead, cable pits and covered trenches, bollards and a floodlight."""
    m = power_mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(box((18.0, 10.0, 0.08), (0, 0, 0.0), tile=2.0), m["pad"], smooth=False)
    z0 = 0.15
    for skin, y in (("container_white", -2.0), ("container_blue", 2.0)):
        side = 1 if y > 0 else -1
        container(b, m, (-4.0, y, z0), 0.0, skin)
        feet = bmesh.new()
        for sx in (-1, 1):
            for sy in (-1, 1):
                merge(feet, box((0.45, 0.45, 0.2), (-4.0 + sx * 2.9, y + sy * 1.05, 0.07)))
        b.put(feet, m["footing"], smooth=False)
        _rack_doors(b, m, -6.4, y + side * 1.22, side * math.pi / 2, 4, skin)
        cooler(b, m, (-4.3, y, z0 + 2.59), 0.0, (2.8, 1.6, 0.7), 2, "paint_grey")
        hv = Vector((-7.25, y - 0.3 * side, z0 + 1.4))
        b.put(box((0.42, 0.95, 1.2), tuple(hv), tile=1.0, bevel=0.02), m["paint_white"], smooth=False)
        louvres(b, m, hv + Vector((-0.21, 0, 0.1)), math.pi, 0.75, 0.8)
        b.put(tube(fillet([hv + Vector((0, 0.35, -0.6)), hv + Vector((0, 0.35, -1.3)), hv + Vector((-0.3, 0.35, -1.48))], 0.1), 0.015, 6), m["paint_dark"])
        b.put(box((0.06, 0.12, 0.12), (-0.94, y + 0.9 * side, z0 + 2.35)), m["paint_red"], smooth=False)
        sign(b, m, (-7.04, y + 0.5 * side, z0 + 1.0), math.pi, 0.35, 0.25, 2)

    # Inverter cabinets back to back on a skid, under a canopy.
    skid = bmesh.new()
    for s in (-1, 1):
        merge(skid, member((1.2, s * 0.9, 0.075), (3.8, s * 0.9, 0.075), channel(0.06, 0.15, 0.006, s > 0)))
    for x in (1.2, 2.5, 3.8):
        merge(skid, member((x, -0.9, 0.075), (x, 0.9, 0.075), channel(0.06, 0.15, 0.006)))
    b.put(skid, m["paint_dark"])
    for x in (1.65, 2.5, 3.35):
        for s in (-1, 1):
            cabinet(b, m, (x, s * 0.4, 0.15), s * math.pi / 2, (0.8, 0.82, 1.9), doors=1, cell=0)
    b.put(box((2.9, 2.0, 0.05), (2.5, 0, 2.3), tile=1.0), m["paint_grey"], smooth=False)
    for x in (1.2, 3.8):
        louvres(b, m, (x + (0.01 if x > 2 else -0.01), 0, 1.3), 0.0 if x > 2 else math.pi, 1.2, 0.6)

    # The transformer: tank, finned radiators, bushings, cable box, on a plinth.
    tx = 4.75
    b.put(box((1.5, 1.7, 0.2), (tx, 0, 0.1), tile=1.0), m["footing"], smooth=False)
    b.put(box((1.0, 0.9, 1.3), (tx, 0, 0.85), tile=1.0, bevel=0.02), m["paint_grey"], smooth=False)
    b.put(box((1.06, 0.96, 0.05), (tx, 0, 1.52), tile=1.0), m["paint_grey"], smooth=False)
    fins = bmesh.new()
    for s in (-1, 1):
        for k in range(9):
            merge(fins, box((0.02, 0.36, 1.0), (tx - 0.4 + 0.1 * k, s * 0.63, 0.82)))
        for z in (0.38, 1.27):
            merge(fins, tube([(tx - 0.45, s * 0.47, z), (tx + 0.45, s * 0.47, z)], 0.035, 8))
    b.put(fins, m["paint_grey"], smooth=False)
    bush = bmesh.new()
    sheds = [(0.0, 0.0), (0.06, 0.0)] + [(0.11 if k % 2 else 0.06, 0.05 + 0.05 * k) for k in range(8)] + [(0.04, 0.47), (0.0, 0.47)]
    for k in range(3):
        merge(bush, moved(lathe(sheds, 12, 1.0), (tx - 0.3 + 0.3 * k, -0.22, 1.55)))
    for k in range(4):
        merge(bush, moved(lathe([(0.0, 0.0), (0.05, 0.0), (0.05, 0.12), (0.07, 0.14), (0.04, 0.2), (0.0, 0.2)], 10, 1.0), (tx - 0.33 + 0.22 * k, 0.25, 1.55)))
    b.put(bush, m["glaze"])
    caps = bmesh.new()
    for k in range(3):
        merge(caps, moved(ring(0.03, 0.0, 0.47, 0.6, 8), (tx - 0.3 + 0.3 * k, -0.22, 1.55)))
    for k in range(4):
        merge(caps, moved(ring(0.02, 0.0, 0.2, 0.28, 8), (tx - 0.33 + 0.22 * k, 0.25, 1.55)))
    b.put(caps, m["steel"])
    b.put(box((0.5, 0.25, 0.6), (tx + 0.25, 0.58, 1.0), bevel=0.02), m["paint_grey"], smooth=False)
    sign(b, m, (tx + 0.505, 0, 0.95), 0.0, 0.4, 0.3, 0)

    # The switchgear, the bus's end: three panels with windows, lamps and a meter.
    gx, gy = 6.5, 2.5
    cabinet(b, m, (gx, gy, 0.0), 0.0, (1.2, 2.0, 2.1), doors=3, cell=0)
    front = bmesh.new()
    lamps = bmesh.new()
    for k in range(3):
        y = gy - 2.0 / 2 + 0.03 + (2.0 - 0.06) * (k + 0.5) / 3
        merge(front, box((0.01, 0.3, 0.2), (gx + 0.62, y, 1.55)))
        merge(lamps, box((0.02, 0.04, 0.04), (gx + 0.62, y - 0.12, 1.25)))
        merge(front, moved(ring(0.07, 0.0, 0.0, 0.02, 14), (gx + 0.62, y + 0.1, 1.25), 0.0, math.pi / 2))
    b.put(front, m["vision"], smooth=False)
    b.put(lamps, m["andon"], smooth=False)
    b.places.append(("power", {"at": (gx, gy, 1.2)}))

    # Ladder trays overhead: across the container ends, to the cabinets and on to the switchgear.
    zt = 2.75
    run = [Vector(p) for p in ((-0.55, -2.6, zt), (-0.55, 0.0, zt), (5.9, 0.0, zt), (5.9, 1.3, zt))]
    cross = [Vector((-0.55, -2.6, zt)), Vector((-0.55, 2.6, zt))]
    for line in (fillet(run, 0.5), cross):
        rails, rungs = ladder_tray(line, 0.45)
        b.put(rails, m["galv"], smooth=False)
        b.put(rungs, m["galv"], smooth=False)
        b.put(_cables(line, [(-0.12, 0.03), (-0.04, 0.03), (0.04, 0.03), (0.13, 0.02)], 0.015), m["cable"])
    posts = bmesh.new()
    for x, y, ax, ay in ((-0.55, -2.3, 0.3, 0.0), (-0.55, 2.3, 0.3, 0.0), (0.55, 0.0, 0.0, 0.3), (3.95, 0.0, 0.0, 0.3), (5.9, 0.9, 0.3, 0.0)):
        merge(posts, member((x, y, 0.0), (x, y, zt - 0.06), rect(0.1, 0.1)))
        merge(posts, member((x - ax, y - ay, zt - 0.03), (x + ax, y + ay, zt - 0.03), channel(0.04, 0.06, 0.005)))
        merge(posts, box((0.3, 0.3, 0.02), (x, y, 0.01)))
    b.put(posts, m["galv"], smooth=False)
    drops = bmesh.new()
    for y in (-2.0, 2.0):
        for k in range(2):
            merge(drops, tube(fillet([(-0.55, y + 0.08 * k, zt + 0.02), (-0.9, y + 0.08 * k, zt), (-1.15, y + 0.08 * k, z0 + 2.6)], 0.15), 0.025, 8))
    for x in (1.65, 2.5, 3.35):
        merge(drops, tube(fillet([(x, 0.0, zt), (x, 0.0, 2.33)], 0.1), 0.03, 8))
    merge(drops, tube(fillet([(tx + 0.2, 0.0, zt), (tx + 0.25, 0.4, zt - 0.4), (tx + 0.25, 0.58, 1.32)], 0.2), 0.03, 8))
    merge(drops, tube(fillet([(5.9, 1.3, zt), (5.9, 1.7, zt), (gx, 1.8, 2.26)], 0.2), 0.03, 8))
    b.put(drops, m["cable"])

    # Cable pits where the reactor and solar feeds and the tray to the base
    # come in, covered trenches from them to the batteries.
    for pit in STATION_PITS.values():
        pull_pit(b, m, pit)
        px, py = pit.x, pit.y
        y0, y1 = sorted((py - math.copysign(PIT / 2, py), math.copysign(3.4, py)))
        b.put(box((0.5, y1 - y0, 0.12), (px, (y0 + y1) / 2, 0.04)), m["footing"], smooth=False)
        lids = bmesh.new()
        for k in range(int(y1 - y0)):
            merge(lids, box((0.42, 0.95, 0.02), (px, y0 + k + 0.5, 0.11), tile=1.0))
        b.put(lids, m["tread"], smooth=False)
    bollards = bmesh.new()
    for x, y in ((5.75, -0.9), (5.75, 0.9), (7.5, 1.3), (7.5, 3.7), (-8.4, -4.4), (-8.4, 4.4), (8.4, 4.4)):
        merge(bollards, moved(lathe([(0.0, 0.0), (0.09, 0.0), (0.09, 1.0), (0.06, 1.06), (0.0, 1.08)], 12, 1.0), (x, y, 0)))
    b.put(bollards, m["paint_yellow"])
    light_mast(b, m, (8.0, -4.0, 0), 7.0, 2, math.pi * 0.75)
    for y in (-2.0, 2.0):
        b.collider(box((6.06, 2.44, 3.45), (-4.0, y, 1.72)))
    b.collider(box((4.2, 1.9, 2.35), (3.15, 0.0, 1.17)))
    b.collider(box((1.2, 2.0, 2.2), (gx, gy, 1.1)))
    return b.finish()


def cable_tray(looks, name, path, height=1.2, parent=None):
    """A ladder cable tray on T-posts every three metres along a path, its
    bends swept, its cables laid in it and bent down 0.65 m past each end
    into the ground, at the far end into a pull pit of its own."""
    m = power_mats(looks)
    b = Building(name, parent, (0, 0, 0), 0.0)
    line = fillet([Vector(p) + Vector((0, 0, height)) for p in path], 1.2, 8)
    rails, rungs = ladder_tray(line)
    b.put(rails, m["galv"], smooth=False)
    b.put(rungs, m["galv"], smooth=False)
    specs = [(-0.16, 0.035), (-0.07, 0.035), (0.02, 0.035), (0.1, 0.022), (0.16, 0.022)]
    cables = _cables(line, specs, 0.015)
    for tip, inner in ((line[0], line[1]), (line[-1], line[-2])):
        d = (tip - inner).normalized()
        side = Vector((-d.y, d.x, 0.0))
        for off, r in specs:
            o = tip + side * off + Vector((0, 0, 0.015 + r))
            merge(cables, tube(fillet([o, o + d * 0.35, o + d * 0.6 - Vector((0, 0, height * 0.6)), o + d * 0.65 - Vector((0, 0, height + 0.15))], 0.35), r, 8))
    b.put(cables, m["cable"])
    end = line[-1] + (line[-1] - line[-2]).normalized() * 0.65
    pull_pit(b, m, (end.x, end.y, 0.0))
    cum = lengths(line)
    steel, feet = bmesh.new(), bmesh.new()
    n = max(1, int(cum[-1] / 3.0))
    for k in range(n + 1):
        p, d = point_at(line, cum, 0.4 + (cum[-1] - 0.8) * k / n)
        side = Vector((-d.y, d.x, 0.0))
        g = Vector((p.x, p.y, 0.0))
        merge(steel, member(g + Vector((0, 0, 0.22)), p - Vector((0, 0, 0.07)), rect(0.08, 0.08), up=d))
        merge(steel, member(p - side * 0.32 - Vector((0, 0, 0.035)), p + side * 0.32 - Vector((0, 0, 0.035)), channel(0.04, 0.07, 0.005)))
        merge(steel, rod(g + Vector((0, 0, p.z - 0.5)), p + side * 0.24 - Vector((0, 0, 0.07)), 0.015, 6))
        merge(steel, moved(box((0.25, 0.25, 0.02), (0, 0, 0.23)), tuple(g), math.atan2(d.y, d.x)))
        merge(feet, moved(box((0.4, 0.4, 0.3), (0, 0, 0.07)), tuple(g), math.atan2(d.y, d.x)))
    b.put(steel, m["galv"])
    b.put(feet, m["footing"], smooth=False)
    return b.finish()


def ground_cable(looks, name, path, parent=None, radius=0.06, joint=False):
    """An armoured power cable laid on the dust along a path in long easy
    curves, half sunk where it settled, with splice housings every 40 m and
    route markers carrying warning plates every 12 m beside it. It starts
    in a coupler or, with `joint`, out of the front of a junction box
    (JUNCTION) whose back other cables come into."""
    m = power_mats(looks)
    b = Building(name, parent, (0, 0, 0), 0.0)
    line = fillet([Vector(p) for p in path], 6.0, 10)
    cum = lengths(line)
    n = max(2, int(cum[-1] * 2.0))
    pts = []
    for k in range(n + 1):
        s = cum[-1] * k / n
        p, d = point_at(line, cum, s)
        fade = min(1.0, s / 3.0, (cum[-1] - s) / 3.0)
        wander = (0.22 * math.sin(s * 0.21 + 1.3) + 0.1 * math.sin(s * 0.57 + 0.4)) * fade
        pts.append(p + Vector((-d.y, d.x, 0.0)) * wander + Vector((0, 0, radius * 0.7)))
    b.put(tube(pts, radius, 10), m["cable"])
    d = (pts[1] - pts[0]).normalized()
    if joint:
        along, across, up = JUNCTION
        at = line[0] - d * (along / 2)
        yaw = math.atan2(d.y, d.x)
        b.put(moved(box((along, across, up), (0, 0, up / 2 + 0.15), tile=1.0, bevel=0.02), tuple(at), yaw), m["paint_grey"], smooth=False)
        b.put(moved(box((along + 0.2, across + 0.2, 0.15), (0, 0, 0.075)), tuple(at), yaw), m["footing"], smooth=False)
        sign(b, m, at + d * (along / 2 + 0.001) + Vector((0, 0, 0.5)), yaw, 0.3, 0.22, 0)
    else:
        b.put(aim(lathe([(0.0, 0.0), (radius * 1.4, 0.0), (radius * 1.4, 0.25), (radius * 1.1, 0.3), (0.0, 0.3)], 12, 1.0), pts[0], d), m["paint_dark"])
    splices = bmesh.new()
    for k in range(1, int(cum[-1] / 40.0) + 1):
        i = min(n - 1, int(k * 40.0 / cum[-1] * n))
        merge(splices, aim(lathe([(0.0, -0.35), (radius * 1.3, -0.35), (radius * 2.2, -0.22), (radius * 2.2, 0.22), (radius * 1.3, 0.35), (0.0, 0.35)], 12, 1.0), pts[i], pts[i + 1] - pts[i]))
    b.put(splices, m["paint_dark"])
    posts = bmesh.new()
    s = 2.0
    while s < cum[-1] - 1.0:
        p, d = point_at(line, cum, s)
        side = Vector((-d.y, d.x, 0.0))
        foot = p + side * 0.8
        merge(posts, rod(foot, foot + Vector((0, 0, 0.95)), 0.03, 6))
        sign(b, m, foot + side * 0.035 + Vector((0, 0, 0.82)), math.atan2(side.y, side.x), 0.22, 0.16, 0)
        s += 12.0
    b.put(posts, m["hazard"])
    return b.finish()


def _slab(corners, t):
    """A flat plate over four corners, t thick across them."""
    a, b_, c, d = [Vector(p) for p in corners]
    n = (b_ - a).cross(d - a).normalized()
    bm = bmesh.new()
    top = [bm.verts.new(p + n * t / 2) for p in (a, b_, c, d)]
    bot = [bm.verts.new(p - n * t / 2) for p in (a, b_, c, d)]
    bm.faces.new(top)
    bm.faces.new(bot[::-1])
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new([bot[i], bot[j], top[j], top[i]])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    project_uv(bm, 1.0)
    return bm


def _stirling():
    """A free-piston Stirling converter along +Z from its hot end: (head,
    body and alternator; finned cold collar)."""
    body = lathe([(0.0, 0.0), (0.08, 0.0), (0.1, 0.04), (0.1, 0.15), (0.12, 0.17), (0.12, 0.32), (0.14, 0.34), (0.14, 0.43), (0.1, 0.46), (0.0, 0.47)], 16, 1.0)
    profile = [(0.12, 0.17)]
    for k in range(6):
        z = 0.175 + 0.022 * k
        profile += [(0.15, z), (0.15, z + 0.008), (0.122, z + 0.012)]
    profile.append((0.12, 0.31))
    return body, lathe(profile, 16, 1.0)


def kilopower(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """A 10 kWe fission unit after NASA's Kilopower, on the deck of the
    lander that brought it: a uranium-molybdenum core in its beryllium-oxide
    reflector under MLI, a lithium-hydride and tungsten shadow shield above
    it, sodium heat pipes up the shield to four opposed pairs of Stirling
    converters, and an umbrella of carbon radiator panels with titanium heat
    pipes on a truss mast; its power lead runs down off the deck to end on
    the ground short of KILOPOWER_LEAD."""
    m = power_mats(looks)
    b = Building(name, parent, at, yaw)
    deck = 1.0
    top = deck + 0.12

    # The lander: legs with secondary struts, footpads, a ring truss, an octagonal deck.
    frame, pads = bmesh.new(), bmesh.new()
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        c = Vector((math.cos(a), math.sin(a), 0.0))
        across = Vector((-c.y, c.x, 0.0))
        foot = c * 2.1 + Vector((0, 0, 0.14))
        merge(frame, rod(c * 1.25 + Vector((0, 0, deck)), foot, 0.06, 10))
        for t in (-1, 1):
            merge(frame, rod(c * 0.95 + across * t * 0.6 + Vector((0, 0, deck - 0.05)), foot + Vector((0, 0, 0.12)), 0.03, 8))
        merge(pads, moved(lathe([(0.0, 0.0), (0.28, 0.0), (0.32, 0.05), (0.3, 0.1), (0.08, 0.14), (0.0, 0.16)], 16, 1.0), tuple(c * 2.1)))
    octagon = [Vector((1.3 * math.cos(a), 1.3 * math.sin(a), deck - 0.06)) for a in [math.pi / 8 + TAU * i / 8 for i in range(9)]]
    merge(frame, tube(octagon, 0.04, 8))
    for k in range(4):
        a = k * math.pi / 2
        merge(frame, rod((0, 0, deck - 0.06), (1.3 * math.cos(a), 1.3 * math.sin(a), deck - 0.06), 0.035, 8))
    b.put(frame, m["galv"])
    b.put(pads, m["alu"])
    b.put(lathe([(0.0, deck), (1.45, deck), (1.45, top), (0.0, top)], 8, 1.0, start=math.pi / 8), m["paint_white"], smooth=False)

    # Core, reflector and the shadow shield; the control rod drive under the deck.
    b.put(lathe([(0.0, top), (0.42, top), (0.42, top + 0.12), (0.0, top + 0.12)], 24, 1.0), m["galv"])
    b.put(lathe([(0.0, top + 0.12), (0.34, top + 0.12), (0.36, top + 0.2), (0.36, 1.72), (0.34, 1.8), (0.0, 1.8)], 32, 1.0), m["gold_mli"])
    b.put(lathe([(0.0, 1.8), (0.42, 1.8), (0.62, 2.72), (0.58, 2.78), (0.0, 2.78)], 32, 1.0), m["jacket"])
    bands = ring(0.542, 0.0, 2.3, 2.34, 32)
    merge(bands, ring(0.476, 0.0, 1.98, 2.02, 32))
    b.put(bands, m["steel"])
    b.put(lathe([(0.0, 0.35), (0.06, 0.35), (0.06, deck), (0.0, deck)], 12, 1.0), m["jacket"])
    b.put(box((0.26, 0.26, 0.3), (0, 0, 0.32), bevel=0.02), m["paint_dark"], smooth=False)

    # Four opposed pairs of converters on a spider over the shield, heat pipes up to their hot ends.
    zc, rc = 3.02, 0.82
    hot, bodies, collars, pipes = bmesh.new(), bmesh.new(), bmesh.new(), bmesh.new()
    cold = []
    for j in range(4):
        a = math.pi / 4 + j * math.pi / 2
        c = Vector((rc * math.cos(a), rc * math.sin(a), zc))
        t = Vector((-math.sin(a), math.cos(a), 0.0))
        merge(hot, moved(box((0.24, 0.22, 0.24), (0, 0, 0), bevel=0.02), tuple(c), a))
        for s in (-1, 1):
            body, collar = _stirling()
            merge(bodies, aim(body, c + t * s * 0.11, t * s))
            merge(collars, aim(collar, c + t * s * 0.11, t * s))
            cold.append(c + t * s * 0.35)
            da = 0.28 * s
            d = Vector((math.cos(a + da), math.sin(a + da), 0.0))
            merge(pipes, tube(fillet([d * 0.3 + Vector((0, 0, 1.6)), d * 0.45 + Vector((0, 0, 1.82)), d * 0.65 + Vector((0, 0, 2.74)),
                                      d * 0.7 + Vector((0, 0, 2.86)), c + t * s * 0.06 - Vector((0, 0, 0.1))], 0.12), 0.022, 8))
        merge(pipes, rod(Vector((math.cos(a), math.sin(a), 0.0)) * 0.55 + Vector((0, 0, 2.75)), c - Vector((0, 0, 0.12)), 0.03, 6))
    b.put(hot, m["jacket"], smooth=False)
    b.put(bodies, m["alu"])
    b.put(collars, m["paint_dark"])
    b.put(pipes, m["jacket"])

    # The radiator umbrella: carbon panels, titanium heat pipes, rings, spokes and the mast.
    r0, z0, r1, z1, count = 0.95, 3.4, 3.0, 6.0, 12
    gap = math.radians(1.2)
    slabs, ribs = bmesh.new(), bmesh.new()
    lift = Vector((-(z1 - z0), r1 - r0)).normalized()
    for k in range(count):
        a0, a1 = TAU * k / count + gap, TAU * (k + 1) / count - gap
        corners = [(r * math.cos(a), r * math.sin(a), z) for r, z, a in ((r0, z0, a0), (r0, z0, a1), (r1, z1, a1), (r1, z1, a0))]
        merge(slabs, _slab(corners, 0.03))
        for q in range(5):
            a = a0 + (a1 - a0) * (q + 0.5) / 5
            u = Vector((math.cos(a), math.sin(a), 0.0))
            up = u * lift.x + Vector((0, 0, lift.y))
            merge(ribs, rod(u * (r0 + 0.05) + Vector((0, 0, z0 + 0.06)) + up * 0.02, u * (r1 - 0.05) + Vector((0, 0, z1 - 0.06)) + up * 0.02, 0.014, 6))
    b.put(slabs, m["carbon"], smooth=False)
    b.put(ribs, m["alu"])
    struct = tube([(r1 * math.cos(a), r1 * math.sin(a), z1) for a in [TAU * i / 24 for i in range(25)]], 0.035, 8)
    merge(struct, tube([(r0 * math.cos(a), r0 * math.sin(a), z0) for a in [TAU * i / 24 for i in range(25)]], 0.05, 8))
    merge(struct, truss((0, 0, 2.78), (0, 0, 6.4), 0.22, 0.22, 7, 0.025, 0.012))
    for k in range(count):
        a = TAU * (k + 0.5) / count
        u = Vector((math.cos(a), math.sin(a), 0.0))
        merge(struct, rod((0, 0, 6.35), u * r1 + Vector((0, 0, z1)), 0.015, 6))
        merge(struct, rod((0, 0, 4.1), u * 2.0 + Vector((0, 0, z0 + (2.0 - r0) / (r1 - r0) * (z1 - z0) - 0.05)), 0.018, 6))
    for p in cold:
        q = Vector((p.x, p.y, 0.0)).normalized() * r0 + Vector((0, 0, z0))
        merge(struct, tube(fillet([p + Vector((0, 0, 0.12)), Vector((p.x, p.y, zc + 0.28)), q], 0.1), 0.02, 6))
    b.put(struct, m["jacket"])
    b.put(sphere(0.12, (0, 0, 6.52), 10, 5), m["red"])

    # The power cable off the deck, and radiation signs.
    jb = Vector((0.0, -1.22, top))
    b.put(box((0.4, 0.3, 0.36), tuple(jb + Vector((0, 0, 0.18))), bevel=0.02), m["paint_grey"], smooth=False)
    sign(b, m, jb + Vector((0, -0.155, 0.2)), -math.pi / 2, 0.24, 0.24, 1)
    lead = KILOPOWER_LEAD + Vector((0.0, 0.04, 0.042))
    b.put(tube(fillet([jb + Vector((0, -0.1, 0.05)), jb + Vector((0, -0.35, -0.2)), Vector((0.0, -1.65, 0.042)), Vector((lead.x, -2.6, 0.042)), lead], 0.3), 0.045, 10), m["cable"])
    for a in (0.0, math.pi):
        sign(b, m, (1.35 * math.cos(a), 1.35 * math.sin(a), deck + 0.06), a, 0.3, 0.1, 1)
    b.collider(lathe([(1.6, 0.0), (1.6, top), (0.75, top), (0.75, z0), (r1, z1), (0.0, z1)], 12, 2.0))
    return b.finish()


def berm(looks, name, centre, radius, start, end, height=2.6, parent=None):
    """An arc of bulldozed regolith, a radiation and blast screen: slopes at
    the angle of repose, a rounded crest that wanders in height, ends that
    run out into the ground, clasts along its toes."""
    m = power_mats(looks)
    b = Building(name, parent, centre, 0.0)
    rng = random.Random(7)
    crest = 0.7
    reach = crest + height / REPOSE + 0.8
    span = radius * (end - start)
    run = 6.0
    phase = Vector((rng.uniform(0, 100), rng.uniform(0, 100), 0.0))

    def surface(s, t, p):
        taper = min(1.0, max(0.0, (s + run) / (run + 4.0)), max(0.0, (span + run - s) / (run + 4.0)))
        taper = taper * taper * (3.0 - 2.0 * taper)
        peak = height * (0.9 + 0.1 * noise.noise(Vector((s * 0.06, 0.0, 9.0)) + phase)) * taper
        shoulder = math.sqrt(max(abs(t) - crest, 0.0) ** 2 + 0.4) - math.sqrt(0.4)
        z = peak - REPOSE * shoulder
        grain = noise.fractal(Vector((p.x * 0.4, p.y * 0.4, 0.0)) + phase, 1.0, 2.0, 4)
        lumps = noise.fractal(Vector((p.x * 1.5, p.y * 1.5, 3.0)) + phase, 0.8, 2.0, 3)
        return z + (0.25 * grain + 0.08 * lumps) * min(1.0, max(z, 0.0) / 0.6)

    steps, across = int((span + 2 * run) / 0.6), 26
    rows = []
    for j in range(across + 1):
        t = -reach + 2.0 * reach * j / across
        row = []
        for i in range(steps + 1):
            s = -run + (span + 2 * run) * i / steps
            a = start + s / radius
            p = Vector((math.cos(a), math.sin(a), 0.0)) * (radius + t)
            row.append((p.x, p.y, surface(s, t, p)))
        rows.append(row)
    b.put(grid(rows, tile=2.5), m["spoil"])
    rocks = bmesh.new()
    for _ in range(60):
        s, side = rng.uniform(0.0, span), rng.choice((-1, 1))
        t = side * (crest + height / REPOSE * rng.uniform(0.6, 1.05))
        a = start + s / radius
        p = Vector((math.cos(a), math.sin(a), 0.0)) * (radius + t)
        merge(rocks, clast(rng, (p.x, p.y, max(surface(s, t, p), 0.0)), rng.uniform(0.1, 0.4)))
    b.put(rocks, m["basalt"])
    toe = crest + height / REPOSE
    profile = [(-toe, 0.0), (toe, 0.0), (crest, height), (-crest, height)]
    arc = [(radius * math.cos(a), radius * math.sin(a), 0.0) for a in [start + (end - start) * i / 40 for i in range(41)]]
    b.collider(sweep([(y, x) for x, y in profile], arc[::5], 1.2))
    return b.finish()
