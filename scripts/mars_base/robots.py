"""The robots that tend the crops, each its own machine file.

    rail gantry   after DLR's EVE: a trolley rides a ceiling rail, a mast
                  drops from it, a turret at its foot swings a telescoping
                  arm over either side. EDEN's room and the arch farm's
                  vaults each have one, at their own sizes.
    polar gantry  a dome's boom, slewing on the cistern at its centre: a
                  carriage runs out along it and a telescoping mast drops
                  to any ring of the terraces.

Every joint has a motor and a `builtin:joint_position` controller of its
name, taking a position 0..1 of its range. The heads carry a recognition
camera looking down, a light sensor looking up and a climate sensor, so a
robot sees which plant it is over and what it grows in.
"""
import math
from dataclasses import dataclass

from usda import Usda, cube, cylinder, end, fmt, materials, schemas, xform

LOOKS = {
    "housing": ((0.08, 0.08, 0.09), 0.4, 0.4, False),
    "frame": ((0.55, 0.56, 0.58), 1.0, 0.4, False),
    "profile": ((0.80, 0.81, 0.82), 1.0, 0.35, False),
    "rail": ((0.25, 0.26, 0.28), 0.9, 0.3, False),
    "paint": ((0.92, 0.55, 0.08), 0.3, 0.45, False),
    "white": ((0.88, 0.88, 0.86), 0.2, 0.4, False),
    "led": ((0.95, 0.30, 0.65), 0.0, 0.3, True),
}


def body(u, d, name, at, mass, inertia, link=None, values=()):
    xform(u, d, name, at, ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"], [
        *([f'token gearbox:link:name = "{link}"', 'token gearbox:link:role = "base"'] if link else []),
        f"float physics:mass = {mass}",
        f"float3 physics:diagonalInertia = {fmt(inertia)}",
        *values,
    ])


def sensor(u, d, name, kind, at, rotate, attrs=()):
    u.emit(d, f'def Xform "{name}" (prepend apiSchemas = ["GearboxLinkAPI"])')
    u.emit(d, "{")
    u.emit(d + 1, 'token gearbox:link:role = "sensor"')
    u.emit(d + 1, f'token gearbox:sensor:kind = "{kind}"')
    for attr in attrs:
        u.emit(d + 1, attr)
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(at)}")
    u.emit(d + 1, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
    u.emit(d, "}")


def joint(u, d, kind, name, body0, body1, pos0, axis=None, limits=None, motor=None):
    u.emit(d, f'def Physics{kind}Joint "{name}"')
    u.emit(d, "{")
    if body0:
        u.emit(d + 1, f"rel physics:body0 = </robot/{body0}>")
    u.emit(d + 1, f"rel physics:body1 = </robot/{body1}>")
    if axis:
        u.emit(d + 1, f'uniform token physics:axis = "{axis}"')
    u.emit(d + 1, f"point3f physics:localPos0 = {fmt(pos0)}")
    u.emit(d + 1, "point3f physics:localPos1 = (0, 0, 0)")
    if limits:
        u.emit(d + 1, f"float physics:lowerLimit = {limits[0]}")
        u.emit(d + 1, f"float physics:upperLimit = {limits[1]}")
    if motor:
        u.emit(d + 1, f"float gearbox:motor:maxForce = {motor[0]}")
        u.emit(d + 1, f"float gearbox:motor:maxVelocity = {motor[1]}")
    u.emit(d, "}")


def head_sensors(u, d, at):
    """The tending head's camera (down, naming plants), light sensor (up) and climate sensor."""
    x, y, z = at
    sensor(u, d, "camera_link", "camera", (x, y, z - 0.05), (0, 90, 0), [
        "bool gearbox:sensor:recognition = true",
        "int gearbox:sensor:width = 160",
        "int gearbox:sensor:height = 120",
        "float gearbox:sensor:vfov_deg = 70",
        "float gearbox:sensor:range_m = 4",
        "float gearbox:sensor:rate_hz = 2",
    ])
    sensor(u, d, "light_link", "light", (x, y, z + 0.04), (0, -90, 0), ["float gearbox:sensor:rate_hz = 2"])
    sensor(u, d, "climate_link", "climate", (x, y + 0.05, z), (0, 0, 0))


def open_machine(u, kind, controllers, base, note):
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "scripts/mars_base/robots.py: {note}"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/robots.py and build again.")
    u.emit(0, "# Each joint takes a position 0..1 of its range: `gearbox machine set-value <joint> position 0.5 --machine <id>`.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'PhysicsArticulationRootAPI', *(f'GearboxControllerAPI:{c}' for c in controllers)])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, f'token gearbox:machine:kind = "{kind}"')
    u.emit(1, f"rel gearbox:machine:body = </robot/{base}>")
    for c in controllers:
        u.emit(1, f'token gearbox:controller:{c}:type = "builtin:joint_position"')
        u.emit(1, f"rel gearbox:controller:{c}:target = </robot/Joints/{c}>")
    materials(u, 1, LOOKS)


@dataclass
class Rail:
    """A rail gantry's sizes, metres, from the rail's underside at its start."""
    rail_len: float
    mast_len: float
    mast_z: float
    turret_z: float
    lift: float
    arm: tuple
    beam: float
    head: float
    reach: float
    draw_rail: bool = False
    hangers: float = 0.0
    scale: float = 1.0


EDEN_RAIL = Rail(rail_len=5.4, mast_len=0.9, mast_z=-0.58, turret_z=-1.08, lift=1.25, arm=(0.0, -0.20, -1.16), beam=0.36, head=-0.215, reach=0.65)


def arch_rail(length):
    """The ridge robot of an arch greenhouse `length` long: its rail 5 m up,
    starting 2 m in from the door, the turret clear under the lamps."""
    return Rail(rail_len=length - 4.0, mast_len=3.1, mast_z=-1.65, turret_z=-3.15, lift=0.65, arm=(0.0, -0.35, -3.23), beam=0.9, head=-0.5,
                reach=2.4, draw_rail=True, hangers=0.75, scale=1.4)


def rail_gantry(r, note):
    """A rail gantry machine: rail (held to the world), trolley, mast, turret, arm."""
    u = Usda("/robot/Looks")
    open_machine(u, "rail_robot", ("travel", "lift", "turn", "reach"), "rail", note)
    s = r.scale
    body(u, 1, "rail", (r.rail_len / 2, 0, 0.02), 40 * s, (1, 100 * s, 100 * s), link="base_link")
    if r.draw_rail:
        cube(u, 2, "beam", (0, 0, 0.06), (r.rail_len + 0.6, 0.14, 0.12), "rail", collide=False)
        for k in range(int(r.rail_len / 3.0) + 1):
            x = -r.rail_len / 2 + k * (r.rail_len / int(r.rail_len / 3.0))
            cube(u, 2, f"hanger{k}", (x, 0, 0.12 + r.hangers / 2), (0.05, 0.05, r.hangers), "frame", collide=False)
    end(u, 1)
    travel = r.rail_len - 0.4
    trolley_z = -0.07 * s
    body(u, 1, "trolley", (0.2, 0, trolley_z), 18 * s, (0.12 * s, 0.2 * s, 0.25 * s), values=[f"float gearbox:value:range = {travel:g}"])
    cube(u, 2, "carriage", (0, 0, 0), (0.34 * s, 0.24 * s, 0.12 * s), "housing")
    for x in (-0.11 * s, 0.11 * s):
        for y in (-0.075 * s, 0.075 * s):
            cylinder(u, 2, f"roller_{'e' if x > 0 else 'w'}{'n' if y > 0 else 's'}", (x, y, 0.065 * s), 0.025 * s, 0.03 * s, "frame", collide=False, axis="Y")
    end(u, 1)
    body(u, 1, "mast", (0.2, 0, r.mast_z), 9 * s, (0.6 * s, 0.6 * s, 0.01), values=[f"float gearbox:value:range = {-r.lift}"])
    cube(u, 2, "column", (0, 0, 0), (0.07 * s, 0.07 * s, r.mast_len), "profile")
    end(u, 1)
    body(u, 1, "turret", (0.2, 0, r.turret_z), 3 * s, (0.01, 0.01, 0.01), values=["float gearbox:value:range = 3.14159"])
    cylinder(u, 2, "hub", (0, 0, 0), 0.07 * s, 0.1 * s, "housing")
    end(u, 1)
    body(u, 1, "arm", (0.2 + r.arm[0], r.arm[1], r.arm[2]), 2.5 * s, (0.04 * s, 0.005, 0.04 * s), values=[f"float gearbox:value:range = {-r.reach}"])
    cube(u, 2, "beam", (0, 0, 0), (0.05 * s, r.beam, 0.05 * s), "profile")
    cube(u, 2, "head", (0, r.head, -0.01), (0.09 * s, 0.07 * s, 0.07 * s), "housing")
    cube(u, 2, "lens", (0, r.head, -0.047 * s), (0.03, 0.03, 0.006), "led", collide=False)
    head_sensors(u, 2, (0, r.head, -0.01))
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "rail", rail_anchor(r))
    joint(u, 2, "Prismatic", "travel", "rail", "trolley", (0.2 - r.rail_len / 2, 0, trolley_z - 0.02), "X", (0, travel), (400 * s, 0.3 * s))
    joint(u, 2, "Prismatic", "lift", "trolley", "mast", (0, 0, r.mast_z - trolley_z), "Z", (-r.lift, 0), (600 * s, 0.25 * s))
    joint(u, 2, "Revolute", "turn", "mast", "turret", (0, 0, r.turret_z - r.mast_z), "Z", (-5, 185), (60 * s, 1.0))
    joint(u, 2, "Prismatic", "reach", "turret", "arm", (r.arm[0], r.arm[1], r.arm[2] - r.turret_z), "Y", (-r.reach, 0), (200 * s, 0.2 * s))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


@dataclass
class Polar:
    """A polar gantry's sizes, metres, from the dome's floor at its centre."""
    pivot_z: float = 4.0
    boom_z: float = 2.0
    inner: float = 3.0
    travel: float = 12.0
    leg_r: float = 16.0
    floor_z: float = 0.45
    carriage_z: float = 1.6
    head_z: float = 3.3
    lift: float = 2.2


def polar_gantry(p, note):
    """A dome's polar gantry: pedestal (held to the world), boom, carriage, mast."""
    u = Usda("/robot/Looks")
    open_machine(u, "polar_gantry", ("slew", "travel", "lift"), "pedestal", note)
    body(u, 1, "pedestal", (0, 0, p.pivot_z - 0.3), 300, (60, 60, 60), link="base_link")
    cylinder(u, 2, "drum", (0, 0, 0), 0.8, 0.6, "white")
    end(u, 1)
    span = p.leg_r + 1.8
    boom_mass = 500.0
    body(u, 1, "boom", (0, 0, p.pivot_z), boom_mass, (200, boom_mass * span * span / 3, boom_mass * span * span / 3),
         values=["float gearbox:value:range = 6.28319"])
    cylinder(u, 2, "slewing_ring", (0, 0, 0.25), 0.7, 0.5, "housing")
    cube(u, 2, "tower", (0, 0, 1.25), (0.5, 0.5, 1.5), "paint")
    mid = (p.leg_r - 1.8) / 2
    for z, name in ((p.boom_z + 0.3, "top_chord"), (p.boom_z - 0.3, "bottom_chord")):
        for y in (-0.35, 0.35):
            cube(u, 2, f"{name}_{'l' if y < 0 else 'r'}", (mid, y, z), (span, 0.08, 0.08), "paint", collide=False)
    for k in range(int(span / 1.5) + 1):
        x = -1.8 + k * (span / int(span / 1.5))
        for y in (-0.35, 0.35):
            cube(u, 2, f"post{k}_{'l' if y < 0 else 'r'}", (x, y, p.boom_z), (0.06, 0.06, 0.6), "paint", collide=False)
        cube(u, 2, f"tie{k}", (x, 0, p.boom_z - 0.3), (0.05, 0.7, 0.05), "paint", collide=False)
    cube(u, 2, "rail", (mid + 1.0, 0, p.boom_z - 0.36), (span - 2.0, 0.12, 0.06), "rail", collide=False)
    cube(u, 2, "counterweight", (-1.4, 0, p.boom_z - 0.1), (0.8, 0.9, 0.9), "housing")
    leg = p.boom_z - (p.floor_z - p.pivot_z)
    cube(u, 2, "leg", (p.leg_r, 0, p.boom_z - leg / 2), (0.25, 0.25, leg), "paint")
    cube(u, 2, "bogie", (p.leg_r, 0, p.floor_z - p.pivot_z + 0.12), (0.7, 0.4, 0.22), "housing")
    end(u, 1)
    body(u, 1, "carriage", (p.inner, 0, p.pivot_z + p.carriage_z), 40, (2, 2, 2), values=[f"float gearbox:value:range = {p.travel:g}"])
    cube(u, 2, "trolley", (0, 0, 0.08), (0.7, 0.8, 0.25), "housing")
    sleeve = p.pivot_z + p.carriage_z - p.head_z - 0.1
    cube(u, 2, "sleeve", (0, 0, -sleeve / 2), (0.22, 0.22, sleeve), "white")
    end(u, 1)
    body(u, 1, "mast", (p.inner, 0, p.head_z), 12, (1, 1, 0.05), values=[f"float gearbox:value:range = {-p.lift}"])
    inner_tube = sleeve + 0.2
    cube(u, 2, "tube", (0, 0, inner_tube / 2), (0.14, 0.14, inner_tube), "profile", collide=False)
    cube(u, 2, "head", (0, 0, -0.06), (0.28, 0.28, 0.12), "housing")
    cube(u, 2, "lens", (0, 0, -0.123), (0.05, 0.05, 0.006), "led", collide=False)
    head_sensors(u, 2, (0, 0, -0.07))
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "pedestal", polar_anchor(p))
    joint(u, 2, "Revolute", "slew", "pedestal", "boom", (0, 0, 0.3), "Z", (-5, 365), (4000, 0.15))
    joint(u, 2, "Prismatic", "travel", "boom", "carriage", (p.inner, 0, p.carriage_z), "X", (0, p.travel), (800, 0.5))
    joint(u, 2, "Prismatic", "lift", "carriage", "mast", (0, 0, p.head_z - p.pivot_z - p.carriage_z), "Z", (-p.lift, 0), (800, 0.3))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def rail_anchor(r):
    """Where a rail gantry's anchor holds its rail, in the machine's frame."""
    return (r.rail_len / 2, 0.0, 0.02)


def polar_anchor(p):
    """Where a polar gantry's anchor holds its pedestal, in the machine's frame."""
    return (0.0, 0.0, p.pivot_z - 0.3)


def fleet(placed, machines_dir):
    """A machine-category file placing each robot: (name, file, base-frame
    at, yaw, anchor). A robot held to the world by a joint with an empty
    body has that joint's frame in world space, which a placement above it
    does not move: the fleet sets each anchor where its robot stands."""
    u = Usda("")
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, 'string creator = "scripts/mars_base/build.py: the robots that tend the base"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "Fleet"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: spawn every robot of the base at once with")
    u.emit(0, "# `gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base`.")
    u.emit(0, 'def Xform "Fleet"')
    u.emit(0, "{")
    for name, file, at, yaw, anchor in placed:
        u.emit(1, f'def Xform "{name}" (')
        u.emit(2, f"prepend references = @{machines_dir}/{file}@")
        u.emit(1, ")")
        u.emit(1, "{")
        u.emit(2, f"double3 xformOp:translate = {fmt(at)}")
        u.emit(2, f"float3 xformOp:rotateXYZ = (0, 0, {math.degrees(yaw):.4f})")
        u.emit(2, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
        if anchor is not None:
            ax, ay, az = anchor
            c, s = math.cos(yaw), math.sin(yaw)
            world = (at[0] + c * ax - s * ay, at[1] + s * ax + c * ay, at[2] + az)
            u.emit(2, 'over "Joints"')
            u.emit(2, "{")
            u.emit(3, 'over "anchor"')
            u.emit(3, "{")
            u.emit(4, f"point3f physics:localPos0 = {fmt(world)}")
            u.emit(4, f"quatf physics:localRot0 = ({math.cos(yaw / 2):.6f}, 0, 0, {math.sin(yaw / 2):.6f})")
            u.emit(3, "}")
            u.emit(2, "}")
        u.emit(1, "}")
    u.emit(0, "}")
    return u.text()
