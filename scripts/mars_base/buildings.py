"""The base's materials, shared details, and its greenhouses.

Each structure is built about its own origin, Z up, its front toward +X, and
returns a finished `Building`. `building.places` lists what the gearbox layer
adds inside it: rooms, trays, service spots and lights, in building space.
"""
import math

import bmesh
from mathutils import Vector

import patterns
from kit import (TAU, Building, box, cone, disc, half_disc, ibeam, lathe, merge, moved, project_uv, rect, rod, sphere, sweep, tube, vault)

_MATS = {}


def gantry_rail(radius):
    """Radius of a dome's floor rail, where its polar gantry's outer leg
    runs: inside the vestibule's mouth."""
    return radius - 2.0



def mats(looks):
    """The base's material set, made once."""
    if _MATS.get("looks") is looks:
        return _MATS["set"]
    g = looks.get
    m = {
        "hull": g("hull_panels", (0.88, 0.88, 0.86), 0.45, 0.15, maps=lambda: patterns.panels((0.80, 0.80, 0.78), cols=1, rows=2, seam_w=0.003)),
        "hull_orange": g("hull_orange", (0.85, 0.40, 0.10), 0.45, 0.1, maps=lambda: patterns.panels((0.62, 0.16, 0.03), cols=1, rows=2, seed=4)),
        "hull_grey": g("hull_grey", (0.5, 0.5, 0.5), 0.45, 0.2, maps=lambda: patterns.panels((0.22, 0.23, 0.24), cols=2, rows=2, seed=8)),
        "radiator": g("radiator_panels", (0.9, 0.9, 0.9), 0.35, 0.3, maps=lambda: patterns.panels((0.84, 0.85, 0.86), cols=1, rows=16, rivets=False, grime=0.05, seed=2)),
        "steel": g("steel", (0.66, 0.67, 0.68), 0.32, 1.0),
        "dark": g("dark_metal", (0.12, 0.12, 0.13), 0.45, 0.8),
        "frame": g("frame", (0.30, 0.31, 0.33), 0.45, 0.85),
        "white_frame": g("white_frame", (0.86, 0.86, 0.84), 0.35, 0.4),
        "glass": g("glass", (0.62, 0.72, 0.75), 0.04, 0.0, alpha=0.16),
        "window": g("window", (0.05, 0.06, 0.07), 0.08, 0.0, emit=((1.0, 0.80, 0.55), 3.0)),
        "etfe": g("etfe", (0.86, 0.90, 0.92), 0.08, 0.0, alpha=0.16),
        "concrete": g("regolith_concrete", (0.60, 0.38, 0.25), 0.9, 0.0, "ribbed_concrete_wall"),
        "printed": g("printed_regolith", (0.68, 0.42, 0.27), 0.85, 0.0, "precast_concrete_wall", contrast=1.0),
        "floor": g("floor", (0.50, 0.49, 0.48), 0.7, 0.0, "hangar_concrete_floor"),
        "grate": g("grating", (0.42, 0.42, 0.42), 0.5, 0.9, "metal_grate_rusty", contrast=0.6),
        "gold": g("gold_foil", (1.0, 0.70, 0.28), 0.3, 1.0, "bi_stretch", contrast=1.6),
        "fabric": g("hab_fabric", (0.90, 0.88, 0.84), 0.75, 0.0, "bi_stretch", contrast=1.2),
        "bags": g("sandbags", (0.62, 0.36, 0.22), 0.95, 0.0, "hessian_230", contrast=1.2),
        "pad": g("pad", (0.62, 0.52, 0.44), 0.85, 0.0, "hexagonal_concrete_paving"),
        "tiles": g("heat_tiles", (0.05, 0.05, 0.05), 0.6, 0.0, maps=patterns.heat_tiles),
        "stainless": g("stainless", (0.80, 0.80, 0.82), 0.25, 1.0, "metal_plate", contrast=0.35),
        "solar": g("solar_cells", (0.05, 0.08, 0.20), 0.15, 0.3, maps=patterns.solar),
        "hazard": g("hazard", (0.95, 0.65, 0.05), 0.5, 0.0, maps=patterns.stripes),
        "yellow": g("safety_yellow", (0.95, 0.68, 0.06), 0.45, 0.3),
        "paint": g("pad_paint", (0.92, 0.92, 0.90), 0.6, 0.0),
        "soil": g("soil", (0.22, 0.13, 0.08), 0.95, 0.0, "rough_concrete", contrast=1.4),
        "regolith": g("regolith_pile", (0.58, 0.33, 0.20), 0.95, 0.0, "rough_concrete", contrast=1.3),
        "led": g("led_bar", (0.95, 0.30, 0.65), 0.3, 0.0, emit=((0.95, 0.30, 0.65), 8.0)),
        "red": g("beacon", (1.0, 0.1, 0.05), 0.3, 0.0, emit=((1.0, 0.1, 0.05), 25.0)),
        "green": g("beacon_green", (0.1, 1.0, 0.2), 0.3, 0.0, emit=((0.1, 1.0, 0.2), 25.0)),
        "lamp": g("floodlamp", (1.0, 0.95, 0.85), 0.3, 0.0, emit=((1.0, 0.95, 0.85), 40.0)),
        "rubber": g("rubber", (0.07, 0.07, 0.07), 0.85, 0.0, "rubber_tiles"),
        "container_orange": g("container_orange", (0.78, 0.36, 0.12), 0.6, 0.3, "container_side", contrast=0.8),
        "container_white": g("container_white", (0.84, 0.84, 0.82), 0.6, 0.3, "container_side", contrast=0.8),
        "container_blue": g("container_blue", (0.12, 0.26, 0.50), 0.6, 0.3, "container_side", contrast=0.8),
        "corrugated": g("corrugated", (0.80, 0.80, 0.78), 0.45, 0.8, "corrugated_iron_02", contrast=0.7),
        "pipe_white": g("pipe_insulated", (0.85, 0.84, 0.80), 0.6, 0.0, "bi_stretch", contrast=0.6),
        "pipe_yellow": g("pipe_gas", (0.90, 0.70, 0.10), 0.45, 0.3),
        "pipe_blue": g("pipe_water", (0.15, 0.35, 0.65), 0.45, 0.3),
        "cable": g("cable", (0.04, 0.04, 0.04), 0.7, 0.0),
        "tray": g("tray", (0.12, 0.12, 0.13), 0.6, 0.0),
        "tracks": g("tracks", (0.6, 0.6, 0.6), 0.85, 0.0, maps=patterns.tracks),
    }
    _MATS.update(looks=looks, set=m)
    return m


# Shared details


def door(b, m, at, yaw, w=1.2, h=2.1):
    """A hatch: frame, door leaf, hazard sill and a lamp over it, facing +X of its yaw."""
    frame = bmesh.new()
    merge(frame, box((0.16, w + 0.4, 0.2), (0, 0, h + 0.1)))
    for s in (-1, 1):
        merge(frame, box((0.16, 0.2, h + 0.2), (0, s * (w / 2 + 0.1), h / 2)))
    b.put(moved(frame, at, yaw), m["frame"], smooth=False)
    leaf = box((0.08, w, h), (0.0, 0, h / 2), tile=1.0)
    merge(leaf, box((0.04, w * 0.5, 0.35), (0.05, 0, h * 0.72), tile=1.0))
    b.put(moved(leaf, at, yaw), m["hull_grey"], smooth=False)
    b.put(moved(box((0.22, w + 0.4, 0.08), (0.04, 0, 0.04), tile=0.5), at, yaw), m["hazard"], smooth=False)
    b.put(moved(box((0.12, 0.3, 0.08), (0.12, 0, h + 0.32)), at, yaw), m["lamp"], smooth=False)
    b.places.append(("light", {"at": tuple(Vector(at) + Vector((math.cos(yaw), math.sin(yaw), 0)) * 0.4 + Vector((0, 0, h + 0.3))), "kind": "door"}))


def unit(b, m, at, yaw=0.0, size=(1.6, 1.0, 1.2), fans=2):
    """An air handler or chiller: a panelled box with fan grilles on top and a pipe stub."""
    w, d, h = size
    b.put(moved(box((w, d, h), (0, 0, h / 2), tile=1.0, bevel=0.03), at, yaw), m["hull"], smooth=False)
    for k in range(fans):
        x = -w / 2 + w * (k + 0.5) / fans
        ring = lathe([(d * 0.4, h), (d * 0.4, h + 0.06), (d * 0.34, h + 0.06), (d * 0.34, h + 0.02)], 20, 1.0)
        b.put(moved(moved(ring, (x, 0, 0)), at, yaw), m["dark"])
        b.put(moved(moved(disc(d * 0.34, h + 0.025, 20, 1.0), (x, 0, 0)), at, yaw), m["grate"])
    b.put(moved(tube([(w / 2 - 0.2, d / 2, 0.4), (w / 2 - 0.2, d / 2 + 0.3, 0.4), (w / 2 - 0.2, d / 2 + 0.3, 0.05)], 0.06, 10), at, yaw), m["pipe_white"])
    b.put(moved(box((w + 0.1, d + 0.1, 0.1), (0, 0, 0.05)), at, yaw), m["frame"], smooth=False)


def railing(b, m, path, height=1.0, step=1.5):
    """Posts along a path with a top and a mid rail."""
    path = [Vector(p) for p in path]
    for a, c in zip(path, path[1:]):
        n = max(1, int((c - a).length / step))
        for k in range(n + 1):
            p = a + (c - a) * (k / n)
            b.put(rod(p, p + Vector((0, 0, height)), 0.025, 6), m["yellow"])
    for z in (height, height * 0.5):
        b.put(tube([p + Vector((0, 0, z)) for p in path], 0.025, 6), m["yellow"])


def stairs(b, m, at, yaw, rise, width=1.0, going=0.28, riser=0.2):
    """A steel stair climbing +X from `at` by `rise`, with rails."""
    steps = max(1, int(round(rise / riser)))
    run = steps * going
    flight = bmesh.new()
    for k in range(steps):
        merge(flight, box((going, width, 0.04), (k * going + going / 2, 0, (k + 1) * rise / steps - 0.02)))
    for s in (-1, 1):
        merge(flight, sweep(rect(0.04, 0.2), [(0, s * width / 2, 0.0), (run, s * width / 2, rise)], 1.0))
    b.put(moved(flight, at, yaw), m["grate"], smooth=False)
    for s in (-1, 1):
        p0 = Vector(at) + Vector((-math.sin(yaw), math.cos(yaw), 0)) * (s * width / 2)
        p1 = p0 + Vector((math.cos(yaw), math.sin(yaw), 0)) * run + Vector((0, 0, rise))
        railing(b, m, [p0, p1], 0.95, 1.0)


def light_mast(b, m, at, height=8.0, heads=2, yaw=0.0):
    """A floodlight mast: a tapered pole on a footing, lamp heads on a bar."""
    at = Vector(at)
    b.put(moved(cone(0.14, 0.08, height, segments=12), tuple(at)), m["steel"])
    b.put(moved(box((0.8, 0.8, 0.3), (0, 0, 0.15)), tuple(at)), m["concrete"], smooth=False)
    bar = moved(box((0.08, 1.4, 0.08), (0, 0, height)), tuple(at), yaw)
    b.put(bar, m["steel"], smooth=False)
    for k in range(heads):
        y = -0.5 + k * (1.0 / max(heads - 1, 1))
        head = box((0.3, 0.42, 0.16), (0.12, y, height - 0.05), bevel=0.02)
        b.put(moved(head, tuple(at), yaw), m["dark"], smooth=False)
        b.put(moved(box((0.02, 0.36, 0.12), (0.28, y, height - 0.07)), tuple(at), yaw), m["lamp"], smooth=False)
    b.places.append(("light", {"at": tuple(at + Vector((math.cos(yaw), math.sin(yaw), 0)) * 0.4 + Vector((0, 0, height - 0.1))), "kind": "flood", "yaw": yaw}))


# Greenhouses


def geodesic_dome(looks, name, radius=12.0, wall=1.4, at=(0, 0, 0), yaw=0.0, parent=None, rings=3):
    """A glazed geodesic dome on a regolith-concrete ring wall: white struts
    with hubs, an arched vestibule with an airlock at +X, chillers round the
    wall, and terraced crop rings inside."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.places = []
    base = wall + 0.06 * radius
    glass = bmesh.new()
    bmesh.ops.create_icosphere(glass, subdivisions=3, radius=radius)
    bmesh.ops.bisect_plane(glass, geom=glass.verts[:] + glass.edges[:] + glass.faces[:], plane_co=(0, 0, -0.06 * radius), plane_no=(0, 0, 1), clear_inner=True)
    bmesh.ops.translate(glass, vec=(0, 0, base), verts=glass.verts)
    hubs = [v.co.copy() for v in glass.verts if v.co.z > wall + 0.3]
    frame = glass.copy()
    bmesh.ops.wireframe(frame, faces=frame.faces[:], thickness=0.24, offset=0.0, use_replace=True, use_even_offset=True, use_boundary=True)
    project_uv(frame, 1.0)
    project_uv(glass, 4.0)
    b.put(frame, m["white_frame"], sharp=30)
    b.put(glass, m["glass"], smooth=False)
    nodes = bmesh.new()
    for p in hubs:
        merge(nodes, sphere(0.2, tuple(p), 8, 4))
    b.put(nodes, m["white_frame"])
    # Ring wall with a parapet, and the floor slab.
    b.put(lathe([(radius + 0.9, -0.1), (radius + 0.9, wall), (radius + 0.4, wall + 0.3), (radius - 0.4, wall + 0.3), (radius - 0.4, 0.35)], 128, 2.0), m["concrete"], sharp=50)
    b.put(lathe([(radius + 2.2, -0.3), (radius + 2.2, 0.15), (radius + 0.9, 0.3), (0.0, 0.35)], 128, 3.0), m["floor"], sharp=50)
    # Vestibule through the wall to an airlock.
    length, half = 7.0, 2.0
    x0, x1 = radius - 1.2, radius + length
    b.put(vault(x0, x1, half, 0.35, 20, 2.0), m["hull"])
    for x in (radius + 1.0, radius + 3.5, radius + 6.0):
        b.put(sweep(rect(0.16, 0.3), [(x, -half * 1.04 * math.cos(a), 0.35 + half * 1.04 * math.sin(a)) for a in [math.pi * i / 20 for i in range(21)]], 1.0), m["frame"])
    b.put(half_disc(half, x1, 0.35, 1.0, 20, 2.0), m["hull"], smooth=False)
    b.put(box((length + 1.2, half * 2 + 0.6, 0.35), ((x0 + x1) / 2, 0, 0.175), tile=2.0), m["concrete"], smooth=False)
    door(b, m, (x1 + 0.06, 0, 0.35), 0.0, 1.3, 2.2)
    railing(b, m, [(x1 + 0.3, -1.6, 0.35), (x1 + 1.6, -1.6, 0.35)], 1.0, 1.3)
    # Chillers and a pipe run outside the wall.
    for k, a in enumerate((0.9, 1.25, -0.9, -1.25, 2.6, -2.6)):
        p = Vector((math.cos(a), math.sin(a), 0)) * (radius + 3.6)
        unit(b, m, tuple(p), a + math.pi / 2, (2.2, 1.2, 1.4), 2)
    arc = [Vector((math.cos(a), math.sin(a), 0)) * (radius + 1.25) + Vector((0, 0, 0.5)) for a in [0.6 + 1.9 * i / 24 for i in range(25)]]
    b.put(tube(arc, 0.12, 10), m["pipe_white"])
    b.put(tube([p + Vector((0, 0, 0.3)) for p in arc], 0.07, 8), m["pipe_yellow"])
    # Terraced crop rings inside, rising outward, from the cistern's aisle to
    # inside the gantry rail, with a path between each.
    trays = []
    rail = gantry_rail(radius)
    inner, outer = 3.5, rail - 0.6
    band = (outer - inner) / rings
    width = band - 0.5
    for k in range(rings):
        r0 = inner + k * band
        r1 = r0 + width
        h = 0.55 + 0.5 * k
        b.put(lathe([(r1, 0.35), (r1, 0.35 + h), (r0, 0.35 + h), (r0, 0.35)], 128, 1.0), m["concrete"], sharp=50)
        b.put(lathe([(r1 - 0.08, 0.36 + h), (r0 + 0.08, 0.36 + h)], 128, 1.0), m["soil"])
        rc = (r0 + r1) / 2
        count = int(TAU * rc / 1.9)
        for i in range(count):
            a = TAU * (i + 0.5) / count
            if abs(((a + math.pi) % TAU) - math.pi) < 0.22:
                continue
            trays.append((rc * math.cos(a), rc * math.sin(a), 0.37 + h, a + math.pi / 2, k))
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, (base + radius) / 2), "size": (radius * 1.7, radius * 1.7, base + radius),
                              "volume": 2.0 / 3.0 * math.pi * radius ** 3 + math.pi * radius ** 2 * base, "glazing": 0.72}))
    for x, y, z, a, k in trays:
        b.places.append(("tray", {"at": (x, y, z), "yaw": a, "size": (1.6, width - 0.25), "ring": k}))
    chiller = Vector((math.cos(0.9), math.sin(0.9), 0)) * (radius + 3.6)
    b.places.append(("service", {"fault": "fans", "at": (chiller.x, chiller.y, 1.2)}))
    # The polar gantry's floor rail, which its outer leg runs round.
    b.put(lathe([(rail + 0.12, 0.35), (rail + 0.12, 0.42), (rail - 0.12, 0.42), (rail - 0.12, 0.35)], 160, 1.0), m["steel"], sharp=40)
    # The cistern, a grated path to it and a water wall round it.
    b.put(lathe([(2.2, 0.35), (2.2, 2.9), (1.9, 3.3), (0.0, 3.4)], 48, 2.0), m["hull"], sharp=50)
    b.put(lathe([(2.6, 0.35), (2.6, 0.9), (2.25, 0.9)], 48, 1.0), m["concrete"], sharp=50)
    b.put(box((radius - 2.0, 1.6, 0.06), ((radius + 2.0) / 2 - 0.2, 0, 0.39), tile=1.0), m["grate"], smooth=False)
    for p in ((0, 3.1), (0, -3.1)):
        b.put(moved(box((0.5, 0.5, 1.0), (0, 0, 0.85)), (p[0], p[1], 0)), m["hull_grey"], smooth=False)
    # What stops a rover: ring wall, floor, shell, vestibule, terraces, cistern.
    b.collider(lathe([(radius + 0.9, -0.1), (radius + 0.9, wall), (radius - 0.4, wall + 0.3), (radius - 0.4, 0.35)], 24, 4.0))
    b.collider(disc(radius + 0.9, 0.35, 24, 4.0))
    shell = [(radius * math.cos(a), base + radius * math.sin(a)) for a in [math.pi / 2 * i / 6 for i in range(7)]]
    shell[-1] = (0.0, base + radius)
    b.collider(lathe(shell, 24, 4.0))
    b.collider(vault(x0, x1, half, 0.35, 6, 2.0))
    b.collider(half_disc(half, x1, 0.35, 1.0, 6, 2.0))
    for k in range(rings):
        r0, h = inner + k * band, 0.55 + 0.5 * k
        b.collider(lathe([(r0 + width, 0.35), (r0 + width, 0.35 + h), (r0, 0.35 + h), (r0, 0.35)], 24, 2.0))
    b.collider(lathe([(2.2, 0.35), (2.2, 3.4), (0.0, 3.4)], 12, 2.0))
    return b.finish()


def arch_greenhouse(looks, name, length=48.0, width=10.0, bay=3.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """A vaulted greenhouse along +X: steel I-beam arches carrying ETFE
    cushions that bulge between them, on a concrete curb, an airlock at each
    end, and two rows of grow tables down its length."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.places = []
    r = width / 2
    curb = 0.9
    bays = int(round(length / bay))
    bay = length / bays
    arc = [math.pi * i / 24 for i in range(25)]
    for k in range(bays + 1):
        x = k * bay
        b.put(sweep(ibeam(0.2, 0.34, 0.035), [(x, -r * math.cos(a), curb + r * math.sin(a)) for a in arc], 1.0), m["white_frame"], sharp=30)
        for s in (-1, 1):
            b.put(box((0.5, 0.4, 0.3), (x, s * r, curb + 0.1)), m["steel"], smooth=False)
    cushion = bmesh.new()
    uv = cushion.loops.layers.uv.new("st")
    steps = 8
    grid = []
    for k in range(bays * steps + 1):
        x = k * bay / steps
        bulge = 0.25 * math.sin(math.pi * (k % steps) / steps)
        grid.append([cushion.verts.new((x, -(r + bulge * math.sin(a) ** 0.5) * math.cos(a), curb + (r + bulge * math.sin(a) ** 0.5) * math.sin(a))) for a in arc])
    for k in range(len(grid) - 1):
        for i in range(len(arc) - 1):
            face = cushion.faces.new([grid[k][i], grid[k + 1][i], grid[k + 1][i + 1], grid[k][i + 1]])
            for loop in face.loops:
                loop[uv].uv = (loop.vert.co.x / 4.0, (loop.vert.co.z + loop.vert.co.y) / 4.0)
    b.put(cushion, m["etfe"])
    for a in (0.3, 0.75, 1.2, math.pi / 2, math.pi - 1.2, math.pi - 0.75, math.pi - 0.3):
        y, z = -r * 0.99 * math.cos(a), curb + r * 0.99 * math.sin(a)
        b.put(tube([(0, y, z), (length, y, z)], 0.06, 8), m["white_frame"])
    # Curb walls with vents, floor, and gable walls.
    for s in (-1, 1):
        b.put(box((length + 0.6, 0.5, curb + 0.35), (length / 2, s * (r + 0.05), (curb - 0.35) / 2), tile=2.0), m["concrete"], smooth=False)
        for k in range(1, bays, 2):
            b.put(box((0.6, 0.06, 0.3), (k * bay, s * (r + 0.32), curb * 0.55)), m["grate"], smooth=False)
    b.put(box((length, width, 0.12), (length / 2, 0, 0.06), tile=2.0), m["floor"], smooth=False)
    for x, facing in ((0.0, -1.0), (length, 1.0)):
        b.put(half_disc(r, x, curb, facing, 24, 2.0), m["hull"], smooth=False)
        b.put(box((0.3, width + 0.6, curb), (x, 0, curb / 2), tile=2.0), m["concrete"], smooth=False)
        # An airlock box with its own door, and a chiller by it.
        ax = x + facing * 1.6
        b.put(box((3.0, 3.2, 2.9), (ax, 0, 1.45), tile=2.0, bevel=0.05), m["hull"], smooth=False)
        b.put(box((3.1, 3.3, 0.25), (ax, 0, 3.0), tile=2.0), m["hull_grey"], smooth=False)
        door(b, m, (x + facing * 3.12, 0, 0.0), 0.0 if facing > 0 else math.pi, 1.3, 2.3)
        unit(b, m, (x + facing * 1.6, 3.6, 0), 0.0, (2.4, 1.1, 1.3), 2)
        b.places.append(("service", {"fault": "hvac", "at": (x + facing * 1.6, 3.6, 1.2)}))
    # Tables down both sides; the gearbox layer adds trays and lamps on them.
    table_len = 1.3
    tables = int((length - 3.0) / (table_len + 0.25))
    span = tables * (table_len + 0.25) - 0.25
    for s in (-1, 1):
        y = s * (r * 0.5)
        for t in range(tables):
            x = (length - span) / 2 + table_len / 2 + t * (table_len + 0.25)
            legs = bmesh.new()
            for lx in (-0.55, 0.55):
                for ly in (-0.6, 0.6):
                    merge(legs, box((0.05, 0.05, 0.82), (x + lx, y + ly, 0.53)))
            b.put(legs, m["steel"], smooth=False)
            b.put(box((table_len, 1.35, 0.06), (x, y, 0.96), tile=1.0), m["frame"], smooth=False)
            b.places.append(("tray", {"at": (x, y, 0.99), "yaw": 0.0, "size": (table_len - 0.1, 1.25), "lamp": 1.05}))
            b.put(rod((x, y, 2.04 + 0.05), (x, y, curb + r * 0.97), 0.015, 6), m["frame"])
    b.put(box((length - 2.0, 1.4, 0.04), (length / 2, 0, 0.14), tile=1.0), m["grate"], smooth=False)
    b.places.append(("room", {"name": name.lower(), "centre": (length / 2, 0, (curb + r) / 2), "size": (length, width, curb + r),
                              "volume": length * (math.pi * r * r / 2 + width * curb), "glazing": 0.5}))
    # What stops a rover: the vault, curbs, floor, gables, airlocks and the rows of tables.
    b.collider(vault(0.0, length, r, curb, 8, 4.0))
    for s in (-1, 1):
        b.collider(box((length + 0.6, 0.5, curb + 0.35), (length / 2, s * (r + 0.05), (curb - 0.35) / 2)))
        b.collider(box((span, 1.35, 0.99), (length / 2, s * r * 0.5, 0.495)))
    b.collider(box((length, width, 0.12), (length / 2, 0, 0.06)))
    for x, facing in ((0.0, -1.0), (length, 1.0)):
        b.collider(half_disc(r, x, curb, facing, 8, 2.0))
        b.collider(box((0.3, width + 0.6, curb), (x, 0, curb / 2)))
        b.collider(box((3.0, 3.2, 2.9), (x + facing * 1.6, 0, 1.45)))
    return b.finish()
