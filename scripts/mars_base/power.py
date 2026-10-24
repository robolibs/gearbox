"""Power for the base: a solar field, its power station, and fission units
behind a berm, joined by cables on the ground."""
import math

import bmesh
from mathutils import Matrix, Vector

from buildings import light_mast, mats, unit
from kit import (TAU, Building, box, disc, lathe, merge, moved, project_uv, rect, rod, sphere, sweep, tube)


def _panel(w, h, t=0.04):
    """A PV module, w along its row and h up its slope, its face mapped whole to the cell texture."""
    bm = box((w, h, t), (0, 0, 0), tile=1.0)
    uv = bm.loops.layers.uv.get("st")
    for face in bm.faces:
        if face.normal.z > 0.9:
            for loop in face.loops:
                loop[uv].uv = ((loop.vert.co.x / w) + 0.5, (loop.vert.co.y / h) + 0.5)
        else:
            for loop in face.loops:
                loop[uv].uv = (0.001, 0.001)
    return bm


def solar_field(looks, name, rows=10, panels=36, pitch=9.0, tilt=22.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """Rows of fixed-tilt tables facing -X (the equator from the base), each
    two modules high, on posts and a torque beam, with combiner boxes."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    w, h = 1.1, 2.2
    slope = math.radians(tilt)
    depth = 2 * h
    length = panels * (w + 0.04)
    area = 0.0
    # Module frame: x along the row, y up the slope, which rises toward +X.
    tilt_frame = Matrix.Rotation(-slope, 4, "Y") @ Matrix.Rotation(-math.pi / 2, 4, "Z")
    for r in range(rows):
        x = r * pitch
        faces = bmesh.new()
        for k in range(panels):
            y = -length / 2 + (k + 0.5) * (w + 0.04)
            for s in (0, 1):
                p = _panel(w, h)
                bmesh.ops.translate(p, vec=(0, (s - 0.5) * (h + 0.03), 0), verts=p.verts)
                merge(faces, p, Matrix.Translation((x, y, 1.6)) @ tilt_frame)
                area += w * h
        b.put(faces, m["solar"], smooth=False)
        frame = bmesh.new()
        merge(frame, box((0.12, length + 0.4, 0.12), (x, 0, 1.45)))
        for k in range(int(length / 4.0) + 1):
            y = -length / 2 + k * (length / int(length / 4.0))
            merge(frame, box((0.1, 0.1, 1.5), (x, y, 0.75)))
            merge(frame, sweep(rect(0.06, 0.06), [(x - depth / 2 * math.cos(slope), y, 1.6 - depth / 2 * math.sin(slope) - 0.05), (x + depth / 2 * math.cos(slope), y, 1.6 + depth / 2 * math.sin(slope) - 0.05)], 1.0))
            merge(frame, box((0.4, 0.4, 0.15), (x, y, 0.075)))
        b.put(frame, m["steel"], smooth=False)
        b.put(box((0.5, 0.25, 0.6), (x + 0.3, length / 2 + 0.4, 0.9), bevel=0.02), m["hull_grey"], smooth=False)
        b.put(tube([(x + 0.3, length / 2 + 0.4, 0.6), (x + 0.3, length / 2 + 0.4, 0.06), (x + 0.3, length / 2 + 2.0, 0.06)], 0.05, 6), m["cable"])
    b.put(tube([(-1.0, length / 2 + 2.0, 0.06), ((rows - 1) * pitch + 1.0, length / 2 + 2.0, 0.06)], 0.09, 8), m["cable"])
    b.places.append(("solar", {"area": area}))
    for r in range(rows):
        b.collider(box((depth * math.cos(slope), length, 2.2), (r * pitch, 0, 1.1)))
    return b.finish()


def power_station(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """Two containers of batteries, an inverter skid and a switch cabinet on
    a gravel pad: where the arrays and the fission units meet the bus."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(box((18.0, 10.0, 0.08), (0, 0, 0.0), tile=2.0), m["pad"], smooth=False)
    for k in range(2):
        y = -2.0 + k * 4.0
        b.put(box((6.06, 2.44, 2.6), (-4.0, y, 1.4), tile=2.4), m["container_blue"] if k else m["container_white"], smooth=False)
        for v in range(4):
            b.put(box((0.06, 0.6, 0.4), (-0.96, y - 0.8 + v * 0.5, 1.9)), m["grate"], smooth=False)
        unit(b, m, (-4.0, y, 2.7), 0.0, (2.4, 1.4, 0.7), 2)
    b.put(box((2.4, 1.6, 2.0), (2.5, 0.0, 1.1), tile=1.0, bevel=0.04), m["hull_grey"], smooth=False)
    for f in range(10):
        b.put(box((0.04, 1.5, 1.6), (3.8 + f * 0.09, 0.0, 1.1)), m["dark"], smooth=False)
    b.put(box((1.2, 2.0, 2.2), (6.5, 2.5, 1.1), tile=1.0, bevel=0.04), m["hull"], smooth=False)
    b.put(box((0.06, 0.8, 1.8), (7.12, 2.5, 1.0)), m["hull_grey"], smooth=False)
    light_mast(b, m, (8.0, -4.0, 0), 7.0, 2, math.pi * 0.75)
    b.places.append(("power", {"at": (6.5, 2.5, 1.2)}))
    for k in range(2):
        b.collider(box((6.06, 2.44, 3.1), (-4.0, -2.0 + k * 4.0, 1.55)))
    b.collider(box((3.4, 1.6, 2.1), (3.0, 0.0, 1.05)))
    b.collider(box((1.2, 2.0, 2.2), (6.5, 2.5, 1.1)))
    return b.finish()


def cable_tray(looks, name, path, height=1.2, parent=None):
    """A raised cable tray on posts along a path."""
    m = mats(looks)
    path = [Vector(p) for p in path]
    b = Building(name, parent, (0, 0, 0), 0.0)
    for a, c in zip(path, path[1:]):
        run = c - a
        n = max(1, int(run.length / 4.0))
        for k in range(n + 1):
            p = a + run * (k / n)
            b.put(rod(p, p + Vector((0, 0, height)), 0.05, 6), m["steel"])
    b.put(sweep([(-0.03, -0.25), (0.12, -0.25), (0.12, -0.22), (0.0, -0.22), (0.0, 0.22), (0.12, 0.22), (0.12, 0.25), (-0.03, 0.25)], [p + Vector((0, 0, height)) for p in path], 1.0), m["steel"], sharp=30)
    for d in (-0.12, 0.0, 0.12):
        b.put(sweep([(0.0, d - 0.04), (0.06, d - 0.04), (0.06, d + 0.04), (0.0, d + 0.04)], [p + Vector((0, 0, height + 0.05)) for p in path], 1.0), m["cable"])
    return b.finish()


def ground_cable(looks, name, path, parent=None, radius=0.06):
    m = mats(looks)
    b = Building(name, parent, (0, 0, 0), 0.0)
    pts = []
    path = [Vector(p) for p in path]
    for a, c in zip(path, path[1:]):
        n = max(2, int((c - a).length / 3.0))
        for k in range(n):
            t = k / n
            p = a + (c - a) * t
            side = Vector((-(c - a).y, (c - a).x, 0)).normalized()
            pts.append(p + side * 0.25 * math.sin(k * 1.7 + len(pts)) + Vector((0, 0, radius)))
    pts.append(path[-1] + Vector((0, 0, radius)))
    b.put(tube(pts, radius, 8, caps=True), m["cable"])
    for k in range(0, len(pts), 6):
        p = pts[k]
        b.put(rod(p + Vector((0.6, 0, -radius)), p + Vector((0.6, 0, 1.2)), 0.03, 6), m["hazard"])
    return b.finish()


def kilopower(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """A 10 kWe fission unit, after NASA's Kilopower: the core in its shield
    at the foot, Stirling converters round it, a heat pipe up the mast to an
    umbrella of radiator panels, all on a lander deck."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(lathe([(0.0, 0.0), (1.6, 0.0), (1.6, 0.35), (0.0, 0.35)], 6, 1.0), m["hull_grey"], smooth=False)
    for i in range(4):
        a = TAU * i / 4 + TAU / 8
        c = Vector((math.cos(a), math.sin(a), 0))
        b.put(rod(c * 1.2 + Vector((0, 0, 0.35)), c * 2.0, 0.07, 8), m["steel"])
        b.put(moved(disc(0.35, 0.02, 16, 1.0), tuple(c * 2.0)), m["dark"])
    b.put(lathe([(0.0, 0.35), (0.65, 0.35), (0.65, 1.9), (0.45, 2.2), (0.0, 2.2)], 32, 1.0), m["gold"], sharp=40)
    for i in range(8):
        a = TAU * i / 8
        c = Vector((math.cos(a), math.sin(a), 0)) * 0.85
        b.put(moved(lathe([(0.0, 0.0), (0.16, 0.0), (0.16, 0.7), (0.1, 0.8), (0.0, 0.8)], 12, 1.0), tuple(c + Vector((0, 0, 2.2)))), m["steel"])
    b.put(rod((0, 0, 2.2), (0, 0, 6.0), 0.12, 12), m["steel"])
    fins = bmesh.new()
    for i in range(12):
        a = TAU * i / 12
        c = Vector((math.cos(a), math.sin(a), 0))
        lo = c * 0.3 + Vector((0, 0, 3.0))
        hi = c * 3.0 + Vector((0, 0, 5.9))
        across = Vector((-c.y, c.x, 0))
        p = [lo - across * 0.15, lo + across * 0.15, hi + across * 0.65, hi - across * 0.65]
        fins.faces.new([fins.verts.new(v) for v in p])
        fins.faces.new([fins.verts.new(v - Vector((0, 0, 0.02))) for v in p[::-1]])
    project_uv(fins, 1.0)
    b.put(fins, m["hull_grey"], smooth=False)
    b.put(sphere(0.2, (0, 0, 6.1), 10, 5), m["red"])
    b.collider(lathe([(1.6, 0.0), (1.6, 0.35), (0.7, 0.35), (0.7, 3.0), (3.0, 5.9), (0.0, 5.9)], 12, 2.0))
    return b.finish()


def berm(looks, name, centre, radius, start, end, height=2.6, parent=None):
    """An arc of stacked regolith bags, a radiation and blast screen."""
    m = mats(looks)
    b = Building(name, parent, centre, 0.0)
    profile = [(-1.6, 0.0), (1.6, 0.0), (0.6, height), (-0.6, height)]
    arc = [(radius * math.cos(a), radius * math.sin(a), 0.0) for a in [start + (end - start) * i / 40 for i in range(41)]]
    b.put(sweep([(y, x) for x, y in profile], arc, 1.2), m["bags"], sharp=30)
    b.collider(sweep([(y, x) for x, y in profile], arc[::5], 1.2))
    return b.finish()
