"""What the crew's buildings hold inside: decks and linings, hatches and
portholes, racks and stowage, crew quarters, galley, tables and seats,
exercise, hygiene, laboratory, medical, workshop and EVA kit, after NASA's
Common Habitat, HDU/HERA, MDRS, Mars Dune Alpha and the ISS
(plans/003-mars-base-research.md).

Each builder places into a Building in the building's own frame; `at` is a
point on the floor and `yaw` the way the item's front faces (+X turned by
`yaw`). Sizes are in metres."""
import math

import bmesh
import bpy
from mathutils import Vector

import patterns
from industry import plant_mats
from kit import TAU, box, cone, lathe, merge, moved, project_uv, rod, sphere, tube

_SET = {}

# An International Standard Payload Rack: width, depth, height.
RACK = (1.05, 0.86, 2.0)
# A pressure hatch, 60 × 40 in, with its 16 in step-over.
HATCH = (1.02, 1.52, 0.41)
# A single Cargo Transfer Bag.
CTB = (0.50, 0.43, 0.23)


def _indoor(mat):
    """An indoor copy of a base material: the same look, never weathered."""
    name = f"{mat.name}_in"
    own = bpy.data.materials.get(name)
    if own is None:
        own = mat.copy()
        own.name = name
        own["gearbox_indoor"] = True
    return own


def mats(looks):
    """Indoor copies of the habitat's materials and the plant's paints, and
    the cabin's own, every one marked indoors only."""
    if _SET.get("looks") is looks:
        return _SET["set"]
    import habitat
    g = looks.get
    base = dict(plant_mats(looks))
    base.update(habitat._mats(looks))
    m = {key: _indoor(mat) for key, mat in base.items()}
    own = {
        "liner": g("cabin_liner", (0.86, 0.85, 0.81), 0.7, 0.0,
                   maps=lambda: patterns.panels((0.86, 0.85, 0.81), cols=1, rows=2, seam_w=0.006, rivets=False, grime=0.03, seed=61)),
        "deck": g("cabin_deck", (0.56, 0.58, 0.60), 0.75, 0.05,
                  maps=lambda: patterns.panels((0.56, 0.58, 0.60), cols=2, rows=2, seam_w=0.004, rivets=True, grime=0.05, seed=62)),
        "tread": g("cabin_tread", (0.66, 0.67, 0.68), 0.45, 0.8, "metal_plate", contrast=0.4),
        "ceiling": g("cabin_ceiling", (0.90, 0.90, 0.88), 0.8, 0.0,
                     maps=lambda: patterns.panels((0.90, 0.90, 0.88), cols=2, rows=2, seam_w=0.005, rivets=False, grime=0.02, seed=63)),
        "rack": g("cabin_rack", (0.80, 0.79, 0.74), 0.6, 0.0,
                  maps=lambda: patterns.panels((0.80, 0.79, 0.74), cols=2, rows=4, seam_w=0.005, rivets=False, grime=0.04, seed=64)),
        "locker": g("cabin_locker", (0.70, 0.71, 0.70), 0.55, 0.1),
        "bag": g("cabin_ctb", (0.88, 0.87, 0.80), 0.85, 0.0, "bi_stretch", contrast=0.4),
        "cushion": g("cabin_cushion", (0.17, 0.23, 0.35), 0.9, 0.0, "bi_stretch", contrast=0.5),
        "sheet": g("cabin_bedding", (0.66, 0.71, 0.78), 0.9, 0.0, "bi_stretch", contrast=0.4),
        "curtain": g("cabin_curtain", (0.30, 0.36, 0.46), 0.9, 0.0, "bi_stretch", contrast=0.5),
        "screen": g("cabin_screen", (0.02, 0.03, 0.05), 0.2, 0.0, emit=((0.35, 0.55, 0.85), 1.4)),
        "display": g("cabin_display", (0.02, 0.03, 0.03), 0.2, 0.0, emit=((0.30, 0.85, 0.55), 1.2)),
        "panel_lamp": g("cabin_lamp", (1.0, 0.98, 0.94), 0.3, 0.0, emit=((1.0, 0.97, 0.92), 6.0)),
        "rail": g("cabin_rail", (0.26, 0.38, 0.60), 0.5, 0.2),
        "suit": g("cabin_suit", (0.90, 0.90, 0.88), 0.85, 0.0, "bi_stretch", contrast=0.5),
        "visor": g("cabin_visor", (0.62, 0.46, 0.16), 0.1, 1.0),
        "greens": g("cabin_greens", (0.16, 0.40, 0.11), 0.7, 0.0, "bi_stretch", contrast=0.8),
        "porcelain": g("cabin_porcelain", (0.93, 0.93, 0.92), 0.25, 0.0),
        "glass": g("cabin_glass", (0.70, 0.78, 0.80), 0.05, 0.0, alpha=0.25),
        "pane": g("cabin_pane", (0.05, 0.05, 0.06), 0.1, 0.0, emit=((0.95, 0.70, 0.50), 0.6)),
        "mat": g("cabin_mat", (0.08, 0.08, 0.09), 0.9, 0.0, "rubber_tiles"),
        "red_cross": g("cabin_red", (0.75, 0.08, 0.06), 0.5, 0.0),
        "tool": g("cabin_tool", (0.55, 0.56, 0.58), 0.35, 0.9),
        "orange": g("cabin_orange", (0.90, 0.40, 0.06), 0.55, 0.0),
    }
    for mat in own.values():
        mat["gearbox_indoor"] = True
    m.update(own)
    _SET.update(looks=looks, set=m)
    return m


# Frames


def polar(a, r, z=0.0):
    """The point at angle `a` and radius `r` about Z, height `z`."""
    return (r * math.cos(a), r * math.sin(a), z)


def put(b, bm, mat, at=(0, 0, 0), yaw=0.0, smooth=False):
    """Puts a part built about its own origin at `at`, turned by `yaw`."""
    b.put(moved(bm, at, yaw), mat, smooth)


def against(a, r, depth):
    """Where an item `depth` deep stands with its back on a round wall of
    radius `r` at angle `a`: its middle on the floor, and the way it faces."""
    return polar(a, r - depth / 2 - 0.02), a + math.pi


def hull_span(r, width, at_r):
    """The angle an item `width` wide takes up at radius `at_r` in a hull of radius `r`."""
    return 2.0 * math.asin(min(0.99, width / 2 / at_r))


# Surfaces


def floor(r, z, t=0.1, holes=(), step=0.35):
    """A round deck of radius `r`, its top at `z` and `t` thick, with
    rectangular `holes` (x0, x1, y0, y1): strips along X, each running
    between the deck's edge and the holes it crosses."""
    bm = bmesh.new()
    ys = {-r + 2 * r * k / max(2, math.ceil(2 * r / step)) for k in range(max(2, math.ceil(2 * r / step)) + 1)}
    ys |= {y for h in holes for y in (h[2], h[3]) if -r < y < r}
    ys = sorted(ys)
    def half(y):
        return math.sqrt(max(r * r - y * y, 0.0))

    for y0, y1 in zip(ys, ys[1:]):
        cuts = sorted((h[0], h[1]) for h in holes if h[2] <= y0 + 1e-6 and h[3] >= y1 - 1e-6)
        edges = [None] + [x for c in cuts for x in c] + [None]
        for left, right in zip(edges[::2], edges[1::2]):
            xl = [-half(y0), -half(y1)] if left is None else [left, left]
            xr = [half(y0), half(y1)] if right is None else [right, right]
            if xr[0] - xl[0] < 1e-4 and xr[1] - xl[1] < 1e-4:
                continue
            for z_, flip in ((z, False), (z - t, True)):
                vs = [bm.verts.new((xl[0], y0, z_)), bm.verts.new((xr[0], y0, z_)), bm.verts.new((xr[1], y1, z_)), bm.verts.new((xl[1], y1, z_))]
                bm.faces.new(vs[::-1] if flip else vs)
    project_uv(bm, 1.2)
    return bm


def lining(r, z0, z1, segments=72, tile=1.2):
    """A cabin's wall lining on a round hull of radius `r` from `z0` to `z1`, facing in."""
    return lathe([(r, z1), (r, z0)], segments, tile)


def skirt(r, z, h=0.12, segments=72):
    """The kick strip where a deck meets its round wall."""
    return lathe([(r, z + h), (r - 0.03, z + h), (r - 0.03, z), (r, z)], segments, 1.0)


def ceiling_panels(b, m, z, spots, size=(0.6, 0.6)):
    """Flush LED panels in a ceiling at `z`, each at (x, y), and a light under every other one."""
    lamps = bmesh.new()
    for k, (x, y) in enumerate(spots):
        merge(lamps, box((size[0], size[1], 0.02), (x, y, z - 0.012)))
        if k % 2 == 0:
            b.places.append(("light", {"at": (x, y, z - 0.08), "kind": "cabin"}))
    b.put(lamps, m["panel_lamp"], smooth=False)


def partition(b, m, a, c, z, h, t=0.07, gaps=(), mat="liner"):
    """A straight wall from `a` to `c` (x, y) on the floor at `z`, `h` high,
    with door `gaps` as (start, end) metres along it."""
    a, c = Vector((a[0], a[1], 0)), Vector((c[0], c[1], 0))
    run = c - a
    length = run.length
    yaw = math.atan2(run.y, run.x)
    edges = [0.0] + [d for g in sorted(gaps) for d in g] + [length]
    for s0, s1 in zip(edges[::2], edges[1::2]):
        if s1 - s0 > 0.01:
            mid = a + run * ((s0 + s1) / 2 / length)
            put(b, box((s1 - s0, t, h), (0, 0, h / 2), tile=1.2), m[mat], (mid.x, mid.y, z), yaw)
    for g0, g1 in gaps:
        mid = a + run * ((g0 + g1) / 2 / length)
        if h > 2.15:
            put(b, box((g1 - g0, t, h - 2.1), (0, 0, 2.1 + (h - 2.1) / 2), tile=1.2), m[mat], (mid.x, mid.y, z), yaw)
        put(b, box((g1 - g0 + 0.08, t + 0.04, 0.05), (0, 0, 2.1)), m["locker"], (mid.x, mid.y, z), yaw)
    b.collider(moved(box((length, t, h), (0, 0, h / 2)), tuple(a + run / 2 + Vector((0, 0, z))), yaw))


def railing(b, m, path, z, h=1.0):
    """A handrail at `h` along `path` (x, y), on posts, with a mid rail."""
    rails = bmesh.new()
    pts = [Vector((x, y, z)) for x, y in path]
    for p, q in zip(pts, pts[1:]):
        for k in (h, h * 0.5):
            merge(rails, rod(p + Vector((0, 0, k)), q + Vector((0, 0, k)), 0.022, 8))
    for p in pts:
        merge(rails, rod(p, p + Vector((0, 0, h)), 0.025, 8))
    b.put(rails, m["rail"], smooth=False)


# Openings


def hatch_door(b, m, at, yaw, w=HATCH[0], h=HATCH[1], sill=HATCH[2], open_=0.0):
    """A pressure hatch in a wall, seen from the side it faces: a raised
    frame, a rounded leaf with a viewport and a lever, two hinges. `open_`
    swings the leaf about its hinge side (radians)."""
    x, y, z = at
    frame = bmesh.new()
    zc = z + sill + h / 2
    for dy, dz, sx, sz in ((-(w / 2 + 0.06), 0, 0.12, h + 0.24), ((w / 2 + 0.06), 0, 0.12, h + 0.24), (0, h / 2 + 0.06, w + 0.24, 0.12), (0, -(h / 2 + 0.06), w + 0.24, 0.12)):
        merge(frame, box((0.08, sx, sz), (0.04, dy, dz), bevel=0.01))
    put(b, frame, m["hull_grey"], (x, y, zc), yaw)
    leaf = box((0.06, w - 0.02, h - 0.02), (0.03, w / 2 - 0.01, 0), tile=0.8, bevel=0.06)
    merge(leaf, box((0.02, 0.22, 0.22), (0.065, w / 2, h * 0.22)))
    merge(leaf, box((0.03, 0.5, 0.05), (0.08, w / 2, -h * 0.1)))
    pane = box((0.01, 0.18, 0.18), (0.075, w / 2, h * 0.22))
    for part, key in ((leaf, "locker"), (pane, "pane")):
        put(b, moved(moved(part, yaw=open_), (0.0, -w / 2, 0)), m[key], (x, y, zc), yaw)
    for dz in (h * 0.3, -h * 0.3):
        put(b, box((0.1, 0.06, 0.16), (0.05, -w / 2 - 0.02, dz)), m["steel"], (x, y, zc), yaw)
    put(b, box((0.01, 0.12, 0.06), (0.09, w / 2 + 0.2, h / 2 - 0.1)), m["display"], (x, y, zc), yaw)


def porthole(b, m, at, yaw, r=0.3):
    """A round window seen from inside: a padded reveal round a recessed pane, facing +X of `yaw`."""
    ring = lathe([(r + 0.12, 0.0), (r + 0.1, 0.05), (r, 0.06), (r, -0.04)], 32, 0.5)
    moved(ring, pitch=math.pi / 2)
    put(b, ring, m["locker"], at, yaw, smooth=True)
    put(b, moved(lathe([(r, -0.03), (0.0, -0.03)], 32), pitch=math.pi / 2), m["pane"], at, yaw)


# Racks and stowage


def rack(b, m, at, yaw, face="stowage", size=RACK):
    """A standard rack on the floor, its back to the wall, `face` its front:
    `stowage` lockers, `eclss` (grille, gauges, valves, drawers), `freezer`
    (four dewar doors), `avionics` (switch rows and displays), `medical`
    (drawers and a display), `galley` (warmers and a dispenser)."""
    w, d, h = size
    x, y, z = at
    put(b, box((d - 0.03, w, h), (-0.015, 0, h / 2), tile=1.0, bevel=0.01), m["rack"], (x, y, z), yaw)
    b.collider(moved(box((d, w, h), (0, 0, h / 2)), (x, y, z), yaw))
    fx = d / 2
    plates, dark, lit, metal = bmesh.new(), bmesh.new(), bmesh.new(), bmesh.new()

    def door(z0, z1, y0, y1, handle=True):
        merge(plates, box((0.015, y1 - y0 - 0.012, z1 - z0 - 0.012), (fx, (y0 + y1) / 2, (z0 + z1) / 2), tile=0.5))
        if handle:
            merge(metal, box((0.02, min(0.3, (y1 - y0) * 0.5), 0.025), (fx + 0.018, (y0 + y1) / 2, z1 - 0.06)))

    if face == "stowage":
        for row in range(5):
            for col in range(2):
                door(0.1 + row * 0.38, 0.1 + (row + 1) * 0.38, -w / 2 + col * w / 2, -w / 2 + (col + 1) * w / 2)
    elif face == "eclss":
        merge(dark, box((0.01, w - 0.1, 0.5), (fx + 0.005, 0, 1.6)))
        for k in range(8):
            merge(metal, box((0.02, w - 0.12, 0.012), (fx + 0.012, 0, 1.38 + k * 0.06)))
        merge(plates, box((0.015, w - 0.06, 0.6), (fx, 0, 1.0)))
        for dy in (-0.25, 0.0, 0.25):
            merge(metal, moved(lathe([(0.0, 0.0), (0.06, 0.0), (0.06, 0.03), (0.0, 0.03)], 16, 0.2), (fx + 0.008, dy, 1.12), pitch=math.pi / 2))
            merge(metal, rod((fx, dy, 0.86), (fx + 0.07, dy, 0.86), 0.02, 8))
        merge(lit, box((0.01, 0.24, 0.12), (fx + 0.012, 0.3, 0.86)))
        for row in range(2):
            door(0.1 + row * 0.3, 0.4 + row * 0.3, -w / 2, w / 2)
    elif face == "freezer":
        for row in range(4):
            door(0.12 + row * 0.44, 0.54 + row * 0.44, -w / 2 + 0.05, w / 2 - 0.05)
            merge(metal, box((0.03, 0.06, 0.2), (fx + 0.02, w / 2 - 0.16, 0.33 + row * 0.44)))
        merge(lit, box((0.01, 0.2, 0.08), (fx + 0.012, -0.3, 1.94)))
    elif face == "avionics":
        for row in range(6):
            merge(plates, box((0.015, w - 0.08, 0.2), (fx, 0, 0.3 + row * 0.24)))
            for k in range(8):
                merge(metal, box((0.025, 0.015, 0.04), (fx + 0.02, -w / 2 + 0.12 + k * 0.11, 0.3 + row * 0.24)))
        merge(lit, box((0.01, w - 0.2, 0.3), (fx + 0.012, 0, 1.78)))
    elif face == "medical":
        for row in range(4):
            door(0.1 + row * 0.28, 0.38 + row * 0.28, -w / 2, w / 2)
        merge(lit, box((0.01, 0.5, 0.3), (fx + 0.012, 0, 1.5)))
        put(b, box((0.01, 0.16, 0.05), (fx + 0.02, 0, 1.86)), m["red_cross"], (x, y, z), yaw)
        put(b, box((0.01, 0.05, 0.16), (fx + 0.021, 0, 1.86)), m["red_cross"], (x, y, z), yaw)
    elif face == "galley":
        for k in range(3):
            door(1.0, 1.32, -w / 2 + k * w / 3, -w / 2 + (k + 1) * w / 3)
        merge(dark, box((0.05, w - 0.1, 0.3), (fx - 0.03, 0, 0.82)))
        for k in range(3):
            merge(metal, rod((fx - 0.02, -0.3 + 0.3 * k, 0.93), (fx - 0.02, -0.3 + 0.3 * k, 0.88), 0.012, 8))
        merge(lit, box((0.01, 0.3, 0.1), (fx + 0.012, 0, 1.5)))
        for row in range(2):
            door(0.08 + row * 0.3, 0.38 + row * 0.3, -w / 2, w / 2)
    for bm, key in ((plates, "locker"), (dark, "dark"), (lit, "display"), (metal, "tool")):
        put(b, bm, m[key], (x, y, z), yaw)


def bags(b, m, at, yaw, cols=2, rows=4, deep=1, size=CTB):
    """Cargo Transfer Bags stowed in an open frame: `cols` × `rows` faces, `deep` bags back."""
    w, d, h = size
    x, y, z = at
    soft = bmesh.new()
    for i in range(cols):
        for j in range(rows):
            for k in range(deep):
                if (i * 7 + j * 3 + k) % 11 == 5:
                    continue
                merge(soft, box((d - 0.02, w - 0.02, h - 0.02), (-k * d, (i - (cols - 1) / 2) * w, 0.1 + j * (h + 0.04) + h / 2), tile=0.5, bevel=0.03))
    put(b, soft, m["bag"], (x, y, z), yaw)
    tags = bmesh.new()
    for i in range(cols):
        for j in range(rows):
            if (i * 7 + j * 3) % 11 != 5:
                merge(tags, box((0.005, 0.16, 0.08), (d / 2 - 0.005, (i - (cols - 1) / 2) * w + 0.08, 0.1 + j * (h + 0.04) + h * 0.65)))
    put(b, tags, m["label"], (x, y, z), yaw)
    frame = bmesh.new()
    top = 0.1 + rows * (h + 0.04)
    for j in range(rows + 1):
        merge(frame, box((d * deep, w * cols + 0.04, 0.025), (-(deep - 1) * d / 2, 0, 0.08 + j * (h + 0.04))))
    for sy in (-1, 1):
        for sx in (0, -(deep - 1) * d - d / 2 + 0.02, d / 2 - 0.02):
            merge(frame, box((0.03, 0.03, top), (sx, sy * (w * cols / 2 + 0.02), top / 2)))
    put(b, frame, m["alu"], (x, y, z), yaw)
    b.collider(moved(box((d * deep, w * cols, top), (-(deep - 1) * d / 2, 0, top / 2)), (x, y, z), yaw))


# Furniture


def chair(b, m, at, yaw):
    """A swivel chair: a five-star base, a gas post, seat and back cushions, facing +X of `yaw`."""
    base = bmesh.new()
    for k in range(5):
        a = TAU * k / 5
        merge(base, rod((0, 0, 0.08), (0.3 * math.cos(a), 0.3 * math.sin(a), 0.05), 0.02, 6))
        merge(base, sphere(0.03, (0.3 * math.cos(a), 0.3 * math.sin(a), 0.03), 8, 4))
    merge(base, rod((0, 0, 0.06), (0, 0, 0.42), 0.025, 8))
    put(b, base, m["dark"], at, yaw)
    put(b, box((0.46, 0.46, 0.08), (0.0, 0, 0.46), bevel=0.03), m["cushion"], at, yaw)
    put(b, box((0.07, 0.42, 0.5), (-0.23, 0, 0.78), bevel=0.03), m["cushion"], at, yaw)


def stool(b, m, at):
    """A round stool on one post."""
    put(b, rod((0, 0, 0.02), (0, 0, 0.44), 0.025, 8), m["tool"], at)
    put(b, lathe([(0.0, 0.0), (0.2, 0.0), (0.2, 0.02), (0.0, 0.02)], 20, 0.4), m["dark"], at)
    put(b, lathe([(0.0, 0.44), (0.18, 0.44), (0.18, 0.5), (0.0, 0.52)], 20, 0.4), m["cushion"], at)


def table(b, m, at, r=0.6, h=0.74, seats=4, start=0.0, chairs=False):
    """A round table on a pedestal with `seats` stools, or chairs, round it."""
    put(b, lathe([(0.0, h - 0.03), (r, h - 0.03), (r, h), (0.0, h)], 48, 1.0), m["locker"], at)
    put(b, lathe([(0.0, 0.0), (0.3, 0.0), (0.3, 0.03), (0.04, 0.05), (0.04, h - 0.03), (0.0, h - 0.03)], 16, 0.5), m["tool"], at)
    b.collider(moved(box((2 * r, 2 * r, h), (0, 0, h / 2)), at))
    for k in range(seats):
        a = start + TAU * k / seats
        if chairs:
            chair(b, m, (at[0] + (r + 0.38) * math.cos(a), at[1] + (r + 0.38) * math.sin(a), at[2]), a + math.pi)
        else:
            stool(b, m, (at[0] + (r + 0.32) * math.cos(a), at[1] + (r + 0.32) * math.sin(a), at[2]))


def spiral(b, m, at, z0, z1, r=1.05, turn=TAU * 0.85, start=0.0, tread=0.21):
    """A spiral stair round a central column from `z0` to `z1`: wedge treads,
    an outer stringer and handrail following the helix."""
    x, y, _ = at
    steps = max(4, round((z1 - z0) / tread))
    rise = (z1 - z0) / steps
    treads = bmesh.new()
    for k in range(1, steps + 1):
        a0 = start + turn * (k - 1) / steps
        a1 = start + turn * k / steps + 0.02
        z = z0 + k * rise
        pts = [(0.12 * math.cos(a0), 0.12 * math.sin(a0)), (r * math.cos(a0), r * math.sin(a0)), (r * math.cos(a1), r * math.sin(a1)), (0.12 * math.cos(a1), 0.12 * math.sin(a1))]
        lo = [treads.verts.new((px, py, z - 0.04)) for px, py in pts]
        hi = [treads.verts.new((px, py, z)) for px, py in pts]
        treads.faces.new(hi)
        treads.faces.new(lo[::-1])
        for i in range(4):
            j = (i + 1) % 4
            treads.faces.new([lo[i], lo[j], hi[j], hi[i]])
    project_uv(treads, 0.6)
    put(b, treads, m["tread"], (x, y, 0.0))
    put(b, rod((0, 0, z0), (0, 0, z1 + 1.0), 0.1, 16), m["steel"], (x, y, 0.0))
    def helix(rr, dz):
        return [Vector((rr * math.cos(start + turn * k / 24), rr * math.sin(start + turn * k / 24), z0 + (z1 - z0) * k / 24 + dz)) for k in range(25)]

    put(b, tube(helix(r, -0.1), 0.04, 8), m["steel"], (x, y, 0.0))
    put(b, tube(helix(r - 0.03, 0.95), 0.022, 8), m["rail"], (x, y, 0.0))
    posts = bmesh.new()
    for p in helix(r - 0.03, 0.0)[::3]:
        merge(posts, rod(p, p + Vector((0, 0, 0.95)), 0.015, 6))
    put(b, posts, m["rail"], (x, y, 0.0))


def long_table(b, m, at, yaw, length, width=0.9, h=0.74, seats=6):
    """A long table on two pedestals with chairs down both sides."""
    put(b, box((length, width, 0.04), (0, 0, h - 0.02), tile=1.0, bevel=0.01), m["locker"], at, yaw)
    for sx in (-1, 1):
        put(b, box((0.08, width * 0.6, h - 0.04), (sx * length * 0.32, 0, (h - 0.04) / 2)), m["tool"], at, yaw)
    b.collider(moved(box((length, width, h), (0, 0, h / 2)), at, yaw))
    per = seats // 2
    for side in (-1, 1):
        for k in range(per):
            x = -length / 2 + length * (k + 0.5) / per
            c, s = math.cos(yaw), math.sin(yaw)
            dx, dy = x, side * (width / 2 + 0.35)
            chair(b, m, (at[0] + c * dx - s * dy, at[1] + s * dx + c * dy, at[2]), yaw - side * math.pi / 2)


def desk(b, m, at, yaw, w=1.2, d=0.6, screens=1, h=0.74):
    """A work desk with its back to a wall and its front +X of `yaw`: a top,
    a drawer pedestal, `screens` monitors, a keyboard, and a chair."""
    put(b, box((d, w, 0.03), (0, 0, h), tile=1.0, bevel=0.005), m["locker"], at, yaw)
    put(b, box((d - 0.05, 0.4, h - 0.02), (0, -w / 2 + 0.22, (h - 0.02) / 2), tile=0.8), m["rack"], at, yaw)
    put(b, box((0.03, 0.03, h), (d / 2 - 0.04, w / 2 - 0.04, h / 2)), m["tool"], at, yaw)
    put(b, box((0.03, 0.03, h), (-d / 2 + 0.04, w / 2 - 0.04, h / 2)), m["tool"], at, yaw)
    for k in range(screens):
        dy = (k - (screens - 1) / 2) * 0.58
        put(b, box((0.03, 0.55, 0.34), (-d / 2 + 0.1, dy, h + 0.28), bevel=0.005), m["dark"], at, yaw)
        put(b, box((0.005, 0.51, 0.3), (-d / 2 + 0.118, dy, h + 0.28)), m["screen"], at, yaw)
        put(b, box((0.12, 0.04, 0.12), (-d / 2 + 0.06, dy, h + 0.07)), m["dark"], at, yaw)
    put(b, box((0.16, 0.44, 0.02), (0.05, 0, h + 0.025)), m["dark"], at, yaw)
    b.collider(moved(box((d, w, h), (0, 0, h / 2)), at, yaw))
    c, s = math.cos(yaw), math.sin(yaw)
    chair(b, m, (at[0] + c * (d / 2 + 0.3), at[1] + s * (d / 2 + 0.3), at[2]), yaw + math.pi)


def counter(b, m, at, yaw, length, depth=0.6, h=0.9, sink=None, lockers=True):
    """A run of base cabinets with a top, a sink at `sink` metres along it,
    and wall lockers over it."""
    put(b, box((depth, length, 0.04), (0, 0, h - 0.02), tile=1.0), m["stainless"], at, yaw)
    put(b, box((depth - 0.06, length - 0.02, h - 0.12), (-0.02, 0, 0.08 + (h - 0.12) / 2), tile=1.0), m["rack"], at, yaw)
    put(b, box((depth - 0.1, length - 0.02, 0.08), (-0.05, 0, 0.04)), m["dark"], at, yaw)
    fronts = bmesh.new()
    n = max(1, int(length / 0.5))
    for k in range(n):
        y = -length / 2 + length * (k + 0.5) / n
        merge(fronts, box((0.02, length / n - 0.02, 0.28), (depth / 2 - 0.04, y, h - 0.22)))
        merge(fronts, box((0.02, length / n - 0.02, 0.4), (depth / 2 - 0.04, y, 0.34)))
    put(b, fronts, m["locker"], at, yaw)
    if sink is not None:
        y = -length / 2 + sink
        put(b, box((0.34, 0.42, 0.02), (0.02, y, h + 0.001)), m["dark"], at, yaw)
        put(b, tube([Vector((-depth / 2 + 0.08, y, h)), Vector((-depth / 2 + 0.08, y, h + 0.28)), Vector((-depth / 2 + 0.24, y, h + 0.3))], 0.015, 8), m["stainless"], at, yaw)
    if lockers:
        put(b, box((0.34, length, 0.6), (-depth / 2 + 0.17, 0, h + 0.75), tile=1.0, bevel=0.01), m["rack"], at, yaw)
        for k in range(n):
            y = -length / 2 + length * (k + 0.5) / n
            put(b, box((0.02, length / n - 0.03, 0.56), (-depth / 2 + 0.35, y, h + 0.75)), m["locker"], at, yaw)
    b.collider(moved(box((depth, length, h), (0, 0, h / 2)), at, yaw))


def fridge(b, m, at, yaw, h=1.8):
    """An upright fridge-freezer, its doors facing +X of `yaw`."""
    put(b, box((0.62, 0.6, h), (0, 0, h / 2), bevel=0.02), m["stainless"], at, yaw)
    for z0, z1 in ((0.05, 0.7), (0.74, h - 0.04)):
        put(b, box((0.02, 0.56, z1 - z0), (0.32, 0, (z0 + z1) / 2)), m["locker"], at, yaw)
        put(b, box((0.04, 0.03, 0.4), (0.34, 0.24, z1 - 0.3)), m["tool"], at, yaw)
    b.collider(moved(box((0.62, 0.6, h), (0, 0, h / 2)), at, yaw))


def warmer(b, m, at, yaw):
    """A food warmer on a counter: a box with a hinged door and a status light."""
    put(b, box((0.36, 0.46, 0.28), (0, 0, 0.14), bevel=0.01), m["locker"], at, yaw)
    put(b, box((0.01, 0.38, 0.2), (0.18, 0, 0.14)), m["dark"], at, yaw)
    put(b, box((0.01, 0.04, 0.03), (0.185, 0.18, 0.24)), m["display"], at, yaw)


def bunk(b, m, at, yaw, length=2.0, width=0.85, h=0.5):
    """A berth: a drawer base, a mattress, a pillow and a turned-down blanket."""
    put(b, box((width, length, h - 0.12), (0, 0, (h - 0.12) / 2), tile=1.0), m["rack"], at, yaw)
    put(b, box((width - 0.04, length - 0.04, 0.14), (0, 0, h - 0.06), bevel=0.04), m["sheet"], at, yaw, True)
    put(b, box((width - 0.2, 0.42, 0.1), (0, length / 2 - 0.3, h + 0.05), bevel=0.04), m["porcelain"], at, yaw, True)
    put(b, box((width - 0.02, length * 0.55, 0.05), (0, -length * 0.2, h + 0.02), bevel=0.02), m["cushion"], at, yaw, True)
    for k in range(2):
        put(b, box((0.02, 0.5, 0.15), (width / 2 + 0.005, -length / 4 + k * length / 2, 0.2)), m["locker"], at, yaw)
    b.collider(moved(box((width, length, h), (0, 0, h / 2)), at, yaw))


def treadmill(b, m, at, yaw):
    """A treadmill with a harness frame for Mars-gravity running (CHAPEA, T2): deck, console, uprights."""
    put(b, box((1.9, 0.8, 0.22), (0, 0, 0.11), bevel=0.03), m["dark"], at, yaw)
    put(b, box((1.6, 0.52, 0.02), (0, 0, 0.23)), m["mat"], at, yaw)
    frame = bmesh.new()
    for sy in (-1, 1):
        merge(frame, rod((0.85, sy * 0.36, 0.2), (0.75, sy * 0.36, 1.2), 0.03, 8))
        merge(frame, rod((-0.6, sy * 0.5, 0.0), (-0.6, sy * 0.5, 2.1), 0.035, 8))
        merge(frame, rod((0.6, sy * 0.5, 0.0), (0.6, sy * 0.5, 2.1), 0.035, 8))
    merge(frame, rod((-0.6, -0.5, 2.1), (0.6, -0.5, 2.1), 0.035, 8))
    merge(frame, rod((-0.6, 0.5, 2.1), (0.6, 0.5, 2.1), 0.035, 8))
    merge(frame, rod((0.0, -0.5, 2.1), (0.0, 0.5, 2.1), 0.035, 8))
    put(b, frame, m["tool"], at, yaw)
    put(b, box((0.12, 0.7, 0.3), (0.75, 0, 1.25), bevel=0.02), m["dark"], at, yaw)
    put(b, box((0.01, 0.4, 0.2), (0.69, 0, 1.28)), m["screen"], at, yaw)
    for sy in (-1, 1):
        put(b, rod((0.0, sy * 0.15, 2.1), (0.0, sy * 0.15, 1.3), 0.012, 6), m["orange"], at, yaw)
    b.collider(moved(box((1.9, 1.0, 1.2), (0, 0, 0.6)), at, yaw))


def resistive(b, m, at, yaw):
    """A resistive exercise device after ARED: a platform, two cylinders behind a frame, a lift bar on arms."""
    put(b, box((1.6, 1.1, 0.12), (0, 0, 0.06)), m["dark"], at, yaw)
    frame = bmesh.new()
    for sy in (-1, 1):
        merge(frame, box((0.1, 0.1, 2.0), (-0.7, sy * 0.5, 1.0)))
        merge(frame, rod((-0.65, sy * 0.45, 0.6), (0.3, sy * 0.3, 1.1), 0.04, 8))
    merge(frame, box((0.1, 1.1, 0.1), (-0.7, 0, 2.0)))
    put(b, frame, m["paint_grey"], at, yaw)
    for sy in (-1, 1):
        put(b, rod((-0.72, sy * 0.25, 0.15), (-0.72, sy * 0.25, 1.4), 0.09, 16), m["tool"], at, yaw)
    put(b, rod((0.3, -0.6, 1.1), (0.3, 0.6, 1.1), 0.025, 10), m["steel"], at, yaw)
    put(b, box((0.7, 0.5, 0.1), (0.1, 0, 0.45)), m["cushion"], at, yaw)
    b.collider(moved(box((1.6, 1.1, 2.0), (0, 0, 1.0)), at, yaw))


def cycle(b, m, at, yaw):
    """A cycle ergometer after CEVIS: a flywheel housing, saddle and handlebar on a frame, facing +X of `yaw`."""
    put(b, moved(lathe([(0.0, -0.08), (0.28, -0.08), (0.28, 0.08), (0.0, 0.08)], 24, 0.5), (0.35, 0, 0.45), roll=math.pi / 2), m["dark"], at, yaw)
    frame = bmesh.new()
    merge(frame, rod((-0.4, 0, 0.05), (0.5, 0, 0.05), 0.035, 8))
    merge(frame, rod((-0.25, 0, 0.05), (-0.25, 0, 0.85), 0.03, 8))
    merge(frame, rod((0.4, 0, 0.45), (0.45, 0, 1.05), 0.03, 8))
    merge(frame, rod((0.45, -0.25, 1.05), (0.45, 0.25, 1.05), 0.02, 8))
    put(b, frame, m["tool"], at, yaw)
    put(b, box((0.28, 0.22, 0.07), (-0.25, 0, 0.88), bevel=0.03), m["cushion"], at, yaw)
    put(b, box((0.08, 0.2, 0.14), (0.47, 0, 1.15)), m["screen"], at, yaw)
    b.collider(moved(box((1.0, 0.5, 1.1), (0, 0, 0.55)), at, yaw))


def sofa(b, m, at, yaw, length=2.0):
    """A low sofa: a base, seat and back cushions, two arms; seated facing +X of `yaw`."""
    put(b, box((0.8, length, 0.25), (0, 0, 0.125)), m["dark"], at, yaw)
    n = max(1, int(length / 0.7))
    for k in range(n):
        y = -length / 2 + 0.1 + (length - 0.2) * (k + 0.5) / n
        put(b, box((0.62, (length - 0.2) / n - 0.02, 0.16), (0.05, y, 0.33), bevel=0.05), m["cushion"], at, yaw, True)
        put(b, box((0.18, (length - 0.2) / n - 0.02, 0.45), (-0.28, y, 0.6), bevel=0.06), m["cushion"], at, yaw, True)
    for sy in (-1, 1):
        put(b, box((0.8, 0.1, 0.55), (0, sy * (length / 2 - 0.05), 0.28), bevel=0.03), m["curtain"], at, yaw, True)
    b.collider(moved(box((0.8, length, 0.8), (0, 0, 0.4)), at, yaw))


def shelf(b, m, at, yaw, w=1.0, d=0.35, h=1.8, levels=5, things=True):
    """An open shelf unit against a wall, its shelves holding boxes and binders."""
    frame = bmesh.new()
    for sy in (-1, 1):
        merge(frame, box((d, 0.02, h), (0, sy * w / 2, h / 2)))
    for k in range(levels):
        merge(frame, box((d, w, 0.02), (0, 0, 0.05 + k * (h - 0.1) / (levels - 1))))
    put(b, frame, m["locker"], at, yaw)
    if things:
        stuff = bmesh.new()
        for k in range(levels - 1):
            z = 0.06 + k * (h - 0.1) / (levels - 1)
            y = -w / 2 + 0.05
            while y < w / 2 - 0.12:
                sw = 0.06 + ((k * 13 + int(y * 37)) % 5) * 0.04
                sh = min((h - 0.1) / (levels - 1) - 0.06, 0.18 + ((k * 7 + int(y * 23)) % 4) * 0.04)
                merge(stuff, box((d * 0.8, sw, sh), (0, y + sw / 2, z + sh / 2)))
                y += sw + 0.01
        put(b, stuff, m["bag"] if levels > 4 else m["rack"], at, yaw)
    b.collider(moved(box((d, w, h), (0, 0, h / 2)), at, yaw))


def screen_wall(b, m, at, yaw, w=2.4, h=1.3, z=1.0):
    """A wall display, its face +X of `yaw`, in a thin bezel."""
    put(b, box((0.05, w + 0.06, h + 0.06), (0, 0, z + h / 2)), m["dark"], at, yaw)
    put(b, box((0.005, w, h), (0.027, 0, z + h / 2)), m["screen"], at, yaw)


# Hygiene


def hygiene(b, m, at, yaw, w=1.1, d=1.3, h=2.2):
    """A hygiene pair after the Common Habitat cluster: a toilet compartment
    and a wet bath side by side, their curtained doors facing +X of `yaw`, a
    wash basin outside between them."""
    x, y, z = at
    c, s = math.cos(yaw), math.sin(yaw)

    def p(dx, dy):
        return (x + c * dx - s * dy, y + s * dx + c * dy, z)

    for k in (-1, 0, 1):
        partition(b, m, p(d / 2, k * w), p(-d / 2, k * w), z, h)
    partition(b, m, p(d / 2, -w), p(d / 2, w), z, h, gaps=((0.2, w - 0.2), (w + 0.2, 2 * w - 0.2)))
    put(b, box((d, 2 * w, 0.04), (0, 0, h), tile=1.0), m["liner"], (x, y, z), yaw)
    for k in (-1, 1):
        put(b, box((0.03, w - 0.42, 1.9), (d / 2 + 0.06, k * w / 2 - 0.15, 0.98), bevel=0.01), m["curtain"], (x, y, z), yaw)
    # The toilet: a waste and hygiene compartment's pedestal, seat, urine funnel on its hose.
    put(b, lathe([(0.0, 0.0), (0.2, 0.0), (0.18, 0.38), (0.22, 0.44), (0.0, 0.45)], 20, 0.5), m["porcelain"], p(-d / 2 + 0.35, -w / 2), yaw, True)
    put(b, tube([Vector((-d / 2 + 0.15, -w / 2 + 0.3, 0.5)), Vector((-d / 2 + 0.2, -w / 2 + 0.35, 0.9)), Vector((-d / 2 + 0.12, -w / 2 + 0.38, 1.1))], 0.025, 8), m["hose"], (x, y, z), yaw)
    put(b, cone(0.06, 0.03, 0.14, (-d / 2 + 0.12, -w / 2 + 0.38, 1.1), 12), m["locker"], (x, y, z), yaw)
    put(b, box((0.25, 0.4, 0.6), (-d / 2 + 0.13, -w / 2 + 0.3, 1.5)), m["rack"], (x, y, z), yaw)
    # The wet bath: a tray, a hand shower on its rail, a grab rail, a fold-down seat.
    put(b, box((d - 0.1, w - 0.1, 0.06), (0, w / 2, 0.03)), m["porcelain"], (x, y, z), yaw)
    put(b, box((0.03, 0.03, 0.9), (-d / 2 + 0.08, w / 2, 1.5)), m["stainless"], (x, y, z), yaw)
    put(b, cone(0.04, 0.07, 0.04, (-d / 2 + 0.12, w / 2, 1.9), 12), m["stainless"], (x, y, z), yaw)
    put(b, rod((-d / 2 + 0.08, w - 0.08, 1.0), (d / 2 - 0.2, w - 0.08, 1.0), 0.016, 8), m["stainless"], (x, y, z), yaw)
    put(b, box((0.35, 0.4, 0.04), (-d / 2 + 0.25, w * 0.85 - 0.1, 0.45)), m["locker"], (x, y, z), yaw)
    # The basin between the doors.
    put(b, box((0.36, 0.5, 0.08), (d / 2 + 0.2, 0, 0.84)), m["porcelain"], (x, y, z), yaw)
    put(b, box((0.3, 0.38, 0.78), (d / 2 + 0.17, 0, 0.4)), m["rack"], (x, y, z), yaw)
    put(b, box((0.01, 0.4, 0.5), (d / 2 + 0.04, 0, 1.4)), m["glass"], (x, y, z), yaw)


def compactor(b, m, at, yaw, tiles=7, side=1):
    """A heat melt compactor (ICES-2020-326): a squat cabinet with a
    top-loading chamber lid and a display, the trash tiles it pressed
    (16 in square) stacked beside it for shielding."""
    put(b, box((0.7, 0.8, 1.1), (0, 0, 0.55), bevel=0.02), m["paint_white"], at, yaw)
    put(b, box((0.5, 0.6, 0.04), (-0.02, 0, 1.12)), m["locker"], at, yaw)
    put(b, box((0.01, 0.3, 0.16), (0.355, 0.15, 0.85)), m["display"], at, yaw)
    put(b, box((0.01, 0.6, 0.3), (0.355, 0, 0.35)), m["dark"], at, yaw)
    stack = bmesh.new()
    for k in range(tiles):
        merge(stack, box((0.41, 0.41, 0.045), (0.0, side * (0.63 + 0.45 * (k // 7)), 0.03 + 0.05 * (k % 7)), bevel=0.005))
    put(b, stack, m["basalt"], at, yaw)
    b.collider(moved(box((0.7, 1.3, 1.1), (0, side * 0.25, 0.55)), at, yaw))


# Laboratory and medical


def glovebox(b, m, at, yaw, w=1.2, d=0.7, airlock=None):
    """A glovebox on a lab bench: a cabinet with a sloped window, two glove
    ports with gloves hanging in, an exhaust filter on top, and, when
    `airlock` is a length, a sample airlock tube out of its back that long."""
    bench(b, m, at, yaw, w + 0.3, d + 0.1)
    x, y, z = at
    top = 0.92
    put(b, box((d - 0.1, w, 0.65), (-0.05, 0, top + 0.33), bevel=0.01), m["paint_white"], (x, y, z), yaw)
    put(b, box((0.02, w - 0.12, 0.5), (d / 2 - 0.12, 0, top + 0.33)), m["glass"], (x, y, z), yaw)
    for sy in (-0.25, 0.25):
        put(b, moved(lathe([(0.075, -0.03), (0.1, -0.03), (0.1, 0.03), (0.075, 0.03)], 16, 0.2), pitch=math.pi / 2), m["rubber"], (x + math.cos(yaw) * (d / 2 - 0.12) - math.sin(yaw) * sy, y + math.sin(yaw) * (d / 2 - 0.12) + math.cos(yaw) * sy, z + top + 0.3), yaw)
        put(b, tube([Vector((d / 2 - 0.14, sy, top + 0.3)), Vector((d / 2 - 0.34, sy, top + 0.22)), Vector((d / 2 - 0.42, sy, top + 0.1))], 0.05, 10), m["rubber"], (x, y, z), yaw)
    put(b, box((0.25, 0.4, 0.15), (-0.15, 0, top + 0.72)), m["louvre"], (x, y, z), yaw)
    if airlock:
        put(b, rod((-d / 2 + 0.05, 0.3, top + 0.35), (-d / 2 - airlock, 0.3, top + 0.35), 0.15, 16), m["stainless"], (x, y, z), yaw)
        put(b, moved(lathe([(0.15, 0.0), (0.2, 0.0), (0.2, 0.05), (0.15, 0.05)], 16, 0.2), (-d / 2 - 0.2, 0.3, top + 0.35), pitch=math.pi / 2), m["tool"], (x, y, z), yaw)


def bench(b, m, at, yaw, w=1.6, d=0.75, h=0.9, skin="paint_white", top="dark"):
    """A bench: a resin or steel top over a cabinet base, its front +X of `yaw`."""
    put(b, box((d, w, 0.04), (0, 0, h - 0.02), tile=1.0), m[top], at, yaw)
    put(b, box((d - 0.06, w - 0.04, h - 0.1), (-0.02, 0, (h - 0.1) / 2 + 0.04), tile=1.0), m[skin], at, yaw)
    fronts = bmesh.new()
    n = max(1, int(w / 0.5))
    for k in range(n):
        merge(fronts, box((0.02, w / n - 0.03, h - 0.2), (d / 2 - 0.04, -w / 2 + w * (k + 0.5) / n, (h - 0.2) / 2 + 0.08)))
    put(b, fronts, m["locker"], at, yaw)
    b.collider(moved(box((d, w, h), (0, 0, h / 2)), at, yaw))


def microscope(b, m, at, yaw):
    """A microscope on a bench top: base, pillar, stage, head and eyepieces."""
    put(b, box((0.25, 0.2, 0.05), (0, 0, 0.025)), m["paint_white"], at, yaw)
    put(b, box((0.06, 0.08, 0.35), (-0.08, 0, 0.2)), m["paint_white"], at, yaw)
    put(b, box((0.16, 0.16, 0.02), (0.02, 0, 0.16)), m["dark"], at, yaw)
    put(b, box((0.14, 0.09, 0.08), (0.0, 0, 0.36)), m["paint_white"], at, yaw)
    for sy in (-0.025, 0.025):
        put(b, rod((0.0, sy, 0.4), (0.07, sy, 0.47), 0.012, 8), m["dark"], at, yaw)
    put(b, rod((0.03, 0, 0.32), (0.03, 0, 0.2), 0.015, 8), m["tool"], at, yaw)


def core_rack(b, m, at, yaw, w=1.2, levels=4):
    """A rack of drill cores in split liners, labelled, on shelves."""
    shelf(b, m, at, yaw, w, 0.4, 1.6, levels, things=False)
    cores = bmesh.new()
    for k in range(levels - 1):
        z = 0.06 + k * 1.5 / (levels - 1)
        for j in range(5):
            merge(cores, rod((-0.15, -w / 2 + 0.12 + j * (w - 0.2) / 5, z + 0.05), (0.15, -w / 2 + 0.12 + j * (w - 0.2) / 5, z + 0.05), 0.04, 10))
    put(b, cores, m["basalt"], at, yaw)


def med_bed(b, m, at, yaw):
    """A surgical bed with room all round it: a pedestal, a padded top, side
    rails, an overhead surgical light on its arm, a monitor on a pole."""
    put(b, box((0.5, 0.4, 0.7), (0, 0, 0.35)), m["paint_white"], at, yaw)
    put(b, box((0.7, 0.6, 0.06), (0, 0, 0.03)), m["dark"], at, yaw)
    put(b, box((2.0, 0.7, 0.1), (0, 0, 0.78), bevel=0.04), m["cushion"], at, yaw, True)
    for sy in (-1, 1):
        put(b, rod((-0.7, sy * 0.38, 0.9), (0.7, sy * 0.38, 0.9), 0.012, 6), m["stainless"], at, yaw)
    put(b, rod((0, 0, 2.6), (0, 0, 2.2), 0.04, 8), m["tool"], at, yaw)
    put(b, rod((0, 0, 2.2), (0.3, 0.2, 2.0), 0.03, 8), m["tool"], at, yaw)
    put(b, lathe([(0.0, 1.88), (0.32, 1.9), (0.3, 2.0), (0.0, 2.04)], 24, 0.5), m["paint_white"], (at[0], at[1], at[2]), yaw)
    put(b, lathe([(0.0, 1.87), (0.28, 1.875), (0.0, 1.876)], 24, 0.5), m["panel_lamp"], (at[0], at[1], at[2]), yaw)
    b.places.append(("light", {"at": (at[0], at[1], at[2] + 1.8), "kind": "cabin"}))
    put(b, rod((-1.2, -0.5, 0.0), (-1.2, -0.5, 1.6), 0.02, 8), m["stainless"], at, yaw)
    put(b, box((0.06, 0.4, 0.3), (-1.2, -0.5, 1.6)), m["dark"], at, yaw)
    put(b, box((0.005, 0.36, 0.26), (-1.165, -0.5, 1.6)), m["display"], at, yaw)
    b.collider(moved(box((2.0, 0.7, 0.85), (0, 0, 0.42)), at, yaw))


def cart(b, m, at, yaw, kind="ultrasound"):
    """A wheeled cart: an ultrasound with its screen, or a crash cart with drawers and an AED."""
    put(b, box((0.5, 0.45, 0.85), (0, 0, 0.5), bevel=0.02), m["paint_white"] if kind == "ultrasound" else m["paint_red"], at, yaw)
    for sx in (-0.2, 0.2):
        for sy in (-0.18, 0.18):
            put(b, sphere(0.04, (sx, sy, 0.04), 8, 4), m["dark"], at, yaw)
    if kind == "ultrasound":
        put(b, box((0.05, 0.4, 0.3), (-0.1, 0, 1.2), bevel=0.01), m["dark"], at, yaw)
        put(b, box((0.005, 0.36, 0.26), (-0.072, 0, 1.2)), m["screen"], at, yaw)
        put(b, rod((0.2, 0.15, 0.95), (0.25, 0.15, 1.05), 0.02, 8), m["dark"], at, yaw)
    else:
        for k in range(4):
            put(b, box((0.02, 0.4, 0.15), (0.255, 0, 0.2 + k * 0.18)), m["locker"], at, yaw)
        put(b, box((0.25, 0.3, 0.12), (0, 0, 0.99)), m["paint_yellow"], at, yaw)


# Workshop


def tool_wall(b, m, at, yaw, w=2.0, h=1.2, z=1.0):
    """A pegboard with tools hung on it, facing +X of `yaw`."""
    put(b, box((0.02, w, h), (0, 0, z + h / 2), tile=0.6), m["rack"], at, yaw)
    tools = bmesh.new()
    for i in range(int(w / 0.18)):
        for j in range(3):
            y = -w / 2 + 0.12 + i * 0.18
            zz = z + 0.2 + j * 0.35
            k = (i * 5 + j * 3) % 4
            if k == 0:
                merge(tools, box((0.02, 0.03, 0.22), (0.03, y, zz)))
            elif k == 1:
                merge(tools, box((0.03, 0.12, 0.04), (0.03, y, zz + 0.08)))
                merge(tools, box((0.02, 0.025, 0.16), (0.03, y, zz)))
            elif k == 2:
                merge(tools, rod((0.03, y, zz - 0.1), (0.03, y, zz + 0.12), 0.012, 6))
    put(b, tools, m["tool"], at, yaw)


def printer(b, m, at, yaw, size=(0.6, 0.6, 0.7), on_floor=False):
    """A printer cabinet: a frame with a window, a lit build chamber, a spool on its side."""
    w, d, h = size
    z0 = 0.0 if on_floor else 0.9
    put(b, box((d, w, h), (0, 0, z0 + h / 2), bevel=0.02), m["dark"], at, yaw)
    put(b, box((0.01, w - 0.12, h - 0.2), (d / 2 + 0.002, 0, z0 + h / 2 + 0.05)), m["glass"], at, yaw)
    put(b, box((d - 0.12, w - 0.16, 0.01), (0, 0, z0 + 0.12)), m["display"], at, yaw)
    put(b, moved(lathe([(0.04, -0.04), (0.12, -0.04), (0.12, 0.04), (0.04, 0.04)], 16, 0.2), (0, w / 2 + 0.06, z0 + h * 0.6), roll=math.pi / 2), m["orange"], at, yaw)
    b.collider(moved(box((d, w, h), (0, 0, z0 + h / 2)), at, yaw))


def mill(b, m, at, yaw):
    """A small vertical mill: a base cabinet, column, head with its spindle, and a table on its saddle."""
    put(b, box((0.7, 0.9, 0.85), (0, 0, 0.425)), m["paint_grey"], at, yaw)
    put(b, box((0.3, 0.35, 1.0), (-0.2, 0, 1.35)), m["paint_grey"], at, yaw)
    put(b, box((0.45, 0.3, 0.35), (0.05, 0, 1.75)), m["paint_grey"], at, yaw)
    put(b, rod((0.15, 0, 1.58), (0.15, 0, 1.4), 0.04, 12), m["tool"], at, yaw)
    put(b, box((0.35, 0.8, 0.06), (0.12, 0, 0.98)), m["tool"], at, yaw)
    b.collider(moved(box((0.7, 0.9, 1.9), (0, 0, 0.95)), at, yaw))


def lathe_tool(b, m, at, yaw, length=1.4):
    """A small lathe on a stand: the bed, headstock with its chuck, tailstock and carriage."""
    put(b, box((0.5, length, 0.75), (0, 0, 0.375)), m["paint_grey"], at, yaw)
    put(b, box((0.25, length, 0.12), (0, 0, 0.81)), m["tool"], at, yaw)
    put(b, box((0.32, 0.35, 0.32), (0, -length / 2 + 0.18, 1.03)), m["paint_grey"], at, yaw)
    put(b, rod((0, -length / 2 + 0.36, 1.03), (0, -length / 2 + 0.46, 1.03), 0.08, 12), m["tool"], at, yaw)
    put(b, box((0.22, 0.2, 0.22), (0, length / 2 - 0.15, 0.98)), m["paint_grey"], at, yaw)
    put(b, box((0.3, 0.2, 0.1), (0.05, 0.0, 0.92)), m["tool"], at, yaw)
    b.collider(moved(box((0.5, length, 1.2), (0, 0, 0.6)), at, yaw))


def welding_booth(b, m, at, yaw, w=1.6, d=1.4):
    """A welding booth: a steel table, orange screens on three sides, a fume arm from above."""
    x, y, z = at
    put(b, box((d - 0.4, w - 0.4, 0.04), (0, 0, 0.85)), m["steel"], at, yaw)
    put(b, box((d - 0.5, w - 0.5, 0.8), (0, 0, 0.42)), m["paint_dark"], at, yaw)
    for dy, sx, sy in ((-w / 2, d, 0.02), (w / 2, d, 0.02)):
        put(b, box((sx, sy, 1.8), (0, dy, 0.95)), m["orange"], at, yaw)
    put(b, box((0.02, w, 1.8), (-d / 2, 0, 0.95)), m["orange"], at, yaw)
    put(b, tube([Vector((-d / 2, 0, 2.4)), Vector((-0.1, 0, 2.2)), Vector((0.1, 0, 1.6))], 0.08, 12), m["paint_yellow"], at, yaw)
    put(b, cone(0.18, 0.08, 0.15, (0.1, 0, 1.45), 16), m["paint_yellow"], at, yaw)
    b.collider(moved(box((d, w, 1.9), (0, 0, 0.95)), at, yaw))


def hood(b, m, at, yaw, w=1.4, z=2.0):
    """A vent hood over a bench, its duct going up."""
    put(b, box((0.7, w, 0.35), (0, 0, z + 0.175), bevel=0.01), m["stainless"], at, yaw)
    put(b, box((0.3, 0.3, 0.6), (-0.15, 0, z + 0.65)), m["stainless"], at, yaw)


# EVA kit


def suit(b, m, at, yaw, kneel=0.0):
    """An EVA suit standing on its don/doff stand, its front facing +X of
    `yaw`: torso and life-support pack, helmet with a gold visor, arms,
    legs and boots."""
    x, y, z = at
    parts = bmesh.new()
    merge(parts, box((0.32, 0.5, 0.62), (0.0, 0, 1.25), bevel=0.1))
    merge(parts, box((0.3, 0.46, 0.25), (0.0, 0, 0.88), bevel=0.08))
    for sy in (-1, 1):
        merge(parts, tube([Vector((0, sy * 0.12, 0.8)), Vector((0.03, sy * 0.13, 0.48)), Vector((0.0, sy * 0.13, 0.12))], 0.085, 12))
        merge(parts, tube([Vector((0, sy * 0.3, 1.45)), Vector((0.05, sy * 0.36, 1.2)), Vector((0.14, sy * 0.34, 1.0))], 0.07, 12))
    merge(parts, sphere(0.17, (0.02, 0, 1.75), 16, 10))
    put(b, parts, m["suit"], (x, y, z), yaw)
    put(b, box((0.25, 0.48, 0.6), (-0.28, 0, 1.25), bevel=0.04), m["paint_white"], (x, y, z), yaw)
    put(b, sphere(0.15, (0.07, 0, 1.75), 16, 10), m["visor"], (x, y, z), yaw)
    boots = bmesh.new()
    for sy in (-1, 1):
        merge(boots, box((0.28, 0.13, 0.1), (0.05, sy * 0.13, 0.05), bevel=0.03))
        merge(boots, sphere(0.06, (0.18, sy * 0.36, 0.98), 8, 5))
    put(b, boots, m["dark"], (x, y, z), yaw)
    put(b, box((0.06, 0.06, 1.4), (-0.45, 0, 0.7)), m["tool"], (x, y, z), yaw)
    put(b, box((0.12, 0.5, 0.05), (-0.42, 0, 1.4)), m["tool"], (x, y, z), yaw)
    put(b, box((0.6, 0.6, 0.03), (-0.1, 0, 0.015)), m["tool"], (x, y, z), yaw)
    b.collider(moved(box((0.6, 0.7, 1.95), (-0.1, 0, 0.97)), (x, y, z), yaw))


def gas_wand(b, m, at, yaw):
    """A dust-off gas wand on its wall hook, the hose coiled under it."""
    put(b, box((0.05, 0.1, 0.1), (0, 0, 1.5)), m["tool"], at, yaw)
    put(b, rod((0.05, 0, 1.55), (0.12, 0, 1.2), 0.015, 8), m["dark"], at, yaw)
    put(b, moved(lathe([(0.12, -0.02), (0.16, -0.02), (0.16, 0.02), (0.12, 0.02)], 16, 0.2), (0.06, 0, 1.25), pitch=math.pi / 2), m["hose"], at, yaw)


def plants_box(b, m, at, yaw, w=1.88, d=0.64, h=0.61, z=0.9):
    """A plant growth chamber (Common Habitat: 1.88 × 0.64 × 0.61 m): a lit box with greens inside and a glass front."""
    put(b, box((d, w, h), (0, 0, z + h / 2), bevel=0.01), m["rack"], at, yaw)
    put(b, box((0.01, w - 0.06, h - 0.06), (d / 2 + 0.003, 0, z + h / 2)), m["glass"], at, yaw)
    put(b, box((d - 0.06, w - 0.06, 0.01), (0, 0, z + h - 0.04)), m["grow_lamp"], at, yaw)
    greens = bmesh.new()
    for k in range(int(w / 0.2)):
        for j in range(2):
            merge(greens, sphere(0.07 + 0.02 * ((k + j) % 2), (-0.12 + j * 0.24, -w / 2 + 0.12 + k * 0.2, z + 0.1), 8, 5))
    put(b, greens, m["greens"], at, yaw, True)
