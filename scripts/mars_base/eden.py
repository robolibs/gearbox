"""The EDEN growth module's inside, after DLR's EDEN ISS
(Zabel et al. 2017): two 20 ft high-cube containers end to end. The first
holds a cold porch (the airlock) and a service section with the nutrient
tanks, a workbench and a regolith bed; a sealed door leads into the second,
the growth room, 5.7 x 2.15 x 2.55 m inside over a 300 mm sub-floor, with a
1 m aisle between four racks a side of one, two or four levels. Each level
holds two 600 x 400 mm trays under their own LED panels.

Everything is in the module's frame: the porch door at x = 0, x along the
module, z up from its floor slab.
"""
from usda import cube, cylinder, end, service, tray, xform, zone

LENGTH = 12.19
WIDTH = 2.44
HEIGHT = 2.90
WALL = 0.08
SUBFLOOR = 0.30
PORCH = 1.6
PARTITION = 6.06
GROW_X0 = PARTITION + 0.30
GROW_LEN = 5.70
GROW_W = 2.15
AISLE = 1.0
RACK_D = 0.575
RACK_H = 2.25
TRAY = (0.6, 0.4, 0.08)
DOOR_W = 1.0

# Each side's racks, from the door: (levels, crop, plants across x down, ppfd, hours, sown days ago)
RACKS = [
    (1, "tomato", (2, 1), 600, 12, (64, 40)),
    (2, "potato", (2, 1), 600, 12, (70, 20, 95, 5)),
    (4, "lettuce", (3, 2), 300, 16, (27, 20, 13, 6, 24, 17, 10, 2)),
    (2, "wheat", (4, 3), 600, 20, (50, 30, 14, 2)),
]

# Where the rail robot's rail runs: its underside, from its porch end.
RAIL_UNDER = HEIGHT - 0.16
RAIL_X = GROW_X0 + 0.15


def walls(u, d):
    """Inner walls, roof, partitions, the growth room's grating, ducts and rail."""
    xform(u, d, "Walls", (0, 0, 0))
    cx = LENGTH / 2
    cube(u, d + 1, "floor", (cx, 0, 0.1 + 0.03), (LENGTH, WIDTH, 0.06), "shell")
    cube(u, d + 1, "wall_north", (cx, WIDTH / 2 - WALL / 2, HEIGHT / 2), (LENGTH, WALL, HEIGHT), "laminate")
    cube(u, d + 1, "wall_south", (cx, -WIDTH / 2 + WALL / 2, HEIGHT / 2), (LENGTH, WALL, HEIGHT), "laminate")
    cube(u, d + 1, "roof", (cx, 0, HEIGHT - WALL / 2), (LENGTH, WIDTH, WALL), "laminate")
    cube(u, d + 1, "wall_far", (LENGTH - WALL / 2, 0, HEIGHT / 2), (WALL, WIDTH, HEIGHT), "laminate")
    side = (WIDTH - DOOR_W) / 2
    for flank, y in (("left", -WIDTH / 2 + side / 2), ("right", WIDTH / 2 - side / 2)):
        cube(u, d + 1, f"end_{flank}", (WALL / 2, y, HEIGHT / 2), (WALL, side, HEIGHT), "laminate")
    cube(u, d + 1, "end_lintel", (WALL / 2, 0, 2.2 + (HEIGHT - 2.2) / 2), (WALL, DOOR_W, HEIGHT - 2.2), "laminate")
    for name, x in (("porch_wall", PORCH), ("partition", PARTITION + 0.15)):
        side = (WIDTH - 0.9) / 2
        for flank, y in (("left", -WIDTH / 2 + side / 2), ("right", WIDTH / 2 - side / 2)):
            cube(u, d + 1, f"{name}_{flank}", (x, y, HEIGHT / 2), (0.1, side, HEIGHT), "laminate")
        cube(u, d + 1, f"{name}_lintel", (x, 0, 2.1 + (HEIGHT - 2.1) / 2), (0.1, 0.9, HEIGHT - 2.1), "laminate")
    grow_cx = GROW_X0 + GROW_LEN / 2
    cube(u, d + 1, "basin", (grow_cx, 0, 0.18), (GROW_LEN, GROW_W, 0.04), "tank", collide=False)
    cube(u, d + 1, "grating", (grow_cx, 0, 0.1 + SUBFLOOR - 0.02), (GROW_LEN, GROW_W, 0.04), "grating")
    cube(u, d + 1, "step", (PARTITION + 0.3, 0, 0.1 + SUBFLOOR / 2), (0.6, 0.9, SUBFLOOR), "grating")
    for side_y in (-0.42, 0.42):
        cube(u, d + 1, f"duct_{'s' if side_y < 0 else 'n'}", (grow_cx, side_y, HEIGHT - 0.18), (GROW_LEN - 0.2, 0.22, 0.16), "duct", collide=False)
    cube(u, d + 1, "rail", (grow_cx, 0, HEIGHT - 0.12), (GROW_LEN - 0.3, 0.12, 0.08), "rail")
    end(u, d)


def rack(u, d, name, x, y_inner, levels, side):
    """One rack: four aluminium posts, shelves at each level."""
    xform(u, d, name, (x, 0, 0))
    floor = 0.1 + SUBFLOOR
    outer = y_inner + side * RACK_D
    yc = (y_inner + outer) / 2
    for i, (px, py) in enumerate(((-0.69, y_inner), (0.69, y_inner), (-0.69, outer - side * 0.02), (0.69, outer - side * 0.02))):
        cube(u, d + 1, f"post{i}", (px, py, floor + RACK_H / 2), (0.03, 0.03, RACK_H), "profile")
    spacing = (RACK_H - 0.15) / levels
    for level in range(levels):
        z = floor + 0.15 + level * spacing
        cube(u, d + 1, f"shelf{level}", (0, yc, z - 0.02), (1.40, RACK_D - 0.02, 0.03), "profile")
    end(u, d)
    return floor, spacing, yc


def growth_room(u, d):
    zone(u, d, "GrowthRoom", "grow", (0, 0, 0), 0.0, GROW_LEN * GROW_W * (HEIGHT - 0.1 - SUBFLOOR - WALL),
         (GROW_X0 + GROW_LEN / 2, 0, (0.1 + SUBFLOOR + HEIGHT - WALL) / 2), (GROW_LEN, GROW_W, HEIGHT - WALL - 0.1 - SUBFLOOR))
    rack_len = GROW_LEN / 4
    for s, side in (("S", -1), ("N", 1)):
        for r, (levels, crop, slots, ppfd, hours, sown) in enumerate(RACKS):
            x = GROW_X0 + rack_len * (r + 0.5)
            floor, spacing, yc = rack(u, d + 1, f"Rack{s}{r}", x, side * AISLE / 2, levels, side)
            for level in range(levels):
                top = floor + 0.15 + level * spacing + TRAY[2] - 0.005
                lamp = min(spacing - 0.16, 1.0)
                for t, dx in enumerate((-0.33, 0.33)):
                    days = sown[(level * 2 + t) % len(sown)] + (3 if s == "N" else 0)
                    tray(u, d + 1, f"Tray{s}{r}{level}{'ab'[t]}", (x + dx, yc, top), crop, slots, "grow", ppfd, hours, lamp, days, TRAY)
    end(u, d)


def service_room(u, d):
    zone(u, d, "ServiceRoom", "service", (0, 0, 0), 0.0, (PARTITION - PORCH) * (WIDTH - 2 * WALL) * (HEIGHT - 0.2),
         ((PORCH + PARTITION) / 2, 0, (0.1 + HEIGHT - WALL) / 2), (PARTITION - PORCH, WIDTH - 2 * WALL, HEIGHT - WALL - 0.1),
         (19, 0.6, 800, 101.3), tank=200, ec=1.8, work=900)
    x = PORCH
    cylinder(u, d + 1, "tank_a", (x + 0.6, 0.72, 0.1 + 0.45), 0.35, 0.9, "tank")
    cylinder(u, d + 1, "tank_b", (x + 1.4, 0.72, 0.1 + 0.45), 0.35, 0.9, "tank")
    cube(u, d + 1, "bench", (x + 2.9, 0.78, 0.1 + 0.45), (1.6, 0.7, 0.9), "bench")
    cube(u, d + 1, "bed_frame", (x + 2.2, -0.70, 0.1 + 0.2), (3.0, 0.9, 0.4), "bench")
    tray(u, d + 1, "RegolithBed", (x + 2.2, -0.70, 0.1 + 0.4), "potato", (5, 2), "service", 400, 12, 1.5, 0, (2.9, 0.8, 0.02),
         medium="regolith", perchlorate=0.5)
    end(u, d)


def services(u, d):
    """Where each piece of equipment is serviced: a robot reaching the spot can repair it."""
    xform(u, d, "Services", (0, 0, 0))
    for name, fault, room, at in (
        ("air_handler", "fans", "grow", (GROW_X0 + GROW_LEN - 0.3, 0.0, HEIGHT - 0.4)),
        ("hvac", "hvac", "grow", (PARTITION - 0.4, -0.8, 1.6)),
        ("co2_bottles", "co2", "grow", (PARTITION - 0.4, 0.8, 0.8)),
        ("pump", "pump", "grow", (PORCH + 0.6, 0.35, 0.5)),
        ("tank_valve", "water_leak", "grow", (PORCH + 1.4, 0.35, 0.3)),
        ("shell_patch", "leak", "grow", (GROW_X0 + GROW_LEN / 2, 1.0, 2.0)),
    ):
        service(u, d + 1, name, at, fault, room)
    end(u, d)


def module(u, d, at):
    """The module's inside, standing at `at` in the base."""
    xform(u, d, "Eden", at)
    walls(u, d + 1)
    growth_room(u, d + 1)
    service_room(u, d + 1)
    services(u, d + 1)
    end(u, d)


# Flat looks of the module's inside: (colour, metal, rough, emissive).
EDEN_LOOKS = {
    "laminate": ((0.86, 0.86, 0.84), 0.0, 0.55, False),
    "shell": ((0.72, 0.72, 0.70), 0.3, 0.45, False),
    "profile": ((0.80, 0.81, 0.82), 1.0, 0.35, False),
    "grating": ((0.22, 0.23, 0.24), 0.9, 0.5, False),
    "tray": ((0.10, 0.10, 0.11), 0.0, 0.6, False),
    "housing": ((0.08, 0.08, 0.09), 0.4, 0.4, False),
    "led": ((0.95, 0.30, 0.65), 0.0, 0.3, True),
    "tank": ((0.88, 0.90, 0.92), 0.0, 0.25, False),
    "regolith": ((0.42, 0.20, 0.10), 0.0, 0.95, False),
    "door": ((0.62, 0.64, 0.66), 0.6, 0.4, False),
    "duct": ((0.75, 0.76, 0.77), 0.9, 0.3, False),
    "bench": ((0.30, 0.31, 0.33), 0.2, 0.5, False),
    "rail": ((0.25, 0.26, 0.28), 0.9, 0.3, False),
    "frame": ((0.55, 0.56, 0.58), 1.0, 0.4, False),
}
