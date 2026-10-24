#!/usr/bin/env python3
"""Write the Mars greenhouse world: bin/gearbox/assets/world/mars_greenhouse.usda.

The module follows DLR's EDEN ISS (Zabel et al. 2017): two 20 ft high-cube
containers end to end. The first holds a cold porch (the airlock) and a
service section with the nutrient tanks, a workbench and a regolith bed; a
sealed door leads into the second, the growth room, 5.7 x 2.15 x 2.55 m
inside over a 300 mm sub-floor, with a 1 m centre aisle between four racks a
side, 575 mm deep and 2.25 m tall, of one, two or four levels. Each level
holds two 600 x 400 mm trays, each under its own LED panel. Outside, the
module is bermed in regolith; a solar array and a fission power unit stand on
the pad.

Gearbox reads the greenhouse from `GearboxZoneAPI` (rooms), `GearboxTrayAPI`
(trays and their lamps), `GearboxPowerAPI` and `GearboxServiceAPI` prims; see
specs/GREENHOUSE_SPEC.md. Run from the repository root (standard library only):

    python3 scripts/greenhouse/build_mars_greenhouse.py
"""
from pathlib import Path

OUT = Path("bin/gearbox/assets/world/mars_greenhouse.usda")

# Module footprint: starts this far along +X from the pad's centre, so a
# machine spawned at the centre stands clear of it.
X0 = 6.0
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

MATERIALS = {
    "laminate": ((0.86, 0.86, 0.84), 0.0, 0.55),
    "shell": ((0.72, 0.72, 0.70), 0.3, 0.45),
    "profile": ((0.80, 0.81, 0.82), 1.0, 0.35),
    "grating": ((0.22, 0.23, 0.24), 0.9, 0.5),
    "tray": ((0.10, 0.10, 0.11), 0.0, 0.6),
    "housing": ((0.08, 0.08, 0.09), 0.4, 0.4),
    "led": ((0.95, 0.30, 0.65), 0.0, 0.3),
    "tank": ((0.88, 0.90, 0.92), 0.0, 0.25),
    "regolith": ((0.42, 0.20, 0.10), 0.0, 0.95),
    "bags": ((0.55, 0.30, 0.17), 0.0, 0.9),
    "solar": ((0.03, 0.05, 0.13), 0.3, 0.15),
    "frame": ((0.55, 0.56, 0.58), 1.0, 0.4),
    "reactor": ((0.65, 0.66, 0.66), 0.8, 0.35),
    "radiator": ((0.18, 0.18, 0.19), 0.6, 0.45),
    "door": ((0.62, 0.64, 0.66), 0.6, 0.4),
    "duct": ((0.75, 0.76, 0.77), 0.9, 0.3),
    "bench": ((0.30, 0.31, 0.33), 0.2, 0.5),
    "rail": ((0.25, 0.26, 0.28), 0.9, 0.3),
}

lines = []


def emit(depth, text):
    lines.append("    " * depth + text)


def fmt(values):
    return "(" + ", ".join(f"{round(v, 4):g}" for v in values) + ")"


def box(depth, name, centre, size, material, collide=True, rotate=None):
    """A Cube scaled to `size`, centred at `centre`."""
    schemas = ["MaterialBindingAPI"] + (["PhysicsCollisionAPI"] if collide else [])
    emit(depth, f'def Cube "{name}" (')
    emit(depth + 1, f"prepend apiSchemas = {schemas!r}".replace("'", '"'))
    emit(depth, ")")
    emit(depth, "{")
    emit(depth + 1, "double size = 1")
    emit(depth + 1, f"double3 xformOp:translate = {fmt(centre)}")
    order = ['"xformOp:translate"']
    if rotate:
        emit(depth + 1, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
        order.append('"xformOp:rotateXYZ"')
    emit(depth + 1, f"float3 xformOp:scale = {fmt(size)}")
    order.append('"xformOp:scale"')
    emit(depth + 1, f"uniform token[] xformOpOrder = [{', '.join(order)}]")
    emit(depth + 1, f"rel material:binding = </Greenhouse/Looks/{material}>")
    emit(depth, "}")


def cylinder(depth, name, centre, radius, height, material, collide=True, axis="Z"):
    schemas = ["MaterialBindingAPI"] + (["PhysicsCollisionAPI"] if collide else [])
    emit(depth, f'def Cylinder "{name}" (')
    emit(depth + 1, f"prepend apiSchemas = {schemas!r}".replace("'", '"'))
    emit(depth, ")")
    emit(depth, "{")
    emit(depth + 1, f'uniform token axis = "{axis}"')
    emit(depth + 1, f"double radius = {radius}")
    emit(depth + 1, f"double height = {height}")
    emit(depth + 1, f"double3 xformOp:translate = {fmt(centre)}")
    emit(depth + 1, 'uniform token[] xformOpOrder = ["xformOp:translate"]')
    emit(depth + 1, f"rel material:binding = </Greenhouse/Looks/{material}>")
    emit(depth, "}")


def xform(depth, name, at, schemas=(), attrs=()):
    if schemas:
        emit(depth, f'def Xform "{name}" (')
        emit(depth + 1, f"prepend apiSchemas = {list(schemas)!r}".replace("'", '"'))
        emit(depth, ")")
    else:
        emit(depth, f'def Xform "{name}"')
    emit(depth, "{")
    for attr in attrs:
        emit(depth + 1, attr)
    emit(depth + 1, f"double3 xformOp:translate = {fmt(at)}")
    emit(depth + 1, 'uniform token[] xformOpOrder = ["xformOp:translate"]')


def looks():
    emit(1, 'def Scope "Looks"')
    emit(1, "{")
    for name, (colour, metal, rough) in MATERIALS.items():
        emit(2, f'def Material "{name}"')
        emit(2, "{")
        emit(3, f"token outputs:surface.connect = </Greenhouse/Looks/{name}/Surface.outputs:surface>")
        emit(3, 'def Shader "Surface"')
        emit(3, "{")
        emit(4, 'uniform token info:id = "UsdPreviewSurface"')
        emit(4, f"color3f inputs:diffuseColor = {fmt(colour)}")
        emit(4, f"float inputs:metallic = {metal}")
        emit(4, f"float inputs:roughness = {rough}")
        if name == "led":
            emit(4, f"color3f inputs:emissiveColor = {fmt(colour)}")
        emit(4, "token outputs:surface")
        emit(3, "}")
        emit(2, "}")
    emit(1, "}")


def shell():
    """Floor, walls, roof, the partition with its door, and the outer door."""
    emit(1, 'def Xform "Shell"')
    emit(1, "{")
    x1 = X0 + LENGTH
    cx = X0 + LENGTH / 2
    box(2, "slab", (cx, 0, 0.05), (LENGTH + 0.6, WIDTH + 0.6, 0.1), "regolith")
    box(2, "floor", (cx, 0, 0.1 + 0.03), (LENGTH, WIDTH, 0.06), "shell")
    box(2, "wall_north", (cx, WIDTH / 2 - WALL / 2, HEIGHT / 2), (LENGTH, WALL, HEIGHT), "laminate")
    box(2, "wall_south", (cx, -WIDTH / 2 + WALL / 2, HEIGHT / 2), (LENGTH, WALL, HEIGHT), "laminate")
    box(2, "roof", (cx, 0, HEIGHT - WALL / 2), (LENGTH, WIDTH, WALL), "laminate")
    box(2, "wall_far", (x1 - WALL / 2, 0, HEIGHT / 2), (WALL, WIDTH, HEIGHT), "laminate")
    # The outer end: a door into the porch, closed but for robots it opens.
    side = (WIDTH - 1.0) / 2
    for flank, y in (("left", -WIDTH / 2 + side / 2), ("right", WIDTH / 2 - side / 2)):
        box(2, f"end_{flank}", (X0 + WALL / 2, y, HEIGHT / 2), (WALL, side, HEIGHT), "laminate")
    box(2, "end_lintel", (X0 + WALL / 2, 0, 2.2 + (HEIGHT - 2.2) / 2), (WALL, 1.0, HEIGHT - 2.2), "laminate")
    # Porch to service section, and service section to growth room: framed doorways.
    for name, x in (("porch_wall", X0 + PORCH), ("partition", X0 + PARTITION + 0.15)):
        side = (WIDTH - 0.9) / 2
        for flank, y in (("left", -WIDTH / 2 + side / 2), ("right", WIDTH / 2 - side / 2)):
            box(2, f"{name}_{flank}", (x, y, HEIGHT / 2), (0.1, side, HEIGHT), "laminate")
        box(2, f"{name}_lintel", (x, 0, 2.1 + (HEIGHT - 2.1) / 2), (0.1, 0.9, HEIGHT - 2.1), "laminate")
    # The growth room stands on grating over a sub-floor and leak basin.
    grow_cx = X0 + GROW_X0 + GROW_LEN / 2
    box(2, "basin", (grow_cx, 0, 0.18), (GROW_LEN, GROW_W, 0.04), "tank", collide=False)
    box(2, "grating", (grow_cx, 0, 0.1 + SUBFLOOR - 0.02), (GROW_LEN, GROW_W, 0.04), "grating")
    box(2, "step", (X0 + PARTITION + 0.3, 0, 0.1 + SUBFLOOR / 2), (0.6, 0.9, SUBFLOOR), "grating")
    # Air handling along the ceiling: eight louvred ducts feed the canopy.
    for side_y in (-0.42, 0.42):
        box(2, f"duct_{'s' if side_y < 0 else 'n'}", (grow_cx, side_y, HEIGHT - 0.18), (GROW_LEN - 0.2, 0.22, 0.16), "duct", collide=False)
    # The ceiling rail a harvesting arm runs along (DLR's EVE).
    box(2, "rail", (grow_cx, 0, HEIGHT - 0.12), (GROW_LEN - 0.3, 0.12, 0.08), "rail")
    emit(1, "}")


def rack(depth, name, x, y_inner, levels, side):
    """One rack: four aluminium posts, shelves at each level."""
    xform(depth, name, (x, 0, 0))
    floor = 0.1 + SUBFLOOR
    outer = y_inner + side * RACK_D
    yc = (y_inner + outer) / 2
    for i, (px, py) in enumerate(((-0.69, y_inner), (0.69, y_inner), (-0.69, outer - side * 0.02), (0.69, outer - side * 0.02))):
        box(depth + 1, f"post{i}", (px, py, floor + RACK_H / 2), (0.03, 0.03, RACK_H), "profile")
    spacing = (RACK_H - 0.15) / levels
    for level in range(levels):
        z = floor + 0.15 + level * spacing
        box(depth + 1, f"shelf{level}", (0, yc, z - 0.02), (1.40, RACK_D - 0.02, 0.03), "profile")
    emit(depth, "}")
    return floor, spacing, yc


def tray(depth, name, at, species, slots, zone, ppfd, hours, lamp_height, sown_days, medium="hydroponic", perchlorate=0.0, size=TRAY):
    attrs = [
        f'token gearbox:tray:species = "{species}"',
        f"int2 gearbox:tray:slots = ({slots[0]}, {slots[1]})",
        f'token gearbox:tray:medium = "{medium}"',
        f'token gearbox:tray:zone = "{zone}"',
        f"float2 gearbox:tray:size = ({size[0]}, {size[1]})",
        f"float gearbox:tray:ppfd = {ppfd}",
        f"float gearbox:tray:lampHours = {hours}",
        f"float gearbox:tray:lampHeight = {lamp_height:.3f}",
        f"float gearbox:tray:sownDays = {sown_days}",
    ]
    if medium == "regolith":
        attrs.append(f"float gearbox:tray:perchlorate = {perchlorate}")
    xform(depth, name, at, ["GearboxTrayAPI"], attrs)
    box(depth + 1, "basin", (0, 0, -size[2] / 2), size, "regolith" if medium == "regolith" else "tray")
    xform(depth + 1, "lamp", (0, 0, lamp_height))
    box(depth + 2, "housing", (0, 0, 0.02), (size[0] + 0.02, size[1] + 0.02, 0.04), "housing", collide=False)
    box(depth + 2, "face", (0, 0, -0.002), (size[0] - 0.04, size[1] - 0.04, 0.004), "led", collide=False)
    emit(depth + 1, "}")
    emit(depth, "}")


# Each side's racks, from the door: (levels, crop, plants across x down, ppfd, hours, sown days ago)
RACKS = [
    (1, "tomato", (2, 1), 600, 12, (64, 40)),
    (2, "potato", (2, 1), 600, 12, (70, 20, 95, 5)),
    (4, "lettuce", (3, 2), 300, 16, (27, 20, 13, 6, 24, 17, 10, 2)),
    (2, "wheat", (4, 3), 600, 20, (50, 30, 14, 2)),
]


def growth_room():
    emit(1, 'def Xform "GrowthRoom" (')
    emit(2, 'prepend apiSchemas = ["GearboxZoneAPI"]')
    emit(1, ")")
    emit(1, "{")
    for attr in (
        'token gearbox:zone:name = "grow"',
        f"float gearbox:zone:volume = {GROW_LEN * GROW_W * (HEIGHT - 0.1 - SUBFLOOR - WALL):.2f}",
        "float gearbox:zone:temperature = 21",
        "float gearbox:zone:humidity = 0.75",
        "float gearbox:zone:co2 = 1000",
        "float gearbox:zone:pressure = 101.3",
        "float gearbox:zone:tank = 400",
        "float gearbox:zone:ec = 2.5",
        "float gearbox:zone:workLight = 0",
        f"float3 gearbox:zone:centre = ({X0 + GROW_X0 + GROW_LEN / 2:.3f}, 0, {(0.1 + SUBFLOOR + HEIGHT - WALL) / 2:.3f})",
        f"float3 gearbox:zone:size = ({GROW_LEN:.3f}, {GROW_W:.3f}, {HEIGHT - WALL - 0.1 - SUBFLOOR:.3f})",
    ):
        emit(2, attr)
    rack_len = GROW_LEN / 4
    for s, side in (("S", -1), ("N", 1)):
        for r, (levels, crop, slots, ppfd, hours, sown) in enumerate(RACKS):
            x = X0 + GROW_X0 + rack_len * (r + 0.5)
            y_inner = side * AISLE / 2
            floor, spacing, yc = rack(2, f"Rack{s}{r}", x, y_inner, levels, side)
            for level in range(levels):
                top = floor + 0.15 + level * spacing + TRAY[2] - 0.005
                lamp = min(spacing - 0.16, 1.0)
                for t, dx in enumerate((-0.33, 0.33)):
                    days = sown[(level * 2 + t) % len(sown)] + (3 if s == "N" else 0)
                    tray(2, f"Tray{s}{r}{level}{'ab'[t]}", (x + dx, yc, top), crop, slots, "grow", ppfd, hours, lamp, days)
    emit(1, "}")


def service_room():
    emit(1, 'def Xform "ServiceRoom" (')
    emit(2, 'prepend apiSchemas = ["GearboxZoneAPI"]')
    emit(1, ")")
    emit(1, "{")
    for attr in (
        'token gearbox:zone:name = "service"',
        f"float gearbox:zone:volume = {(PARTITION - PORCH) * (WIDTH - 2 * WALL) * (HEIGHT - 0.2):.2f}",
        "float gearbox:zone:temperature = 19",
        "float gearbox:zone:humidity = 0.6",
        "float gearbox:zone:co2 = 800",
        "float gearbox:zone:pressure = 101.3",
        "float gearbox:zone:tank = 200",
        "float gearbox:zone:ec = 1.8",
        "float gearbox:zone:workLight = 900",
        f"float3 gearbox:zone:centre = ({X0 + (PORCH + PARTITION) / 2:.3f}, 0, {(0.1 + HEIGHT - WALL) / 2:.3f})",
        f"float3 gearbox:zone:size = ({PARTITION - PORCH:.3f}, {WIDTH - 2 * WALL:.3f}, {HEIGHT - WALL - 0.1:.3f})",
    ):
        emit(2, attr)
    x = X0 + PORCH
    # Nutrient tanks, the workbench, and the bed of Martian regolith.
    cylinder(2, "tank_a", (x + 0.6, 0.72, 0.1 + 0.45), 0.35, 0.9, "tank")
    cylinder(2, "tank_b", (x + 1.4, 0.72, 0.1 + 0.45), 0.35, 0.9, "tank")
    box(2, "bench", (x + 2.9, 0.78, 0.1 + 0.45), (1.6, 0.7, 0.9), "bench")
    box(2, "bed_frame", (x + 2.2, -0.70, 0.1 + 0.2), (3.0, 0.9, 0.4), "bench")
    tray(2, "RegolithBed", (x + 2.2, -0.70, 0.1 + 0.4), "potato", (5, 2), "service", 400, 12, 1.5, 0,
         medium="regolith", perchlorate=0.5, size=(2.9, 0.8, 0.02))
    emit(1, "}")


def services():
    """Where each piece of equipment is serviced: a robot reaching the spot can repair it."""
    emit(1, 'def Scope "Services"')
    emit(1, "{")
    grow_x = X0 + GROW_X0
    for name, fault, zone, at in (
        ("air_handler", "fans", "grow", (grow_x + GROW_LEN - 0.3, 0.0, HEIGHT - 0.4)),
        ("hvac", "hvac", "grow", (X0 + PARTITION - 0.4, -0.8, 1.6)),
        ("co2_bottles", "co2", "grow", (X0 + PARTITION - 0.4, 0.8, 0.8)),
        ("pump", "pump", "grow", (X0 + PORCH + 0.6, 0.35, 0.5)),
        ("tank_valve", "water_leak", "grow", (X0 + PORCH + 1.4, 0.35, 0.3)),
        ("power_panel", "power", "", (X0 + PORCH + 0.2, -1.0, 1.4)),
        ("shell_patch", "leak", "grow", (grow_x + GROW_LEN / 2, 1.0, 2.0)),
    ):
        xform(2, name, at, ["GearboxServiceAPI"], [f'token gearbox:service:fault = "{fault}"', f'token gearbox:service:zone = "{zone}"'])
        box(3, "panel", (0, 0, 0), (0.3, 0.12, 0.3), "door", collide=False)
        emit(2, "}")
    emit(1, "}")


def outside():
    emit(1, 'def Xform "Outside"')
    emit(1, "{")
    cx = X0 + LENGTH / 2
    # Regolith in sandbags banked against both long walls to the roof, which
    # carries its own cover: shielding from radiation, and no sun inside.
    for side, y in (("n", WIDTH / 2 + 0.6), ("s", -WIDTH / 2 - 0.6)):
        box(2, f"berm_{side}", (cx, y, (HEIGHT + 0.3) / 2), (LENGTH + 0.4, 1.2, HEIGHT + 0.3), "bags")
        box(2, f"berm_{side}_slope", (cx, y + (1.1 if side == "n" else -1.1), 0.75), (LENGTH + 0.4, 1.4, 1.5), "bags", rotate=(35 if side == "n" else -35, 0, 0))
    box(2, "roof_cover", (cx, 0, HEIGHT + 0.15), (LENGTH + 0.4, WIDTH + 0.4, 0.3), "bags")
    # Solar array: four 2 x 5 m panels at 30 degrees, facing the equator.
    for i in range(4):
        px = X0 + 1.0 + i * 3.0
        box(2, f"panel{i}", (px, -9.0, 1.4), (2.0, 5.0, 0.05), "solar", rotate=(30, 0, 0))
        for k, dy in enumerate((-1.8, 1.8)):
            box(2, f"panel{i}_post{k}", (px, -9.0 + dy, 0.7 + dy * 0.28), (0.08, 0.08, 1.4 + dy * 0.56), "frame")
    xform(2, "Power", (-14.0, 16.0, 0.0), ["GearboxPowerAPI"], ["float gearbox:power:reactor = 10000", "float gearbox:power:solarArea = 40", "float gearbox:power:battery = 40000"])
    cylinder(3, "core", (0, 0, 1.3), 0.55, 2.6, "reactor")
    for k in range(4):
        box(3, f"radiator{k}", (0, 0, 1.6), (0.05, 3.2, 2.0), "radiator", rotate=(0, 0, k * 45))
    emit(2, "}")
    emit(1, "}")


def main():
    emit(0, "#usda 1.0")
    emit(0, "(")
    emit(1, "customLayerData = {")
    emit(2, 'string creator = "scripts/greenhouse/build_mars_greenhouse.py: an EDEN ISS-like growth module on Mars"')
    emit(1, "}")
    emit(1, 'defaultPrim = "Greenhouse"')
    emit(1, "metersPerUnit = 1")
    emit(1, 'upAxis = "Z"')
    emit(0, ")")
    emit(0, "")
    emit(0, "# Generated: edit scripts/greenhouse/build_mars_greenhouse.py and run it again.")
    emit(0, "# Load it as a world: `gearbox run --planet mars --world bin/gearbox/assets/world/mars_greenhouse.usda`.")
    emit(0, 'def Xform "Greenhouse"')
    emit(0, "{")
    looks()
    shell()
    growth_room()
    service_room()
    services()
    outside()
    emit(0, "}")
    OUT.write_text("\n".join(lines) + "\n")
    trays = sum(1 for line in lines if "GearboxTrayAPI" in line)
    print(f"wrote {OUT}: {len(lines)} lines, {trays} trays")


main()
