"""The base's materials, shared details, and its greenhouses.

Each structure is built about its own origin, Z up, its front toward +X, and
returns a finished `Building`. `building.places` lists what the gearbox layer
adds inside it: rooms, trays, service spots and lights, in building space.
"""
import math

import bmesh
import numpy as np
from mathutils import Matrix, Vector

import patterns
from kit import (TAU, Building, box, cone, disc, half_disc, ibeam, lathe, merge, moved, project_uv, rect, rod, sphere, sweep, tube, vault)

_MATS = {}

# Top of the arch ridge robot's rail hangers: robots.arch_rail placed 5 m up.
ARCH_HANGERS = 5.89


def gantry_rail(radius):
    """Radius of a dome's floor rail, where its polar gantry's outer leg
    runs: inside the vestibule's mouth."""
    return radius - 2.0


def _film():
    """ETFE foil over a 4 m tile: clear with a faint milky haze, fine creases
    and welded seams every third of the tile, as RGBA colour, normals and
    roughness."""
    u, v = patterns.grid()
    f = (v * 3.0) % 1.0
    weld = np.clip(1.0 - np.minimum(f, 1.0 - f) / 0.015, 0.0, 1.0)
    haze = patterns.noise(cells=6, seed=21)
    crease = patterns.noise(cells=40, seed=22, octaves=2)
    colour = np.array([0.76, 0.81, 0.83], np.float32) * (0.95 + 0.08 * haze)[..., None] + 0.12 * weld[..., None]
    alpha = 0.10 + 0.07 * haze + 0.4 * weld
    diff = np.concatenate([np.clip(colour, 0.0, 1.0), alpha[..., None]], axis=-1)
    height = weld + 0.12 * crease
    rough = np.clip(0.2 + 0.12 * haze + 0.3 * weld, 0.0, 1.0)
    return diff.astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 2.0), rough.astype(np.float32)


def _coil():
    """Condenser coil: bright aluminium fin edges, 64 to a tile, over dark
    tube rows, as colour, normals and roughness."""
    u, v = patterns.grid()
    fin = np.clip(1.0 - np.abs((u * 64.0) % 1.0 - 0.5) / 0.2, 0.0, 1.0)
    tubes = (np.abs((v * 12.0) % 1.0 - 0.5) < 0.07).astype(np.float32)
    shade = 0.1 + 0.42 * fin - 0.05 * tubes
    diff = shade[..., None] * np.array([1.0, 1.02, 1.05], np.float32)
    height = fin - 0.5 * tubes
    rough = 0.32 + 0.25 * (1.0 - fin)
    return diff.astype(np.float32), patterns.normals_from_height(height.astype(np.float32), 3.0), rough.astype(np.float32)


def mats(looks):
    """The base's material set, made once."""
    if _MATS.get("looks") is looks:
        return _MATS["set"]
    g = looks.get
    m = {
        "hull": g("hull_panels", (0.85, 0.85, 0.83), 0.6, 0.0, maps=lambda: patterns.panels((0.80, 0.80, 0.78), cols=1, rows=2, seam_w=0.003)),
        "hull_orange": g("hull_orange", (0.85, 0.40, 0.10), 0.6, 0.0, maps=lambda: patterns.panels((0.62, 0.16, 0.03), cols=1, rows=2, seed=4)),
        "hull_grey": g("hull_grey", (0.5, 0.5, 0.5), 0.55, 0.1, maps=lambda: patterns.panels((0.22, 0.23, 0.24), cols=2, rows=2, seed=8)),
        "radiator": g("radiator_panels", (0.87, 0.87, 0.87), 0.45, 0.1, maps=lambda: patterns.panels((0.84, 0.85, 0.86), cols=1, rows=16, rivets=False, grime=0.05, seed=2)),
        "steel": g("steel", (0.66, 0.67, 0.68), 0.38, 1.0),
        "dark": g("dark_metal", (0.12, 0.12, 0.13), 0.5, 0.8),
        "frame": g("frame", (0.30, 0.31, 0.33), 0.45, 0.85),
        "white_frame": g("white_frame", (0.85, 0.85, 0.83), 0.55, 0.0),
        "glass": g("glass", (0.62, 0.72, 0.75), 0.05, 0.0, alpha=0.16),
        "window": g("window", (0.05, 0.06, 0.07), 0.08, 0.0, emit=((1.0, 0.80, 0.55), 3.0)),
        "etfe": g("etfe", (0.86, 0.90, 0.92), 0.2, 0.0, alpha=0.16, maps=_film),
        "concrete": g("regolith_concrete", (0.46, 0.32, 0.24), 0.9, 0.0, "ribbed_concrete_wall"),
        "printed": g("printed_regolith", (0.68, 0.42, 0.27), 0.85, 0.0, "precast_concrete_wall", contrast=1.0),
        "floor": g("floor", (0.50, 0.49, 0.48), 0.7, 0.0, "hangar_concrete_floor"),
        "grate": g("grating", (0.42, 0.42, 0.42), 0.5, 0.9, "metal_grate_rusty", contrast=0.6),
        "gold": g("gold_foil", (1.0, 0.70, 0.28), 0.3, 1.0, "bi_stretch", contrast=1.6),
        "fabric": g("hab_fabric", (0.87, 0.85, 0.81), 0.75, 0.0, "bi_stretch", contrast=1.2),
        "bags": g("sandbags", (0.62, 0.36, 0.22), 0.95, 0.0, "hessian_230", contrast=1.2),
        "pad": g("pad", (0.44, 0.33, 0.26), 0.85, 0.0, "hexagonal_concrete_paving"),
        "tiles": g("heat_tiles", (0.05, 0.05, 0.05), 0.6, 0.0, maps=patterns.heat_tiles),
        "stainless": g("stainless", (0.80, 0.80, 0.82), 0.3, 1.0, "metal_plate", contrast=0.35),
        "solar": g("solar_cells", (0.05, 0.08, 0.20), 0.15, 0.3, maps=patterns.solar),
        "hazard": g("hazard", (0.95, 0.65, 0.05), 0.6, 0.0, maps=patterns.stripes),
        "yellow": g("safety_yellow", (0.95, 0.68, 0.06), 0.55, 0.0),
        "paint": g("pad_paint", (0.88, 0.88, 0.86), 0.6, 0.0),
        "soil": g("soil", (0.22, 0.13, 0.08), 0.95, 0.0, "rough_concrete", contrast=1.4),
        "regolith": g("regolith_pile", (0.33, 0.23, 0.17), 0.95, 0.0, "rough_concrete", contrast=1.3),
        "led": g("led_bar", (0.95, 0.30, 0.65), 0.3, 0.0, emit=((0.95, 0.30, 0.65), 8.0)),
        "red": g("beacon", (1.0, 0.1, 0.05), 0.3, 0.0, emit=((1.0, 0.1, 0.05), 25.0)),
        "green": g("beacon_green", (0.1, 1.0, 0.2), 0.3, 0.0, emit=((0.1, 1.0, 0.2), 25.0)),
        "lamp": g("floodlamp", (1.0, 0.95, 0.85), 0.3, 0.0, emit=((1.0, 0.95, 0.85), 40.0)),
        "rubber": g("rubber", (0.07, 0.07, 0.07), 0.85, 0.0, "rubber_tiles"),
        "container_orange": g("container_orange", (0.78, 0.36, 0.12), 0.6, 0.3, "container_side", contrast=0.8),
        "container_white": g("container_white", (0.84, 0.84, 0.82), 0.6, 0.3, "container_side", contrast=0.8),
        "container_blue": g("container_blue", (0.12, 0.26, 0.50), 0.6, 0.3, "container_side", contrast=0.8),
        "corrugated": g("corrugated", (0.80, 0.80, 0.78), 0.45, 0.8, "corrugated_iron_02", contrast=0.7),
        "pipe_white": g("pipe_insulated", (0.85, 0.84, 0.80), 0.65, 0.0, "bi_stretch", contrast=0.6),
        "pipe_yellow": g("pipe_gas", (0.90, 0.70, 0.10), 0.55, 0.0),
        "pipe_blue": g("pipe_water", (0.15, 0.35, 0.65), 0.55, 0.0),
        "cable": g("cable", (0.04, 0.04, 0.04), 0.7, 0.0),
        "tray": g("tray", (0.12, 0.12, 0.13), 0.6, 0.0),
        "alu": g("aluminium", (0.74, 0.75, 0.76), 0.38, 0.9),
        "galv": g("galv_steel", (0.62, 0.63, 0.62), 0.5, 0.85),
        "gasket": g("epdm", (0.045, 0.045, 0.045), 0.8, 0.0),
        "dome_glass": g("laminated_glass", (0.70, 0.78, 0.77), 0.06, 0.0, alpha=0.13),
        "coil": g("coil_fins", (0.5, 0.52, 0.54), 0.4, 0.8, maps=_coil),
        "grow_lamp": g("grow_lamp", (1.0, 0.80, 0.92), 0.3, 0.0, emit=((1.0, 0.62, 0.85), 5.0)),
        "strap": g("kevlar_webbing", (0.70, 0.56, 0.28), 0.8, 0.0, "bi_stretch", contrast=0.6),
        "cast": g("cast_regolith", (0.47, 0.36, 0.28), 0.9, 0.0, "rough_concrete", contrast=0.9),
    }
    m["regolith"]["gearbox_ground"] = True
    _MATS.update(looks=looks, set=m)
    return m


# Geometry


def _frame(t, up):
    """A rotation taking X along `t` and Z toward `up`."""
    t = Vector(t).normalized()
    s = Vector(up).cross(t).normalized()
    return Matrix((t, s, t.cross(s))).transposed()


def _bar(bm, a, c, w, h, up=(0, 0, 1)):
    """Appends a straight bar from a to c, `w` across and `h` deep toward
    `up`, to bm."""
    a, c = Vector(a), Vector(c)
    rot = _frame(c - a, up)
    s, u = rot.col[1], rot.col[2]
    ring = [s * (y * w / 2) + u * (z * h / 2) for y, z in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    lo = [bm.verts.new(a + d) for d in ring]
    hi = [bm.verts.new(c + d) for d in ring]
    bm.faces.new(lo[::-1])
    bm.faces.new(hi)
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new((lo[i], lo[j], hi[j], hi[i]))


def _drum(bm, c, axis, radius, depth, segments=10):
    """Appends a closed cylinder centred at c along `axis` to bm."""
    axis = Vector(axis).normalized()
    rot = _frame(axis, axis.orthogonal())
    s, u = rot.col[1], rot.col[2]
    angles = [TAU * i / segments for i in range(segments)]
    lo, hi = ([bm.verts.new(Vector(c) + axis * x + (s * math.cos(a) + u * math.sin(a)) * radius) for a in angles] for x in (-depth / 2, depth / 2))
    bm.faces.new(lo[::-1])
    bm.faces.new(hi)
    for i in range(segments):
        j = (i + 1) % segments
        bm.faces.new((lo[i], lo[j], hi[j], hi[i]))


def _pillow(bm, c, yaw, size):
    """Appends a sandbag, a squared-off pillow of `size` turned by `yaw`, to bm."""
    lx, ly, lz = (s / 2 for s in size)
    rot = Matrix.Rotation(yaw, 3, "Z")
    c = Vector(c)
    rings = []
    for z in (-0.55, 0.55):
        row = []
        for i in range(8):
            ca, sa = math.cos(TAU * i / 8), math.sin(TAU * i / 8)
            row.append(bm.verts.new(c + rot @ Vector((lx * math.copysign(abs(ca) ** 0.5, ca), ly * math.copysign(abs(sa) ** 0.5, sa), lz * z))))
        rings.append(row)
    lo, hi = rings
    bottom, top = bm.verts.new(c - Vector((0, 0, lz))), bm.verts.new(c + Vector((0, 0, lz)))
    for i in range(8):
        j = (i + 1) % 8
        bm.faces.new((lo[i], lo[j], hi[j], hi[i]))
        bm.faces.new((hi[i], hi[j], top))
        bm.faces.new((lo[j], lo[i], bottom))


def _hash(i, j, seed=0.0):
    """A repeatable pseudo-random number in [0, 1) for a lattice point."""
    v = math.sin(i * 12.9898 + j * 78.233 + seed * 37.719) * 43758.5453
    return v - math.floor(v)


def _mound(r_in, r_out, height, a0, a1, segments, rows=8, seed=0.0):
    """A dumped regolith berm about Z between angles a0 and a1: `height`
    high at r_in, sloping out to the ground at r_out, its surface lumpy and
    its ends cut flat."""
    bm = bmesh.new()
    grid = []
    for i in range(segments + 1):
        a = a0 + (a1 - a0) * i / segments
        row = []
        for j in range(rows + 1):
            t = j / rows
            lump = 0.0 if j == 0 else (_hash(i, j, seed) - 0.5) * 0.16 + 0.08 * math.sin(7.0 * a + 2.0 * j + seed)
            r = r_in + (r_out - r_in) * t + 0.6 * lump
            z = height * (1.0 - t ** 1.6) + (lump if j < rows else 0.0)
            row.append(bm.verts.new((r * math.cos(a), r * math.sin(a), max(z, 0.0))))
        grid.append(row)
    for i in range(segments):
        for j in range(rows):
            bm.faces.new((grid[i][j], grid[i][j + 1], grid[i + 1][j + 1], grid[i + 1][j]))
    for row, a, side in ((grid[0], a0, -1.0), (grid[-1], a1, 1.0)):
        face = bm.faces.new(row + [bm.verts.new((r_in * math.cos(a), r_in * math.sin(a), 0.0))])
        face.normal_update()
        if face.normal.dot(Vector((-math.sin(a), math.cos(a), 0.0)) * side) < 0:
            face.normal_flip()
    return _uv(bm, 1.5)


def _uv(bm, tile):
    """Box-projected UVs once a directly built bmesh's normals are known."""
    bm.normal_update()
    project_uv(bm, tile)
    return bm


def _sector(profile, a0, a1, segments, tile=2.0, swap=False):
    """A solid of revolution about Z between angles a0 and a1: the closed
    (radius, z) outline `profile` swept round, with flat ends."""
    bm = lathe(profile + profile[:1], segments, tile, a1 - a0, a0, swap)
    uv = bm.loops.layers.uv["st"]
    for a, side in ((a0, -1.0), (a1, 1.0)):
        c, s = math.cos(a), math.sin(a)
        face = bm.faces.new([bm.verts.new((r * c, r * s, z)) for r, z in profile])
        face.normal_update()
        if face.normal.dot(Vector((-s, c, 0.0)) * side) < 0:
            face.normal_flip()
        for loop in face.loops:
            co = loop.vert.co
            st = (math.hypot(co.x, co.y) / tile, co.z / tile)
            loop[uv].uv = st[::-1] if swap else st
    return bm


def _panel(points, x, facing, tile=2.0):
    """A flat polygon in the plane X = x from (y, z) points, facing ±X."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    face = bm.faces.new([bm.verts.new((x, y, z)) for y, z in points])
    face.normal_update()
    if face.normal.x * facing < 0:
        face.normal_flip()
    for loop in face.loops:
        loop[uv].uv = (loop.vert.co.y / tile, loop.vert.co.z / tile)
    return bm


def _rounded(w, h, rc, n=4):
    """A rounded rectangle `h` tall and `w` wide as a sweep profile: (up,
    across) points round its outline."""
    pts = []
    for cx, cy, a0 in ((h / 2 - rc, w / 2 - rc, 0.0), (rc - h / 2, w / 2 - rc, math.pi / 2), (rc - h / 2, rc - w / 2, math.pi), (h / 2 - rc, rc - w / 2, 1.5 * math.pi)):
        for i in range(n + 1):
            a = a0 + math.pi / 2 * i / n
            pts.append((cx + rc * math.cos(a), cy + rc * math.sin(a)))
    return pts


# Shared details


def _hatch(b, m, at, yaw, w, h):
    """A pressure door facing +X of its yaw: a rebated steel frame on a
    hazard sill, a recessed leaf with a raised panel, kick plate and vision
    panel, three hinges, a lever handle and a keypad by its latch."""
    frame = box((0.16, w + 0.4, 0.2), (0, 0, h + 0.1))
    for s in (-1, 1):
        merge(frame, box((0.16, 0.2, h + 0.2), (0, s * (w / 2 + 0.1), h / 2)))
        merge(frame, box((0.03, 0.03, h), (0.06, s * (w / 2 - 0.015), h / 2)))
    merge(frame, box((0.03, w, 0.03), (0.06, 0, h - 0.015)))
    b.put(moved(frame, at, yaw), m["frame"], smooth=False)
    leaf = box((0.06, w - 0.02, h - 0.02), (0.0, 0, h / 2), tile=1.0)
    merge(leaf, box((0.015, w - 0.24, h * 0.32), (0.035, 0, h * 0.3), tile=1.0))
    b.put(moved(leaf, at, yaw), m["hull_grey"], smooth=False)
    b.put(moved(box((0.008, w - 0.1, 0.25), (0.034, 0, 0.16), tile=0.5), at, yaw), m["stainless"], smooth=False)
    vision = bmesh.new()
    for s in (-1, 1):
        merge(vision, box((0.02, 0.04, 0.46), (0.04, s * 0.17, h * 0.72)))
        merge(vision, box((0.02, 0.38, 0.04), (0.04, 0, h * 0.72 + s * 0.21)))
    merge(vision, box((0.03, 0.12, 0.18), (0.095, w / 2 + 0.1, 1.3)))
    b.put(moved(vision, at, yaw), m["dark"], smooth=False)
    b.put(moved(box((0.01, 0.3, 0.38), (0.036, 0, h * 0.72)), at, yaw), m["window"], smooth=False)
    b.put(moved(box((0.005, 0.03, 0.03), (0.112, w / 2 + 0.1, 1.36)), at, yaw), m["green"], smooth=False)
    metal = bmesh.new()
    for z in (0.25, h / 2, h - 0.25):
        merge(metal, rod((0.09, -w / 2 - 0.01, z - 0.08), (0.09, -w / 2 - 0.01, z + 0.08), 0.025, 8))
        merge(metal, box((0.012, 0.09, 0.14), (0.082, -w / 2 - 0.05, z)))
    merge(metal, box((0.02, 0.06, 0.2), (0.04, w / 2 - 0.12, 1.05)))
    merge(metal, rod((0.04, w / 2 - 0.12, 1.1), (0.08, w / 2 - 0.12, 1.1), 0.014, 6))
    merge(metal, rod((0.08, w / 2 - 0.12, 1.1), (0.08, w / 2 - 0.28, 1.1), 0.014, 6))
    b.put(moved(metal, at, yaw), m["steel"])
    b.put(moved(box((0.22, w + 0.4, 0.08), (0.04, 0, 0.04), tile=0.5), at, yaw), m["hazard"], smooth=False)


def door(b, m, at, yaw, w=1.2, h=2.1):
    """A pressure door facing +X of its yaw, as `_hatch`, with a bulkhead
    lamp over it."""
    _hatch(b, m, at, yaw, w, h)
    housing = box((0.08, 0.06, 0.12), (0.05, 0, h + 0.27))
    merge(housing, box((0.16, 0.36, 0.1), (0.12, 0, h + 0.36)))
    b.put(moved(housing, at, yaw), m["dark"], smooth=False)
    b.put(moved(box((0.12, 0.3, 0.015), (0.13, 0, h + 0.305)), at, yaw), m["lamp"], smooth=False)
    b.places.append(("light", {"at": tuple(Vector(at) + Vector((math.cos(yaw), math.sin(yaw), 0)) * 0.4 + Vector((0, 0, h + 0.3))), "kind": "door"}))


def unit(b, m, at, yaw=0.0, size=(1.6, 1.0, 1.2), fans=2):
    """An air-cooled chiller or air handler on a skid: a panelled cabinet on
    corner posts, finned coils behind guard bars down its long sides, fan
    shrouds with blades, motors and wire guards on top, an access panel and
    an electrical box at its ends, and an insulated pipe stub with a valve."""
    w, d, h = size
    body = h - 0.1

    def put(bm, mat, smooth=True):
        b.put(moved(bm, at, yaw), m[mat], smooth=smooth)

    skid = box((w + 0.1, d + 0.1, 0.1), (0, 0, 0.05))
    for s in (-1, 1):
        merge(skid, box((0.05, 0.1, 0.1), (s * (w / 2 + 0.07), 0, 0.08)))
    put(skid, "frame")
    put(box((w - 0.08, d - 0.08, body), (0, 0, 0.1 + body / 2), tile=1.0, bevel=0.01), "hull")
    posts = box((w, d, 0.05), (0, 0, h - 0.025))
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(posts, box((0.06, 0.06, body), (sx * (w / 2 - 0.03), sy * (d / 2 - 0.03), 0.1 + body / 2)))
        merge(posts, box((0.015, d * 0.62, body * 0.7), (sx * (w / 2 - 0.035), 0, 0.1 + body * 0.45), tile=1.0))
    put(posts, "hull_grey")
    coils, guards = bmesh.new(), bmesh.new()
    coil_h = body * 0.62
    for s in (-1, 1):
        merge(coils, box((w - 0.16, 0.03, coil_h), (0, s * (d / 2 - 0.03), 0.1 + body * 0.46), tile=1.0))
        for k in (-1, 0, 1):
            z = 0.1 + body * 0.46 + coil_h * 0.33 * k
            merge(guards, rod((0.08 - w / 2, s * d / 2, z), (w / 2 - 0.08, s * d / 2, z), 0.008, 4))
    put(coils, "coil")
    rf = min(d * 0.4, w / fans * 0.42)
    shrouds, blades = bmesh.new(), bmesh.new()
    for k in range(fans):
        x = -w / 2 + w * (k + 0.5) / fans
        merge(shrouds, moved(lathe([(rf + 0.04, h - 0.01), (rf + 0.04, h + 0.1), (rf, h + 0.1), (rf, h - 0.01)], 24, 1.0), (x, 0, 0)))
        merge(shrouds, cone(0.07, 0.07, 0.12, (x, 0, h - 0.04), 12))
        for i in range(5):
            a = TAU * i / 5
            merge(blades, box((rf * 0.8, 0.1, 0.01), (x + math.cos(a) * rf * 0.45, math.sin(a) * rf * 0.45, h + 0.04),
                              rot=Matrix.Rotation(a, 4, "Z") @ Matrix.Rotation(0.35, 4, "X")))
        for f in (0.35, 0.65, 0.97):
            merge(guards, disc(rf * f + 0.006, h + 0.105, 24, 1.0, rf * f - 0.006))
        for a in (0.0, math.pi / 2):
            merge(guards, box((2 * rf, 0.012, 0.012), (x, 0, h + 0.105), rot=Matrix.Rotation(a, 4, "Z")))
    put(shrouds, "dark")
    put(blades, "frame", smooth=False)
    put(guards, "steel")
    ends = box((0.14, 0.34, min(0.45, body * 0.6)), (-(w / 2 + 0.07), -d * 0.18, 0.1 + body * 0.5))
    merge(ends, rod((-(w / 2 + 0.07), 0.1 - d * 0.18, 0.1 + body * 0.5 - min(0.45, body * 0.6) / 2), (-(w / 2 + 0.07), 0.1 - d * 0.18, 0.05), 0.02, 6))
    for z in (0.1 + body * 0.3, 0.1 + body * 0.62):
        merge(ends, box((0.03, 0.12, 0.03), (w / 2 - 0.02, d * 0.22, z)))
    put(ends, "hull_grey")
    put(tube([(w / 2 - 0.2, d / 2, 0.4), (w / 2 - 0.2, d / 2 + 0.3, 0.4), (w / 2 - 0.2, d / 2 + 0.3, 0.05)], 0.06, 10), "pipe_white")
    put(box((0.16, 0.16, 0.12), (w / 2 - 0.2, d / 2 + 0.3, 0.24)), "steel")


def railing(b, m, path, height=1.0, step=1.5):
    """A guard rail along a path: posts on base plates, a top rail, a knee
    rail, and a toe board along its level runs."""
    path = [Vector(p) for p in path]
    posts, plates, toes = bmesh.new(), bmesh.new(), bmesh.new()
    for n, (a, c) in enumerate(zip(path, path[1:])):
        count = max(1, int((c - a).length / step))
        for k in range(1 if n else 0, count + 1):
            p = a + (c - a) * (k / count)
            merge(posts, rod(p, p + Vector((0, 0, height)), 0.024, 6))
            merge(plates, box((0.1, 0.1, 0.012), tuple(p + Vector((0, 0, 0.006)))))
        if abs(c.z - a.z) < 0.02 * (c - a).length:
            merge(toes, sweep(rect(0.1, 0.008), [a + Vector((0, 0, 0.06)), c + Vector((0, 0, 0.06))], 1.0))
    b.put(posts, m["yellow"])
    b.put(plates, m["steel"], smooth=False)
    for z, radius in ((height, 0.025), (height * 0.5, 0.018)):
        b.put(tube([p + Vector((0, 0, z)) for p in path], radius, 6), m["yellow"])
    if len(toes.verts):
        b.put(toes, m["yellow"], smooth=False)
    else:
        toes.free()


def stairs(b, m, at, yaw, rise, width=1.0, going=0.28, riser=0.2):
    """A steel stair climbing +X from `at` by `rise`: channel stringers on
    base plates, grating treads on end cleats with yellow nosings, and rails."""
    steps = max(1, int(round(rise / riser)))
    run = steps * going
    step = rise / steps
    treads, nosings, steel = bmesh.new(), bmesh.new(), bmesh.new()
    for k in range(steps):
        z = (k + 1) * step
        merge(treads, box((going, width - 0.04, 0.035), (k * going + going / 2, 0, z - 0.0175), tile=1.0))
        merge(nosings, box((0.035, width - 0.04, 0.036), (k * going + 0.0175, 0, z - 0.018)))
        for s in (-1, 1):
            merge(steel, box((going * 0.8, 0.008, 0.07), (k * going + going / 2, s * (width / 2 - 0.025), z - 0.06)))
    for s in (-1, 1):
        channel = [(-0.1, 0.0), (0.1, 0.0), (0.1, 0.065 * s), (0.09, 0.065 * s), (0.09, 0.01 * s), (-0.09, 0.01 * s), (-0.09, 0.065 * s), (-0.1, 0.065 * s)]
        merge(steel, sweep(channel, [(-0.05, s * width / 2, 0.5 * step - 0.05 * rise / run), (run, s * width / 2, rise + 0.5 * step)], 1.0))
        merge(steel, box((0.22, 0.14, 0.012), (0.06, s * (width / 2 - 0.04), 0.006)))
    b.put(moved(treads, at, yaw), m["grate"], smooth=False)
    b.put(moved(nosings, at, yaw), m["yellow"], smooth=False)
    b.put(moved(steel, at, yaw), m["steel"])
    for s in (-1, 1):
        p0 = Vector(at) + Vector((-math.sin(yaw), math.cos(yaw), 0)) * (s * width / 2) + Vector((0, 0, 0.5 * step))
        p1 = p0 + Vector((math.cos(yaw), math.sin(yaw), 0)) * run + Vector((0, 0, rise))
        railing(b, m, [p0, p1], 0.95, 1.0)


def light_mast(b, m, at, height=8.0, heads=2, yaw=0.0):
    """A floodlight mast: a tapered octagonal pole on a base plate bolted to
    a concrete pier, a hand-hole and a junction box at its foot, a cross-arm
    carrying LED flood heads tilted on yokes, and an obstruction light."""
    at = Vector(at)
    b.put(moved(box((0.8, 0.8, 0.3), (0, 0, 0.15), bevel=0.03), tuple(at)), m["cast"], smooth=False)
    foot = box((0.44, 0.44, 0.03), (0, 0, 0.315))
    for sx in (-1, 1):
        for sy in (-1, 1):
            merge(foot, cone(0.022, 0.022, 0.09, (sx * 0.16, sy * 0.16, 0.3), 6))
            merge(foot, box((0.06, 0.06, 0.025), (sx * 0.16, sy * 0.16, 0.345)))
    merge(foot, cone(0.17, 0.13, 0.3, (0, 0, 0.33), 8))
    b.put(moved(foot, tuple(at)), m["steel"], sharp=30)
    b.put(moved(cone(0.13, 0.07, height - 0.33, (0, 0, 0.33), 8), tuple(at)), m["steel"], sharp=30)
    gear = box((0.03, 0.12, 0.3), (0.13, 0, 0.95))
    merge(gear, box((0.14, 0.26, 0.34), (-0.2, 0, 1.45)))
    merge(gear, rod((-0.2, 0, 1.28), (-0.2, 0, 0.3), 0.025, 6))
    b.put(moved(gear, tuple(at), yaw), m["hull_grey"], smooth=False)
    arm = box((0.1, 1.5, 0.1), (0, 0, height))
    for s in (-1, 1):
        merge(arm, box((0.12, 0.02, 0.12), (0, s * 0.75, height)))
    merge(arm, rod((0, 0, height + 0.05), (0, 0, height + 0.1), 0.015, 6))
    b.put(moved(arm, tuple(at), yaw), m["steel"], smooth=False)
    b.put(moved(sphere(0.045, (0, 0, height + 0.13), 8, 4), tuple(at)), m["red"])
    for k in range(heads):
        y = -0.5 + k * (1.0 / max(heads - 1, 1))
        yoke = box((0.2, 0.52, 0.02), (0.08, y, height - 0.06))
        for s in (-1, 1):
            merge(yoke, box((0.03, 0.02, 0.14), (0.16, y + s * 0.25, height - 0.13)))
        b.put(moved(yoke, tuple(at), yaw), m["dark"], smooth=False)
        head = box((0.12, 0.44, 0.3), (0, 0, 0))
        for i in range(6):
            merge(head, box((0.06, 0.008, 0.28), (-0.09, -0.2 + 0.08 * i, 0)))
        lens = box((0.01, 0.4, 0.26), (0.062, 0, 0))
        for part, mat in ((head, "dark"), (lens, "lamp")):
            b.put(moved(moved(part, (0.16, y, height - 0.2), 0.0, 0.6), tuple(at), yaw), m[mat], smooth=False)
    b.places.append(("light", {"at": tuple(at + Vector((math.cos(yaw), math.sin(yaw), 0)) * 0.4 + Vector((0, 0, height - 0.1))), "kind": "flood", "yaw": yaw}))


def _lock(b, m, at, yaw, length, width=3.0, base=0.1, top=3.2, bore=None, axis=1.6):
    """A rigid airlock running `length` along +X of `yaw` from `at`, its
    building end: a rounded shell on footings with stiffener frames,
    portholes, a pipe run and a roof cable tray. With a `bore` its outer end
    is an active berthing ring, as a corridor's passive one, its face at
    `length` on an axis `axis` high, with latch housings and alignment
    guides; without one it closes in a flat bulkhead at `length`."""
    h, zc = top - base, (top + base) / 2
    rc = 0.6
    body = length - 0.35 if bore else length
    parts = {}

    def add(mat, bm):
        merge(parts.setdefault(mat, bmesh.new()), bm)

    shell = sweep(_rounded(width, h, rc), [(0, 0, zc), (body, 0, zc)], 2.0)
    uv = shell.loops.layers.uv["st"]
    for f in shell.faces:
        if abs(f.normal.x) > 0.9:
            for loop in f.loops:
                loop[uv].uv = (loop.vert.co.y / 2.0, loop.vert.co.z / 2.0)
    add("hull", shell)
    count = max(2, int(body / 1.4) + 1)
    for k in range(count):
        x = 0.08 + (body - 0.16) * k / (count - 1)
        add("hull_grey", sweep(_rounded(width + 0.12, h + 0.12, rc + 0.06), [(x - 0.06, 0, zc), (x + 0.06, 0, zc)], 1.0))
    for x in (body * 0.25, body * 0.75):
        add("cast", box((0.6, width + 0.3, base + 0.06), (x, 0, base / 2 - 0.03)))
    for x in ([body / 2] if body < 4.0 else [body * 0.22, body * 0.78]):
        for s in (-1, 1):
            add("hull_grey", moved(lathe([(0.27, 0.0), (0.27, 0.05), (0.18, 0.05), (0.18, 0.0)], 16, 1.0), (x, s * width / 2, zc + 0.3), 0, 0, -s * math.pi / 2))
            add("window", moved(disc(0.18, 0.03, 16, 1.0), (x, s * width / 2, zc + 0.3), 0, 0, -s * math.pi / 2))
    side = -(width / 2 + 0.16)
    for z, mat, radius in ((zc + 0.6, "pipe_blue", 0.05), (zc + 0.35, "pipe_yellow", 0.04)):
        add(mat, tube([(0.0, side, z), (body - 0.25, side, z)], radius, 8))
    for k in range(int(body / 1.5) + 1):
        add("steel", box((0.05, 0.2, 0.36), (min(0.4 + 1.5 * k, body - 0.35), side + 0.08, zc + 0.47)))
    add("hull_grey", box((0.3, 0.3, 0.45), (body - 0.15, side + 0.05, zc + 0.45)))
    for s in (-1, 1):
        add("galv", box((body - 0.2, 0.03, 0.08), (body / 2, s * 0.22, top + 0.16)))
        for k in range(int(body / 1.5) + 1):
            add("galv", box((0.04, 0.04, 0.12), (min(0.3 + 1.5 * k, body - 0.2), s * 0.22, top + 0.06)))
    for k in range(int((body - 0.3) / 0.3) + 1):
        add("galv", box((0.03, 0.44, 0.02), (0.15 + 0.3 * k, 0, top + 0.13)))
    for y in (-0.08, 0.06):
        add("cable", tube([(0.0, y, top + 0.17), (body - 0.12, y, top + 0.17)], 0.035, 6))
    if bore:
        rb, rf, back = bore + 0.08, bore + 0.26, length - 0.14
        add("hull_grey", moved(lathe([(rb, body - 0.02), (rb, back)], 32, 1.0), (0, 0, axis), 0, math.pi / 2))
        ring = [(bore - 0.05, back), (rf - 0.03, back), (rf, back + 0.03), (rf, length - 0.025), (rf - 0.025, length), (bore - 0.05, length)]
        add("alu", moved(lathe(ring + ring[:1], 48, 0.5), (0, 0, axis), 0, math.pi / 2))
        bolts = bmesh.new()
        for i in range(16):
            a = TAU * (i + 0.5) / 16
            _drum(bolts, (back - 0.025, (bore + 0.18) * math.cos(a), axis + (bore + 0.18) * math.sin(a)), (1, 0, 0), 0.03, 0.05, 6)
        add("steel", _uv(bolts, 0.5))
        for k in range(4):
            add("hull_grey", moved(box((0.36, 0.22, 0.12), (back - 0.2, 0, rb + 0.06), bevel=0.02), (0, 0, axis), 0, 0, TAU * (k + 0.5) / 4))
            add("alu", moved(box((0.22, 0.05, 0.08), (length - 0.01, 0, rf + 0.03)), (0, 0, axis), 0, 0, TAU * k / 4))
    for mat, bm in parts.items():
        b.put(moved(bm, at, yaw), m[mat], sharp=50)
    b.collider(moved(box((body, width, h), (body / 2, 0, zc)), at, yaw))
    return length


# Greenhouses


def geodesic_dome(looks, name, radius=12.0, wall=1.4, at=(0, 0, 0), yaw=0.0, parent=None, rings=3):
    """A glazed geodesic greenhouse: an 8-frequency shell of aluminium
    glazing bars and disc hubs carrying laminated glass under capped EPDM
    gaskets, springing from a steel ring beam on a printed-regolith knee wall
    banked with sandbags `wall` high. A rigid airlock through a portal at
    +X meets the corridor flange to flange and has an EVA hatch on its side;
    chillers stand round the berm, their pipes bridging it into the wall.
    Inside, printed terraces with steel copings, drip risers and gutters rise
    outward round a cistern, under rings of grow lights hung clear of the
    polar gantry's boom."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.places = []
    spring = 2.9
    beam = (spring - 0.345, spring - 0.045)
    centre = Vector((0.0, 0.0, spring))
    ico = bmesh.new()
    bmesh.ops.create_icosphere(ico, subdivisions=4, radius=1.0)
    bmesh.ops.delete(ico, geom=[v for v in ico.verts if v.co.z < -1e-4], context="VERTS")
    ico.verts.index_update()
    dirs = [v.co.normalized() for v in ico.verts]
    edges = [(e.verts[0].index, e.verts[1].index) for e in ico.edges]
    faces = [[v.index for v in f.verts] for f in ico.faces]
    ico.free()

    def node(i, r):
        return centre + dirs[i] * r

    # The shell: glazing bars, capped gaskets over glass, hubs at the nodes.
    bars, caps, seals, glazing, hubs, bolts = (bmesh.new() for _ in range(6))
    for i, j in edges:
        up = dirs[i] + dirs[j]
        _bar(bars, node(i, radius - 0.11), node(j, radius - 0.11), 0.09, 0.2, up)
        for into, r, w, h in ((caps, radius + 0.03, 0.07, 0.04), (seals, radius + 0.006, 0.11, 0.012)):
            a, c = node(i, r), node(j, r)
            d = (c - a).normalized() * 0.12
            _bar(into, a + d, c - d, w, h, up)
    for f in faces:
        pts = [node(i, radius) for i in f]
        mid = sum(pts, Vector()) / len(pts)
        face = glazing.faces.new([glazing.verts.new(mid + (p - mid) * 0.95) for p in pts])
        face.normal_update()
        if face.normal.dot(mid - centre) < 0:
            face.normal_flip()
    for i, d in enumerate(dirs):
        _drum(hubs, node(i, radius - 0.12), d, 0.15, 0.24, 10)
        _drum(hubs, node(i, radius + 0.06), d, 0.12, 0.03, 8)
        _drum(bolts, node(i, radius + 0.085), d, 0.035, 0.03, 6)
    b.put(_uv(bars, 1.0), m["alu"], sharp=30)
    b.put(_uv(caps, 1.0), m["alu"], sharp=30)
    b.put(_uv(seals, 1.0), m["gasket"], sharp=30)
    b.put(_uv(glazing, 4.0), m["dome_glass"], smooth=False)
    b.put(_uv(hubs, 0.5), m["alu"], sharp=30)
    b.put(_uv(bolts, 0.5), m["steel"], sharp=30)
    # Ring beam with a shoe and anchor bolts under each base node, and its flashing.
    shoes = bmesh.new()
    for d in dirs:
        if abs(d.z) < 1e-3:
            a = math.atan2(d.y, d.x)
            p = Vector((math.cos(a), math.sin(a), 0.0)) * (radius - 0.1)
            merge(shoes, box((0.34, 0.3, 0.03), (p.x, p.y, beam[1] + 0.015), rot=Matrix.Rotation(a, 4, "Z")))
            for s in (-1, 1):
                q = p + Vector((-math.sin(a), math.cos(a), 0.0)) * (s * 0.11)
                merge(shoes, rod((q.x, q.y, beam[1]), (q.x, q.y, beam[1] + 0.09), 0.018, 6))
    b.put(shoes, m["steel"])
    b.put(lathe([(radius + 0.1, beam[0]), (radius + 0.1, beam[1]), (radius - 0.3, beam[1]), (radius - 0.3, beam[0])], 160, 1.0), m["galv"], sharp=40)
    b.put(lathe([(radius + 0.56, beam[0] - 0.06), (radius + 0.1, beam[0] + 0.08)], 160, 1.0), m["alu"], sharp=40)
    # Knee wall, printed in courses, the floor slab, and the entrance portal.
    b.put(lathe([(radius + 0.9, -0.1), (radius + 0.5, beam[0]), (radius - 0.4, beam[0]), (radius - 0.4, 0.3)], 160, 0.8, swap=True), m["concrete"], sharp=50)
    b.put(lathe([(radius - 0.39, 0.0), (radius - 0.39, 0.35), (0.0, 0.35)], 128, 3.0), m["floor"], sharp=50)
    b.put(box((0.8, 4.0, 3.75), (radius + 0.55, 0, 1.875), tile=2.0, bevel=0.04), m["cast"], smooth=False)
    lamps = bmesh.new()
    for s in (-1, 1):
        merge(lamps, box((0.16, 0.4, 0.12), (radius + 1.03, s * 1.75, 3.55)))
    b.put(lamps, m["dark"], smooth=False)
    b.put(box((0.12, 0.34, 0.015), (radius + 1.05, 1.75, 3.485)), m["lamp"], smooth=False)
    b.put(box((0.12, 0.34, 0.015), (radius + 1.05, -1.75, 3.485)), m["lamp"], smooth=False)
    # A regolith berm `wall` high banked on the knee wall, open where the
    # airlock passes, its cut ends held by sandbags.
    berm_in, berm_out = radius + 0.6, radius + 2.4
    cut = math.asin(2.1 / (radius + 1.0))
    b.put(_mound(berm_in, berm_out, wall, cut, TAU - cut, 144, 8, 3.0), m["regolith"], sharp=70)
    bags = bmesh.new()
    for a, s in ((cut, 1.0), (TAU - cut, -1.0)):
        radial, side = Vector((math.cos(a), math.sin(a), 0.0)), Vector((-math.sin(a), math.cos(a), 0.0))
        for c in range(int(wall / 0.19)):
            for q in range(3):
                rc = berm_in + 0.3 + 0.6 * q + 0.3 * (c % 2)
                if 0.1 + 0.19 * c > wall * (1.0 - ((rc - berm_in) / (berm_out - berm_in)) ** 1.6) - 0.05:
                    continue
                _pillow(bags, radial * rc - side * (s * 0.2) + Vector((0, 0, 0.1 + 0.19 * c)), a, (0.6, 0.38, 0.21))
    b.put(_uv(bags, 0.6), m["bags"], sharp=60)
    # The airlock, its ring's face where the corridor's starts, and its EVA
    # hatch on +Y with a landing and steps.
    reach = radius + 7.3
    length = _lock(b, m, (radius + 0.95, 0, 0), 0.0, reach - radius - 0.95, 3.0, 0.1, 3.3, 1.3, 1.6)
    xd = radius + 0.95 + (length - 0.35) / 2
    b.put(box((1.7, 0.3, 2.75), (xd, 1.65, 1.475), tile=1.0), m["hull"], smooth=False)
    door(b, m, (xd, 1.82, 0.35), math.pi / 2, 1.1, 2.1)
    landing = bmesh.new()
    for sx in (-1, 1):
        for y in (1.9, 2.75):
            merge(landing, rod((xd + sx * 0.75, y, 0.0), (xd + sx * 0.75, y, 0.3), 0.03, 6))
    b.put(landing, m["steel"])
    b.put(box((1.6, 1.0, 0.05), (xd, 2.3, 0.325), tile=1.0), m["grate"], smooth=False)
    stairs(b, m, (xd, 3.36, 0.0), -math.pi / 2, 0.35, 1.0)
    for sx in (-1, 1):
        railing(b, m, [(xd + sx * 0.78, 1.85, 0.35), (xd + sx * 0.78, 2.78, 0.35)], 1.0, 1.0)
    # Chillers round the berm, their insulated pipes bridging it into the wall.
    pipes, stands, conduits = bmesh.new(), bmesh.new(), bmesh.new()
    for a in (0.9, 1.25, -0.9, -1.25, 2.6, -2.6):
        radial, side = Vector((math.cos(a), math.sin(a), 0.0)), Vector((-math.sin(a), math.cos(a), 0.0))
        p = radial * (radius + 3.6)
        unit(b, m, tuple(p), a + math.pi / 2, (2.2, 1.2, 1.4), 2)
        b.collider(moved(box((2.2, 1.2, 1.4), (0, 0, 0.7)), tuple(p), a + math.pi / 2))
        for o in (-0.4, 0.4):
            merge(pipes, tube([radial * r + side * o + Vector((0, 0, z)) for r, z in
                               ((radius + 3.0, 0.9), (radius + 2.6, 0.9), (radius + 2.35, 1.95), (radius + 0.58, 1.95))], 0.07, 10))
            _drum(stands, radial * (radius + 0.62) + side * o + Vector((0, 0, 1.95)), radial, 0.11, 0.08, 10)
        merge(conduits, tube([radial * r + Vector((0, 0, z)) for r, z in ((radius + 3.0, 0.5), (radius + 2.6, 0.5), (radius + 2.4, 1.76), (radius + 0.6, 1.76))], 0.03, 6))
        foot = radial * (radius + 0.95)
        merge(stands, rod(foot + Vector((0, 0, wall - 0.05)), foot + Vector((0, 0, 1.87)), 0.03, 6))
        _bar(stands, foot - side * 0.55 + Vector((0, 0, 1.86)), foot + side * 0.55 + Vector((0, 0, 1.86)), 0.08, 0.04)
    b.put(pipes, m["pipe_white"])
    b.put(conduits, m["cable"])
    b.put(_uv(stands, 0.5), m["galv"])
    # A glazing cleaning gantry: a lattice arch from a bogie on a rail round
    # the wall top to a slewing bearing on the crown, with a brush carriage.
    track = radius + 0.32
    b.put(lathe([(track + 0.06, beam[0] + 0.01), (track + 0.06, beam[0] + 0.1), (track - 0.06, beam[0] + 0.1), (track - 0.06, beam[0] + 0.01)],
                160, 1.0, TAU - 0.6, 0.3), m["steel"], sharp=40)
    g = -2.0
    out, across = Vector((math.cos(g), math.sin(g), 0.0)), Vector((-math.sin(g), math.cos(g), 0.0))

    def meridian(r, e):
        return centre + (out * math.cos(e) + Vector((0, 0, math.sin(e)))) * r

    elev = [math.radians(1.5 + 86.0 * i / 24) for i in range(25)]
    truss = bmesh.new()
    for o in (-0.25, 0.25):
        for r in (radius + 0.4, radius + 0.75):
            merge(truss, tube([meridian(r, e) + across * o for e in elev], 0.035, 6))
    for k, e in enumerate(elev):
        merge(truss, rod(meridian(radius + 0.75, e) - across * 0.25, meridian(radius + 0.75, e) + across * 0.25, 0.018, 4))
        for o in (-0.25, 0.25):
            merge(truss, rod(meridian(radius + 0.4, e) + across * o, meridian(radius + 0.75, e) + across * o, 0.018, 4))
            if k + 1 < len(elev):
                merge(truss, rod(meridian(radius + 0.4, e) + across * o, meridian(radius + 0.75, elev[k + 1]) + across * o, 0.015, 4))
    crown = centre + Vector((0, 0, radius + 0.42))
    for r in (radius + 0.4, radius + 0.75):
        for o in (-0.25, 0.25):
            merge(truss, rod(meridian(r, elev[-1]) + across * o, crown + out * 0.25 + across * o, 0.025, 6))
    b.put(truss, m["white_frame"])
    gear = bmesh.new()
    _drum(gear, crown, (0, 0, 1), 0.32, 0.26, 16)
    _drum(gear, crown - Vector((0, 0, 0.2)), (0, 0, 1), 0.18, 0.16, 12)
    _bar(gear, out * (radius + 0.15) + Vector((0, 0, beam[0] + 0.38)), out * (radius + 1.0) + Vector((0, 0, beam[0] + 0.38)), 0.8, 0.56)
    _bar(gear, out * (radius + 0.6) - across * 0.7 + Vector((0, 0, beam[0] + 0.4)), out * (radius + 0.6) - across * 0.4 + Vector((0, 0, beam[0] + 0.4)), 0.3, 0.26)
    for o in (-0.3, 0.3):
        _drum(gear, out * track + across * o + Vector((0, 0, beam[0] + 0.17)), across, 0.08, 0.06, 10)
    carriage = meridian(radius + 0.4, math.radians(32))
    _bar(gear, carriage - across * 0.35, carriage + across * 0.35, 0.3, 0.4, out)
    b.put(_uv(gear, 0.5), m["dark"])
    brush = bmesh.new()
    tip = meridian(radius + 0.13, math.radians(32))
    merge(brush, tube([tip - across * 1.3, tip + across * 1.3], 0.05, 8))
    for o in (-0.8, 0.8):
        merge(brush, rod(carriage + across * (o * 0.3), tip + across * o, 0.02, 6))
    b.put(brush, m["cable"])
    # Inside: the polar gantry's floor rail with clips and painted margins.
    rail = gantry_rail(radius)
    b.put(lathe([(rail + 0.16, 0.35), (rail + 0.16, 0.365), (rail + 0.03, 0.365), (rail + 0.03, 0.395), (rail + 0.05, 0.4), (rail + 0.05, 0.42),
                 (rail - 0.05, 0.42), (rail - 0.05, 0.4), (rail - 0.03, 0.395), (rail - 0.03, 0.365), (rail - 0.16, 0.365), (rail - 0.16, 0.35)], 192, 1.0), m["steel"], sharp=40)
    clips = bmesh.new()
    count = int(TAU * rail)
    for i in range(count):
        a = TAU * i / count
        radial, side = Vector((math.cos(a), math.sin(a), 0.0)), Vector((-math.sin(a), math.cos(a), 0.0))
        for s in (-1, 1):
            p = radial * (rail + s * 0.09) + Vector((0, 0, 0.375))
            _bar(clips, p - side * 0.05, p + side * 0.05, 0.1, 0.02)
    b.put(_uv(clips, 0.5), m["dark"])
    for r0, r1 in ((rail + 0.37, rail + 0.42), (rail - 0.42, rail - 0.37)):
        b.put(disc(r1, 0.356, 192, 1.0, r0), m["yellow"])
    # Terraced crop rings inside, rising outward, from the cistern's court to
    # inside the gantry rail, cut for the aisle from the airlock where no
    # trays stand; ring 0 is crossed by a stile.
    trays = []
    inner, outer = 3.5, rail - 0.6
    band = (outer - inner) / rings
    width = band - 0.5
    trims, drips = bmesh.new(), bmesh.new()
    for k in range(rings):
        r0 = inner + k * band
        r1 = r0 + width
        h = 0.55 + 0.5 * k
        top = 0.35 + h
        cut = math.asin(0.85 / r0) if k else 0.0
        a0, span = cut, TAU - 2 * cut
        outline = [(r0, 0.35), (r1, 0.35), (r1, top), (r0, top)]
        body = _sector(outline, a0, a0 + span, 128, 0.8, swap=True) if k else lathe(outline[1:] + outline[:1], 128, 0.8, swap=True)
        b.put(body, m["concrete"], sharp=50)
        b.put(lathe([(r1 - 0.08, 0.36 + h), (r0 + 0.08, 0.36 + h)], 128, 1.0, span, a0), m["soil"])
        for edge, inward in ((r1, -1), (r0, 1)):
            b.put(lathe([(edge - inward * 0.012, top - 0.1), (edge - inward * 0.012, top + 0.025), (edge + inward * 0.07, top + 0.025), (edge + inward * 0.07, top)],
                        128, 1.0, span, a0), m["galv"], sharp=40)
        b.put(lathe([(r1 + 0.1, 0.35), (r1 + 0.1, 0.41), (r1 + 0.085, 0.41), (r1 + 0.085, 0.37), (r1, 0.37)], 128, 1.0, span, a0), m["galv"], sharp=40)
        for a in ((a0, a0 + span) if k else ()):
            for r in (r0, r1):
                p = Vector((r * math.cos(a), r * math.sin(a), 0.35))
                _bar(trims, p, p + Vector((0, 0, top - 0.33)), 0.06, 0.06, (math.cos(a), math.sin(a), 0.0))
        b.put(disc(inner + (k + 1) * band - 0.01 if k + 1 < rings else rail - 0.18, 0.37, 160, 1.0, r1 + 0.12), m["grate"])
        header = top - 0.25
        arc = [a0 + 0.03 + (span - 0.06) * i / 96 for i in range(97)]
        b.put(tube([((r1 + 0.04) * math.cos(a), (r1 + 0.04) * math.sin(a), header) for a in arc], 0.025, 6), m["pipe_blue"])
        rc = (r0 + r1) / 2
        count = int(TAU * rc / 1.9)
        for i in range(count):
            a = TAU * (i + 0.5) / count
            if abs(((a + math.pi) % TAU) - math.pi) < 0.22:
                continue
            trays.append((rc * math.cos(a), rc * math.sin(a), 0.37 + h, a + math.pi / 2, k))
            radial = Vector((math.cos(a), math.sin(a), 0.0))
            merge(drips, tube([radial * (r1 + 0.04) + Vector((0, 0, header)), radial * (r1 + 0.04) + Vector((0, 0, top + 0.07)),
                               radial * (r1 - 0.16) + Vector((0, 0, top + 0.07))], 0.012, 5))
    b.put(_uv(trims, 0.5), m["galv"])
    b.put(drips, m["cable"])
    # The aisle from the airlock to ring 0, split round the rail, and the stile.
    ring0 = inner + width
    for x0, x1 in ((ring0 + 0.1, rail - 0.18), (rail + 0.18, radius - 0.42)):
        b.put(box((x1 - x0, 1.5, 0.04), ((x0 + x1) / 2, 0, 0.37), tile=1.0), m["grate"], smooth=False)
    stile, legs = bmesh.new(), bmesh.new()
    top0 = 0.9
    merge(stile, box((width + 0.1, 0.3, 0.04), (inner + width / 2, 0, top0 + 0.045), tile=1.0))
    for x in (ring0 + 0.14, inner - 0.14):
        merge(stile, box((0.26, 0.3, 0.04), (x, 0, 0.645), tile=1.0))
        for s in (-1, 1):
            merge(legs, box((0.26, 0.01, 0.3), (x, s * 0.15, 0.5)))
    b.put(stile, m["grate"], smooth=False)
    b.put(legs, m["steel"], smooth=False)
    b.places.append(("room", {"name": name.lower(), "centre": (0, 0, (spring + radius) / 2), "size": (radius * 1.7, radius * 1.7, spring + radius),
                              "volume": 2.0 / 3.0 * math.pi * radius ** 3 + math.pi * radius ** 2 * spring, "glazing": 0.72}))
    for x, y, z, a, k in trays:
        b.places.append(("tray", {"at": (x, y, z), "yaw": a, "size": (1.6, width - 0.25), "ring": k}))
    chiller = Vector((math.cos(0.9), math.sin(0.9), 0)) * (radius + 3.6)
    b.places.append(("service", {"fault": "fans", "at": (chiller.x, chiller.y, 1.2)}))
    # The cistern: banded tank, the gantry's mounting ring, a ladder, its bund and pumps.
    b.put(lathe([(2.2, 0.35), (2.2, 2.9), (1.9, 3.3), (0.0, 3.4)], 48, 2.0), m["hull"], sharp=50)
    for z in (1.0, 1.8, 2.6):
        b.put(lathe([(2.2, z - 0.05), (2.24, z - 0.04), (2.24, z + 0.04), (2.2, z + 0.05)], 48, 1.0), m["hull_grey"], sharp=40)
    b.put(lathe([(1.02, 3.33), (1.02, 3.405), (0.82, 3.405), (0.82, 3.35)], 32, 1.0), m["steel"], sharp=40)
    b.put(lathe([(2.6, 0.35), (2.6, 0.9), (2.25, 0.9)], 48, 1.0), m["cast"], sharp=50)
    b.put(lathe([(2.62, 0.84), (2.62, 0.92), (2.23, 0.92)], 48, 1.0), m["galv"], sharp=40)
    ladder = bmesh.new()
    for s in (-1, 1):
        merge(ladder, rod((-2.42, s * 0.2, 0.9), (-2.42, s * 0.2, 3.55), 0.022, 6))
        for z in (1.4, 2.4, 3.2):
            merge(ladder, rod((-2.42, s * 0.2, z), (-2.18, s * 0.2, z), 0.015, 6))
    for k in range(9):
        z = 1.15 + 0.28 * k
        merge(ladder, rod((-2.42, -0.2, z), (-2.42, 0.2, z), 0.014, 6))
    b.put(ladder, m["yellow"])
    for s in (-1, 1):
        y = s * 3.1
        b.put(box((1.0, 0.5, 0.1), (0, y, 0.4)), m["frame"], smooth=False)
        pump = bmesh.new()
        _drum(pump, (-0.2, y, 0.65), (0, 1, 0), 0.17, 0.2, 14)
        b.put(_uv(pump, 0.5), m["pipe_blue"])
        motor = bmesh.new()
        _drum(motor, (0.17, y, 0.65), (1, 0, 0), 0.13, 0.44, 14)
        _drum(motor, (0.42, y, 0.65), (1, 0, 0), 0.1, 0.06, 10)
        b.put(_uv(motor, 0.5), m["hull_grey"])
        b.put(tube([(-0.2, s * 2.19, 1.15), (-0.2, s * 2.85, 1.15), (-0.2, s * 2.97, 0.82)], 0.06, 10), m["steel"])
        b.put(tube([(-0.2, y, 0.82), (-0.2, y, 1.3), (-0.65, y, 1.3), (-0.65, y, 0.36)], 0.05, 10), m["pipe_blue"])
        b.put(box((0.14, 0.14, 0.14), (-0.42, y, 1.3)), m["hull_orange"], smooth=False)
    # Grow lights in rings over the terraces, hung from the shell above the boom.
    housings, lenses, hangers = bmesh.new(), bmesh.new(), bmesh.new()
    z_lamp = 7.0
    for k in range(rings):
        rc = inner + k * band + width / 2
        b.put(tube([(rc * math.cos(a), rc * math.sin(a), z_lamp + 0.1) for a in [TAU * i / 96 for i in range(97)]], 0.03, 6), m["alu"])
        count = int(TAU * rc / 1.4)
        for i in range(count):
            a = TAU * (i + 0.5) / count
            radial, side = Vector((math.cos(a), math.sin(a), 0.0)), Vector((-math.sin(a), math.cos(a), 0.0))
            p = radial * rc + Vector((0, 0, z_lamp))
            _bar(housings, p - side * 0.55, p + side * 0.55, 0.18, 0.07)
            _bar(lenses, p - side * 0.5 - Vector((0, 0, 0.04)), p + side * 0.5 - Vector((0, 0, 0.04)), 0.14, 0.012)
        for i in range(6 + 2 * k):
            a = TAU * i / (6 + 2 * k) + 0.1
            p = Vector((rc * math.cos(a), rc * math.sin(a), z_lamp + 0.1))
            merge(hangers, rod(p, (p.x, p.y, spring + math.sqrt((radius - 0.24) ** 2 - rc * rc)), 0.008, 4))
    b.put(_uv(housings, 0.5), m["dark"])
    b.put(_uv(lenses, 0.5), m["grow_lamp"], smooth=False)
    b.put(hangers, m["steel"])
    # Wall services: an irrigation main, a cable tray, wash lights, the inner
    # hatch, and a fertigation cabinet with its dosing tanks.
    wall_arc = [0.2 + (TAU - 0.4) * i / 160 for i in range(161)]
    b.put(tube([((radius - 0.47) * math.cos(a), (radius - 0.47) * math.sin(a), 0.62) for a in wall_arc], 0.04, 8), m["pipe_blue"])
    b.put(lathe([(radius - 0.42, 2.12), (radius - 0.42, 2.2), (radius - 0.43, 2.2), (radius - 0.43, 2.13), (radius - 0.65, 2.13), (radius - 0.65, 2.2),
                 (radius - 0.66, 2.2), (radius - 0.66, 2.12), (radius - 0.42, 2.12)], 160, 1.0, TAU - 0.4, 0.2), m["galv"], sharp=40)
    for r in (radius - 0.5, radius - 0.58):
        b.put(tube([(r * math.cos(a), r * math.sin(a), 2.17) for a in wall_arc], 0.025, 5), m["cable"])
    washers, glows = bmesh.new(), bmesh.new()
    for i in range(16):
        a = TAU * (i + 0.5) / 16
        if abs(((a + math.pi) % TAU) - math.pi) < 0.3:
            continue
        radial, side = Vector((math.cos(a), math.sin(a), 0.0)), Vector((-math.sin(a), math.cos(a), 0.0))
        p = radial * (radius - 0.48) + Vector((0, 0, 2.4))
        _bar(washers, p - side * 0.3, p + side * 0.3, 0.15, 0.1)
        _bar(glows, p - side * 0.27 + Vector((0, 0, 0.052)), p + side * 0.27 + Vector((0, 0, 0.052)), 0.11, 0.008)
    b.put(_uv(washers, 0.5), m["dark"])
    b.put(_uv(glows, 0.5), m["lamp"], smooth=False)
    _hatch(b, m, (radius - 0.42, 0, 0.35), math.pi, 1.1, 2.1)
    b.put(box((0.03, 0.2, 0.06), (radius - 0.48, 0, 2.7)), m["green"], smooth=False)
    a = 0.32
    b.put(moved(box((0.6, 1.4, 1.9), (0, 0, 1.3), tile=1.0, bevel=0.01), ((radius - 0.75) * math.cos(a), (radius - 0.75) * math.sin(a), 0), a), m["hull"], smooth=False)
    for a in (0.45, 0.5):
        b.put(cone(0.26, 0.26, 1.2, ((radius - 0.85) * math.cos(a), (radius - 0.85) * math.sin(a), 0.35), 16), m["pipe_white"], sharp=40)
    # What stops a rover: knee wall and berm, floor, shell, portal, terraces, cistern.
    b.collider(lathe([(radius + 2.0, 0.0), (radius + 1.1, wall), (radius + 0.5, beam[0]), (radius - 0.4, beam[0]), (radius - 0.4, 0.35)], 24, 4.0))
    b.collider(disc(radius - 0.4, 0.35, 24, 4.0))
    shell = [(radius * math.cos(a), spring + radius * math.sin(a)) for a in [math.pi / 2 * i / 6 for i in range(7)]]
    shell[-1] = (0.0, spring + radius)
    b.collider(lathe(shell, 24, 4.0))
    b.collider(box((0.8, 4.0, 3.75), (radius + 0.55, 0, 1.875)))
    for k in range(rings):
        r0, h = inner + k * band, 0.55 + 0.5 * k
        b.collider(lathe([(r0 + width, 0.35), (r0 + width, 0.35 + h), (r0, 0.35 + h), (r0, 0.35)], 24, 2.0))
    b.collider(lathe([(2.2, 0.35), (2.2, 3.4), (0.0, 3.4)], 12, 2.0))
    return b.finish()


def arch_greenhouse(looks, name, length=48.0, width=10.0, bay=3.0, at=(0, 0, 0), yaw=0.0, parent=None):
    """An inflated vault along +X: painted I-beam arches carrying two-layer
    ETFE cushions under clamp caps, held by Kevlar restraint straps to
    ground anchors, on a concrete curb with a clamp sill, skirt, inflation
    header, blowers and louvred vents. Framed end walls carry insulated
    panels, glazing, exhaust louvres and air-handler ducts; a rigid lock at
    -X meets the corridor flange to flange and one at +X is the EVA exit.
    Inside, two rows of ebb-and-flow tables under lamp rails, and a ridge
    girder over the ridge robot's rail."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.places = []
    r = width / 2
    curb = max(0.9, ARCH_HANGERS + 0.41 - r)
    bays = int(round(length / bay))
    bay = length / bays
    seg = 32
    arc = [math.pi * i / seg for i in range(seg + 1)]

    def on(x, a, rad):
        return Vector((x, -rad * math.cos(a), curb + rad * math.sin(a)))

    # Arches with clamp caps, and base plates with stiffeners and nuts.
    ribs, clamps, plates = bmesh.new(), bmesh.new(), bmesh.new()
    for k in range(bays + 1):
        x = k * bay
        merge(ribs, sweep(ibeam(0.2, 0.34, 0.035), [on(x, a, r) for a in arc], 1.0))
        merge(clamps, sweep(rect(0.05, 0.16), [on(x, a, r + 0.195) for a in arc], 1.0))
        for s in (-1, 1):
            merge(plates, box((0.34, 0.5, 0.04), (x, s * r, curb + 0.02)))
            for gx in (-0.07, 0.07):
                merge(plates, box((0.015, 0.3, 0.22), (x + gx, s * r, curb + 0.13)))
            for bx in (-0.12, 0.12):
                for by in (-0.19, 0.19):
                    merge(plates, cone(0.025, 0.025, 0.05, (x + bx, s * r + by, curb + 0.04), 6))
    b.put(ribs, m["white_frame"], sharp=30)
    b.put(clamps, m["alu"], sharp=30)
    b.put(plates, m["steel"], sharp=30)
    # Cushions: the outer foil bulging out between arches, the inner one in.
    for amp in (0.28, -0.12):
        skin = bmesh.new()
        uv = skin.loops.layers.uv.new("st")
        steps = 10
        grid = []
        for k in range(bays * steps + 1):
            bulge = amp * math.sin(math.pi * (k % steps) / steps)
            grid.append([skin.verts.new(on(k * bay / steps, a, r + bulge * max(math.sin(a), 0.0) ** 0.5)) for a in arc])
        for k in range(len(grid) - 1):
            for i in range(seg):
                face = skin.faces.new((grid[k][i], grid[k + 1][i], grid[k + 1][i + 1], grid[k][i + 1]))
                for loop in face.loops:
                    co = loop.vert.co
                    loop[uv].uv = (co.x / 4.0, r * math.atan2(co.z - curb, -co.y) / 4.0)
        b.put(skin, m["etfe"])
    # Restraint straps over each bay's crest down to anchors before the curb.
    straps, anchors = bmesh.new(), bmesh.new()
    for j in range(bays):
        xm = (j + 0.5) * bay
        crest = [on(xm, a, r + 0.28 * max(math.sin(a), 0.0) ** 0.5 + 0.025) for a in arc]
        ends = [[Vector((xm, s * (r + 0.75), 0.12)), Vector((xm, s * (r + 0.34), 0.55)), Vector((xm, s * (r + 0.34), curb + 0.03)), Vector((xm, s * (r + 0.2), curb + 0.16))] for s in (-1, 1)]
        merge(straps, sweep(rect(0.012, 0.1), ends[0] + crest[1:-1] + ends[1][::-1], 1.0))
        for s in (-1, 1):
            merge(anchors, box((0.3, 0.3, 0.04), (xm, s * (r + 0.82), 0.02)))
            merge(anchors, rod((xm, s * (r + 0.8), 0.04), (xm, s * (r + 0.62), 0.3), 0.022, 6))
            merge(anchors, cone(0.05, 0.05, 0.08, (xm, s * (r + 0.82), 0.04), 8))
    b.put(straps, m["strap"], sharp=60)
    b.put(anchors, m["steel"], sharp=30)
    # Ties under the arches, bracing in the end bays, and the ridge girder.
    frame = bmesh.new()
    ties = (0.4, 0.85, math.pi - 0.85, math.pi - 0.4)
    for a in ties:
        merge(frame, tube([on(0.0, a, r - 0.24), on(length, a, r - 0.24)], 0.04, 8))
    for x0, x1 in ((0.0, bay), (length - bay, length)):
        for a1, a2 in ((ties[0], ties[1]), (ties[2], ties[3])):
            merge(frame, rod(on(x0, a1, r - 0.24), on(x1, a2, r - 0.24), 0.016, 6))
            merge(frame, rod(on(x0, a2, r - 0.24), on(x1, a1, r - 0.24), 0.016, 6))
    ridge_top = curb + r - 0.175
    merge(frame, sweep(ibeam(0.16, ridge_top - ARCH_HANGERS, 0.02), [(0.0, 0, (ridge_top + ARCH_HANGERS) / 2), (length, 0, (ridge_top + ARCH_HANGERS) / 2)], 1.0))
    b.put(frame, m["white_frame"], sharp=30)
    # The curbs: clamp sill, skirt, inflation header with a hose to each
    # cushion, louvred vents, and a blower each side.
    vents, backing = bmesh.new(), bmesh.new()
    for s in (-1, 1):
        b.put(box((length + 0.6, 0.5, curb + 0.35), (length / 2, s * (r + 0.05), (curb - 0.35) / 2), tile=2.0, bevel=0.03), m["cast"], smooth=False)
        b.put(box((length, 0.18, 0.1), (length / 2, s * r, curb + 0.05), tile=1.0), m["alu"], smooth=False)
        b.put(box((length + 0.6, 0.012, 0.32), (length / 2, s * (r + 0.306), curb - 0.15)), m["gasket"], smooth=False)
        b.put(tube([(0.2, s * (r + 0.2), curb + 0.08), (length - 0.2, s * (r + 0.2), curb + 0.08)], 0.04, 8), m["pipe_white"])
        hoses = bmesh.new()
        for j in range(bays):
            xm = (j + 0.5) * bay + 0.35
            merge(hoses, tube([(xm, s * (r + 0.2), curb + 0.08), (xm, s * (r + 0.2), curb + 0.25), (xm, s * (r + 0.06), curb + 0.38)], 0.025, 6))
        b.put(hoses, m["cable"])
        for k in range(1, bays, 2):
            y = s * (r + 0.32)
            merge(vents, box((0.04, 0.03, 0.42), (k * bay - 0.35, y, curb * 0.5)))
            merge(vents, box((0.04, 0.03, 0.42), (k * bay + 0.35, y, curb * 0.5)))
            merge(vents, box((0.74, 0.03, 0.04), (k * bay, y, curb * 0.5 + 0.21)))
            merge(vents, box((0.74, 0.03, 0.04), (k * bay, y, curb * 0.5 - 0.21)))
            merge(backing, box((0.68, 0.012, 0.4), (k * bay, s * (r + 0.3), curb * 0.5)))
            for i in range(4):
                merge(vents, box((0.68, 0.06, 0.012), (k * bay, y + s * 0.01, curb * 0.5 - 0.15 + 0.1 * i), rot=Matrix.Rotation(-s * 0.7, 4, "X")))
        unit(b, m, (length / 2, s * (r + 1.2), 0), 0.0, (1.2, 0.8, 0.8), 1)
        b.put(tube([(length / 2 - 0.45, s * (r + 0.95), 0.8), (length / 2 - 0.45, s * (r + 0.95), curb + 0.08), (length / 2 - 0.45, s * (r + 0.24), curb + 0.08)], 0.06, 8), m["pipe_white"])
    b.put(vents, m["frame"], smooth=False)
    b.put(backing, m["dark"], smooth=False)
    b.put(box((length, width, 0.12), (length / 2, 0, 0.06), tile=2.0), m["floor"], smooth=False)
    # End walls: curb and bulkhead round the lock, mullions and transoms,
    # insulated panels low and glazing high, exhaust louvres, the air
    # handler's duct, the inner hatch and the lock itself.
    lw, ri = 1.55, r - 0.17
    t0, t1 = 3.2, 4.7

    def arch_z(y):
        return curb + math.sqrt(max(ri * ri - y * y, 0.0))

    def arch_y(z):
        return math.sqrt(max(ri * ri - (z - curb) ** 2, 0.0))

    a_t = math.asin((t0 - curb) / ri)
    for x, facing in ((0.0, -1.0), (length, 1.0)):
        for s in (-1, 1):
            b.put(box((0.3, r + 0.3 - lw, curb), (x, s * (r + 0.3 + lw) / 2, curb / 2), tile=2.0), m["cast"], smooth=False)
            low = [(-lw, curb)] + [(-ri * math.cos(a), curb + ri * math.sin(a)) for a in [a_t * i / 8 for i in range(9)]] + [(-lw, t0)]
            b.put(_panel([(s * -y, z) for y, z in low], x + facing * 0.03, facing), m["hull"], smooth=False)
        b.put(box((0.08, 2 * lw, t0), (x, 0, t0 / 2), tile=1.0), m["hull_grey"], smooth=False)
        cap = [(-ri * math.cos(a), curb + ri * math.sin(a)) for a in [a_t + (math.pi - 2 * a_t) * i / 24 for i in range(25)]]
        b.put(_panel(cap, x, facing, 4.0), m["dome_glass"], smooth=False)
        mullions = bmesh.new()
        for y in (-4.2, -3.0, -lw, lw, 3.0, 4.2):
            z0 = 0.12 if abs(y) <= lw else curb
            merge(mullions, box((0.18, 0.12, arch_z(y) - z0), (x + facing * 0.06, y, (z0 + arch_z(y)) / 2)))
        for z in (t0, t1):
            merge(mullions, box((0.18, 2 * arch_y(z), 0.12), (x + facing * 0.06, 0, z)))
        b.put(mullions, m["white_frame"], smooth=False)
        louvres, backs = bmesh.new(), bmesh.new()
        for s in (-1, 1):
            yc, zc = s * 2.28, 3.95
            merge(backs, box((0.03, 1.1, 1.15), (x + facing * 0.17, yc, zc)))
            for dz in (-0.6, 0.6):
                merge(louvres, box((0.08, 1.16, 0.05), (x + facing * 0.2, yc, zc + dz)))
            for dy in (-0.57, 0.57):
                merge(louvres, box((0.08, 0.05, 1.2), (x + facing * 0.2, yc + dy, zc)))
            for i in range(7):
                merge(louvres, box((0.16, 1.1, 0.012), (x + facing * 0.22, yc, zc - 0.48 + 0.16 * i), rot=Matrix.Rotation(facing * 0.6, 4, "Y")))
        b.put(louvres, m["frame"], smooth=False)
        b.put(backs, m["dark"], smooth=False)
        duct = bmesh.new()
        _bar(duct, (x + facing * 0.42, 3.6, 0.85), (x + facing * 0.2, 3.6, 0.85), 0.36, 0.36)
        _bar(duct, (x + facing * 0.2, 3.6, 0.67), (x + facing * 0.2, 3.6, 2.58), 0.36, 0.36, (1, 0, 0))
        _bar(duct, (x + facing * 0.2, 3.6, 2.4), (x + facing * 0.02, 3.6, 2.4), 0.36, 0.36)
        b.put(_uv(duct, 1.0), m["galv"], smooth=False)
        unit(b, m, (x + facing * 1.6, 3.6, 0), 0.0, (2.4, 1.1, 1.3), 2)
        b.collider(box((2.4, 1.1, 1.3), (x + facing * 1.6, 3.6, 0.65)))
        b.places.append(("service", {"fault": "hvac", "at": (x + facing * 1.6, 3.6, 1.2)}))
        _hatch(b, m, (x - facing * 0.06, 0, 0.12), math.pi if facing > 0 else 0.0, 1.2, 2.2)
        if facing < 0:
            _lock(b, m, (x - 0.1, 0, 0), math.pi, 3.1, 2.8, 0.05, 3.0, 1.2, 1.4)
        else:
            end = _lock(b, m, (x + 0.1, 0, 0), 0.0, 3.1, 2.8, 0.05, 3.0)
            door(b, m, (x + 0.1 + end + 0.08, 0, 0.12), 0.0, 1.3, 2.3)
            b.put(box((0.7, 1.7, 0.06), (x + 0.1 + end + 0.45, 0, 0.03), tile=1.0), m["grate"], smooth=False)
    # Tables down both sides; the gearbox layer adds trays and lamps on them.
    table_len = 1.3
    tables = int((length - 3.0) / (table_len + 0.25))
    span = tables * (table_len + 0.25) - 0.25
    x_first = (length - span) / 2 + table_len / 2
    legs, rims, lamp_rods = bmesh.new(), bmesh.new(), bmesh.new()
    for s in (-1, 1):
        y = s * (r * 0.5)
        for t in range(tables):
            x = x_first + t * (table_len + 0.25)
            for lx in (-0.55, 0.55):
                for ly in (-0.6, 0.6):
                    merge(legs, box((0.05, 0.05, 0.82), (x + lx, y + ly, 0.53)))
                    merge(legs, box((0.1, 0.1, 0.02), (x + lx, y + ly, 0.13)))
                merge(legs, box((0.04, 1.2, 0.04), (x + lx, y, 0.35)))
            for ly in (-0.6, 0.6):
                merge(legs, box((1.1, 0.04, 0.04), (x, y + ly, 0.35)))
            merge(legs, rod((x - 0.45, y - 0.3, 0.72), (x - 0.45, y - 0.3, 0.93), 0.012, 6))
            merge(legs, rod((x + 0.45, y + 0.2, 0.93), (x + 0.45, y + 0.2, 0.6), 0.02, 6))
            b.put(box((table_len, 1.35, 0.06), (x, y, 0.96), tile=1.0), m["frame"], smooth=False)
            for dy in (-0.66, 0.66):
                merge(rims, box((table_len, 0.03, 0.05), (x, y + dy, 1.005)))
            for dx in (-0.635, 0.635):
                merge(rims, box((0.03, 1.35, 0.05), (x + dx, y, 1.005)))
            b.places.append(("tray", {"at": (x, y, 0.99), "yaw": 0.0, "size": (table_len - 0.1, 1.25), "lamp": 1.05}))
            merge(lamp_rods, rod((x, y, 2.04 + 0.05), (x, y, 5.16), 0.012, 6))
        x_last = x_first + (tables - 1) * (table_len + 0.25)
        b.put(tube([(x_first - 0.7, y - 0.3, 0.72), (x_last + 0.6, y - 0.3, 0.72)], 0.03, 8), m["pipe_blue"])
        b.put(tube([(x_first - 0.7, y + 0.2, 0.6), (x_last + 0.6, y + 0.2, 0.6)], 0.04, 8), m["pipe_white"])
        b.put(box((x_last - x_first + 1.4, 0.06, 0.08), ((x_first + x_last) / 2, y, 5.2)), m["alu"], smooth=False)
        for k in range(bays + 1):
            if x_first - 0.7 <= k * bay <= x_last + 0.7:
                merge(lamp_rods, rod((k * bay, y, 5.24), (k * bay, y, arch_z(y)), 0.012, 6))
    b.put(legs, m["steel"], smooth=False)
    b.put(rims, m["frame"], smooth=False)
    b.put(lamp_rods, m["steel"])
    b.put(box((length - 2.0, 1.4, 0.04), (length / 2, 0, 0.14), tile=1.0), m["grate"], smooth=False)
    # Circulation fans hung from the arches, and a fertigation skid by the inner hatch.
    for x in (2 * bay, length - 2 * bay):
        for s in (-1, 1):
            fan = bmesh.new()
            _drum(fan, (x, s * 1.4, 4.4), (1, 0, 0), 0.3, 0.32, 16)
            b.put(_uv(fan, 0.5), m["hull_grey"])
            b.put(moved(disc(0.27, 0.17, 16, 0.5), (x, s * 1.4, 4.4), 0, math.pi / 2), m["grate"])
            b.put(rod((x, s * 1.4, 4.7), (x, s * 1.4, arch_z(1.4)), 0.015, 6), m["steel"])
    b.put(box((0.6, 1.0, 1.6), (0.5, -4.2, 0.92), tile=1.0, bevel=0.01), m["hull"], smooth=False)
    for y in (-4.4, -3.85):
        b.put(cone(0.24, 0.24, 1.0, (1.25, y, 0.12), 16), m["pipe_white"], sharp=40)
    b.put(tube([(1.25, -3.85, 0.4), (1.6, -3.85, 0.4), (1.6, -2.8, 0.72), (x_first - 0.7, -2.8, 0.72)], 0.03, 8), m["pipe_blue"])
    b.places.append(("room", {"name": name.lower(), "centre": (length / 2, 0, (curb + r) / 2), "size": (length, width, curb + r),
                              "volume": length * (math.pi * r * r / 2 + width * curb), "glazing": 0.5}))
    # What stops a rover: the vault, curbs, floor, gables and the rows of tables.
    b.collider(vault(0.0, length, r, curb, 8, 4.0))
    for s in (-1, 1):
        b.collider(box((length + 0.6, 0.5, curb + 0.35), (length / 2, s * (r + 0.05), (curb - 0.35) / 2)))
        b.collider(box((span, 1.35, 0.99), (length / 2, s * r * 0.5, 0.495)))
    b.collider(box((length, width, 0.12), (length / 2, 0, 0.06)))
    for x, facing in ((0.0, -1.0), (length, 1.0)):
        b.collider(half_disc(r, x, curb, facing, 8, 2.0))
        b.collider(box((0.3, width + 0.6, curb), (x, 0, curb / 2)))
    return b.finish()
