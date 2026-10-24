"""Where everything in the base stands. Base frame: X north, Y west, Z up,
the spawn plaza at the origin.

The base is sized to what a few cargo landers could bring: every module
fits a 9 m fairing or packs flat and inflates; power is four 10 kWe fission
units and a field of roll-out arrays; ISRU is container-sized.

    north  two glazed domes off a second hub
    east   the arch farm: three inflated vaults off a spine of corridors
    centre the habitat cluster: a hub and three towers, the EDEN module
    west   solar arrays, the power station, fission units behind a berm
    south  ISRU: two process units, their tanks, an ice well, dug regolith
    far SE the landing zone: a Starship pad and a crew lander pad
"""
import math
import os

from mathutils import Vector

import buildings
import habitat
import industry
import power
import props
import spaceport
from buildings import gantry_rail, light_mast, mats
from kit import Building, disc, lathe

NODE = Vector((-10.0, -42.0, 0.0))
NODE_N = Vector((38.0, 0.0, 0.0))
TOWERS = [NODE + 19.0 * Vector((math.cos(math.radians(a)), math.sin(math.radians(a)), 0)) for a in (110, 200, 280)]
SPINE_TOP = NODE + 12.0 * Vector((math.cos(math.radians(330)), math.sin(math.radians(330)), 0))
DOME_R = 12.0
DOME_REACH = DOME_R + 7.2
DOMES = [("DomeA", Vector((62.0, -20.0, 0.0))), ("DomeB", Vector((62.0, 20.0, 0.0)))]
ARCH_LEN = 30.0
ARCHES = [(f"Arch{c}", Vector((14.0, -62.0 - 15.0 * k, 0.0))) for k, c in enumerate("ABC")]
JUNCTION_R = 2.0
EDEN = Vector((14.0, 22.0, 0.0))
INFLATABLE = Vector((-34.0, 26.0, 0.0))
HANGAR = Vector((-46.0, -8.0, 0.0))
PRINTED = Vector((-62.0, -52.0, 0.0))
SOLAR = Vector((-10.0, 86.0, 0.0))
STATION = Vector((22.0, 60.0, 0.0))
REACTORS = [Vector((-58.0, 196.0, 0.0)), Vector((-44.0, 204.0, 0.0)), Vector((-30.0, 204.0, 0.0)), Vector((-16.0, 196.0, 0.0))]
ISRU = [Vector((-96.0, -12.0, 0.0)), Vector((-96.0, 0.0, 0.0))]
TANKS = Vector((-94.0, 22.0, 0.0))
DRILL = Vector((-124.0, -30.0, 0.0))
WORKS = Vector((-120.0, 4.0, 0.0))
PAD_SHIP = Vector((-220.0, -160.0, 0.0))
PAD_LANDER = Vector((-150.0, -232.0, 0.0))
MAST = Vector((-24.0, 62.0, 0.0))
DISH = Vector((48.0, 120.0, 0.0))
NASA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".work", "mars_base", "nasa")

ROADS = [
    [(-8, -4, 0), (-24, -8, 0)],
    [(-10, 6, 0), (-50, 10, 0), (-88, 6, 0)],
    [(-100, -20, 0), (-150, -70, 0), (-200, -125, 0)],
    [(-165, -95, 0), (-152, -205, 0)],
    [(-104, -20, 0), (-120, -26, 0)],
    [(-104, 6, 0), (-112, 6, 0)],
    [(-4, 14, 0), (-2, 40, 0), (16, 54, 0)],
    [(10, -4, 0), (33, -5, 0)],
    [(-8, 10, 0), (-24, 20, 0)],
    [(-88, 6, 0), (-94, 14, 0)],
]


def toward(a, b):
    d = b - a
    return math.atan2(d.y, d.x)


def link(looks, base, name, a, b, ra, rb, radius, z):
    """A corridor between two round structures, from rim to rim; corridors
    of one length, bore and height share a module."""
    d = (b - a).normalized()
    built = habitat.corridor(looks, name, tuple(a + d * ra), tuple(b - d * rb), radius, base, z)
    built.kind = f"corridor_{round((b - a).length - ra - rb):d}m_r{round(radius * 10):d}_z{round(z * 10):d}"
    return built


def kind(built, name):
    """Marks a building as one of a kind of module that others share."""
    built.kind = name
    return built


def build(looks):
    """Builds the base; returns its root and every building with places.
    A building's `kind` names the module file it is exported to."""
    base = Building("MarsBase")
    base.finish()
    m = mats(looks)
    out = []
    add = out.append

    # The plaza a spawned machine stands on.
    plaza = Building("Plaza", base)
    plaza.put(lathe([(15.0, -0.1), (15.0, 0.03), (14.4, 0.06), (0.0, 0.06)], 96, 3.0), m["pad"], sharp=40)
    plaza.put(disc(9.0, 0.065, 96, 2.0, 8.6), m["paint"])
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        light_mast(plaza, m, (14.0 * math.cos(a), 14.0 * math.sin(a), 0), 8.0, 2, a + math.pi)
    add(plaza.finish())

    # Habitat cluster: a hub, three towers, and the second hub of the domes.
    hub_ports = [toward(NODE, NODE_N), toward(NODE, SPINE_TOP), toward(NODE, PRINTED)] + [toward(NODE, t) for t in TOWERS]
    add(habitat.hub_node(looks, "Hub", 3.8, tuple(NODE), base, hub_ports))
    for k, t in enumerate(TOWERS):
        add(kind(habitat.hab_tower(looks, f"Hab{k + 1}", 4.5, 8.0, tuple(t), toward(t, NODE), base, ports=(0.0,)), "hab_tower"))
        add(link(looks, base, f"Link{k + 1}", NODE, t, 5.0, 5.6, 1.3, 2.5))
    doors = {name: at + Vector((math.cos(toward(at, NODE_N)), math.sin(toward(at, NODE_N)), 0)) * DOME_REACH for name, at in DOMES}
    add(habitat.hub_node(looks, "HubNorth", 3.8, tuple(NODE_N), base, [toward(NODE_N, NODE)] + [toward(NODE_N, d) for d in doors.values()]))
    add(link(looks, base, "LinkNorth", NODE, NODE_N, 5.0, 5.0, 1.3, 2.5))

    # The domes.
    for name, at in DOMES:
        add(kind(buildings.geodesic_dome(looks, name, DOME_R, 1.4, tuple(at), toward(at, NODE_N), base), "geodesic_dome"))
        add(link(looks, base, f"Link{name}", doors[name], NODE_N, 0.1, 5.0, 1.3, 1.6))

    # Arch farm off a spine of corridors and junction drums.
    stops = [SPINE_TOP] + [Vector((SPINE_TOP.x, at.y, 0.0)) for _, at in ARCHES]
    add(link(looks, base, "Spine", NODE, SPINE_TOP, 5.0, JUNCTION_R, 1.3, 2.5))
    # The spine runs toward -Y; each arch's door lies toward +X of its junction.
    back, on, out_ = math.pi / 2, -math.pi / 2, 0.0
    add(kind(habitat.junction(looks, "SpineBend", tuple(SPINE_TOP), base, (toward(SPINE_TOP, NODE), on)), "junction_bend"))
    for k, ((name, at), stop) in enumerate(zip(ARCHES, stops[1:])):
        last = k == len(ARCHES) - 1
        ports = (back, out_) if last else (back, on, out_)
        add(kind(habitat.junction(looks, f"Junction{name}", tuple(stop), base, ports), "junction_end" if last else "junction_tee"))
        add(link(looks, base, f"Spine{name}", stops[k], stop, JUNCTION_R, JUNCTION_R, 1.3, 2.5))
        add(kind(buildings.arch_greenhouse(looks, name, ARCH_LEN, 10.0, 3.0, tuple(at), 0.0, base), "arch_greenhouse"))
        add(link(looks, base, f"Link{name}", stop, Vector((at.x - 3.2, at.y, 0.0)), JUNCTION_R, 0.0, 1.2, 1.4))

    # Round the plaza: the EDEN module, the inflatable hab, the hangar, a printed tower.
    add(habitat.eden_skin(looks, "EdenSkin", 12.19, 2.44, 2.90, 1.0, tuple(EDEN), 0.0, base))
    add(habitat.inflatable_hab(looks, "Hab", 7.5, 4.6, tuple(INFLATABLE), toward(INFLATABLE, Vector((0, 0, 0))), base))
    add(habitat.hangar(looks, "Hangar", 22.0, 7.0, tuple(HANGAR), 0.0, base))
    add(habitat.printed_tower(looks, "Printed", 5.0, 15.0, tuple(PRINTED), toward(PRINTED, NODE), base))
    add(link(looks, base, "LinkPrinted", NODE, PRINTED, 5.0, 5.4, 1.2, 1.4))

    # Power.
    add(power.solar_field(looks, "Solar", 4, 18, 8.0, 22.0, tuple(SOLAR), 0.0, base))
    add(power.power_station(looks, "PowerStation", tuple(STATION), 0.0, base))
    for k, at in enumerate(REACTORS):
        add(kind(power.kilopower(looks, f"Fission{k + 1}", tuple(at), 0.0, base), "kilopower"))
    add(power.berm(looks, "ReactorBerm", (-37.0, 200.0, 0.0), 34.0, math.radians(-160), math.radians(-20), 3.0, base))
    add(power.ground_cable(looks, "ReactorCable", [(-37.0, 196.0, 0), (-30.0, 150.0, 0), (6.0, 116.0, 0), (18.0, 68.0, 0)], base))
    add(power.cable_tray(looks, "PowerTray", [(16.0, 52.0, 0), (16.0, 16.0, 0), (30.0, 4.0, 0)], 1.4, base))

    # ISRU.
    for k, at in enumerate(ISRU):
        add(kind(industry.isru_unit(looks, f"Isru{k + 1}", tuple(at), 0.0, base), "isru_unit"))
    add(industry.propellant_tanks(looks, "Tanks", tuple(TANKS), 0.0, base))
    add(industry.drill_rig(looks, "IceWell", tuple(DRILL), 0.3, base))
    add(industry.regolith_works(looks, "RegolithWorks", tuple(WORKS), 0.0, base))
    add(industry.pipe_run(looks, "PipesTanks", [(-92.0, -6.0, 0), (-88.0, 4.0, 0), (-88.0, 16.0, 0), (-92.0, 18.0, 0)], base))
    add(industry.pipe_run(looks, "PipesBase", [(-88.0, -10.0, 0), (-62.0, -16.0, 0), (-48.0, -36.0, 0), (-30.0, -50.0, 0)], base, ("pipe_white", "pipe_blue")))

    # The landing zone.
    add(spaceport.landing_pad(looks, "PadShip", 32.0, tuple(PAD_SHIP), base))
    add(spaceport.starship(looks, "Starship", tuple(PAD_SHIP), 0.4, base))
    add(spaceport.landing_pad(looks, "PadLander", 24.0, tuple(PAD_LANDER), base, 8))
    add(spaceport.crew_lander(looks, "Lander", tuple(PAD_LANDER), 0.8, base))

    # Comms and science.
    add(props.lattice_mast(looks, "CommsMast", tuple(MAST), 14.0, base))
    add(props.dish(looks, "Dish", 5.0, tuple(DISH), math.radians(200), 40.0, base))
    add(props.weather_station(looks, "Weather", (12.0, -18.0, 0), base))

    # Roads, cargo and markers.
    for k, path in enumerate(ROADS):
        add(props.tracks(looks, f"Road{k}", path, 7.0 if k < 3 else 5.5, base))
    yard = Building("Yard", base)
    props.marker_poles(yard, m, ROADS[2], 16.0)
    for at, yaw, skin in (
        ((-84.0, -26.0, 0), 0.2, "container_orange"), ((-84.0, -30.0, 0), 0.25, "container_white"),
        ((-176.0, -112.0, 0), 1.0, "container_white"), ((-179.0, -117.0, 0), 1.1, "container_orange"),
        ((-36.0, -26.0, 0), 1.5, "container_blue"), ((40.0, 66.0, 0), 0.0, "container_orange"),
    ):
        props.container(yard, m, at, yaw, skin)
    props.crates(yard, m, (6.0, 30.0, 0), 0.1, 1)
    props.crates(yard, m, (-28.0, 4.0, 0), 1.2, 2)
    props.crates(yard, m, (-82.0, 14.0, 0), 0.4, 3, 8)
    props.barrels(yard, m, (-80.0, 20.0, 0), 4, 8)
    props.barrels(yard, m, (26.0, 48.0, 0), 5, 6)
    for k in range(3):
        props.cable_reel(yard, m, (32.0 + 2.4 * k, 72.0, 0), 0.0)
    add(yard.finish())

    # NASA's own models that stand still: a radome, and Viking as the plaza's
    # monument. The source glTFs are decompressed from Draco first, into
    # .work/mars_base/nasa (gltf-transform resize).
    for file, name, at, yaw, height, solid in (
        ("Radome_plain.glb", "Radome", (70.0, 80.0, 0.0), 0.0, None, (5.5, 10.0)),
        ("Viking.glb", "Viking", (8.0, -8.0, 0.06), 0.6, 2.2, (1.4, 1.6)),
    ):
        path = os.path.abspath(os.path.join(NASA, file))
        if os.path.exists(path):
            add(props.nasa_model(path, name, base, at, yaw, height, looks.textures, solid))
    return base, out


def fleet_places(eden_rail, arch_rail, dome_anchor):
    """The robots that tend the base: (name, machine file, where, yaw,
    anchor in the machine's frame or None) in the base frame."""
    import eden
    places = [("Eve", "eden_gantry.usdz", tuple(EDEN + Vector((eden.RAIL_X, 0.0, eden.RAIL_UNDER))), 0.0, eden_rail)]
    for name, at in ARCHES:
        places.append((f"{name}Bot", "arch_gantry.usdz", tuple(at + Vector((2.0, 0.0, 5.0))), 0.0, arch_rail))
    for name, at in DOMES:
        places.append((f"{name}Bot", "dome_gantry.usdz", tuple(at), toward(at, NODE_N), dome_anchor))
    # The rovers drive toward their -Y: a yaw of pi/2 points one north.
    places.append(("Rover1", "sev_rover.usdz", (-16.0, -9.0, 0.0), math.pi, None))
    places.append(("Rover2", "sev_rover.usdz", (-22.0, 38.0, 0.0), math.pi / 2, None))
    return places


def dome_gantry_rail():
    """The polar gantry's leg radius in these domes."""
    return gantry_rail(DOME_R)
