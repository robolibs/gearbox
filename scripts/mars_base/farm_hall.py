"""The farm hall: where crops come in from the grow rooms and go out as food,
and where the farm's next crops are started. It is the vertical farm's
printed-regolith vault (its shell, lock, lights and radiator come from
`vertical`), shielded for the crew who work in it, and it closes the spine.
The rooms follow the process chains of EDEN ISS, Lunar Palace 1, BIOS-3 and
the DLR lunar greenhouse module, along +X from the lock:

    x  0.6 .. 3.0   entry: the tray wash station EDEN ISS lacked, lockers
       3.0 .. 9.6   post-harvest: the receiving table, a wash flume, a spin
                    dryer, a sorting belt under its camera hood; across the
                    aisle the packing station and its arm, and crates
       9.6 .. 12.6  the cold store at 2 to 4 °C, and the fruit store with its
                    ethylene scrubber
      12.6 .. 19.4  processing: thresher and winnower, rice huller, stone mill
                    and flour sifter; across the aisle the oil press, the soy
                    kettle and tofu press, the dehydrators
      19.4 .. 25.6  propagation: seed vault freezers, germination chambers,
                    the seeding bench under its needle-seeder gantry; across
                    the aisle the nursery rack of plug trays under LEDs
      25.6 .. 29.9  resource recovery: the biomass shredder and the solid-state
                    fermenter (Lunar Palace's straw loop); across the aisle the
                    mushroom room and the mealworm racks
Outside: on the -Y side the nutrient and water plant (stock tanks, dosing,
reverse osmosis and UV, the condensate tank), on the +Y side the spirulina
photobioreactor house (MELiSSA's compartment IVa).

Everything is in the hall's frame, as `vertical`'s: the -X end wall's outer
face at x = -0.1, the aisle on y = 0, z up from the ground.
"""
import math

import bmesh
from mathutils import Vector

from buildings import _drum, _uv
from industry import cabinet, drum, gauge, pipe, valve, vessel
from kit import Building, box, cone, disc, lathe, merge, moved, rod, tube
from vertical import FLOOR, X0, X1, _lights_and_fans, _mats, _outside, _shell, arch_y, arch_z

# The nursery rack: its tray levels' tops and the LED bars' height over them.
NURSERY_X = (19.9, 25.3)
NURSERY_Y = 2.35
NURSERY_TRAY = (1.25, 0.8)
NURSERY_LEVELS = (FLOOR + 0.55, FLOOR + 1.15, FLOOR + 1.75)
NURSERY_LAMP = 0.42
# Outside: the nutrient plant's pad and the photobioreactor house, (x0, x1, y0, y1).
NUTRIENT_PAD = (2.0, 13.0, -10.2, -6.4)
ALGAE_HOUSE = (3.0, 13.5, 6.6, 9.8)
# Grain and seed the processing line threshes and mills a sol, kg: several
# times what a crew of twelve eats of them.
PROCESSING_KG = 40.0
# The nursery robot's rail: along the rack's aisle edge, its underside at
# RAIL_UNDER, from RAIL_X0 along +X.
RAIL_Y = NURSERY_Y - 0.75
RAIL_UNDER = FLOOR + 3.1
RAIL_X0 = NURSERY_X[0] - 0.3


def nursery_rail():
    """The nursery robot's sizes: a rail robot like EDEN ISS's, hung from the
    vault, its mast dropping to each of the rack's levels and its arm
    reaching in across a tray between the tray and its lamp."""
    import robots
    return robots.Rail(rail_len=NURSERY_X[1] - NURSERY_X[0] + 0.6, mast_len=0.9, mast_z=-0.58, turret_z=-1.04, lift=1.3,
                       arm=(0.0, -0.20, -1.12), beam=0.5, head=-0.215, reach=1.2, draw_rail=True,
                       hangers=arch_z(abs(RAIL_Y)) - RAIL_UNDER - 0.2)


def _bench(b, m, x0, x1, y0, y1, top=0.9, skin="worktop"):
    """A work bench: its top, a cabinet base, and a collider."""
    xm, ym = (x0 + x1) / 2, (y0 + y1) / 2
    b.put(box((x1 - x0, y1 - y0, 0.04), (xm, ym, FLOOR + top), tile=1.0), m[skin], smooth=False)
    b.put(box((x1 - x0 - 0.04, y1 - y0 - 0.06, top - 0.12), (xm, ym, FLOOR + 0.06 + (top - 0.12) / 2), tile=1.0), m["paint_white"], smooth=False)
    b.collider(box((x1 - x0, y1 - y0, top), (xm, ym, FLOOR + top / 2)))


def _crates(b, m, at, rows=2, cols=2, high=3):
    """Stacked harvest crates, green and grey, on a dolly."""
    crates = bmesh.new()
    x, y = at
    for i in range(cols):
        for j in range(rows):
            for k in range(high - (i + j) % 2):
                merge(crates, box((0.58, 0.38, 0.22), (x + i * 0.6, y + j * 0.4, FLOOR + 0.13 + 0.235 * k), tile=0.5, bevel=0.01))
    b.put(crates, m["hull_grey"] if (rows + cols) % 2 else m["paint_grey"], smooth=False)
    b.put(box((cols * 0.6, rows * 0.4, 0.04), (x + (cols - 1) * 0.3, y + (rows - 1) * 0.2, FLOOR + 0.03)), m["steel"], smooth=False)


def _hood(b, m, x0, x1, y, z):
    """A camera and strobe hood over a belt, on two posts."""
    b.put(box((x1 - x0, 0.7, 0.35), ((x0 + x1) / 2, y, z + 0.18), tile=0.5, bevel=0.01), m["paint_grey"], smooth=False)
    b.put(box((x1 - x0 - 0.1, 0.6, 0.01), ((x0 + x1) / 2, y, z - 0.002)), m["lamp"], smooth=False)
    for x in (x0 + 0.05, x1 - 0.05):
        b.put(box((0.05, 0.05, z - FLOOR), (x, y + 0.38, (z + FLOOR) / 2)), m["steel"], smooth=False)


def _belt(b, m, x0, x1, y, top=0.9, width=0.5):
    """A belt conveyor along X: its belt, side frames, rollers and legs."""
    xm = (x0 + x1) / 2
    b.put(box((x1 - x0, width, 0.02), (xm, y, FLOOR + top), tile=1.0), m["rubber"], smooth=False)
    frame = bmesh.new()
    for s in (-1, 1):
        merge(frame, box((x1 - x0, 0.03, 0.12), (xm, y + s * (width / 2 + 0.02), FLOOR + top - 0.04)))
    for x in (x0 + 0.1, xm, x1 - 0.1):
        for s in (-1, 1):
            merge(frame, box((0.04, 0.04, top - 0.1), (x, y + s * width / 2, FLOOR + (top - 0.1) / 2)))
    b.put(frame, m["steel"], smooth=False)
    for x in (x0 + 0.03, x1 - 0.03):
        b.put(rod((x, y - width / 2, FLOOR + top - 0.04), (x, y + width / 2, FLOOR + top - 0.04), 0.045, 12), m["alu"])
    b.collider(box((x1 - x0, width + 0.06, top), (xm, y, FLOOR + top / 2)))


def _arm(b, m, at, yaw):
    """A packing arm on its pedestal, folded over the table: base, upper and
    fore arm, and a vacuum gripper (a cobot like the harvesters' UR10e)."""
    x, y = at
    b.put(moved(lathe([(0.0, 0.0), (0.16, 0.0), (0.16, 0.08), (0.09, 0.12), (0.09, 0.78), (0.0, 0.8)], 24, 1.0), (x, y, FLOOR)), m["paint_grey"])
    c, s = math.cos(yaw), math.sin(yaw)
    shoulder = Vector((x, y, FLOOR + 0.92))
    elbow = shoulder + Vector((c * 0.42, s * 0.42, 0.38))
    wrist = elbow + Vector((c * 0.45, s * 0.45, -0.3))
    b.put(moved(lathe([(0.0, -0.1), (0.075, -0.1), (0.075, 0.1), (0.0, 0.1)], 20, 1.0), tuple(shoulder)), m["hull"])
    b.put(tube([shoulder, elbow], 0.055, 16), m["hull"])
    b.put(tube([elbow, wrist], 0.045, 16), m["hull"])
    for joint in (elbow, wrist):
        b.put(moved(lathe([(0.0, -0.07), (0.065, -0.07), (0.065, 0.07), (0.0, 0.07)], 18, 1.0), tuple(joint), 0, math.pi / 2), m["paint_dark"])
    b.put(rod(wrist, wrist - Vector((0, 0, 0.18)), 0.03, 12), m["paint_dark"])
    b.put(cone(0.06, 0.04, 0.05, tuple(wrist - Vector((0, 0, 0.23))), 16), m["rubber"])


def _post_harvest(b, m):
    """Receiving, washing, drying and sorting along the -Y wall; packing and
    crates across the aisle."""
    _bench(b, m, 3.2, 4.4, -2.9, -1.7, skin="stainless")
    _crates(b, m, (3.4, -2.75), 2, 2, 1)
    # The wash flume: a stainless tank, its water, an inclined basket belt out of it.
    fx0, fx1, fy = 4.6, 6.8, -2.35
    b.put(box((fx1 - fx0, 0.8, 0.75), ((fx0 + fx1) / 2, fy, FLOOR + 0.5), tile=1.0, bevel=0.01), m["stainless"], smooth=False)
    b.put(box((fx1 - fx0 - 0.1, 0.7, 0.02), ((fx0 + fx1) / 2, fy, FLOOR + 0.82)), m["glass"], smooth=False)
    for x in (fx0 + 0.3, fx1 - 0.3):
        b.put(box((0.06, 0.06, 0.5), (x, fy + 0.35, FLOOR + 0.12 + 0.25)), m["steel"], smooth=False)
    b.put(tube([(fx0 + 0.2, fy - 0.45, FLOOR + 1.1), (fx1 - 0.2, fy - 0.45, FLOOR + 1.1)], 0.02, 8), m["steel"])
    b.collider(box((fx1 - fx0, 0.8, 0.9), ((fx0 + fx1) / 2, fy, FLOOR + 0.45)))
    # The spin dryer: a drum on a frame with a hinged lid.
    b.put(moved(lathe([(0.0, 0.0), (0.36, 0.0), (0.38, 0.1), (0.38, 0.82), (0.34, 0.9), (0.0, 0.9)], 28, 1.0), (7.2, -2.4, FLOOR)), m["stainless"])
    b.put(moved(disc(0.33, 0.0, 28, 1.0), (7.2, -2.4, FLOOR + 0.91)), m["paint_dark"])
    b.collider(box((0.8, 0.8, 0.9), (7.2, -2.4, FLOOR + 0.45)))
    # The sorting belt under the vision hood, crates of sorted produce at its end.
    _belt(b, m, 7.7, 9.5, -2.4)
    _hood(b, m, 8.2, 8.9, -2.4, FLOOR + 1.45)
    # Packing: the table, the arm over it, crates waiting.
    _bench(b, m, 5.6, 8.6, 1.8, 2.8, skin="stainless")
    _arm(b, m, (7.1, 2.3), -2.2)
    for x in (5.9, 6.3, 7.9, 8.3):
        b.put(box((0.34, 0.26, 0.12), (x, 2.05, FLOOR + 0.98), tile=0.3, bevel=0.005), m["paint_white"], smooth=False)
    _crates(b, m, (3.3, 2.0), 2, 3, 3)


def _cold_store(b, m):
    """The walk-in cold store against the -Y wall, its refrigeration unit on
    top; across the aisle the fruit store with its ethylene scrubber."""
    for (y0, y1, h, door) in ((-3.0, -0.9, 2.3, -0.9), (1.2, 3.0, 2.1, 1.2)):
        x0, x1 = 9.8, 12.4
        xm, ym = (x0 + x1) / 2, (y0 + y1) / 2
        b.put(box((x1 - x0, y1 - y0, h), (xm, ym, FLOOR + h / 2), tile=1.0, bevel=0.02), m["paint_white"], smooth=False)
        seams = bmesh.new()
        for k in range(1, 4):
            merge(seams, box((0.02, y1 - y0 + 0.01, h + 0.01), (x0 + (x1 - x0) * k / 4, ym, FLOOR + h / 2)))
        b.put(seams, m["steel"], smooth=False)
        b.put(box((0.9, 0.06, 2.0 if h > 2.2 else 1.9), (xm, door, FLOOR + 1.0), tile=0.5, bevel=0.01), m["stainless"], smooth=False)
        b.put(box((0.3, 0.08, 0.04), (xm + 0.3, door + (0.04 if door > 0 else -0.04), FLOOR + 1.0)), m["paint_dark"], smooth=False)
        b.collider(box((x1 - x0, y1 - y0, h), (xm, ym, FLOOR + h / 2)))
    # The cold store's condensing unit and its fan.
    b.put(box((1.2, 0.8, 0.5), (11.1, -2.0, FLOOR + 2.55), tile=0.5, bevel=0.02), m["paint_grey"], smooth=False)
    b.put(moved(disc(0.25, 0.0, 20, 0.5), (11.4, -2.0, FLOOR + 2.81)), m["grate"])
    # The fruit store's scrubber cabinet.
    b.put(box((0.5, 0.4, 0.8), (12.15, 2.0, FLOOR + 2.5), tile=0.5, bevel=0.01), m["paint_grey"], smooth=False)
    b.put(box((0.3, 0.02, 0.08), (12.15, 1.79, FLOOR + 2.6)), m["andon"], smooth=False)


def _processing(b, m):
    """Grain and seed lines along the -Y wall; oil, soy and drying across the aisle."""
    # Thresher and winnower: a body with a feed hopper, the fan's drum, grain and chaff spouts.
    tx0, tx1, ty = 12.9, 14.9, -2.3
    b.put(box((tx1 - tx0, 1.0, 1.3), ((tx0 + tx1) / 2, ty, FLOOR + 0.65), tile=1.0, bevel=0.02), m["paint_orange"], smooth=False)
    b.put(moved(cone(0.12, 0.5, 0.5, (0, 0, 0), 4), (tx0 + 0.5, ty, FLOOR + 1.3), math.pi / 4), m["steel"])
    fan = bmesh.new()
    _drum(fan, (tx1 - 0.35, ty, FLOOR + 1.5), (0, 1, 0), 0.32, 0.5, 18)
    b.put(_uv(fan, 0.5), m["paint_grey"])
    b.put(tube([(tx1 - 0.2, ty + 0.5, FLOOR + 0.6), (tx1 - 0.2, ty + 0.75, FLOOR + 0.35)], 0.07, 10), m["steel"])
    b.put(box((0.45, 0.35, 0.3), (tx1 - 0.2, ty + 0.75, FLOOR + 0.15), tile=0.4), m["tray"], smooth=False)
    b.collider(box((tx1 - tx0, 1.0, 1.8), ((tx0 + tx1) / 2, ty, FLOOR + 0.9)))
    # Rice huller: rubber-roll husker in a small housing with its hopper.
    b.put(box((0.7, 0.7, 1.0), (15.6, -2.45, FLOOR + 0.5), tile=0.5, bevel=0.02), m["paint_yellow"], smooth=False)
    b.put(moved(cone(0.08, 0.3, 0.35, (0, 0, 0), 4), (15.6, -2.45, FLOOR + 1.0), math.pi / 4), m["steel"])
    b.collider(box((0.7, 0.7, 1.35), (15.6, -2.45, FLOOR + 0.68)))
    # Stone mill: an octagonal tun over the stones, a hopper, the flour spout.
    b.put(moved(lathe([(0.0, 0.0), (0.45, 0.0), (0.45, 0.75), (0.38, 0.85), (0.0, 0.85)], 8, 1.0, start=math.pi / 8), (16.9, -2.4, FLOOR)),
          m["paint_white"], smooth=False)
    b.put(moved(lathe([(0.32, 0.85), (0.36, 0.88), (0.36, 0.98), (0.0, 1.0)], 24, 1.0), (16.9, -2.4, FLOOR)), m["stainless"])
    b.put(moved(cone(0.06, 0.25, 0.4, (0, 0, 0), 4), (16.9, -2.4, FLOOR + 1.0), math.pi / 4), m["steel"])
    b.collider(box((0.9, 0.9, 1.4), (16.9, -2.4, FLOOR + 0.7)))
    # Flour sifter: a box of stacked sieves on springs, its bins under.
    b.put(box((1.3, 0.8, 0.5), (18.4, -2.4, FLOOR + 1.0), tile=0.5, bevel=0.02), m["stainless"], smooth=False)
    for x in (17.85, 18.95):
        for y in (-2.7, -2.1):
            b.put(rod((x, y, FLOOR + 0.3), (x, y, FLOOR + 0.75), 0.03, 8), m["paint_dark"])
    for x in (18.0, 18.8):
        b.put(box((0.6, 0.6, 0.4), (x, -2.4, FLOOR + 0.22), tile=0.4, bevel=0.01), m["hull_grey"], smooth=False)
    b.collider(box((1.3, 0.8, 1.25), (18.4, -2.4, FLOOR + 0.63)))
    # Oil press: an expeller barrel on its stand, a drive at one end, an oil pan.
    b.put(box((1.6, 0.6, 0.6), (13.8, 2.3, FLOOR + 0.3), tile=0.5, bevel=0.02), m["paint_grey"], smooth=False)
    b.put(moved(lathe([(0.0, 0.0), (0.14, 0.0), (0.14, 1.1), (0.0, 1.1)], 20, 1.0), (13.25, 2.3, FLOOR + 0.82), 0.0, math.pi / 2), m["steel"])
    b.put(moved(cone(0.08, 0.28, 0.3, (0, 0, 0), 4), (13.2, 2.3, FLOOR + 0.95), math.pi / 4), m["steel"])
    b.put(box((0.35, 0.35, 0.45), (14.5, 2.3, FLOOR + 0.82), tile=0.3, bevel=0.01), m["paint_dark"], smooth=False)
    b.put(box((0.9, 0.35, 0.06), (13.8, 2.3, FLOOR + 0.63)), m["stainless"], smooth=False)
    b.collider(box((1.6, 0.6, 1.1), (13.8, 2.3, FLOOR + 0.55)))
    # Soy: the kettle, the curd press beside it.
    vessel(b, m, (15.6, 2.35, FLOOR), 0.38, 0.8, "stainless", 0.35, 3)
    b.collider(box((0.8, 0.8, 1.6), (15.6, 2.35, FLOOR + 0.8)))
    b.put(box((0.6, 0.5, 0.06), (16.6, 2.3, FLOOR + 0.85)), m["stainless"], smooth=False)
    frame = bmesh.new()
    for x in (16.35, 16.85):
        merge(frame, box((0.05, 0.05, 1.5), (x, 2.3, FLOOR + 0.75)))
    merge(frame, box((0.6, 0.06, 0.06), (16.6, 2.3, FLOOR + 1.48)))
    b.put(frame, m["steel"], smooth=False)
    b.put(rod((16.6, 2.3, FLOOR + 1.45), (16.6, 2.3, FLOOR + 1.05), 0.025, 8), m["paint_dark"])
    b.put(box((0.4, 0.3, 0.12), (16.6, 2.3, FLOOR + 0.98)), m["paint_white"], smooth=False)
    # Dehydrators: cabinets of racked trays with grilled doors.
    for x in (17.9, 18.8):
        b.put(box((0.8, 0.7, 1.8), (x, 2.45, FLOOR + 0.9), tile=0.5, bevel=0.02), m["paint_white"], smooth=False)
        b.put(box((0.7, 0.02, 1.5), (x, 2.09, FLOOR + 1.0)), m["grate"], smooth=False)
    b.collider(box((1.7, 0.7, 1.8), (18.35, 2.45, FLOOR + 0.9)))


def _propagation(b, m):
    """Seed vault, germination chambers and the seeding bench along the -Y
    wall; the nursery rack across the aisle, its trays the gearbox layer's."""
    # Seed vault: two upright freezers at -18 °C.
    for x in (19.9, 20.6):
        b.put(box((0.65, 0.7, 1.9), (x, -2.6, FLOOR + 0.95), tile=0.5, bevel=0.02), m["paint_white"], smooth=False)
        b.put(box((0.6, 0.02, 1.7), (x, -2.24, FLOOR + 1.0)), m["glass"], smooth=False)
        b.put(box((0.04, 0.04, 0.4), (x + 0.24, -2.22, FLOOR + 1.1)), m["paint_dark"], smooth=False)
    b.collider(box((1.4, 0.7, 1.9), (20.25, -2.6, FLOOR + 0.95)))
    # Germination chambers: dark, warm and humid, seedlings behind glass under a dim pink light.
    for x in (21.5, 22.4):
        b.put(box((0.8, 0.75, 2.0), (x, -2.55, FLOOR + 1.0), tile=0.5, bevel=0.02), m["paint_grey"], smooth=False)
        b.put(box((0.68, 0.02, 1.7), (x, -2.17, FLOOR + 1.05)), m["glass"], smooth=False)
        for z in (0.5, 1.0, 1.5):
            b.put(box((0.66, 0.6, 0.02), (x, -2.55, FLOOR + z)), m["tray"], smooth=False)
            b.put(box((0.6, 0.02, 0.02), (x, -2.25, FLOOR + z + 0.4)), m["grow_lamp"], smooth=False)
    b.collider(box((1.7, 0.75, 2.0), (21.95, -2.55, FLOOR + 1.0)))
    # The seeding bench: plug trays under a needle-seeder gantry on rails along the bench.
    sx0, sx1, sy0, sy1 = 23.0, 25.4, -2.95, -1.85
    _bench(b, m, sx0, sx1, sy0, sy1)
    for k in range(4):
        b.put(box((0.5, 0.27, 0.05), (sx0 + 0.35 + 0.55 * k, (sy0 + sy1) / 2, FLOOR + 0.945), tile=0.3), m["tray"], smooth=False)
    gantry = bmesh.new()
    gx = sx0 + 1.4
    for y in (sy0 + 0.05, sy1 - 0.05):
        merge(gantry, box((sx1 - sx0, 0.04, 0.04), ((sx0 + sx1) / 2, y, FLOOR + 0.96)))
        merge(gantry, box((0.06, 0.06, 0.6), (gx, y, FLOOR + 1.22)))
    merge(gantry, box((0.08, sy1 - sy0, 0.08), (gx, (sy0 + sy1) / 2, FLOOR + 1.52)))
    b.put(gantry, m["alu"], smooth=False)
    b.put(box((0.14, 0.12, 0.2), (gx, -2.3, FLOOR + 1.38), tile=0.3, bevel=0.01), m["paint_dark"], smooth=False)
    b.put(rod((gx, -2.3, FLOOR + 1.28), (gx, -2.3, FLOOR + 1.05), 0.008, 6), m["steel"])
    b.put(box((0.3, 0.25, 0.25), (sx1 - 0.25, (sy0 + sy1) / 2, FLOOR + 1.06), tile=0.3, bevel=0.01), m["paint_white"], smooth=False)
    # The nursery rack: three levels of ebb-and-flow trays, each under its LED bars (the gearbox layer's lamps).
    x0, x1 = NURSERY_X
    posts = bmesh.new()
    for x in (x0, (x0 + x1) / 2, x1):
        for dy in (-0.45, 0.45):
            merge(posts, box((0.05, 0.05, NURSERY_LEVELS[-1] + NURSERY_LAMP + 0.15 - FLOOR), (x, NURSERY_Y + dy, (NURSERY_LEVELS[-1] + NURSERY_LAMP + 0.15 + FLOOR) / 2)))
    for z in NURSERY_LEVELS:
        merge(posts, box((x1 - x0, 0.9, 0.03), ((x0 + x1) / 2, NURSERY_Y, z - 0.1)))
    b.put(posts, m["steel"], smooth=False)
    trays = int(round((x1 - x0) / (NURSERY_TRAY[0] + 0.08)))
    for z in NURSERY_LEVELS:
        for k in range(trays):
            x = x0 + (k + 0.5) * (x1 - x0) / trays
            b.places.append(("tray", {"at": (x, NURSERY_Y, z), "yaw": 0.0, "size": NURSERY_TRAY, "lamp": NURSERY_LAMP, "nursery": True}))
    b.collider(box((x1 - x0, 0.95, NURSERY_LEVELS[0] - FLOOR - 0.08), ((x0 + x1) / 2, NURSERY_Y, (NURSERY_LEVELS[0] + FLOOR - 0.08) / 2)))


def _recovery(b, m):
    """The shredder and the solid-state fermenter along the -Y wall; the
    mushroom room and the mealworm racks across the aisle."""
    # Biomass shredder: a hopper over a twin-shaft cutter box, its motor, a bin under its chute.
    b.put(box((0.9, 0.8, 0.8), (26.4, -2.4, FLOOR + 0.55), tile=0.5, bevel=0.02), m["paint_yellow"], smooth=False)
    b.put(moved(cone(0.3, 0.6, 0.55, (0, 0, 0), 4), (26.4, -2.4, FLOOR + 0.95), math.pi / 4), m["steel"])
    b.put(box((0.35, 0.3, 0.3), (26.4, -1.85, FLOOR + 0.6), tile=0.3, bevel=0.01), m["paint_dark"], smooth=False)
    b.put(box((0.7, 0.6, 0.15), (26.4, -2.4, FLOOR + 0.08)), m["tray"], smooth=False)
    b.collider(box((1.2, 1.2, 1.5), (26.4, -2.4, FLOOR + 0.75)))
    # The solid-state fermenter: an insulated drum on trunnions, fed straw and waste, giving up CO2 to the plants.
    drum(b, m, (27.3, -2.4, FLOOR), 0.0, 2.3, 0.55, "paint_white", (0.15, 0.85), 0.3, 0.6)
    b.put(tube([(28.4, -2.4, FLOOR + 1.45), (28.4, -2.4, FLOOR + 2.4), (28.4, -arch_y(FLOOR + 2.4) + 0.1, FLOOR + 2.4)], 0.04, 8), m["pipe_yellow"])
    gauge(b, m, (27.6, -1.84, FLOOR + 1.0), (0, 0, 1), (0, 1, 0))
    b.collider(box((2.3, 1.2, 1.5), (28.45, -2.4, FLOOR + 0.75)))
    # The mushroom room: a humid box of shelves of substrate columns, oyster mushrooms on their sides.
    mx0, mx1, my0, my1, h = 25.9, 27.9, 1.3, 3.0, 2.2
    b.put(box((mx1 - mx0, my1 - my0, h), ((mx0 + mx1) / 2, (my0 + my1) / 2, FLOOR + h / 2), tile=1.0, bevel=0.02), m["paint_white"], smooth=False)
    b.put(box((1.3, 0.02, 1.0), ((mx0 + mx1) / 2 + 0.2, my0 - 0.005, FLOOR + 1.35)), m["glass"], smooth=False)
    b.put(box((0.8, 0.06, 2.0), (mx0 + 0.45, my0, FLOOR + 1.0), tile=0.5, bevel=0.01), m["stainless"], smooth=False)
    shelves, bags, caps = bmesh.new(), bmesh.new(), bmesh.new()
    for z in (0.6, 1.1, 1.6):
        merge(shelves, box((1.4, 0.5, 0.02), ((mx0 + mx1) / 2 + 0.2, my0 + 0.35, FLOOR + z)))
        for k in range(6):
            x = mx0 + 0.75 + 0.22 * k
            merge(bags, moved(lathe([(0.0, 0.0), (0.07, 0.0), (0.075, 0.34), (0.0, 0.36)], 10, 0.5), (x, my0 + 0.35, FLOOR + z + 0.01)))
            for c in range(3):
                a = k * 1.7 + c * 2.1
                merge(caps, moved(lathe([(0.0, 0.0), (0.035, 0.0), (0.03, 0.012), (0.0, 0.016)], 8, 0.5),
                                  (x + 0.08 * math.cos(a), my0 + 0.35 + 0.08 * math.sin(a), FLOOR + z + 0.08 + 0.1 * c)))
    b.put(shelves, m["steel"], smooth=False)
    b.put(bags, m["paint_white"])
    b.put(caps, m["foam"])
    b.collider(box((mx1 - mx0, my1 - my0, h), ((mx0 + mx1) / 2, (my0 + my1) / 2, FLOOR + h / 2)))
    # Mealworm racks: stacks of shallow drawers, frass below, on castors.
    for x in (28.4, 29.2):
        b.put(box((0.7, 0.6, 1.7), (x, 2.5, FLOOR + 0.9), tile=0.5, bevel=0.01), m["paint_grey"], smooth=False)
        drawers = bmesh.new()
        for k in range(10):
            merge(drawers, box((0.66, 0.02, 0.1), (x, 2.19, FLOOR + 0.2 + 0.15 * k)))
        b.put(drawers, m["paint_white"], smooth=False)
    b.collider(box((1.5, 0.6, 1.8), (28.8, 2.5, FLOOR + 0.9)))


def _entry(b, m):
    """The tray wash station inside the hatch and the crew's lockers."""
    _bench(b, m, 1.0, 2.8, 1.65, 2.6, skin="stainless")
    for x in (1.45, 2.25):
        b.put(box((0.6, 0.6, 0.25), (x, 2.12, FLOOR + 0.8), tile=0.4), m["glass"], smooth=False)
    b.put(tube([(1.85, 2.55, FLOOR + 0.92), (1.85, 2.55, FLOOR + 1.35), (1.85, 2.3, FLOOR + 1.35)], 0.015, 6), m["steel"])
    b.put(box((1.6, 0.25, 0.9), (1.9, 2.6, FLOOR + 1.6), tile=0.4), m["grate"], smooth=False)
    for k in range(4):
        cabinet(b, m, (1.0 + 0.45 * k, -2.75, FLOOR), math.pi / 2, (0.45, 0.42, 1.9), "paint_grey", 1, k % 3)
    b.collider(box((1.8, 0.5, 1.9), (1.68, -2.75, FLOOR + 0.95)))


def _nutrient_plant(b, m):
    """Outside on the -Y side: the stock solution tanks, the dosing skid, the
    reverse-osmosis and UV skids and the condensate tank, on a sintered pad,
    their mains into the hall."""
    x0, x1, y0, y1 = NUTRIENT_PAD
    b.put(box((x1 - x0, y1 - y0, 0.15), ((x0 + x1) / 2, (y0 + y1) / 2, 0.06), tile=2.0, bevel=0.03), m["sinter"], smooth=False)
    for k, skin in enumerate(("paint_white", "paint_white", "paint_blue" if "paint_blue" in m else "hull_grey")):
        vessel(b, m, (x0 + 1.0 + 1.5 * k, (y0 + y1) / 2, 0.13), 0.6, 1.4, skin, 0.4, 4)
        b.collider(box((1.3, 1.3, 2.6), (x0 + 1.0 + 1.5 * k, (y0 + y1) / 2, 1.3)))
    # The dosing skid: pumps, a control panel and its lines.
    sx = x0 + 5.3
    b.put(box((1.6, 1.0, 0.1), (sx, (y0 + y1) / 2, 0.2)), m["frame"], smooth=False)
    for dx in (-0.5, 0.0, 0.5):
        b.put(moved(lathe([(0.0, 0.0), (0.11, 0.0), (0.11, 0.3), (0.0, 0.32)], 16, 0.5), (sx + dx, (y0 + y1) / 2 - 0.15, 0.25)), m["paint_grey"])
    b.put(box((0.6, 0.25, 0.9), (sx, (y0 + y1) / 2 + 0.35, 0.7), tile=0.4, bevel=0.01), m["paint_grey"], smooth=False)
    b.collider(box((1.6, 1.0, 1.2), (sx, (y0 + y1) / 2, 0.7)))
    # Reverse osmosis: membrane housings racked in a frame; the UV reactor beside it.
    rx = x0 + 7.6
    rack = bmesh.new()
    for dx in (-0.8, 0.8):
        for dy in (-0.45, 0.45):
            merge(rack, box((0.05, 0.05, 1.5), (rx + dx, (y0 + y1) / 2 + dy, 0.9)))
    b.put(rack, m["steel"], smooth=False)
    for z in (0.6, 0.95, 1.3):
        for dy in (-0.25, 0.25):
            b.put(moved(lathe([(0.0, 0.0), (0.06, 0.0), (0.06, 1.6), (0.0, 1.6)], 14, 0.5), (rx - 0.8, (y0 + y1) / 2 + dy, z), 0.0, math.pi / 2),
                  m["paint_white"])
    b.put(moved(lathe([(0.0, 0.0), (0.1, 0.0), (0.1, 1.1), (0.0, 1.1)], 16, 0.5), (rx + 1.4, (y0 + y1) / 2, 0.2)), m["stainless"])
    b.collider(box((2.2, 1.0, 1.6), (rx + 0.3, (y0 + y1) / 2, 0.95)))
    # The condensate tank, filled from every grow room's cooling coils.
    drum(b, m, (x1 - 2.4, (y0 + y1) / 2, 0.13), 0.0, 2.0, 0.55, "paint_white", (0.2, 0.8), 0.3, 1.0)
    b.collider(box((2.0, 1.2, 1.5), (x1 - 1.4, (y0 + y1) / 2, 0.85)))
    # Mains into the hall's -Y wall, and a valve on each.
    wall = -(arch_y(FLOOR + 1.0) + 0.95)
    for k, (dx, skin) in enumerate(((1.0, "pipe_blue"), (2.5, "pipe_white"), (5.3, "pipe_blue"), (8.6, "pipe_white"))):
        x = x0 + dx
        b.put(pipe([(x, (y0 + y1) / 2 + 0.6, 0.9), (x, y1 + 0.2, 0.9), (x, wall, 0.9)], 0.04), m[skin])
        valve(b, m, (x, y1 + 0.5, 0.9), (0, 1, 0), 0.05)


def _algae_house(b, m):
    """Outside on the +Y side: a glazed house of tubular photobioreactors
    growing spirulina, after MELiSSA's compartment IVa: rows of upright glass
    tubes of green culture before LED panels, airlifted from manifolds."""
    x0, x1, y0, y1 = ALGAE_HOUSE
    h = 2.8
    b.put(box((x1 - x0, y1 - y0, 0.15), ((x0 + x1) / 2, (y0 + y1) / 2, 0.06), tile=2.0, bevel=0.03), m["sinter"], smooth=False)
    frame = bmesh.new()
    for x in [x0 + (x1 - x0) * k / 6 for k in range(7)]:
        for y in (y0, y1):
            merge(frame, box((0.08, 0.08, h), (x, y, 0.13 + h / 2)))
        merge(frame, box((0.08, y1 - y0, 0.08), (x, (y0 + y1) / 2, 0.13 + h)))
    for y in (y0, y1):
        merge(frame, box((x1 - x0, 0.08, 0.08), ((x0 + x1) / 2, y, 0.13 + h)))
    b.put(frame, m["white_frame"], smooth=False)
    for y in (y0, y1):
        b.put(box((x1 - x0, 0.02, h - 0.1), ((x0 + x1) / 2, y, 0.13 + h / 2)), m["glass"], smooth=False)
    for x in (x0, x1):
        b.put(box((0.02, y1 - y0, h - 0.1), (x, (y0 + y1) / 2, 0.13 + h / 2)), m["glass"], smooth=False)
    b.put(box((x1 - x0, y1 - y0, 0.02), ((x0 + x1) / 2, (y0 + y1) / 2, 0.13 + h)), m["glass"], smooth=False)
    # Two rows of tubes, each before its LED panel, manifolds at foot and head.
    tubes, leds, mains = bmesh.new(), bmesh.new(), bmesh.new()
    for yr in (y0 + 1.0, y1 - 1.0):
        merge(leds, box((x1 - x0 - 1.0, 0.04, 1.9), ((x0 + x1) / 2, yr + 0.35, 1.25)))
        for k in range(int((x1 - x0 - 1.2) / 0.22)):
            x = x0 + 0.7 + 0.22 * k
            merge(tubes, moved(lathe([(0.0, 0.0), (0.07, 0.0), (0.07, 2.0), (0.0, 2.0)], 12, 0.5), (x, yr, 0.3)))
        for z in (0.26, 2.34):
            merge(mains, tube([(x0 + 0.5, yr, z), (x1 - 0.5, yr, z)], 0.05, 10))
    b.put(tubes, m["algae"])
    b.put(leds, m["grow_lamp"], smooth=False)
    b.put(mains, m["stainless"])
    b.collider(box((x1 - x0, y1 - y0, h), ((x0 + x1) / 2, (y0 + y1) / 2, 0.13 + h / 2)))
    # Its feed from the hall: culture medium in, harvest out.
    wall = arch_y(FLOOR + 1.2) + 0.95
    for dx, skin in ((2.0, "pipe_white"), (2.4, "pipe_blue")):
        b.put(pipe([(x0 + dx, y0 + 0.2, 1.2), (x0 + dx, y0 - 0.4, 1.2), (x0 + dx, wall, 1.2)], 0.035), m[skin])


def farm_hall(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """The farm hall along +X from its lock at -X: see the module's docstring."""
    m = dict(_mats(looks))
    m["algae"] = looks.get("spirulina_culture", (0.10, 0.42, 0.12), 0.08, 0.0, alpha=0.85, emit=((0.12, 0.55, 0.15), 0.6))
    b = Building(name, parent, at, yaw)
    b.places = []
    out_pts = _shell(b, m)
    _entry(b, m)
    _post_harvest(b, m)
    _cold_store(b, m)
    _processing(b, m)
    _propagation(b, m)
    _recovery(b, m)
    fans = _lights_and_fans(b, m)
    _outside(b, m, out_pts)
    _nutrient_plant(b, m)
    _algae_house(b, m)
    b.places.append(("room", {"name": name.lower(), "centre": ((X0 + X1) / 2, 0, FLOOR + 4.0), "size": (X1 - X0, 7.0, 8.0),
                              "volume": 7.0 * 6.0 * (X1 - X0), "glazing": 0.0, "work": 900,
                              "processing": PROCESSING_KG}))
    b.places.append(("service", {"fault": "hvac", "at": (29.3, 0.0, FLOOR + 1.2)}))
    b.places.append(("service", {"fault": "pump", "at": (NUTRIENT_PAD[0] + 5.3, NUTRIENT_PAD[3] + 0.4, 0.8)}))
    b.places.append(("service", {"fault": "fans", "at": fans[0]}))
    return b.finish()
