"""The crew's buildings fitted out inside, each from `interiors`' kit, after
plans/003-mars-base.md: three hab towers by what their lower decks do (EVA,
science, care and control), the hubs, the commons dome, MARSHA and the rover
hangar.

A fit-out takes the Building, the cabin materials and the shell's frame (a
dict its builder hands over) and works in the building's own frame."""
import math

import bmesh
from mathutils import Vector

import industry
import interiors as ix
from kit import TAU, box, lathe, merge, moved, rod, tube

# How far a tower's lining stands in from its hull, and its decks' clear heights.
LINING_IN = 0.2
CLEAR = (2.6, 2.7)
# Crew quarters on a tower's upper deck: the middle of each wedge (radians
# off the EVA hatch), its half width, and the radius of its front wall.
CQ_AT = (math.radians(120), math.radians(60), math.radians(-60), math.radians(-120))
CQ_HALF = math.radians(25)
CQ_FRONT = 2.55
# The stair well between a tower's decks: (x0, x1, y0, y1), the stair rising toward +Y.
WELL = (-0.6, 0.6, -1.3, 1.7)
STAIR_FOOT = -1.75


def _deg(a):
    return math.radians(a)


# Hab towers


def tower(b, m, f, program):
    """Fits a hab tower: both decks, the stair, the suitlock at the EVA
    hatch, a hygiene pair, then the lower deck by `program` (`eva`,
    `science`, `care`) and the upper deck by it too. Angles are measured in
    the tower's frame, where its corridor port lies at 0 and its EVA hatch at
    pi; `f` holds radius, z0, height, decks, hatch, ports and windows."""
    r = f["radius"] - LINING_IN
    d0 = f["z0"]
    d1 = f["z0"] + f["height"] / f["decks"]
    c0, c1 = d0 + CLEAR[0], d1 + CLEAR[1]
    _decks(b, m, f, r, (d0, d1), (c0, c1))
    _suitlock(b, m, r, d0, c0)
    ix.hygiene(b, m, ix.polar(_deg(242), r - 0.82, d0), _deg(242) + math.pi)
    {"eva": _eva_lower, "science": _science_lower, "care": _care_lower}[program](b, m, r, d0, c0)
    if program == "care":
        _control_upper(b, m, r, d1, c1)
    else:
        for a in CQ_AT:
            _quarters(b, m, r, d1, c1, a)
        _galley_nook(b, m, r, d1)
        (_workstations if program == "eva" else _exercise)(b, m, r, d1)


def _decks(b, m, f, r, decks, ceilings):
    """Floors, ceilings with their light panels, linings, kick strips, the
    stair and its well, the portholes and hatches seen from inside."""
    d0, d1 = decks
    c0, c1 = ceilings
    b.put(ix.floor(r + 0.05, d0, 0.12), m["deck"], smooth=False)
    b.put(ix.floor(r + 0.05, d1, 0.3, [WELL]), m["deck"], smooth=False)
    b.put(ix.floor(r + 0.05, c0 + 0.05, 0.05, [WELL]), m["ceiling"], smooth=False)
    b.put(ix.floor(r + 0.05, c1 + 0.05, 0.05), m["ceiling"], smooth=False)
    b.collider(ix.floor(r, d0, 0.12))
    b.collider(ix.floor(r, d1, 0.3, [WELL]))
    for z0, z1 in ((d0, c0), (d1, c1)):
        b.put(ix.lining(r, z0, z1), m["liner"], smooth=True)
        b.put(ix.skirt(r, z0), m["dark"], smooth=False)
    # The well's lining through the plenum, and the stair up it.
    x0, x1, y0, y1 = WELL
    shaft = bmesh.new()
    for a, c in (((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))):
        mid = ((a[0] + c[0]) / 2, (a[1] + c[1]) / 2)
        length = math.hypot(c[0] - a[0], c[1] - a[1])
        merge(shaft, moved(box((length, 0.03, d1 - c0), (0, 0, (c0 + d1) / 2)), (mid[0], mid[1], 0), math.atan2(c[1] - a[1], c[0] - a[0])))
    b.put(shaft, m["liner"], smooth=False)
    _stair(b, m, (x0 + 0.05, x1 - 0.05), STAIR_FOOT, y1 - 0.25, d0, d1)
    ix.railing(b, m, [(x0, y1 - 0.4), (x0, y0), (x1, y0), (x1, y1 - 0.4)], d1)
    # Portholes and hatches.
    for deck, a, z in f["windows"]:
        ix.porthole(b, m, ix.polar(a, r - 0.005, z), a + math.pi)
    for p in f["ports"]:
        ix.hatch_door(b, m, ix.polar(p, r - 0.02, d0), p + math.pi)
    # Lights: panels in each ceiling, one light under every other.
    ix.ceiling_panels(b, m, c0, [(2.4, 0.0), (1.0, 2.6), (1.0, -2.6), (-1.4, 2.2), (-1.4, -2.2), (2.6, 1.6)])
    ix.ceiling_panels(b, m, c1, [(1.6, 1.4), (1.6, -1.4), (-1.6, 0.0), (0.0, -1.9)])
    for a in CQ_AT:
        ix.ceiling_panels(b, m, c1, [ix.polar(a, (CQ_FRONT + r) / 2)[:2]], size=(0.4, 0.4))


def _stair(b, m, xs, y0, y1, z0, z1, going_m=0.24):
    """A steep ship stair rising along +Y from (y0, z0) to (y1, z1) between
    stringers at `xs`, open treads, a handrail on each side."""
    steps = max(2, round((z1 - z0) / 0.22))
    rise = (z1 - z0) / steps
    run = (y1 - y0) / steps
    treads = bmesh.new()
    for k in range(1, steps):
        merge(treads, box((xs[1] - xs[0], max(going_m, run + 0.03), 0.04), ((xs[0] + xs[1]) / 2, y0 + k * run, z0 + k * rise - 0.02)))
    b.put(treads, m["tread"], smooth=False)
    pitch = math.atan2(z1 - z0, y1 - y0)
    length = math.hypot(z1 - z0, y1 - y0)
    for x in xs:
        b.put(moved(box((0.04, length, 0.2), (0, 0, 0)), (x, (y0 + y1) / 2, (z0 + z1) / 2 - 0.05), 0.0, 0.0, pitch), m["steel"], smooth=False)
        b.put(rod((x, y0, z0 + 0.95), (x, y1, z1 + 0.95), 0.022, 8), m["rail"], smooth=False)
        b.put(rod((x, y0, z0), (x, y0, z0 + 0.95), 0.022, 8), m["rail"], smooth=False)
    b.collider(moved(box((xs[1] - xs[0], length, 0.1), (0, 0, 0)), ((xs[0] + xs[1]) / 2, (y0 + y1) / 2, (z0 + z1) / 2), 0.0, 0.0, pitch))


def _suitlock(b, m, r, z, ceiling):
    """The suitlock behind the EVA hatch (HDU's dust module, Quest's crew
    lock): walls round it, an inner hatch, a grated floor over its dust
    pit, two suits on their don/doff stands, a gas wand to blow the dust
    off, boots and a bench."""
    h = ceiling - z
    front, side = -2.1, 1.3
    back = -math.sqrt(r * r - side * side)
    ix.partition(b, m, (front, -side), (front, side), z, h, gaps=((side - 0.63, side + 0.63),))
    for s in (-1, 1):
        ix.partition(b, m, (back, s * side), (front, s * side), z, h)
    ix.hatch_door(b, m, (front + 0.04, 0.0, z), 0.0)
    ix.hatch_door(b, m, (-r + 0.03, 0.0, z), 0.0, w=1.0, h=1.6, sill=0.05)
    b.put(box((front - back - 0.1, 2 * side - 0.1, 0.03), ((front + back) / 2, 0, z + 0.015), tile=0.6), m["grate"], smooth=False)
    for s in (-1, 1):
        ix.suit(b, m, (-3.25, s * 0.78, z + 0.03), 0.0)
    ix.gas_wand(b, m, (front - 0.5, side - 0.04, z), -math.pi / 2)
    ix.bench(b, m, (front - 0.3, -side + 0.35, z), math.pi / 2, 0.5, 0.4, 0.45, "paint_grey", "rubber")
    ix.ceiling_panels(b, m, ceiling, [((front + back) / 2 + 0.3, 0.0)], size=(0.5, 0.5))


def _wall(a, r, depth):
    """A floor point and facing for something `depth` deep against the lining at angle `a`."""
    return ix.against(a, r, depth)


def _eva_lower(b, m, r, z, ceiling):
    """Tower A's lower deck: the suit service bench and a spare suit beside
    the lock, the life support racks (oxygen generation, CO2 removal, trace
    contaminants, power), the water processor and bag stowage."""
    at, yaw = _wall(_deg(115), r, 0.75)
    ix.bench(b, m, (*at[:2], z), yaw, 1.8, 0.75)
    b.put(moved(box((0.25, 0.45, 0.6), (0, 0, 0)), (*ix.polar(_deg(112), r - 0.6)[:2], z + 1.22), _deg(112)), m["paint_white"], smooth=False)
    ix.tool_wall(b, m, (*ix.polar(_deg(120), r - 0.03)[:2], z), _deg(120) + math.pi, 1.4, 0.8, 1.15)
    ix.suit(b, m, (*ix.polar(_deg(137), r - 0.5)[:2], z), _deg(137) + math.pi)
    for a, face in ((287, "eclss"), (303, "eclss"), (319, "avionics"), (43, "eclss"), (59, "stowage")):
        at, yaw = _wall(_deg(a), r, ix.RACK[1])
        ix.rack(b, m, (*at[:2], z), yaw, face)
    at, yaw = _wall(_deg(75), r, 0.5)
    ix.bags(b, m, (*at[:2], z), yaw, 2, 6)
    at, yaw = _wall(_deg(266), r, 0.7)
    ix.compactor(b, m, (*at[:2], z), yaw, side=-1)


def _science_lower(b, m, r, z, ceiling):
    """Tower B's lower deck, after HDU's GeoLab and Common Habitat's labs: a
    glovebox whose sample airlock goes out through the hull, a microscope
    and balance bench, freezers, a core rack with its logging bench."""
    at, yaw = _wall(_deg(118), r, 0.8)
    ix.glovebox(b, m, (*at[:2], z), yaw, airlock=0.15)
    at, yaw = _wall(_deg(50), r, 0.75)
    ix.bench(b, m, (*at[:2], z), yaw, 1.8, 0.75)
    ix.microscope(b, m, (*ix.polar(_deg(46), r - 0.45)[:2], z + 0.9), _deg(46) + math.pi)
    b.put(moved(box((0.3, 0.25, 0.12), (0, 0, 0.06)), (*ix.polar(_deg(57), r - 0.45)[:2], z + 0.9), _deg(57)), m["paint_white"], smooth=False)
    at, yaw = _wall(_deg(75), r, 0.4)
    ix.core_rack(b, m, (*at[:2], z), yaw)
    for a, face in ((285, "freezer"), (301, "freezer"), (317, "eclss")):
        at, yaw = _wall(_deg(a), r, ix.RACK[1])
        ix.rack(b, m, (*at[:2], z), yaw, face)
    ix.stool(b, m, (*ix.polar(_deg(118), r - 1.25)[:2], z))
    ix.stool(b, m, (*ix.polar(_deg(50), r - 1.2)[:2], z))


def _care_lower(b, m, r, z, ceiling):
    """Tower C's lower deck, the medical bay (Common Habitat, CHAPEA): a
    surgical bed with room all round, a pharmacy and medical racks, a
    medical fridge, ultrasound and crash carts, and an isolation berth
    behind its own wall."""
    ix.med_bed(b, m, (0.9, 2.1, z), 0.0)
    for a, face in ((100, "medical"), (116, "medical"), (43, "freezer"), (59, "stowage")):
        at, yaw = _wall(_deg(a), r, ix.RACK[1])
        ix.rack(b, m, (*at[:2], z), yaw, face)
    ix.fridge(b, m, (*ix.polar(_deg(133), r - 0.35)[:2], z), _deg(133) + math.pi)
    ix.cart(b, m, (2.2, 1.0, z), math.pi, "ultrasound")
    ix.cart(b, m, (-1.2, 2.6, z), -math.pi / 2, "crash")
    # The isolation berth: a wall across the -Y side from the hygiene pair to the ECLSS racks.
    ix.partition(b, m, (0.9, -2.0), (3.2, -2.0), z, ceiling - z, gaps=((0.2, 1.0),))
    ix.partition(b, m, (3.2, -2.0), (3.2, -2.94), z, ceiling - z)
    ix.bunk(b, m, (*ix.polar(_deg(295), r - 0.55)[:2], z), _deg(295))
    b.places.append(("light", {"at": (2.0, -2.9, ceiling - 0.1), "kind": "cabin"}))
    at, yaw = _wall(_deg(317), r, ix.RACK[1])
    ix.rack(b, m, (*at[:2], z), yaw, "eclss")


def _quarters(b, m, r, z, ceiling, a):
    """A crew quarters wedge round angle `a` (Common Habitat, ISS CQ): radial
    walls, a front wall with its door, a berth under the porthole, a desk with
    its screen and chair, a shelf, a privacy curtain half drawn."""
    h = ceiling - z
    a0, a1 = a - CQ_HALF, a + CQ_HALF
    for e in (a0, a1):
        ix.partition(b, m, ix.polar(e, CQ_FRONT)[:2], ix.polar(e, r)[:2], z, h)
    p0, pm, p1 = (ix.polar(e, CQ_FRONT)[:2] for e in (a0, a, a1))
    ix.partition(b, m, p0, pm, z, h, gaps=((0.15, 0.85),))
    ix.partition(b, m, pm, p1, z, h)
    ix.bunk(b, m, (*ix.polar(a, r - 0.55)[:2], z), a)
    t = Vector((-math.sin(a1), math.cos(a1), 0.0))
    at = Vector(ix.polar(a1, CQ_FRONT + 0.4)) - t * 0.32
    ix.desk(b, m, (at.x, at.y, z), a1 - math.pi / 2, 0.6, 0.5)
    at = Vector(ix.polar(a0, r - 0.55)) + Vector((-math.sin(a0), math.cos(a0), 0.0)) * 0.2
    ix.shelf(b, m, (at.x, at.y, z + 0.9), a0 + math.pi / 2, 0.5, 0.3, 1.0, 3)
    door = Vector(ix.polar(a0, CQ_FRONT)).lerp(Vector(ix.polar(a, CQ_FRONT)), 0.3)
    b.put(moved(box((0.03, 0.4, 1.95), (0, 0, 0.98), bevel=0.01), (door.x, door.y, z), a), m["curtain"], smooth=False)


def _galley_nook(b, m, r, z):
    """A tower's galley nook either side of the +X porthole, and a table for four."""
    at, yaw = _wall(_deg(-19), r, 0.6)
    ix.counter(b, m, (*at[:2], z), yaw, 1.5, sink=0.4)
    ix.warmer(b, m, (*ix.polar(_deg(-23), r - 0.35)[:2], z + 0.9), _deg(-23) + math.pi)
    ix.fridge(b, m, (*ix.polar(_deg(22), r - 0.36)[:2], z), _deg(22) + math.pi)
    ix.table(b, m, (2.05, 0.0, z), 0.5, seats=4, start=math.pi / 4)


def _workstations(b, m, r, z):
    """Tower A's upper deck by the stair: two workstations and bag stowage."""
    for a in (160, 200):
        at, yaw = _wall(_deg(a), r, 0.6)
        ix.desk(b, m, (*at[:2], z), yaw, 1.2, 0.6, 2)
    at, yaw = _wall(_deg(180), r, 0.5)
    ix.bags(b, m, (*at[:2], z), yaw, 1, 7)


def _exercise(b, m, r, z):
    """Tower B's upper deck by the stair: a treadmill and a resistive device behind a sweat partition."""
    at, _ = _wall(_deg(165), r, 1.2)
    ix.treadmill(b, m, (*at[:2], z), _deg(165) + math.pi / 2)
    at, _ = _wall(_deg(198), r, 1.7)
    ix.resistive(b, m, (*at[:2], z), _deg(198) + math.pi)
    ix.partition(b, m, ix.polar(_deg(181), r)[:2], ix.polar(_deg(181), r - 1.6)[:2], z, 1.8, t=0.04, mat="curtain")


def _control_upper(b, m, r, z, ceiling):
    """Tower C's upper deck, the command and control centre (Common Habitat
    C2, HDU's telerobotics workstation): a wall of displays, a row of
    consoles facing it, the fleet's telerobotics stations, a private comms
    booth, avionics racks."""
    for a in (160, 180, 200):
        at, yaw = ix.against(_deg(a), r, 0.06)
        ix.screen_wall(b, m, (*at[:2], z), yaw, 1.4, 0.9, 1.0)
    for a in (165, 195):
        at, yaw = _wall(_deg(a), r - 1.4, 0.6)
        ix.desk(b, m, (*at[:2], z), yaw, 1.3, 0.6, 3)
    for a in (35, 60):
        at, yaw = _wall(_deg(a), r, 0.6)
        ix.desk(b, m, (*at[:2], z), yaw, 1.3, 0.6, 3)
        b.put(moved(box((0.12, 0.12, 0.16), (0, 0, 0)), (*ix.polar(_deg(a), r - 0.85)[:2], z + 0.82), 0.0), m["dark"], smooth=False)
    for a, face in ((92, "avionics"), (108, "avionics"), (136, "stowage")):
        at, yaw = _wall(_deg(a), r, ix.RACK[1])
        ix.rack(b, m, (*at[:2], z), yaw, face)
    # The comms booth: a closed cabin with a seat and a screen.
    h = ceiling - z
    for e in (_deg(285), _deg(320)):
        ix.partition(b, m, ix.polar(e, 2.7)[:2], ix.polar(e, r)[:2], z, h)
    ix.partition(b, m, ix.polar(_deg(285), 2.7)[:2], ix.polar(_deg(320), 2.7)[:2], z, h, gaps=((0.6, 1.3),))
    at, yaw = _wall(_deg(302), r, 0.5)
    ix.desk(b, m, (*at[:2], z), yaw, 0.9, 0.5)
    for a in (230, 255):
        at, yaw = _wall(_deg(a), r, 0.5)
        ix.bags(b, m, (*at[:2], z), yaw, 2, 7)
    ix.table(b, m, (2.0, 0.0, z), 0.55, seats=4, start=math.pi / 4)


# Hubs


# A hub's deck stands this far over its drum's floor, a utility trench under the grating.
HUB_DECK = 1.0
# Half the angle a port's hatch frame takes up on a hub's lining.
HUB_PORT = math.radians(13)


def hub(b, m, f):
    """Fits a hub node (ISS Unity and Tranquility, Jenkins 2000): a grated
    deck over a utility trench, the lining, a pressure hatch at every port,
    rack bays between the ports where the wall leaves room, a caution panel
    and an extinguisher where it does not, a ring of lights, and a ladder up
    through the ceiling into the cupola. `f` holds radius, z0, top and the
    ports as (angle, axis height, bore)."""
    r = f["radius"] - LINING_IN
    z = f["z0"] + HUB_DECK
    c = z + CLEAR[0]
    well = (-0.5, 0.5, -0.5, 0.5)
    b.put(ix.floor(r + 0.05, f["z0"] + 0.05, 0.05), m["dark"], smooth=False)
    b.put(ix.floor(r + 0.05, z, 0.04), m["tread"], smooth=False)
    b.put(ix.floor(r + 0.05, c + 0.05, 0.05, [well]), m["ceiling"], smooth=False)
    b.collider(ix.floor(r, z, 0.1))
    b.put(ix.lining(r, f["z0"], c, 64), m["liner"], smooth=True)
    b.put(ix.skirt(r, z, segments=64), m["dark"], smooth=False)
    angles = sorted(a % TAU for a, _, _ in f["ports"])
    for a in angles:
        ix.hatch_door(b, m, ix.polar(a, r - 0.02, z), a + math.pi)
    span = ix.hull_span(r, ix.RACK[0] + 0.06, r - ix.RACK[1])
    faces = ("avionics", "stowage", "eclss", "stowage")
    k = 0
    for a0, a1 in zip(angles, angles[1:] + [angles[0] + TAU]):
        free0, free1 = a0 + HUB_PORT, a1 - HUB_PORT
        n = int((free1 - free0) // span)
        for i in range(n):
            at, yaw = ix.against(free0 + (free1 - free0) * (i + 0.5) / n, r, ix.RACK[1])
            ix.rack(b, m, (*at[:2], z), yaw, faces[k % len(faces)])
            k += 1
        if n == 0 and free1 - free0 > math.radians(9):
            _safety(b, m, (free0 + free1) / 2, r, z)
    ix.ceiling_panels(b, m, c, [ix.polar(TAU * j / 6 + 0.3, 1.9)[:2] for j in range(6)], size=(0.5, 0.5))
    _ladder(b, m, (0.35, 0.0), z, f["top"] + 0.5)


def _safety(b, m, a, r, z):
    """A caution and warning panel on the wall at angle `a`, a red extinguisher and a mask box under it."""
    at, yaw = ix.against(a, r, 0.06)
    b.put(moved(box((0.05, 0.5, 0.35), (0, 0, 1.5), bevel=0.01), (at[0], at[1], z), yaw), m["dark"], smooth=False)
    b.put(moved(box((0.01, 0.42, 0.25), (0.027, 0, 1.5)), (at[0], at[1], z), yaw), m["display"], smooth=False)
    at, yaw = ix.against(a, r, 0.2)
    b.put(moved(rod((0, 0, 0.6), (0, 0, 1.05), 0.08, 12), (at[0], at[1], z), yaw), m["paint_red"], smooth=False)
    b.put(moved(box((0.15, 0.3, 0.2), (0, 0, 0.3)), (at[0], at[1], z), yaw), m["paint_yellow"], smooth=False)


def _ladder(b, m, at, z0, z1, w=0.45):
    """A vertical ladder at (x, y) from `z0` to `z1`, its rungs across Y."""
    x, y = at
    parts = bmesh.new()
    for s in (-1, 1):
        merge(parts, box((0.04, 0.04, z1 - z0), (x, y + s * w / 2, (z0 + z1) / 2)))
    for k in range(int((z1 - z0) / 0.3)):
        merge(parts, rod((x, y - w / 2, z0 + 0.3 * (k + 1)), (x, y + w / 2, z0 + 0.3 * (k + 1)), 0.016, 6))
    b.put(parts, m["rail"], smooth=False)


# The commons dome


# The commons' inner liner's radius, as a share of the skin's.
LINER = 0.955


def _dome_z(f, r):
    """How high the dome's inner liner stands over radius `r`."""
    lat = math.acos(min(1.0, r / (f["radius"] * LINER)))
    return 0.5 + f["height"] * 0.97 * math.sin(lat) ** 0.85


def commons(b, m, f):
    """Fits the inflatable dome as the base's commons for all twelve
    (TransHab level 1, Common Habitat's galley and wardroom): a table for
    twelve under the crown, a galley run, a gym of treadmills, resistive
    devices and cycles before a screen, a lounge, a plant atrium, bag
    stowage and air handlers round the low edge, and pendant lights. `f`
    holds radius, height, the airlocks' angles and the floor height."""
    z = f["floor"]
    edge = f["radius"] - 0.35
    b.put(ix.floor(edge, z, 0.1), m["deck"], smooth=False)
    b.put(lathe([(f["radius"] * LINER * math.cos(lat), 0.5 + f["height"] * 0.97 * math.sin(lat) ** 0.85) for lat in [math.pi / 2 * (12 - k) / 12 for k in range(13)]], 72, 1.2), m["liner"], smooth=True)
    b.collider(ix.floor(edge, z, 0.1))
    for a in f["locks"]:
        ix.hatch_door(b, m, ix.polar(a, f["radius"] - 1.05, z), a + math.pi)
    ix.table(b, m, (0.0, 0.0, z), 1.5, seats=12, chairs=True)
    # The galley along the +Y wall.
    for a in (75, 90, 105):
        at, yaw = ix.against(_deg(a), 6.3, 0.6)
        ix.counter(b, m, (*at[:2], z), yaw, 1.6, sink=0.8 if a == 90 else None, lockers=False)
        ix.warmer(b, m, (*ix.polar(_deg(a - 4), 5.95)[:2], z + 0.9), _deg(a - 4) + math.pi)
    for a in (61, 119):
        ix.fridge(b, m, (*ix.polar(_deg(a), 5.95)[:2], z), _deg(a) + math.pi)
    for a in (55, 125):
        b.put(moved(rod((0, 0, 0), (0, 0, 0.7), 0.2, 16), (*ix.polar(_deg(a), 5.7)[:2], z), 0.0), m["paint_grey"], smooth=False)
    # The gym along the -Y wall, facing a screen.
    for a in (245, 295):
        ix.treadmill(b, m, (*ix.polar(_deg(a), 5.4)[:2], z), _deg(a) + math.pi / 2)
    for a in (227, 313):
        ix.resistive(b, m, (*ix.polar(_deg(a), 5.0)[:2], z), _deg(a) + math.pi)
    for a in (262, 270, 278):
        ix.cycle(b, m, (*ix.polar(_deg(a), 4.2)[:2], z), _deg(a))
    stand = Vector(ix.polar(_deg(270), 5.9))
    ix.screen_wall(b, m, (stand.x, stand.y, z), _deg(90), 2.0, 1.1, 0.9)
    b.put(moved(box((0.1, 0.1, 0.9), (0, 0, 0.45)), (stand.x, stand.y, z), 0.0), m["tool"], smooth=False)
    # The lounge by the -X airlock.
    for a in (150, 210):
        ix.sofa(b, m, (*ix.polar(_deg(a), 4.6)[:2], z), _deg(a) + math.pi, 2.2)
    b.put(moved(box((0.9, 0.6, 0.4), (0, 0, 0.2), bevel=0.02), (-3.7, 0.0, z), 0.0), m["locker"], smooth=False)
    for a in (165, 195):
        ix.shelf(b, m, (*ix.polar(_deg(a), 6.0)[:2], z), _deg(a) + math.pi, 1.0, 0.35, 1.6, 5)
    # The plant atrium by the +X airlock.
    for a, r_ in ((28, 4.6), (42, 4.6), (318, 4.6), (332, 4.6)):
        ix.plants_box(b, m, (*ix.polar(_deg(a), r_)[:2], z), _deg(a) + math.pi / 2, 1.2, 0.64, 0.61, 0.5)
    # Bags and air handlers round the low edge.
    for a in range(10, 360, 22):
        if min(abs((a - lock_deg + 180) % 360 - 180) for lock_deg in (0, 180)) < 20 or 50 < a < 130 or 220 < a < 320:
            continue
        ix.bags(b, m, (*ix.polar(_deg(a), edge - 0.4)[:2], z), _deg(a) + math.pi, 2, 3)
    # Pendant lights round the crown and over the galley and gym.
    lamps = bmesh.new()
    for a, r_ in [(TAU * k / 6, 2.6) for k in range(6)] + [(_deg(90), 4.8), (_deg(270), 4.6)]:
        x, y, _ = ix.polar(a, r_)
        top = _dome_z(f, r_) - 0.05
        merge(lamps, rod((x, y, top), (x, y, z + 2.9), 0.01, 4))
        b.put(moved(lathe([(0.0, 0.0), (0.35, 0.0), (0.3, 0.08), (0.0, 0.1)], 24, 0.5), (x, y, z + 2.8)), m["panel_lamp"], smooth=False)
        b.places.append(("light", {"at": (x, y, z + 2.7), "kind": "cabin"}))
    b.put(lamps, m["tool"], smooth=False)


# MARSHA


# MARSHA's levels: their floors' heights, the skyroom's the last.
MARSHA_LEVELS = (0.3, 3.6, 7.0, 10.4)
MARSHA_WELL = (-1.25, 1.25, -1.25, 1.25)


def _inside(f, z):
    """How far in from MARSHA's axis its printed shell's inner face stands at height `z`."""
    rows = f["rows"]
    for (r0, z0), (r1, z1) in zip(rows, rows[1:]):
        if z0 <= z <= z1:
            return r0 + (r1 - r0) * (z - z0) / max(z1 - z0, 1e-6) - f["thick"]
    return rows[-1][0] - f["thick"]


def marsha(b, m, f):
    """Fits MARSHA (AI SpaceFactory) floor by floor: the garage and EVA
    level with its wet lab and plant (water tanks, ECLSS), the kitchen and
    dry lab, the sleeping pods with a sanitation pod and a hydroponic wall,
    and the skyroom under the skylight; a spiral stair up the middle. `f`
    holds the shell's rows (radius, height), its thickness and height."""
    levels = MARSHA_LEVELS
    lining = [(_inside(f, z) - 0.05, z) for z in [f["height"] - 0.3 - 0.3 * k for k in range(int((f["height"] - 0.6) / 0.3) + 1)]]
    b.put(lathe(lining, 72, 1.2), m["liner"], smooth=True)
    for k, z in enumerate(levels):
        rr = min(_inside(f, z - 0.25), _inside(f, z)) - 0.03
        holes = [MARSHA_WELL] if k else []
        b.put(ix.floor(rr, z, 0.04, holes), m["deck"], smooth=False)
        b.collider(ix.floor(rr, z, 0.1, holes))
        if k:
            b.put(ix.floor(rr, z - 0.22, 0.02, holes), m["ceiling"], smooth=False)
            x0, x1, y0, y1 = MARSHA_WELL
            ix.railing(b, m, [(x0, y0 + 0.9), (x0, y1), (x1, y1), (x1, y0), (x0 + 0.9, y0)], z)
    for k, (z0, z1) in enumerate(zip(levels, levels[1:])):
        ix.spiral(b, m, (0.0, 0.0, 0.0), z0, z1, 1.05, TAU * 0.8, start=math.pi * k)
    for z in levels:
        for a in (0.6, 2.7, 4.8):
            x, y, _ = ix.polar(a, 2.0)
            b.places.append(("light", {"at": (x, y, z + 2.7), "kind": "cabin"}))
            b.put(moved(box((0.5, 0.5, 0.02), (0, 0, 0)), (x, y, z + 2.86 if z < levels[-1] else z + 2.6), a), m["panel_lamp"], smooth=False)
    _marsha_ground(b, m, f, levels[0])
    _marsha_kitchen(b, m, f, levels[1])
    _marsha_pods(b, m, f, levels[2], levels[3] - 0.22)
    _marsha_skyroom(b, m, f, levels[3])


def _marsha_ground(b, m, f, z):
    """The ground level: suits by the -X door, a dust grate, the entry
    airlock's hatch at +X, the wet lab with its hood, water tanks and racks."""
    r = _inside(f, z + 1.0)
    ix.hatch_door(b, m, ix.polar(0.0, r - 0.02, z + 0.1), math.pi)
    ix.hatch_door(b, m, ix.polar(math.pi, r - 0.03, z), 0.0, 1.1, 2.0, 0.0)
    b.put(box((1.6, 2.2, 0.03), (-r + 0.9, 0.0, z + 0.015), tile=0.6), m["grate"], smooth=False)
    for a in (150, 210):
        ix.suit(b, m, (*ix.polar(_deg(a), r - 0.55)[:2], z), _deg(a) + math.pi)
    ix.gas_wand(b, m, (*ix.against(_deg(165), r, 0.1)[0][:2], z), _deg(165) + math.pi)
    at, yaw = ix.against(_deg(80), r, 0.75)
    ix.bench(b, m, (*at[:2], z), yaw, 2.0, 0.75)
    ix.hood(b, m, (*ix.polar(_deg(80), r - 0.4)[:2], z), _deg(80) + math.pi, 1.6, 1.9)
    b.put(moved(box((0.36, 0.42, 0.02), (0, 0, 0.91)), (*ix.polar(_deg(72), r - 0.4)[:2], z), _deg(72)), m["stainless"], smooth=False)
    for a in (255, 290):
        industry.vessel(b, m, ix.polar(_deg(a), r - 0.7, z), 0.5, 1.6, "paint_white", 0.3)
    for a in (310, 325):
        at, yaw = ix.against(_deg(a), r, ix.RACK[1])
        ix.rack(b, m, (*at[:2], z), yaw, "eclss")


def _marsha_kitchen(b, m, f, z):
    """The kitchen and dry lab: a galley run with its sink, a fridge, a
    table for four, desks with screens and a printer."""
    r = _inside(f, z + 1.0)
    at, yaw = ix.against(_deg(55), r, 0.6)
    ix.counter(b, m, (*at[:2], z), yaw, 2.0, sink=0.6)
    ix.warmer(b, m, (*ix.polar(_deg(62), r - 0.35)[:2], z + 0.9), _deg(62) + math.pi)
    ix.fridge(b, m, (*ix.polar(_deg(28), r - 0.35)[:2], z), _deg(28) + math.pi)
    ix.table(b, m, (*ix.polar(_deg(120), 2.6)[:2], z), 0.55, seats=4, start=math.pi / 4, chairs=True)
    for a in (205, 235):
        at, yaw = ix.against(_deg(a), r, 0.6)
        ix.desk(b, m, (*at[:2], z), yaw, 1.3, 0.6, 2)
    at, yaw = ix.against(_deg(265), r, 0.6)
    ix.bench(b, m, (*at[:2], z), yaw, 1.3, 0.6, 0.9, "paint_grey", "stainless")
    ix.printer(b, m, (*ix.polar(_deg(265), r - 0.3)[:2], z), _deg(265) + math.pi)
    for a in (300, 325):
        ix.shelf(b, m, (*ix.against(_deg(a), r, 0.35)[0][:2], z), _deg(a) + math.pi, 1.0, 0.35, 1.9, 6)


def _marsha_pods(b, m, f, z, ceiling):
    """The sleeping level: four pods round the wall, a sanitation pod and a hydroponic wall between them."""
    h = ceiling - z
    front = 2.35
    for a in (45, 135, 225, 315):
        a = _deg(a)
        r = _inside(f, z + 1.2)
        for e in (a - _deg(24), a + _deg(24)):
            ix.partition(b, m, ix.polar(e, front)[:2], ix.polar(e, r + 0.2)[:2], z, h)
        p0, pm, p1 = (ix.polar(a + d, front)[:2] for d in (_deg(-24), 0.0, _deg(24)))
        ix.partition(b, m, p0, pm, z, h, gaps=((0.1, 0.75),))
        ix.partition(b, m, pm, p1, z, h)
        ix.bunk(b, m, (*ix.polar(a, _inside(f, z + 0.5) - 0.55)[:2], z), a, 1.9, 0.85)
        b.put(moved(box((0.03, 0.4, 1.9), (0, 0, 0.95), bevel=0.01), (*ix.polar(a - _deg(16), front)[:2], z), a), m["curtain"], smooth=False)
    r = _inside(f, z + 1.0)
    ix.hygiene(b, m, ix.polar(0.0, r - 0.85, z), math.pi, 0.95, 1.2, min(2.2, h))
    for a in (170, 190):
        ix.plants_box(b, m, (*ix.polar(_deg(a), r - 0.4)[:2], z), _deg(a) + math.pi, 1.1, 0.6, 0.55, 0.4)
        ix.plants_box(b, m, (*ix.polar(_deg(a), r - 0.4)[:2], z), _deg(a) + math.pi, 1.1, 0.6, 0.55, 1.2)
    for a in (85, 265):
        ix.bags(b, m, (*ix.against(_deg(a), r, 0.5)[0][:2], z), _deg(a) + math.pi, 2, 5)


def _marsha_skyroom(b, m, f, z):
    """The skyroom under the water-filled skylight: cycles, a sofa, mats and a morale garden."""
    for a in (30, 150):
        ix.cycle(b, m, (*ix.polar(_deg(a), 2.2)[:2], z), _deg(a) + math.pi)
    ix.sofa(b, m, (*ix.polar(_deg(270), 2.25)[:2], z), _deg(90), 1.8)
    for a in (75, 105):
        b.put(moved(box((1.8, 0.6, 0.01), (0, 0, 0.005)), (*ix.polar(_deg(a), 2.2)[:2], z), _deg(a) + math.pi / 2), m["mat"], smooth=False)
    for a in (210, 330):
        ix.plants_box(b, m, (*ix.polar(_deg(a), 2.2)[:2], z), _deg(a) + math.pi / 2, 1.0, 0.5, 0.5, 0.0)


# The rover hangar


def hangar(b, m, f):
    """Fits the rover hangar after the research's bay plan (SEV, DRA 5.0,
    Jenkins 2000, Common Habitat RMAF): two pressurised-rover bays on the
    door's axis, each with wheel chocks, a charging pedestal with its
    consumables panel and a platform at the rover's suitports; a workshop
    strip along the +Y wall (mill, lathe, welding booth, printers, a
    hooded repair bench and a glovebox with a component airlock); an
    implement and wheel corner; suits stowed by the rear airlock; and a
    grated dust station across the door. `f` holds length, radius, crown,
    the door's half width and the front bulkhead's x."""
    radius, front = f["radius"], f["front"]
    z = 0.15
    # The bays: markings, chocks, a pedestal, a consumables panel, a suitport platform.
    marks = bmesh.new()
    for x0, x1 in ((2.6, 10.4), (11.4, 19.0)):
        for y in (-2.2, 2.2):
            merge(marks, box((x1 - x0, 0.1, 0.005), ((x0 + x1) / 2, y, z + 0.006)))
        for x in (x0, x1):
            merge(marks, box((0.1, 4.4, 0.005), (x, 0, z + 0.006)))
        _bay(b, m, (x0 + x1) / 2, z)
    b.put(marks, m["paint_yellow"], smooth=False)
    # The workshop strip along +Y.
    y = radius - 1.7
    ix.mill(b, m, (7.0, y, z), -math.pi / 2)
    ix.lathe_tool(b, m, (8.8, y + 0.1, z), -math.pi / 2, 1.4)
    ix.welding_booth(b, m, (11.0, y, z), -math.pi / 2)
    for k, x in enumerate((13.2, 14.0)):
        ix.printer(b, m, (x, y + 0.1, z), -math.pi / 2, (0.7, 0.7, 0.9) if k else (0.6, 0.6, 1.4), on_floor=True)
    ix.bench(b, m, (15.8, y + 0.1, z), -math.pi / 2, 1.8, 0.75, 0.9, "paint_grey", "stainless")
    ix.hood(b, m, (15.8, y + 0.25, z), -math.pi / 2, 1.6, 2.0)
    ix.glovebox(b, m, (17.9, y + 0.1, z), -math.pi / 2, 1.0, 0.7, airlock=0.4)
    ix.tool_wall(b, m, (4.0, radius - 1.25, z), -math.pi / 2, 3.0, 1.0, 1.25)
    ix.shelf(b, m, (5.9, y + 0.15, z), -math.pi / 2, 0.9, 0.45, 2.2, 5)
    # The implement and wheel corner along -Y, between and past the stowage racks.
    _implements(b, m, (8.5, -(radius - 1.7), z), (1.6, -(radius - 1.9), z))
    _wheels(b, m, (16.0, -(radius - 1.6), z))
    # Suits stowed by the rear airlock.
    for s in (-1, 1):
        ix.suit(b, m, (0.9, s * 2.4, z), 0.0)
    ix.gas_wand(b, m, (0.25, 1.6, z), 0.0)
    # The dust station across the door: a grated floor over its pit, blower wands on posts.
    b.put(box((2.6, 7.6, 0.02), (front - 1.5, 0, z + 0.01), tile=0.6), m["grate"], smooth=False)
    b.put(box((2.5, 7.5, 0.01), (front - 1.5, 0, z - 0.004)), m["dark"], smooth=False)
    for s in (-1, 1):
        b.put(box((0.12, 0.12, 1.6), (front - 1.5, s * 4.1, z + 0.8)), m["paint_yellow"], smooth=False)
        ix.gas_wand(b, m, (front - 1.5, s * 4.05, z), -s * math.pi / 2)


def _bay(b, m, x, z):
    """A rover bay's fittings round its middle at `x`: chocks, a charging
    pedestal and its consumables panel (O2, N2, water, coolant), and a
    grated platform with steps at the height of the rover's suitports."""
    chocks = bmesh.new()
    for dx in (-1.9, 1.9):
        for s in (-1, 1):
            merge(chocks, box((0.3, 0.35, 0.18), (x + dx, s * 1.4, z + 0.09), bevel=0.03))
    b.put(chocks, m["paint_yellow"], smooth=False)
    ped = (x + 3.0, -2.7, z)
    b.put(moved(box((0.5, 0.4, 1.3), (0, 0, 0.65), bevel=0.03), ped, 0.0), m["paint_white"], smooth=False)
    b.put(moved(box((0.01, 0.3, 0.2), (0.255, 0, 1.05)), ped, 0.0), m["display"], smooth=False)
    b.put(moved(tube([Vector((0.2, 0.1, 0.9)), Vector((0.45, 0.4, 0.4)), Vector((0.3, 0.7, 0.05))], 0.035, 8), ped, 0.0), m["cable"], smooth=False)
    panel = (x + 3.0, -3.3, z)
    b.put(moved(box((0.2, 1.0, 1.0), (0, 0, 1.0)), panel, 0.0), m["paint_grey"], smooth=False)
    for k, key in enumerate(("paint_white", "paint_yellow", "pipe_blue", "paint_orange")):
        b.put(moved(rod((0.1, -0.36 + 0.24 * k, 1.2), (0.2, -0.36 + 0.24 * k, 1.2), 0.03, 10), panel, 0.0), m[key], smooth=False)
    deck = (x - 3.0, 2.9, z)
    b.put(moved(box((1.4, 1.2, 0.06), (0, 0, 1.2)), deck, 0.0), m["grate"], smooth=False)
    legs = bmesh.new()
    for dx in (-0.65, 0.65):
        for dy in (-0.55, 0.55):
            merge(legs, box((0.06, 0.06, 1.2), (dx, dy, 0.6)))
    b.put(moved(legs, deck, 0.0), m["paint_yellow"], smooth=False)
    ix.railing(b, m, [(x - 3.7, 2.3), (x - 3.7, 3.5), (x - 2.3, 3.5)], z + 1.23)
    for k in range(5):
        b.put(box((0.6, 0.25, 0.04), (x - 2.0 + 0.0, 2.3 - 0.25 * k, z + 1.2 - 0.24 * (k + 1))), m["grate"], smooth=False)
    b.collider(box((1.4, 1.2, 1.25), (x - 3.0, 2.9, z + 0.62)))


def _implements(b, m, at, spares):
    """Rover implements on their stands (the SEV's work packages, RASSOR's
    drums): two bucket drums on an A-frame, a dozer blade, a winch and a cable reel."""
    x, y, z = at
    frame = bmesh.new()
    for dx in (-0.7, 0.7):
        merge(frame, box((0.08, 0.6, 0.9), (x + dx, y, z + 0.45)))
    merge(frame, box((1.5, 0.08, 0.08), (x, y, z + 0.9)))
    b.put(frame, m["paint_grey"], smooth=False)
    for dx in (-0.35, 0.35):
        b.put(moved(lathe([(0.0, -0.3), (0.35, -0.3), (0.35, 0.3), (0.0, 0.3)], 20, 0.5), (x + dx, y, z + 1.3), roll=math.pi / 2), m["paint_orange"], smooth=False)
    b.put(moved(box((0.15, 2.0, 0.7), (0, 0, 0.35), bevel=0.02), (x + 1.1, y + 0.2, z), 0.1), m["paint_yellow"], smooth=False)
    sx, sy, sz = spares
    b.put(box((0.5, 0.5, 0.45), (sx + 0.5, sy, sz + 0.225), bevel=0.02), m["paint_dark"], smooth=False)
    b.put(moved(lathe([(0.0, -0.25), (0.45, -0.25), (0.45, -0.2), (0.15, -0.2), (0.15, 0.2), (0.45, 0.2), (0.45, 0.25), (0.0, 0.25)], 24, 0.5), (sx - 0.4, sy, sz + 0.5), roll=math.pi / 2), m["cable"], smooth=False)
    b.collider(box((3.0, 1.2, 1.7), (x, y, z + 0.85)))
    b.collider(box((2.0, 1.0, 1.0), (sx, sy, sz + 0.5)))


def _wheels(b, m, at, count=4):
    """Spare rover wheels (1 m, wire mesh tyres on alloy hubs) on a rack."""
    x, y, z = at
    for k in range(count):
        c = (x - 1.5 + k, y, z + 0.55)
        b.put(moved(lathe([(0.38, -0.15), (0.5, -0.15), (0.5, 0.15), (0.38, 0.15)], 28, 0.5), c, roll=math.pi / 2), m["dark"], smooth=False)
        b.put(moved(lathe([(0.0, -0.1), (0.38, -0.08), (0.38, 0.08), (0.0, 0.1)], 20, 0.5), c, roll=math.pi / 2), m["alu"], smooth=False)
    b.put(box((count + 0.2, 0.5, 0.05), (x, y, z + 0.03)), m["steel"], smooth=False)
    b.collider(box((count + 0.2, 0.5, 1.1), (x, y, z + 0.55)))
