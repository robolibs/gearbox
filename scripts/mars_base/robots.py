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

Their looks are built in Blender from `mechanics`' parts at their bodies'
sizes and written into a geometry layer each body's visual references; the
bodies collide as the hidden boxes and drums they are given here. Every
moving part stays in its own body: a trolley's wheels ride the rail's
flanges, a mast slides in its trolley's guide, an arm telescopes in its
turret's sleeve, each overlapping the other at both ends of its travel.
"""
import math
import os
from dataclasses import dataclass
from types import SimpleNamespace

import bpy
from mathutils import Matrix, Vector

import kit
import mechanics as mk
from kit import box, rod, tube, truss
from mechanics import ALONG, UP, Part, at, bar, cyl, tslot, turned
from usda import Usda, end, fmt, schemas, xform
from vehicles import bindings, export_geom, visual


def body(u, d, name, at_, mass, inertia, link=None, values=()):
    xform(u, d, name, at_, ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"], [
        *([f'token gearbox:link:name = "{link}"', 'token gearbox:link:role = "base"'] if link else []),
        f"float physics:mass = {mass}",
        f"float3 physics:diagonalInertia = {fmt(inertia)}",
        *values,
    ])


def hull(u, d, name, centre, size):
    """A hidden box the body collides as."""
    u.emit(d, f'def Cube "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, 'token visibility = "invisible"')
    u.emit(d + 1, "double size = 1")
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(d + 1, f"float3 xformOp:scale = {fmt(size)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:scale"]')
    u.emit(d, "}")


def drum(u, d, name, centre, radius, height):
    """A hidden upright cylinder the body collides as."""
    u.emit(d, f'def Cylinder "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, 'token visibility = "invisible"')
    u.emit(d + 1, 'uniform token axis = "Z"')
    u.emit(d + 1, f"double radius = {radius}")
    u.emit(d + 1, f"double height = {height}")
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate"]')
    u.emit(d, "}")


def sensor(u, d, name, kind, at_, rotate, attrs=()):
    u.emit(d, f'def Xform "{name}" (prepend apiSchemas = ["GearboxLinkAPI"])')
    u.emit(d, "{")
    u.emit(d + 1, 'token gearbox:link:role = "sensor"')
    u.emit(d + 1, f'token gearbox:sensor:kind = "{kind}"')
    for attr in attrs:
        u.emit(d + 1, attr)
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(at_)}")
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


def head_sensors(u, d, at_):
    """The tending head's camera (down, naming plants), light sensor (up) and climate sensor."""
    x, y, z = at_
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


def open_machine(u, kind, controllers, base, note, name):
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
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{name}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")


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


def rail_gantry(r, note, name, bound):
    """A rail gantry machine: rail (held to the world), trolley, mast, turret, arm."""
    u = Usda("/robot/Looks")
    open_machine(u, "rail_robot", ("travel", "lift", "turn", "reach"), "rail", note, name)
    s = r.scale
    body(u, 1, "rail", (r.rail_len / 2, 0, 0.02), 40 * s, (1, 100 * s, 100 * s), link="base_link")
    visual(u, 2, name, "rail", bound)
    end(u, 1)
    travel = r.rail_len - 0.4
    trolley_z = -0.07 * s
    body(u, 1, "trolley", (0.2, 0, trolley_z), 18 * s, (0.12 * s, 0.2 * s, 0.25 * s), values=[f"float gearbox:value:range = {travel:g}"])
    hull(u, 2, "carriage", (0, 0, 0), (0.34 * s, 0.24 * s, 0.12 * s))
    visual(u, 2, name, "trolley", bound)
    end(u, 1)
    body(u, 1, "mast", (0.2, 0, r.mast_z), 9 * s, (0.6 * s, 0.6 * s, 0.01), values=[f"float gearbox:value:range = {-r.lift}"])
    hull(u, 2, "column", (0, 0, 0), (0.07 * s, 0.07 * s, r.mast_len))
    visual(u, 2, name, "mast", bound)
    end(u, 1)
    body(u, 1, "turret", (0.2, 0, r.turret_z), 3 * s, (0.01, 0.01, 0.01), values=["float gearbox:value:range = 3.14159"])
    drum(u, 2, "hub", (0, 0, 0), 0.07 * s, 0.1 * s)
    visual(u, 2, name, "turret", bound)
    end(u, 1)
    body(u, 1, "arm", (0.2 + r.arm[0], r.arm[1], r.arm[2]), 2.5 * s, (0.04 * s, 0.005, 0.04 * s), values=[f"float gearbox:value:range = {-r.reach}"])
    hull(u, 2, "beam", (0, 0, 0), (0.05 * s, r.beam, 0.05 * s))
    hull(u, 2, "head", (0, r.head, -0.01), (0.09 * s, 0.07 * s, 0.07 * s))
    visual(u, 2, name, "arm", bound)
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


def polar_gantry(p, note, name, bound):
    """A dome's polar gantry: pedestal (held to the world), boom, carriage, mast."""
    u = Usda("/robot/Looks")
    open_machine(u, "polar_gantry", ("slew", "travel", "lift"), "pedestal", note, name)
    body(u, 1, "pedestal", (0, 0, p.pivot_z - 0.3), 300, (60, 60, 60), link="base_link")
    drum(u, 2, "drum", (0, 0, 0), 0.8, 0.6)
    visual(u, 2, name, "pedestal", bound)
    end(u, 1)
    span = p.leg_r + 1.8
    boom_mass = 500.0
    body(u, 1, "boom", (0, 0, p.pivot_z), boom_mass, (200, boom_mass * span * span / 3, boom_mass * span * span / 3),
         values=["float gearbox:value:range = 6.28319"])
    drum(u, 2, "slewing_ring", (0, 0, 0.25), 0.7, 0.5)
    hull(u, 2, "tower", (0, 0, 1.25), (0.5, 0.5, 1.5))
    hull(u, 2, "counterweight", (-1.4, 0, p.boom_z - 0.1), (0.8, 0.9, 0.9))
    leg = p.boom_z - (p.floor_z - p.pivot_z)
    hull(u, 2, "leg", (p.leg_r, 0, p.boom_z - leg / 2), (0.25, 0.25, leg))
    hull(u, 2, "bogie", (p.leg_r, 0, p.floor_z - p.pivot_z + 0.12), (0.7, 0.4, 0.22))
    visual(u, 2, name, "boom", bound)
    end(u, 1)
    body(u, 1, "carriage", (p.inner, 0, p.pivot_z + p.carriage_z), 40, (2, 2, 2), values=[f"float gearbox:value:range = {p.travel:g}"])
    hull(u, 2, "trolley", (0, 0, 0.08), (0.7, 0.8, 0.25))
    sleeve = p.pivot_z + p.carriage_z - p.head_z - 0.1
    hull(u, 2, "sleeve", (0, 0, -sleeve / 2), (0.22, 0.22, sleeve))
    visual(u, 2, name, "carriage", bound)
    end(u, 1)
    body(u, 1, "mast", (p.inner, 0, p.head_z), 12, (1, 1, 0.05), values=[f"float gearbox:value:range = {-p.lift}"])
    hull(u, 2, "head", (0, 0, -0.06), (0.28, 0.28, 0.12))
    visual(u, 2, name, "mast", bound)
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


# Looks


def turn(angle):
    return Matrix.Rotation(angle, 4, "Z")


def frame(part, key, c, inner, outer, depth, axis):
    """A square collar `outer` across round an opening `inner` across, `depth` long along `axis` ("X", "Y" or "Z")."""
    t = (outer - inner) / 2
    for u, v, su, sv in ((0, 1, outer, t), (0, -1, outer, t), (1, 0, t, inner), (-1, 0, t, inner)):
        off = (u * (inner + t) / 2, v * (inner + t) / 2)
        size = {"X": (depth, su, sv), "Y": (su, depth, sv), "Z": (su, sv, depth)}[axis]
        pos = {"X": (0, off[0], off[1]), "Y": (off[0], 0, off[1]), "Z": (off[0], off[1], 0)}[axis]
        part.add(key, box(size, Vector(c) + Vector(pos), tile=0.2))


def rail_frame(r):
    """The sizes a rail gantry's looks are drawn to, from its bodies', in the robot's frame at rest."""
    s = r.scale
    g = SimpleNamespace(s=s, x=0.2, travel=r.rail_len - 0.4)
    g.tz = -0.07 * s
    g.t_top, g.t_bot = g.tz + 0.06 * s, g.tz - 0.06 * s
    if r.draw_rail:
        g.z0, g.depth, g.web, g.flange, g.tf = 0.02, 0.12, 0.006, 0.07, 0.012
        g.cond = (0.21, 0.06)
    else:
        # EDEN's room draws this rail: 0.12 wide, 0.08 deep from z = 0, against the roof.
        g.z0, g.depth, g.web, g.flange, g.tf = 0.0, 0.08, 0.06, 0.082, 0.012
        g.cond = (0.17, -0.015)
    g.top = g.z0 + g.depth
    g.wr = 0.02 * s
    g.wy, g.ww = (g.web + g.flange) / 2, 0.75 * (g.flange - g.web)
    g.wz = g.z0 + g.tf + g.wr
    g.cy, g.ct = g.flange + 0.004, 0.008 * s
    g.bridge = g.t_top - 0.012 * s
    g.cheek = (g.bridge, g.wz + g.wr + 0.008 * s)
    g.half = 0.16 * s
    g.house = (g.t_bot + 0.002 * s, g.bridge)
    g.col = 0.07 * s
    g.slots = 2 if g.col >= 0.08 else 1
    g.c_top, g.c_bot = r.mast_z + r.mast_len / 2, r.turret_z + 0.05 * s
    g.ay, g.az = r.arm[1], r.arm[2] - r.turret_z
    g.bi = 0.05 * s
    g.so = g.bi + 0.014 * s
    g.front = r.arm[1] - r.beam / 2
    g.sf = g.front + 0.01
    g.yb = g.sf + r.reach + g.bi
    g.ys = 0.1 * s
    g.behind = g.c_top - r.lift - 0.15 * s < r.turret_z + 0.2 * s
    # The lift's guide behind the mast, clear of what the arm sweeps there.
    g.rw = 0.015 * s
    g.pw = 0.04 * s
    g.xr = g.x - (g.yb + 0.04 * s)
    g.xg = g.xr - 0.85 * g.rw - g.pw / 2
    g.za = (g.house[0] + g.house[1]) / 2
    g.bz = g.c_top - 0.045 * s
    return g


def rail_part(r, g):
    """The rail: running flanges, buffers at the ends of the travel, the
    conductor rail; where the robot hangs its own rail, the I beam and the
    hangers from the ridge girder."""
    s, length = g.s, r.rail_len
    p = Part("rail", (length / 2, 0, 0.02))
    cy, cz = g.cond
    if r.draw_rail:
        p.add("alu", bar((-0.3, 0, g.z0), (length + 0.3, 0, g.z0), mk.beam_outline(2 * g.flange, g.depth, g.tf), UP, 1.0))
        pieces = math.ceil((length + 0.6) / 6.0)
        for k in range(1, pieces):
            xs = -0.3 + k * (length + 0.6) / pieces
            for y in (-1, 1):
                p.add("black", box((0.24, 0.008, g.depth - 2 * g.tf - 0.01), (xs, y * (g.web / 2 + 0.004), g.z0 + g.depth / 2), tile=0.3))
                mk.bolts(p, at((xs, y * (g.web / 2 + 0.008), g.z0 + g.depth / 2), (0, y, 0)),
                         [(dx, dz) for dx in (-0.09, -0.03, 0.03, 0.09) for dz in (-0.025, 0.025)], 0.006)
        n = int(length / 3.0)
        hangers = [k * length / n for k in range(n + 1)]
        top = g.top + r.hangers
        for x in hangers:
            p.add("black", box((0.06 * s, 2 * g.flange + 0.03, 0.02), (x, 0, g.top + 0.01), tile=0.3))
            p.add("black", box((0.08 * s, 0.16, 0.03), (x, 0, top - 0.015), tile=0.3))
            for y in (-0.05, 0.05):
                p.add("steel", rod((x, y, g.top + 0.02), (x, y, top - 0.03), 0.008, 8))
                for z in (g.top + 0.02, top - 0.042):
                    p.add("steel", mk.hexagon(0.014, z, z + 0.012), Matrix.Translation((x, y, 0)))
        mounts = sorted({round(x, 4) for x in hangers} | {round((a + b) / 2, 4) for a, b in zip(hangers, hangers[1:])})
        mounts = [min(max(x, 0.18), length - 0.04) for x in mounts]
        to = g.top + 0.006
        for x in mounts:
            p.add("steel", box((0.024, cy + 0.021, 0.006), (x, (cy + 0.021) / 2, g.top + 0.003), tile=0.2))
    else:
        for sgn in (-1, 1):
            p.add("steel", box((length, g.flange - g.web, g.tf), (length / 2, sgn * (g.web + g.flange) / 2, g.z0 + g.tf / 2), tile=1.0))
            p.add("steel", box((length, 0.004, 0.03), (length / 2, sgn * (g.web + 0.002), g.z0 + 0.015), tile=1.0))
            mk.bolts(p, at((0, sgn * (g.web + 0.004), g.z0 + 0.02), (0, sgn, 0), (1, 0, 0)),
                     [(0.15 + k * 0.3, 0.0) for k in range(int(length / 0.3))], 0.005)
        mounts = [0.18 + k * (length - 0.22) / 6 for k in range(7)]
        to = g.top
        for x in mounts:
            p.add("steel", box((0.04, 0.05, 0.004), (x, cy + 0.01, to - 0.002), tile=0.2))
    mk.conductor(p, 0.02, length - 0.02, cy, cz, 0.03, 0.05, -1, mounts, to)
    for x_end, d in ((g.x - g.half, -1), (g.x + g.travel + g.half, 1)):
        xb = x_end + d * 0.04 * s
        p.add("black", box((0.035 * s, 2 * (g.cy + g.ct) + 0.02, g.top - g.bridge), (xb + d * 0.0175 * s, 0, (g.top + g.bridge) / 2), tile=0.3,
                           bevel=0.003))
        for y in (-1, 1):
            mk.bumper(p, at((xb, y * (g.cy + g.ct / 2), g.wz), (-d, 0, 0)), 0.012 * s, 0.04 * s)
    return p


def trolley_part(r, g):
    """The trolley: cheek plates round the rail's running flanges with four
    wheels riding them, the bridge under the rail and the drive's cover
    under it, a gear motor on the front wheels, the conductor rail's
    collector, a limit switch; and the guide the mast slides in."""
    s, x = g.s, g.x
    p = Part("trolley", (x, 0, g.tz))
    z0, z1 = g.cheek
    for y in (-1, 1):
        p.add("black", box((2 * g.half, g.ct, z1 - z0), (x, y * (g.cy + g.ct / 2), (z0 + z1) / 2), tile=0.3, bevel=0.002))
        mk.bolts(p, at((x, y * (g.cy + g.ct), z0 + 0.008 * s), (0, y, 0)), [(dx, 0.0) for dx in (-0.14 * s, 0.0, 0.14 * s)], 0.004 * s)
        for dx in (-0.11 * s, 0.11 * s):
            mk.wheel(p, at((x + dx, y * g.wy, g.wz), (0, 1, 0)), g.wr, g.ww)
            p.add("steel", rod((x + dx, y * g.wy, g.wz), (x + dx, y * (g.cy + g.ct + 0.004 * s), g.wz), 0.004 * s, 8))
            p.add("steel", mk.hexagon(0.007 * s, 0.0, 0.005 * s), at((x + dx, y * (g.cy + g.ct), g.wz), (0, y, 0)))
    p.add("black", box((2 * g.half + 0.01 * s, 2 * (g.cy + g.ct), g.t_top - g.bridge), (x, 0, (g.t_top + g.bridge) / 2), tile=0.3))
    lo, hi = g.house
    mk.cover(p, Matrix.Translation((x, 0, (lo + hi) / 2)), (0.3 * s, 0.21 * s, hi - lo), vents=7)
    gx, gy = x + 0.11 * s, -(g.cy + g.ct + 0.02 * s)
    p.add("black", box((0.05 * s, 0.04 * s, 0.05 * s), (gx, gy, g.wz), tile=0.2, bevel=0.003 * s))
    mk.motor(p, at((gx - 0.025 * s, gy, g.wz), (1, 0, 0)), 0.036 * s, 0.05 * s)
    mk.switch(p, at((x - g.half + 0.03 * s, -(g.cy + g.ct + 0.008 * s), z0 - 0.002), (0, 0, -1)), s)
    cy, cz = g.cond
    y0, y1 = g.cy + g.ct, cy - 0.027
    p.add("black", box((0.03 * s, y1 - y0, 0.012 * s), (x, (y0 + y1) / 2, cz), tile=0.2))
    p.add("plastic", box((0.05 * s, 0.016, 0.012 * s), (x, cy - 0.019, cz), tile=0.2, bevel=0.002))
    yc = (y0 + y1) / 2
    p.add("plastic", tube([(x - 0.02 * s, yc, cz - 0.006 * s), (x - 0.02 * s, yc, (lo + hi) / 2), (x - 0.02 * s, 0.105 * s, (lo + hi) / 2)], 0.004 * s, 8))
    p.add("plastic", cyl(0.007 * s, 0.0, 0.008 * s, 10), at((x - 0.02 * s, 0.105 * s, (lo + hi) / 2), (0, 1, 0)))
    if g.behind:
        guide_behind(p, r, g)
    else:
        guide_sleeve(p, r, g)
    return p


def guide_behind(p, r, g):
    """The lift's guide hanging behind the mast, past what the arm sweeps:
    a T-slot upright off a rear arm, a guide rail on its face, a toothed
    belt between its motor's pulley and the idler, buffers and switches."""
    s, x, pw = g.s, g.x, g.pw
    zt = g.za + pw / 2
    zb = g.c_top - r.lift - 0.14 * s
    p.add("alu", bar((x - 0.15 * s, 0, g.za), (g.xg + pw / 2, 0, g.za), tslot(pw), UP, 0.3))
    p.add("alu", bar((g.xg, 0, zt), (g.xg, 0, zb), tslot(pw), ALONG, 0.3))
    for z in (zt + 0.002, zb - 0.002):
        p.add("plastic", box((pw + 0.002, pw + 0.002, 0.004), (g.xg, 0, z), tile=0.2))
    for y in (-1, 1):
        p.add("black", box((0.025 * s, 0.004, 0.025 * s), (g.xg + pw / 2 + 0.0125 * s, y * (pw / 2 + 0.002), g.za - pw / 2 - 0.0125 * s), tile=0.2))
    foot = g.xg + pw / 2
    mk.guide_rail(p, (foot, 0, g.za - pw / 2 - 0.01), (foot, 0, zb + 0.01), g.rw, (1, 0, 0))
    mk.bumper(p, at((foot + 0.4 * g.rw, 0, zb + 0.004), UP), 0.007 * s, 0.018 * s)
    yb = pw / 2
    hz0, hz1 = g.za - pw / 2 - 0.04 * s, zt
    p.add("black", box((pw, 0.022 * s, hz1 - hz0), (g.xg, yb + 0.011 * s, (hz0 + hz1) / 2), tile=0.2, bevel=0.002))
    mk.motor(p, at((g.xg, yb + 0.022 * s, g.za - 0.01 * s), (0, -1, 0)), 0.042 * s, 0.04 * s)
    p.add("black", box((pw, 0.022 * s, 0.05 * s), (g.xg, yb + 0.011 * s, zb + 0.03 * s), tile=0.2, bevel=0.002))
    p.add("plastic", box((0.012 * s, 0.003, hz0 - (zb + 0.055 * s)), (g.xg, yb + 0.0015, (hz0 + zb + 0.055 * s) / 2), tile=0.3))
    for z in (hz0 - 0.03 * s, zb + 0.08 * s):
        mk.switch(p, at((g.xg - pw / 2 - 0.016 * s, -0.01 * s, z), (1, 0, 0)), 0.8 * s)
    cy = -(pw / 2 + 0.006 * s)
    p.add("plastic", tube([(x - 0.15 * s, cy, g.za), (g.xg, cy, g.za), (g.xg, cy, zb + 0.1 * s)], 0.004 * s, 8))


def guide_sleeve(p, r, g):
    """The lift's guide round the mast: a housing under the trolley, a
    collar with the bushing at its foot, and the pinion's gear motor."""
    s, x = g.s, g.x
    ho = g.col + 0.04 * s
    top = g.house[0]
    zh = g.c_top - r.lift - 0.2 * s
    p.add("black", box((ho + 0.04 * s, ho + 0.04 * s, 0.012 * s), (x, 0, top - 0.006 * s), tile=0.3))
    p.add("alu", bar((x, 0, top - 0.012 * s), (x, 0, zh + 0.05 * s), tslot(ho, 2), ALONG, 0.5))
    p.add("black", box((ho + 0.02 * s, ho + 0.02 * s, 0.05 * s), (x, 0, zh + 0.025 * s), tile=0.3, bevel=0.003 * s))
    frame(p, "plastic", (x, 0, zh - 0.003 * s), g.col + 0.002, ho, 0.006 * s, "Z")
    gx = x + ho / 2 + 0.03 * s
    p.add("black", box((0.04 * s, 0.06 * s, 0.05 * s), (gx, 0, zh + 0.025 * s), tile=0.2, bevel=0.003 * s))
    mk.motor(p, at((gx, 0.03 * s, zh + 0.025 * s), (0, -1, 0)), 0.042 * s, 0.06 * s)
    p.add("plastic", tube([(x + 0.05 * s, 0.105 * s, top), (x + 0.05 * s, ho / 2 + 0.006 * s, top - 0.03 * s),
                           (x + 0.05 * s, ho / 2 + 0.006 * s, zh + 0.07 * s), (gx, 0.05 * s, zh + 0.05 * s)], 0.005 * s, 8))


def mast_part(r, g):
    """The mast: a T-slot column with its cap and its foot's flange; behind,
    the carriage block, plate, cantilever, brace and belt clamp riding the
    guide; in a sleeve, the rack its pinion drives."""
    s, x, w = g.s, g.x, g.col
    p = Part("mast", (x, 0, r.mast_z))
    p.add("alu", bar((x, 0, g.c_bot), (x, 0, g.c_top), tslot(w, g.slots), ALONG, 0.5))
    p.add("plastic", box((w + 0.002, w + 0.002, 0.006 * s), (x, 0, g.c_top + 0.003 * s), tile=0.2))
    p.add("black", box((0.1 * s, 0.1 * s, 0.012 * s), (x, 0, g.c_bot + 0.006 * s), tile=0.3, bevel=0.002))
    if g.behind:
        foot = g.xr - 0.85 * g.rw
        mk.block(p, at((foot, 0, g.bz), (1, 0, 0), (0, 0, 1)), g.rw, 0.06 * s)
        face = foot + 1.205 * g.rw
        p.add("black", box((0.01 * s, 0.09 * s, 0.12 * s), (face + 0.005 * s, 0, g.c_top - 0.06 * s), tile=0.2, bevel=0.001))
        face += 0.01 * s
        p.add("alu", bar((face, 0, g.c_top - g.pw / 2), (x - w / 2, 0, g.c_top - g.pw / 2), tslot(g.pw), UP, 0.3))
        p.add("alu", bar((face, 0, g.c_top - 0.1 * s), (x - w / 2, 0, g.c_top - 0.42 * s), tslot(0.03 * s), (0, 1, 0), 0.3))
        for y in (-1, 1):
            p.add("black", box((0.04 * s, 0.004, 0.04 * s), (x - w / 2 - 0.02 * s, y * (g.pw / 2 + 0.002), g.c_top - g.pw - 0.02 * s), tile=0.2))
        y0 = g.pw / 2 + 0.003
        p.add("black", box((face - (g.xg - 0.01 * s), 0.01, 0.04 * s), ((face + g.xg - 0.01 * s) / 2, y0 + 0.005, g.bz), tile=0.2))
    else:
        zp = g.c_top - r.lift - 0.2 * s + 0.025 * s
        z0 = zp - 0.03 * s
        z1 = min(zp + r.lift + 0.03 * s, g.c_top - 0.02 * s)
        p.add("steel", mk.rack(z1 - z0, 0.02 * s, 0.012 * s, 0.01 * s), at((x + w / 2, 0, z0), (1, 0, 0), (0, 0, 1)))
    return p


def turret_part(r, g):
    """The turret: a hollow-shaft rotary actuator under the mast's foot, the
    clamp holding the arm's sleeve, the sleeve and its bushings, and the
    reach drive's gear motor over its pinion at the sleeve's mouth."""
    s, x = g.s, g.x
    p = Part("turret", (x, 0, r.turret_z))
    zs = r.turret_z + g.az
    st = zs + g.so / 2
    place = Matrix.Translation((x, 0, 0))
    p.add("black", cyl(0.066 * s, st + 0.014 * s, g.c_bot - 0.0005, 32), place)
    p.add("orange", cyl(0.0672 * s, g.c_bot - 0.018 * s, g.c_bot - 0.008 * s, 32), place)
    p.add("alu", cyl(0.06 * s, st + 0.004 * s, st + 0.014 * s, 32), place)
    for sx in (-1, 1):
        p.add("black", box((0.006 * s, 0.11 * s, st + 0.014 * s - (zs - g.so / 2 - 0.006 * s)), (x + sx * (g.so / 2 + 0.003 * s), 0,
                           (st + 0.014 * s + zs - g.so / 2 - 0.006 * s) / 2), tile=0.2, bevel=0.001))
    p.add("black", box((0.024 * s, 0.016 * s, 0.03 * s), (x, 0.066 * s + 0.008 * s, (st + g.c_bot) / 2 + 0.01 * s), tile=0.2, bevel=0.002))
    p.add("plastic", cyl(0.006 * s, 0.0, 0.012 * s, 8), at((x, 0.066 * s + 0.016 * s, (st + g.c_bot) / 2 + 0.01 * s), (0, 1, 0)))
    p.add("alu", bar((x, g.sf, zs), (x, g.ys, zs), mk.rect(g.so, g.so), UP, 0.4))
    for y in (g.sf - 0.004 * s, g.ys + 0.004 * s):
        frame(p, "plastic", (x, y, zs), g.bi + 0.001, g.so + 0.004 * s, 0.008 * s, "Y")
    yp = g.sf + 0.04 * s
    p.add("black", box((0.04 * s, 0.04 * s, 0.024 * s), (x, yp, st + 0.012 * s), tile=0.2, bevel=0.002 * s))
    mk.motor(p, at((x, yp, st + 0.024 * s), (0, 0, -1)), 0.028 * s, 0.035 * s)
    return p


def arm_part(r, g):
    """The arm: the inner T-slot beam, its rack in the top groove, and the
    tending head at its tip with camera, ring light, spray nozzles,
    gripper, light and climate sensors."""
    s, x = g.s, g.x
    ox, oy, oz = x + r.arm[0], r.arm[1], r.arm[2]
    p = Part("arm", (ox, oy, oz))
    p.add("alu", bar((ox, g.front, oz), (ox, g.yb, oz), tslot(g.bi), UP, 0.4))
    p.add("plastic", box((g.bi + 0.002, 0.006 * s, g.bi + 0.002), (ox, g.yb + 0.003 * s, oz), tile=0.2))
    y0, y1 = g.front + 0.03 * s, g.yb - 0.01
    p.add("steel", mk.rack(y1 - y0, 0.008 * s, 0.005 * s, 0.006 * s), at((ox, y0, oz + g.bi / 2 - 0.0065 * s), UP, (0, 1, 0)))
    hc = Vector((ox, oy + r.head, oz - 0.01))
    p.add("black", box((0.09 * s, 0.008 * s, 0.07 * s), (ox, g.front - 0.004 * s, hc.z), tile=0.2, bevel=0.002))
    body_c = hc + Vector((0, -0.004 * s, 0.0))
    mk.cover(p, Matrix.Translation(body_c), (0.084 * s, 0.062 * s, 0.06 * s), bevel=0.008 * s)
    bottom = hc.z - 0.03 * s
    mk.lens_ring(p, Matrix.Translation((ox, hc.y, bottom)), 0.008 * s, 0.024 * s)
    for sx in (-1, 1):
        mk.nozzle(p, at((ox + sx * 0.03 * s, hc.y - 0.024 * s, bottom), (-sx * 0.25, 0.35, 1.0)), s)
    mk.gripper(p, at((ox, body_c.y - 0.031 * s, hc.z - 0.008 * s), (0, -1, -0.6)), s)
    top = hc.z + 0.03 * s
    p.add("black", cyl(0.012 * s, top, top + 0.003 * s, 16), Matrix.Translation((ox, hc.y, 0)))
    p.add("paint", turned([(0.0, top + 0.003 * s), (0.009 * s, top + 0.003 * s), (0.008 * s, top + 0.008 * s), (0.0, top + 0.01 * s)], 16),
          Matrix.Translation((ox, hc.y, 0)))
    px = ox + 0.042 * s + 0.012 * s
    p.add("black", box((0.014 * s, 0.01 * s, 0.006 * s), (px - 0.007 * s, hc.y + 0.01 * s, hc.z + 0.016 * s), tile=0.2))
    p.add("paint", mk.louvred(0.009 * s, hc.z - 0.024 * s, hc.z + 0.013 * s), Matrix.Translation((px, hc.y + 0.01 * s, 0)))
    p.add("led", box((0.003, 0.012 * s, 0.006 * s), (ox + 0.042 * s + 0.0012, hc.y - 0.02 * s, hc.z + 0.018 * s), tile=0.2))
    return p


def rail_looks(r):
    """A rail gantry's links as Parts, in the robot's frame at rest."""
    g = rail_frame(r)
    return [rail_part(r, g), trolley_part(r, g), mast_part(r, g), turret_part(r, g), arm_part(r, g)]


def pedestal_part(p):
    """The pedestal on the cistern: a base flange bolted to its mounting
    ring, a gusseted drum, the fixed ring gear the slew drives walk round,
    an access hatch and the junction box feeding the slip ring."""
    q = Part("pedestal", (0, 0, p.pivot_z - 0.3))
    seat, zr = p.pivot_z - 0.595, p.pivot_z - 0.1
    q.add("paint", turned([(0.0, seat), (1.0, seat), (1.0, seat + 0.025), (0.72, seat + 0.025), (0.72, zr), (0.0, zr)], 64))
    q.add("steel", mk.gear(0.75, 0.79, zr, zr + 0.08, 96))
    q.add("black", cyl(0.6, zr + 0.08, zr + 0.085, 48))
    for k in range(16):
        a = 2 * math.pi * (k + 0.5) / 16
        q.add("steel", mk.hexagon(0.02, seat + 0.025, seat + 0.04), Matrix.Translation((0.9 * math.cos(a), 0.9 * math.sin(a), 0)))
    for k in range(8):
        gusset = kit.prism([(0.0, 0.0), (0.26, 0.0), (0.0, 0.3)], -0.008, 0.008, 0.3)
        q.add("paint", gusset, turn(2 * math.pi * k / 8) @ Matrix.Translation((0.72, 0, seat + 0.025)) @ Matrix.Rotation(math.pi / 2, 4, "X"))
    q.add("paint", box((0.03, 0.24, 0.3), (-0.72, 0, seat + 0.3), tile=0.3, bevel=0.004))
    for dy in (-0.09, 0.09):
        for dz in (-0.11, 0.11):
            q.add("steel", cyl(0.008, 0.0, 0.006, 8), at((-0.735, dy, seat + 0.3 + dz), (-1, 0, 0)))
    jb = turn(math.radians(135))
    q.add("grey", box((0.06, 0.16, 0.2), (0.75, 0, seat + 0.33), tile=0.3, bevel=0.005), jb)
    q.add("plastic", tube([jb @ Vector((0.76, 0.03, seat + 0.23)), jb @ Vector((0.76, 0.03, seat + 0.06)), jb @ Vector((0.9, 0.03, seat - 0.02))], 0.012, 8))
    return q


def boom_part(p):
    """The boom: the slewing bearing and turntable with two slew drives
    on the ring gear, the tower, the aluminium box truss with the
    carriage's track, rack, buffers and conductor rail under it, the
    control cabinet and counterweight at its root, the leg and the bogie
    that runs on the floor rail."""
    b = Part("boom", (0, 0, p.pivot_z))
    z = p.pivot_z
    zc = z + p.boom_z
    chord_lo = zc - 0.3 - 0.025
    b.add("black", mk.ring(0.72, 0.6, z - 0.02, z + 0.04, 48))
    b.add("paint", turned([(0.0, z + 0.04), (0.7, z + 0.04), (0.7, z + 0.46), (0.66, z + 0.5), (0.0, z + 0.5)], 48))
    for k in range(12):
        b.add("paint", box((0.05, 0.016, 0.3), (0.72, 0, z + 0.25), tile=0.3), turn(2 * math.pi * (k + 0.5) / 12))
    for sy in (-1, 1):
        c = Vector((0, sy * 0.84, 0))
        b.add("black", box((0.32, 0.3, 0.03), (0, sy * 0.77, z + 0.3), tile=0.3, bevel=0.004))
        b.add("steel", mk.gear(0.045, 0.058, z - 0.1, z - 0.02, 14), Matrix.Translation(c))
        b.add("black", cyl(0.06, z - 0.02, z + 0.285, 24), Matrix.Translation(c))
        mk.motor(b, at((c.x, c.y, z + 0.315), (0, 0, -1)), 0.1, 0.16)
    zt0, zt1 = z + 0.5, chord_lo
    b.add("black", box((0.6, 0.6, 0.02), (0, 0, zt0 + 0.01), tile=0.3))
    b.add("paint", box((0.46, 0.46, zt1 - zt0 - 0.035), (0, 0, (zt0 + 0.02 + zt1 - 0.015) / 2), tile=0.5, bevel=0.01))
    b.add("black", box((0.6, 0.8, 0.015), (0, 0, zt1 - 0.0075), tile=0.3))
    for k in range(4):
        gusset = kit.prism([(0.0, 0.0), (0.12 if k % 2 == 0 else 0.16, 0.0), (0.0, 0.4)], -0.008, 0.008, 0.3)
        b.add("paint", gusset, turn(math.pi / 2 * k) @ Matrix.Translation((0.23, 0, zt1 - 0.015)) @ Matrix.Rotation(-math.pi / 2, 4, "X"))
    x0, x1 = -1.0, p.leg_r + 0.15
    b.add("alu", truss((x0, 0, zc), (x1, 0, zc), 0.7, 0.6, 20, 0.025, 0.012))
    for k in range(4, 20, 4):
        xs = x0 + (x1 - x0) * k / 20
        for y in (-0.35, 0.35):
            for zz in (zc - 0.3, zc + 0.3):
                b.add("steel", cyl(0.032, -0.03, 0.03, 12), at((xs, y, zz), ALONG))
    for x in (x0 - 0.01, x1 + 0.01):
        b.add("black", box((0.02, 0.76, 0.66), (x, 0, zc), tile=0.4))
    bays = [x0 + (x1 - x0) * k / 20 for k in range(21)]
    zr = z + p.carriage_z
    xb0, xb1 = p.inner - 0.7, p.inner + p.travel + 0.7
    b.add("alu", bar((xb0, 0, zr), (xb1, 0, zr), mk.beam_outline(0.12, 0.07, 0.01), UP, 1.0))
    for x in bays:
        if xb0 < x < xb1:
            b.add("black", box((0.05, 0.1, chord_lo + 0.025 - (zr + 0.07)), (x, 0, (chord_lo + 0.025 + zr + 0.07) / 2), tile=0.2))
    b.add("steel", mk.rack(xb1 - xb0 - 0.1, 0.02, 0.014, 0.02), at((xb0 + 0.05, 0, zr), (0, 0, -1), ALONG))
    for x_end, d in ((p.inner - 0.3, -1), (p.inner + p.travel + 0.3, 1)):
        b.add("black", box((0.04, 0.2, 0.1), (x_end + d * 0.06, 0, zr + 0.035), tile=0.3, bevel=0.003))
        for y in (-1, 1):
            mk.bumper(b, at((x_end + d * 0.04, y * 0.068, zr + 0.03), (-d, 0, 0)), 0.012, 0.04)
    mounts = [x for x in bays if xb0 + 0.1 < x < xb1 - 0.05][::2]
    mk.conductor(b, xb0, xb1, -0.22, zr + 0.025, 0.03, 0.05, 1, mounts, chord_lo + 0.003)
    for x in mounts:
        b.add("steel", box((0.024, 0.115, 0.006), (x, -0.2985, chord_lo + 0.0), tile=0.2))
    tz = chord_lo + 0.025
    b.add("grey", box((xb0 + 0.06 - x0, 0.03, 0.04), ((x0 + xb0 + 0.06) / 2, -0.4, tz), tile=0.5))
    b.add("plastic", tube([(xb0 + 0.06, -0.4, tz - 0.02), (xb0 + 0.06, -0.4, zr + 0.025), (xb0 + 0.06, -0.26, zr + 0.025)], 0.01, 8))
    cab = (-1.79, -1.06)
    mid = sum(cab) / 2
    b.add("paint", box((cab[1] - cab[0], 0.88, 0.59), (mid, 0, zc + 0.045), tile=0.5, bevel=0.01))
    b.add("plastic", box((0.004, 0.01, 0.55), (cab[0] - 0.001, 0.0, zc + 0.045), tile=0.2))
    for sy in (-1, 1):
        b.add("black", box((0.02, 0.03, 0.14), (cab[0] - 0.01, sy * 0.05, zc + 0.05), tile=0.2, bevel=0.003))
        for k in range(6):
            b.add("plastic", box((0.08, 0.004, 0.012), (mid - 0.2, sy * 0.4415, zc - 0.12 + 0.03 * k), tile=0.2))
    for k in range(5):
        b.add("steel", box((cab[1] - cab[0] - 0.04 - 0.01 * (k % 2), 0.84, 0.052), (mid, 0, zc - 0.53 + 0.057 * k), tile=0.5, bevel=0.003))
    for sy in (-1, 1):
        b.add("black", box((0.05, 0.05, 0.9), (mid, sy * 0.445, zc - 0.1), tile=0.3))
        b.add("black", box((0.04, 0.12, 0.3), (cab[1] + 0.02, sy * 0.25, zc), tile=0.3))
    lamp = Matrix.Translation((-1.5, 0.32, zc + 0.34))
    b.add("black", cyl(0.03, 0.0, 0.02, 16), lamp)
    b.add("orange", cyl(0.025, 0.02, 0.06, 16), lamp)
    b.add("led", cyl(0.025, 0.06, 0.1, 16), lamp)
    b.add("black", cyl(0.027, 0.1, 0.11, 16), lamp)
    leg_x = p.leg_r
    base, head = p.floor_z + 0.205, chord_lo
    b.add("black", box((0.3, 0.8, 0.015), (leg_x, 0, head - 0.0075), tile=0.3))
    b.add("paint", bar((leg_x, 0, head - 0.015), (leg_x, 0, base + 0.015), mk.rect(0.2, 0.2), ALONG, 0.5))
    b.add("black", box((0.28, 0.28, 0.015), (leg_x, 0, base + 0.0075), tile=0.3))
    for sy in (-1, 1):
        b.add("paint", rod((leg_x - 0.7, sy * 0.35, head), (leg_x - 0.1, sy * 0.1, head - 0.8), 0.025, 10))
    bogie(b, p, leg_x, base)
    return b


def bogie(b, p, x, top):
    """The leg's bogie on the floor rail: side and top plates, two flanged
    wheels, side guide rollers on the rail head, sweepers and an encoder."""
    rail = p.floor_z - 0.03
    wz = rail + 0.075
    for sx in (-1, 1):
        b.add("paint", box((0.015, 0.42, top - 0.02 - (rail + 0.005)), (x + sx * 0.1275, 0, (top - 0.02 + rail + 0.005) / 2), tile=0.3, bevel=0.002))
        b.add("black", box((0.05, 0.06, 0.03), (x + sx * 0.095, 0, rail + 0.005), tile=0.2))
        b.add("plastic", cyl(0.015, rail - 0.02, rail, 14), Matrix.Translation((x + sx * 0.065, 0, 0)))
    for sy in (-1, 1):
        b.add("paint", box((0.27, 0.01, 0.05), (x, sy * 0.215, rail + 0.03), tile=0.2))
        b.add("plastic", box((0.12, 0.012, 0.02), (x, sy * 0.215, rail), tile=0.2))
    b.add("paint", box((0.27, 0.42, 0.02), (x, 0, top - 0.01), tile=0.3))
    profile = [(0.03, -0.045), (0.09, -0.045), (0.09, -0.032), (0.075, -0.028), (0.072, 0.03), (0.03, 0.03), (0.03, -0.045)]
    for sy in (-1, 1):
        m = at((x, sy * 0.12, wz), (1, 0, 0))
        b.add("steel", turned(profile, 28), m)
        b.add("steel", rod((x - 0.14, sy * 0.12, wz), (x + 0.14, sy * 0.12, wz), 0.02, 12))
    b.add("plastic", cyl(0.03, 0.0, 0.03, 16), at((x + 0.135, 0.12, wz), (1, 0, 0)))


def carriage_part(p):
    """The carriage: side plates round the track's lower flange with four
    wheels on it, the bridge, the drive's cover with the travel motor, the
    collector on the conductor rail, and the sleeve the mast slides in with
    its collar, bushing and the lift's gear motor."""
    c = Part("carriage", (p.inner, 0, p.pivot_z + p.carriage_z))
    x, z = p.inner, p.pivot_z + p.carriage_z
    for sy in (-1, 1):
        c.add("black", box((0.6, 0.008, 0.1), (x, sy * 0.068, z + 0.005), tile=0.3, bevel=0.002))
        for dx in (-0.22, 0.22):
            mk.wheel(c, at((x + dx, sy * 0.0325, z + 0.03), (0, 1, 0)), 0.02, 0.02)
            c.add("steel", rod((x + dx, sy * 0.0325, z + 0.03), (x + dx, sy * 0.076, z + 0.03), 0.004, 8))
            c.add("steel", mk.hexagon(0.008, 0.0, 0.006), at((x + dx, sy * 0.072, z + 0.03), (0, sy, 0)))
    c.add("black", box((0.61, 0.16, 0.015), (x, 0, z - 0.0525), tile=0.3))
    c.add("steel", mk.gear(0.01, 0.013, -0.01, 0.01, 12), at((x, 0, z - 0.026), (0, 1, 0)))
    lo, hi = z - 0.17, z - 0.06
    mk.cover(c, Matrix.Translation((x, 0, (lo + hi) / 2)), (0.6, 0.56, hi - lo), vents=9, bevel=0.015)
    c.add("black", box((0.08, 0.06, 0.07), (x + 0.12, 0.31, (lo + hi) / 2), tile=0.2, bevel=0.004))
    mk.motor(c, at((x + 0.12, 0.34, (lo + hi) / 2), (0, -1, 0)), 0.07, 0.07)
    c.add("black", box((0.03, 0.02, z + 0.025 - hi), (x - 0.12, -0.19, (z + 0.025 + hi) / 2), tile=0.2))
    c.add("black", box((0.03, 0.012, 0.012), (x - 0.12, -0.199, z + 0.025), tile=0.2))
    c.add("plastic", box((0.06, 0.016, 0.014), (x - 0.12, -0.211, z + 0.025), tile=0.2, bevel=0.002))
    bottom = p.head_z + 0.005
    c.add("black", box((0.26, 0.26, 0.015), (x, 0, lo - 0.0075), tile=0.3))
    c.add("alu", bar((x, 0, lo - 0.015), (x, 0, bottom + 0.06), tslot(0.2, 2), ALONG, 0.6))
    c.add("black", box((0.23, 0.23, 0.06), (x, 0, bottom + 0.03), tile=0.3, bevel=0.004))
    frame(c, "plastic", (x, 0, bottom + 0.003), 0.142, 0.2, 0.006, "Z")
    gx = x + 0.115 + 0.03
    c.add("black", box((0.06, 0.1, 0.07), (gx, 0, bottom + 0.035), tile=0.2, bevel=0.004))
    mk.motor(c, at((gx, 0.05, bottom + 0.035), (0, -1, 0)), 0.08, 0.12)
    c.add("plastic", tube([(x + 0.05, 0.1, lo - 0.015), (x + 0.05, 0.108, lo - 0.05), (x + 0.05, 0.108, bottom + 0.1), (gx, 0.108, bottom + 0.07)],
                          0.008, 8))
    return c


def polar_mast_part(p):
    """The polar gantry's mast: a T-slot tube with the lift's rack on its
    face, and the tending head at its foot."""
    m = Part("mast", (p.inner, 0, p.head_z))
    x, zh = p.inner, p.head_z
    top = p.pivot_z + p.carriage_z - 0.065
    m.add("alu", bar((x, 0, zh), (x, 0, top), tslot(0.14, 2), ALONG, 0.6))
    m.add("plastic", box((0.142, 0.142, 0.006), (x, 0, top + 0.003), tile=0.2))
    zp = p.head_z + 0.04
    z0, z1 = zp - 0.03, min(zp + p.lift + 0.03, top - 0.01)
    m.add("steel", mk.rack(z1 - z0, 0.03, 0.014, 0.014), at((x + 0.07, 0, z0), (1, 0, 0), (0, 0, 1)))
    k = 2.6
    m.add("black", box((0.18, 0.18, 0.012), (x, 0, zh - 0.006), tile=0.3))
    mk.cover(m, Matrix.Translation((x, 0, zh - 0.057)), (0.26, 0.26, 0.09), bevel=0.015)
    bottom = zh - 0.102
    mk.lens_ring(m, Matrix.Translation((x, 0, bottom)), 0.018, 0.055)
    for sx in (-1, 1):
        for sy in (-1, 1):
            mk.nozzle(m, at((x + sx * 0.09, sy * 0.09, bottom), (-sx * 0.2, -sy * 0.2, 1.0)), k)
    mk.gripper(m, at((x + 0.1, 0, bottom), (0, 0, -1), (0, 1, 0)), 1.6)
    dome = Matrix.Translation((x + 0.11, 0.11, 0))
    m.add("black", cyl(0.019, zh - 0.012, zh - 0.008, 16), dome)
    m.add("paint", turned([(0.0, zh - 0.008), (0.015, zh - 0.008), (0.013, zh + 0.0), (0.0, zh + 0.004)], 16), dome)
    m.add("black", box((0.02, 0.03, 0.012), (x, 0.14, zh - 0.03), tile=0.2))
    m.add("paint", mk.louvred(0.018, zh - 0.1, zh - 0.03), Matrix.Translation((x, 0.16, 0)))
    for sx in (-1, 1):
        m.add("led", box((0.003, 0.16, 0.008), (x + sx * 0.1312, 0, zh - 0.04), tile=0.2))
    return m


def polar_looks(p):
    """A polar gantry's links as Parts, in the robot's frame at rest."""
    return [pedestal_part(p), boom_part(p), carriage_part(p), polar_mast_part(p)]


def build(name, model, note, work):
    """Builds a robot's looks, writes them as `<name>_geom.usdc` and the
    machine as `<name>.usda` into `work`; returns the machine layer's path."""
    kept = (set(bpy.data.materials), set(bpy.data.images), set(bpy.data.meshes))
    mats = mk.materials(kit.Looks(os.path.join(work, "maps")))
    parts = rail_looks(model) if isinstance(model, Rail) else polar_looks(model)
    scene = bpy.data.collections.new(f"{name}_geom")
    bpy.context.scene.collection.children.link(scene)
    objs = [part.finish(mats, scene) for part in parts]
    geom = os.path.join(work, f"{name}_geom.usdc")
    export_geom(scene, geom)
    bound = bindings(geom)
    for obj in objs:
        bpy.data.objects.remove(obj)
    bpy.data.collections.remove(scene)
    for mesh in set(bpy.data.meshes) - kept[2]:
        bpy.data.meshes.remove(mesh)
    for mat in set(bpy.data.materials) - kept[0]:
        bpy.data.materials.remove(mat)
    for image in set(bpy.data.images) - kept[1]:
        bpy.data.images.remove(image)
    text = rail_gantry(model, note, name, bound) if isinstance(model, Rail) else polar_gantry(model, note, name, bound)
    path = os.path.join(work, f"{name}.usda")
    with open(path, "w") as f:
        f.write(text)
    return path


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
    for name, file, at_, yaw, anchor in placed:
        u.emit(1, f'def Xform "{name}" (')
        u.emit(2, f"prepend references = @{machines_dir}/{file}@")
        u.emit(1, ")")
        u.emit(1, "{")
        u.emit(2, f"double3 xformOp:translate = {fmt(at_)}")
        u.emit(2, f"float3 xformOp:rotateXYZ = (0, 0, {math.degrees(yaw):.4f})")
        u.emit(2, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
        if anchor is not None:
            ax, ay, az = anchor
            c, s = math.cos(yaw), math.sin(yaw)
            world = (at_[0] + c * ax - s * ay, at_[1] + s * ax + c * ay, at_[2] + az)
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
