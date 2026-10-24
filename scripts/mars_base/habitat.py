"""Where the crew lives and works: habitat towers round a hub, an inflatable
hab, printed regolith towers, corridors, a rover hangar and the EDEN module's
skin."""
import math

import bmesh
from mathutils import Vector

from buildings import door, light_mast, mats, railing, unit
from kit import (TAU, Building, box, half_disc, lathe, merge, moved, rect, rod, sphere, sweep, tube, vault)


def _port(b, m, angle, radius, z, length=1.5):
    """A berthing port out of a round hull: collar, flange and hatch."""
    c = Vector((math.cos(angle), math.sin(angle), 0))
    port = lathe([(1.3, 0), (1.3, length - 0.2), (1.5, length - 0.2), (1.5, length), (0.0, length)], 32, 1.0)
    b.put(moved(port, tuple(c * (radius - 0.4) + Vector((0, 0, z))), angle, math.pi / 2), m["hull_grey"], sharp=40)


def hab_tower(looks, name, radius=4.5, height=8.0, at=(0, 0, 0), yaw=0.0, parent=None, ports=(math.pi,), decks=2):
    """A vertical habitat on six legs: a domed cylinder of panels, bands of
    lit windows, berthing ports, a railed roof with antennas and a radiator."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    z0 = 1.8
    top = z0 + height
    profile = [(0.0, z0 - 0.7), (radius * 0.72, z0 - 0.5), (radius, z0)]
    for i in range(1, 7):
        z = z0 + height * i / 6
        profile += [(radius, z - 0.07), (radius - 0.04, z - 0.05), (radius - 0.04, z - 0.02), (radius, z)]
    for a in [i * math.pi / 2 / 10 for i in range(1, 11)]:
        profile.append((radius * (0.02 + 0.98 * math.cos(a)), top + radius * 0.42 * math.sin(a)))
    profile[-1] = (0.0, top + radius * 0.42)
    b.put(lathe(profile, 72, 2.0), m["hull"], sharp=35)
    b.put(lathe([(radius + 0.05, z0 + 0.25), (radius + 0.05, z0 + 1.1)], 72, 1.0), m["gold"])
    b.put(lathe([(radius + 0.04, top - 0.6), (radius + 0.04, top - 0.25)], 72, 1.0), m["hull_orange"])
    for deck in range(decks):
        z = z0 + 2.2 + deck * height / decks
        for i in range(10):
            a = TAU * (i + 0.5 * deck) / 10
            if any(abs(((a - p + math.pi) % TAU) - math.pi) < 0.35 for p in ports):
                continue
            c = Vector((math.cos(a), math.sin(a), 0)) * (radius + 0.02) + Vector((0, 0, z))
            b.put(moved(box((0.22, 1.0, 1.2), (0, 0, 0), bevel=0.1), tuple(c), a), m["hull_grey"], smooth=False)
            b.put(moved(box((0.08, 0.78, 0.96), (0.1, 0, 0), bevel=0.06), tuple(c), a), m["window"], smooth=False)
    for p in ports:
        _port(b, m, p, radius, 2.5)
    for i in range(6):
        a = TAU * i / 6 + TAU / 12
        c = Vector((math.cos(a), math.sin(a), 0))
        foot = c * (radius + 1.0)
        b.put(rod(c * radius * 0.78 + Vector((0, 0, z0 + 0.2)), foot + Vector((0, 0, 0.3)), 0.15, 10), m["steel"])
        b.put(rod(c * radius * 0.5 + Vector((0, 0, z0 - 0.5)), foot + Vector((0, 0, 0.3)), 0.09, 8), m["steel"])
        b.put(moved(lathe([(0.0, 0.0), (0.6, 0.0), (0.6, 0.14), (0.22, 0.34), (0.0, 0.34)], 16, 1.0), tuple(foot)), m["dark"])
    roof = top + radius * 0.3
    railing(b, m, [(radius * 0.64 * math.cos(TAU * i / 32), radius * 0.64 * math.sin(TAU * i / 32), roof) for i in range(33)], 1.0, 2.0)
    b.put(rod((0, 0, top + radius * 0.42), (0, 0, top + radius * 0.42 + 3.5), 0.06, 8), m["steel"])
    b.put(sphere(0.16, (0, 0, top + radius * 0.42 + 3.6), 10, 5), m["red"])
    dish = lathe([(0.0, 0.0), (0.5, 0.12), (0.9, 0.38), (0.86, 0.4), (0.0, 0.05)], 24, 1.0)
    b.put(moved(dish, (radius * 0.35, -radius * 0.2, roof + 1.2), 0.6, 0.9), m["white_frame"])
    b.put(rod((radius * 0.35, -radius * 0.2, roof), (radius * 0.35, -radius * 0.2, roof + 1.2), 0.05, 8), m["steel"])
    # A radiator wing on a mast by the tower.
    mast = Vector((0, 1, 0)) * (radius + 2.2)
    b.put(rod(mast, mast + Vector((0, 0, top)), 0.12, 10), m["steel"])
    for k in range(3):
        b.put(box((0.05, 3.0, 2.2), (mast.x, mast.y, 2.5 + k * 2.6), tile=3.0), m["radiator"], smooth=False)
    ladder = Vector((0, -1, 0))
    across = Vector((1, 0, 0))
    base = ladder * (radius + 0.35)
    for s in (-0.25, 0.25):
        b.put(rod(base + across * s, base + across * s + Vector((0, 0, roof + 1.0)), 0.03, 6), m["yellow"])
    rungs = bmesh.new()
    for k in range(int(roof / 0.3)):
        merge(rungs, rod(base - across * 0.25 + Vector((0, 0, 0.3 + k * 0.3)), base + across * 0.25 + Vector((0, 0, 0.3 + k * 0.3)), 0.02, 6))
    b.put(rungs, m["frame"])
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, z0 + height / 2), "size": (radius * 1.5, radius * 1.5, height), "volume": math.pi * radius ** 2 * height, "glazing": 0.0}))
    b.collider(lathe([(0.0, z0 - 0.7), (radius, z0), (radius, top), (0.0, top + radius * 0.42)], 16, 4.0))
    for i in range(6):
        c = Vector((math.cos(TAU * i / 6 + TAU / 12), math.sin(TAU * i / 6 + TAU / 12), 0)) * (radius + 1.0)
        b.collider(box((0.6, 0.6, z0), (c.x, c.y, z0 / 2)))
    return b.finish()


def hub_node(looks, name, radius=3.8, at=(0, 0, 0), parent=None, ports=(0.0,)):
    """The node corridors meet: a squat drum with a port toward each
    corridor (angles about Z), a glazed cupola and radiators on its roof."""
    m = mats(looks)
    b = Building(name, parent, at, 0.0)
    z0, h = 0.6, 4.6
    b.put(lathe([(0.0, z0), (radius, z0), (radius, z0 + h), (radius * 0.75, z0 + h + 0.8), (1.6, z0 + h + 1.0), (1.6, z0 + h + 1.1), (0.0, z0 + h + 1.1)], 64, 2.0), m["hull"], sharp=35)
    b.put(lathe([(1.5, z0 + h + 1.1), (1.5, z0 + h + 1.9), (0.0, z0 + h + 2.3)], 32, 1.0), m["glass"])
    b.put(lathe([(1.55, z0 + h + 1.1), (1.55, z0 + h + 1.25)], 32, 1.0), m["frame"])
    b.put(lathe([(radius + 0.05, z0 + 0.3), (radius + 0.05, z0 + 0.9)], 64, 1.0), m["hull_orange"])
    for angle in ports:
        _port(b, m, angle, radius, z0 + 1.9)
    b.put(lathe([(radius + 0.4, 0.0), (radius + 0.4, z0), (0.0, z0)], 64, 2.0), m["concrete"], sharp=40)
    for k in range(4):
        a = TAU * k / 4 + 0.4
        c = Vector((math.cos(a), math.sin(a), 0)) * (radius * 0.55)
        b.put(moved(box((0.06, 2.6, 1.6), (0, 0, z0 + h + 1.9)), tuple(c), a), m["radiator"], smooth=False)
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, z0 + h / 2), "size": (radius * 1.4, radius * 1.4, h), "volume": math.pi * radius ** 2 * h, "glazing": 0.0}))
    b.collider(lathe([(radius + 0.4, 0.0), (radius + 0.4, z0), (radius, z0), (radius, z0 + h), (0.0, z0 + h + 1.1)], 16, 4.0))
    return b.finish()


def junction(looks, name, at=(0, 0, 0), parent=None, ports=(0.0,), radius=2.0, z=2.5):
    """Where corridors bend or branch: an upright drum with a collar toward
    each corridor, a domed cap and a footing."""
    m = mats(looks)
    b = Building(name, parent, at, 0.0)
    top = z + 1.9
    b.put(lathe([(0.0, 0.3), (radius, 0.3), (radius, top), (radius * 0.6, top + 0.6), (0.0, top + 0.75)], 48, 2.0), m["hull"], sharp=35)
    b.put(lathe([(radius + 0.35, 0.0), (radius + 0.35, 0.3), (0.0, 0.3)], 48, 2.0), m["concrete"], sharp=40)
    b.put(lathe([(radius + 0.05, top - 0.5), (radius + 0.05, top - 0.2)], 48, 1.0), m["hull_orange"])
    for angle in ports:
        c = Vector((math.cos(angle), math.sin(angle), 0))
        collar = lathe([(1.45, 0.0), (1.45, 0.5), (1.6, 0.5), (1.6, 0.7), (0.0, 0.7)], 32, 1.0)
        b.put(moved(collar, tuple(c * (radius - 0.3) + Vector((0, 0, z))), angle, math.pi / 2), m["hull_grey"], sharp=40)
    b.put(sphere(0.15, (0, 0, top + 1.0), 10, 5), m["red"])
    b.put(rod((0, 0, top + 0.7), (0, 0, top + 1.0), 0.04, 6), m["steel"])
    b.collider(lathe([(radius + 0.35, 0.0), (radius + 0.35, 0.3), (radius, 0.3), (radius, top), (0.0, top + 0.75)], 12, 4.0))
    return b.finish()


def corridor(looks, name, a, c, radius=1.4, parent=None, z=None):
    """A pressurised corridor between two points: corrugated rings on saddles."""
    m = mats(looks)
    a, c = Vector(a), Vector(c)
    run = c - a
    b = Building(name, parent, tuple(a), math.atan2(run.y, run.x))
    length = run.length
    mid = radius + 0.5 if z is None else z
    profile = []
    pitch = 0.32
    for i in range(int(length / pitch) + 1):
        profile.append((radius * (1.0 if i % 2 else 1.03), min(i * pitch, length)))
    shell = lathe(profile, 32, 2.0)
    b.put(moved(shell, (0, 0, mid), 0, math.pi / 2), m["corrugated"], sharp=60)
    for k in range(int(length / 4.5) + 1):
        x = min(1.0 + 4.5 * k, length - 1.0)
        saddle = bmesh.new()
        merge(saddle, box((0.35, radius * 1.7, 0.3), (x, 0, 0.15)))
        for s in (-1, 1):
            merge(saddle, rod((x, s * radius * 0.75, 0.25), (x, s * radius * 0.55, mid - radius * 0.6), 0.07))
        b.put(saddle, m["steel"], smooth=False)
        b.put(moved(lathe([(radius * 1.07, -0.18), (radius * 1.07, 0.18)], 32, 1.0), (x, 0, mid), 0, math.pi / 2), m["hull_grey"])
    b.put(moved(tube([(0.4, 0, 0), (length - 0.4, 0, 0)], 0.1, 8), (0, radius * 0.9, mid + radius * 0.55)), m["pipe_yellow"])
    b.put(moved(tube([(0.4, 0, 0), (length - 0.4, 0, 0)], 0.08, 8), (0, radius * 0.95, mid + radius * 0.3)), m["cable"])
    b.collider(moved(lathe([(radius * 1.03, 0.0), (radius * 1.03, length)], 12, 4.0), (0, 0, mid), 0, math.pi / 2))
    b.collider(box((length, radius * 1.7, mid - radius), (length / 2, 0, (mid - radius) / 2)))
    return b.finish()


def inflatable_hab(looks, name, radius=7.5, height=4.6, at=(0, 0, 0), yaw=0.0, parent=None, lobes=18):
    """A soft-goods hab, as in The Martian: a lobed fabric dome under
    restraint straps on a ring footing, with two rigid airlocks."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    seg, rows = lobes * 6, 14
    grid = []
    for j in range(rows + 1):
        t = j / rows
        lat = t * math.pi / 2
        ring = []
        for i in range(seg):
            a = TAU * i / seg
            bulge = 1.0 + 0.035 * math.cos(lobes * a) * math.cos(lat)
            r = radius * math.cos(lat) * bulge if j < rows else 0.0
            ring.append(bm.verts.new((r * math.cos(a), r * math.sin(a), 0.5 + height * math.sin(lat) ** 0.85)))
        grid.append(ring)
    for j in range(rows):
        for i in range(seg):
            k = (i + 1) % seg
            quad = [grid[j][i], grid[j][k], grid[j + 1][k], grid[j + 1][i]]
            face = bm.faces.new(list(dict.fromkeys(quad)))
            for loop in face.loops:
                loop[uv].uv = (math.atan2(loop.vert.co.y, loop.vert.co.x) * radius / 3.0, loop.vert.co.z / 3.0)
    b.put(bm, m["fabric"])
    for i in range(lobes):
        a = TAU * (i + 0.5) / lobes
        strap = [(radius * 1.004 * math.cos(lat) * math.cos(a), radius * 1.004 * math.cos(lat) * math.sin(a), 0.5 + height * math.sin(lat) ** 0.85 + 0.02) for lat in [math.pi / 2 * k / 16 for k in range(16)]]
        b.put(sweep(rect(0.04, 0.12), strap, 1.0), m["hull_grey"])
    b.put(lathe([(radius + 0.4, 0.0), (radius + 0.4, 0.55), (radius - 0.2, 0.6), (radius - 0.2, 0.0)], 96, 2.0), m["hull_grey"], sharp=40)
    for i in range(lobes):
        a = TAU * (i + 0.5) / lobes
        c = Vector((math.cos(a), math.sin(a), 0)) * (radius + 0.6)
        b.put(rod(c, c + Vector((0, 0, 0.7)), 0.08, 8), m["steel"])
    for a in (0.0, math.pi):
        c = Vector((math.cos(a), math.sin(a), 0))
        lock = lathe([(1.3, 0.0), (1.3, 3.2), (1.0, 3.5), (0.0, 3.5)], 32, 2.0)
        b.put(moved(lock, tuple(c * (radius - 1.0) + Vector((0, 0, 1.6))), a, math.pi / 2), m["hull"], sharp=40)
        b.put(moved(box((3.0, 2.4, 0.3), (0, 0, 0.15)), tuple(c * (radius + 1.4)), a), m["steel"], smooth=False)
        door(b, m, tuple(c * (radius + 2.55)), a, 1.0, 2.0)
        b.collider(moved(lathe([(0.0, 0.0), (1.3, 0.0), (1.3, 3.5), (0.0, 3.5)], 12, 2.0), tuple(c * (radius - 1.0) + Vector((0, 0, 1.6))), a, math.pi / 2))
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, 2.5), "size": (radius * 1.4, radius * 1.4, height), "volume": 2.0 / 3.0 * math.pi * radius ** 2 * height, "glazing": 0.0}))
    b.collider(lathe([(radius + 0.4, 0.0), (radius + 0.4, 0.55)] + [(radius * math.cos(t), 0.5 + height * math.sin(t) ** 0.85) for t in [math.pi / 2 * i / 5 for i in range(1, 5)]] + [(0.0, 0.5 + height)], 16, 4.0))
    return b.finish()


def printed_tower(looks, name, radius=5.0, height=15.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """A habitat printed from regolith, after AI SpaceFactory's MARSHA: an
    upright egg of print layers with a spiral of slot windows."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    profile = []
    for k in range(41):
        t = k / 40
        r = radius * (0.92 + 0.18 * math.sin(math.pi * min(t * 1.6, 1.0))) * (1.0 - 0.55 * t ** 2.2)
        profile.append((r, height * t))
    profile.append((0.0, height + 0.3))
    b.put(lathe(profile, 72, 0.6, swap=True), m["concrete"], sharp=60)
    for k in range(14):
        t = 0.12 + 0.7 * k / 14
        a = 2.4 * k
        z = height * t
        r = radius * (0.92 + 0.18 * math.sin(math.pi * min(t * 1.6, 1.0))) * (1.0 - 0.55 * t ** 2.2)
        c = Vector((math.cos(a), math.sin(a), 0)) * (r + 0.02) + Vector((0, 0, z))
        b.put(moved(box((0.25, 0.7, 1.6), (0, 0, 0), bevel=0.15), tuple(c), a), m["hull_grey"], smooth=False)
        b.put(moved(box((0.1, 0.5, 1.4), (0.1, 0, 0), bevel=0.12), tuple(c), a), m["window"], smooth=False)
    b.put(lathe([(radius * 1.25, 0.0), (radius * 1.25, 0.3), (0.0, 0.3)], 72, 2.0), m["printed"], sharp=40)
    door(b, m, (radius * 0.93, 0, 0.3), 0.0, 1.2, 2.3)
    b.put(moved(vault(radius * 0.7, radius * 0.93 + 0.3, 1.5, 0.3, 12, 1.0), (0, 0, 0)), m["printed"])
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, height * 0.4), "size": (radius * 1.4, radius * 1.4, height * 0.8), "volume": 0.6 * math.pi * radius ** 2 * height, "glazing": 0.0}))
    b.collider(lathe(profile[::5] + [profile[-1]], 16, 4.0))
    return b.finish()


def hangar(looks, name, length=22.0, radius=7.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """A rover hangar under regolith: a printed vault, open at +X behind a
    half-raised roller door, lit inside, a workbench and crane rail."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(vault(0.0, length, radius, 0.0, 28, 0.6, 0.82), m["concrete"])
    b.put(vault(-0.6, length + 0.4, radius + 0.6, -0.3, 28, 2.0, 0.85), m["regolith"])
    b.put(half_disc(radius + 0.6, -0.6, -0.3, -1.0, 28, 2.0, 0.85), m["regolith"], smooth=False)
    for k in range(6):
        x = 1.0 + k * (length - 2.0) / 5
        b.put(sweep(rect(0.22, 0.3), [(x, -radius * 0.98 * math.cos(a), radius * 0.82 * 0.98 * math.sin(a)) for a in [math.pi * i / 28 for i in range(29)]], 1.0), m["steel"])
    # Front: a steel portal frame and a roller door raised two thirds.
    portal = bmesh.new()
    for s in (-1, 1):
        merge(portal, box((0.6, 0.6, radius * 0.82), (length, s * (radius - 0.5), radius * 0.41)))
    merge(portal, box((0.6, radius * 2, 0.8), (length, 0, radius * 0.82 - 0.4)))
    b.put(portal, m["hull_grey"], smooth=False)
    b.put(half_disc(radius, length, radius * 0.82 - 0.8, 1.0, 28, 2.0, 0.3), m["hull"], smooth=False)
    b.put(box((0.15, radius * 2 - 1.6, 1.6), (length - 0.1, 0, radius * 0.82 - 1.6), tile=0.5), m["corrugated"], smooth=False)
    b.put(box((0.3, radius * 2 - 1.2, 0.2), (length + 0.2, 0, 0.1), tile=0.5), m["hazard"], smooth=False)
    b.put(box((length, radius * 1.8, 0.15), (length / 2, 0, 0.075), tile=2.0), m["floor"], smooth=False)
    b.put(box((length + 8.0, radius * 1.6, 0.06), (length + 4.0, 0, 0.03), tile=2.0), m["pad"], smooth=False)
    b.put(box((length - 2.0, 0.3, 0.4), (length / 2, 0, radius * 0.8 - 0.7)), m["yellow"], smooth=False)
    b.put(box((3.0, 0.9, 0.95), (4.0, radius - 1.6, 0.47), tile=1.0), m["hull_grey"], smooth=False)
    for k in range(3):
        x = 4.0 + k * 6.0
        b.put(box((1.2, 0.3, 0.1), (x, 0, radius * 0.8 - 0.25)), m["lamp"], smooth=False)
        b.places.append(("light", {"at": (x, 0, radius * 0.8 - 0.4), "kind": "bay"}))
    light_mast(b, m, (length + 7.0, radius + 1.0, 0), 7.0, 2, math.pi)
    b.collider(vault(-0.6, length + 0.4, radius + 0.6, -0.3, 8, 4.0, 0.85))
    b.collider(half_disc(radius + 0.6, -0.6, -0.3, -1.0, 8, 4.0, 0.85))
    b.collider(box((length, radius * 1.8, 0.15), (length / 2, 0, 0.075)))
    return b.finish()


def eden_skin(looks, name, length, width, height, door_w, at=(0, 0, 0), yaw=0.0, parent=None):
    """The outside of the EDEN growth module: two container shells end to
    end, a door at -X, chillers, cable trays, and a sandbag berm."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    t = 0.03
    cx = length / 2
    for s in (-1, 1):
        b.put(box((length + 2 * t, t, height + t), (cx, s * (width / 2 + t / 2), height / 2), tile=2.4), m["container_white"], smooth=False)
    b.put(box((length + 2 * t, width + 2 * t, t), (cx, 0, height + t / 2), tile=2.4), m["container_white"], smooth=False)
    b.put(box((t, width + 2 * t, height + t), (length + t / 2, 0, height / 2), tile=2.4), m["container_white"], smooth=False)
    side = (width - door_w) / 2
    for s in (-1, 1):
        b.put(box((t, side + t, height + t), (-t / 2, s * (width / 2 - side / 2 + t / 2), height / 2), tile=2.4), m["container_white"], smooth=False)
    b.put(box((t, door_w, height - 2.2), (-t / 2, 0, 2.2 + (height - 2.2) / 2), tile=2.4), m["container_white"], smooth=False)
    # Corner castings and the joint between the two containers.
    for x in (0.0, length / 2, length):
        for s in (-1, 1):
            b.put(box((0.18, 0.18, height + 0.04), (x, s * width / 2, height / 2)), m["hull_grey"], smooth=False)
    b.put(box((length + 0.1, 0.06, 0.35), (cx, -(width / 2 + 0.05), height - 0.5), tile=1.0), m["container_blue"], smooth=False)
    b.put(box((length + 0.1, 0.06, 0.35), (cx, width / 2 + 0.05, height - 0.5), tile=1.0), m["container_blue"], smooth=False)
    frame = bmesh.new()
    for s in (-1, 1):
        merge(frame, box((0.12, 0.12, 2.3), (-0.05, s * (door_w / 2 + 0.06), 1.15)))
    merge(frame, box((0.12, door_w + 0.24, 0.12), (-0.05, 0, 2.26)))
    b.put(frame, m["hazard"], smooth=False)
    b.put(box((0.12, 0.3, 0.08), (-0.1, 0, 2.45)), m["lamp"], smooth=False)
    b.places.append(("light", {"at": (-0.5, 0, 2.4), "kind": "door"}))
    for k in range(3):
        unit(b, m, (2.0 + k * 3.6, -(width / 2 + 0.9), 0), 0.0, (2.2, 1.0, 1.2), 2)
    for k, z in enumerate((height + 0.2, height + 0.45)):
        b.put(tube([(0.5, -0.6 + 0.3 * k, z), (length - 0.5, -0.6 + 0.3 * k, z)], 0.07, 8), m["pipe_white"] if k else m["cable"])
    b.put(box((length - 1.0, 0.4, 0.08), (cx, 0.6, height + 0.12)), m["grate"], smooth=False)
    b.put(box((length + 1.6, width + 3.6, 0.15), (cx, 0, 0.0), tile=2.0), m["pad"], smooth=False)
    berm = bmesh.new()
    for k in range(int(length / 0.8)):
        for row in range(3):
            merge(berm, box((0.78, 0.55, 0.34), (0.4 + k * 0.8 + (row % 2) * 0.4, width / 2 + 0.45, 0.17 + row * 0.33), tile=0.6, bevel=0.1))
    b.put(berm, m["bags"])
    b.collider(box((length, 0.6, 1.0), (cx, width / 2 + 0.45, 0.5)))
    for k in range(3):
        b.collider(box((2.2, 1.0, 1.2), (2.0 + k * 3.6, -(width / 2 + 0.9), 0.6)))
    return b.finish()
