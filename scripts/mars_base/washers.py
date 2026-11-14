"""The robots that keep the farm's light coming in. Dust settles on every
glazed roof and on the solar field feeding the grow lamps; with no water to
wash it off, they brush it loose and blow it away with compressed Mars air.

    arch washer   a portal over an arch greenhouse's ETFE vault on crane
                  rails either side, after the gutter-riding roof washers of
                  Venlo glasshouses and the brush carriages of glazed barrel
                  vaults: a carriage sweeps round the portal's arched girder
                  on rack and pinion, pressing a rotating brush shaped to a
                  cushion onto one bay at a time between two air knives fed
                  by its own scroll compressor; the portal runs on batteries
                  and docks past the EVA gable on a charging bar.
    dome washer   a meridian arch over a geodesic dome, as rotating dome
                  maintenance gantries are built: its top slews on a bearing
                  on the crown's pedestal, its foot's bogie drives round a
                  ring girder clear over the portal and takes power from its
                  enclosed conductor rail; a carriage runs down the arch on
                  rack and pinion with a brush along the meridian.
    panel washer  after Ecoppia's waterless robots: a beam across a solar
                  row's slope riding rails along its top and bottom edges on
                  driven wheels and side guide wheels, a microfibre helix
                  brush spinning under a hood, its own PV panel and battery;
                  it parks past the row's end, its brush flicked clean on
                  the dock's comb.

Every joint has a motor; a robot's positioning joints have a
`builtin:joint_position` controller of their name (a position 0..1 of the
range), its brush a `builtin:joint_velocity` one. Each machine's frame is
its module's (a row's for a panel washer) and it is held to the world by a
joint with an empty body at `arch_anchor`, `dome_anchor` or `panel_anchor`.
Looks are built from `mechanics` parts into a geometry layer each body's
visual references; the bodies collide as hidden boxes, clear of their
module's colliders over their whole travel. The track sizes come from the
modules' own `buildings.arch_washer_track`, `buildings.dome_washer_ring`
and `power.panel_washer_track`, so each machine fits what it runs on.
"""
import math
import os
from types import SimpleNamespace

import bmesh
import bpy
from mathutils import Matrix, Quaternion, Vector

import buildings
import kit
import mechanics as mk
import patterns
import power
from industry import heads
from kit import box, rod, tube
from mechanics import UP, Part, at, bar, cyl, turned
from robots import body, sensor
from usda import Usda, end, fmt, schemas
from vehicles import bindings, export_geom, visual

X, Y, Z = Vector((1, 0, 0)), Vector((0, 1, 0)), Vector((0, 0, 1))

# Key: (material name, sRGB colour, roughness, metal, map settings).
EXTRA = {
    "bristle": ("washer_bristle", (0.13, 0.12, 0.11), 0.85, 0.0, {"seed": 71, "grime": 0.3}),
    "fibre": ("washer_microfibre", (0.22, 0.38, 0.56), 0.95, 0.0, {"seed": 72, "grime": 0.35}),
    "yellow": ("washer_yellow", (0.92, 0.68, 0.08), 0.5, 0.0, {"seed": 73, "grime": 0.08}),
    "red": ("washer_red", (0.72, 0.06, 0.04), 0.4, 0.0, {"seed": 74, "grime": 0.04}),
    "copper": ("washer_copper", (0.80, 0.50, 0.32), 0.3, 1.0, {"seed": 75, "grime": 0.1}),
    "tank": ("washer_tank", (0.86, 0.87, 0.85), 0.35, 0.0, {"seed": 76, "grime": 0.05}),
}


def materials(looks):
    """The gantries' finishes and the washers' own: brush fibres, safety
    colours, contacts, tanks, PV cells and an amber beacon."""
    m = mk.materials(looks)
    m.update({key: looks.get(name, colour, rough, metal, maps=mk.finish(colour, rough, **maps)) for key, (name, colour, rough, metal, maps) in EXTRA.items()})
    m["pv"] = looks.get("washer_pv", (0.05, 0.08, 0.20), 0.15, 0.3, maps=patterns.solar)
    m["amber"] = looks.get("washer_amber", (1.0, 0.55, 0.1), 0.3, 0.0, emit=((1.0, 0.55, 0.1), 8.0))
    return m


# The machine layer


def open_machine(u, kind, controllers, base, note, name):
    """The machine's header and its root, `controllers` as (joint, type)."""
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "scripts/mars_base/washers.py: {note}"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/washers.py and build again.")
    u.emit(0, "# A positioning joint takes a position 0..1 of its range: `gearbox machine set-value <joint> position 0.5 --machine <id>`;")
    u.emit(0, "# a brush a velocity in rad/s: `gearbox machine set-value brush velocity 12 --machine <id>`.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'PhysicsArticulationRootAPI', *(f'GearboxControllerAPI:{c}' for c, _ in controllers)])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, f'token gearbox:machine:kind = "{kind}"')
    u.emit(1, f"rel gearbox:machine:body = </robot/{base}>")
    for c, kind_ in controllers:
        u.emit(1, f'token gearbox:controller:{c}:type = "builtin:{kind_}"')
        u.emit(1, f"rel gearbox:controller:{c}:target = </robot/Joints/{c}>")
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{name}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")


def hull(u, d, name, centre, size, rotate=(0.0, 0.0, 0.0)):
    """A hidden box the body collides as, turned `rotate` degrees about X, Y, Z."""
    u.emit(d, f'def Cube "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, 'token visibility = "invisible"')
    u.emit(d + 1, "double size = 1")
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(d + 1, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
    u.emit(d + 1, f"float3 xformOp:scale = {fmt(size)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ", "xformOp:scale"]')
    u.emit(d, "}")


def joint(u, d, kind, name, body0, body1, pos0, axis=None, limits=None, motor=None, rot=None):
    """A joint, its frame at `pos0` in body0 (the stage's when body0 is
    None) and at body1's origin, both turned by the quaternion `rot`."""
    u.emit(d, f'def Physics{kind}Joint "{name}"')
    u.emit(d, "{")
    if body0:
        u.emit(d + 1, f"rel physics:body0 = </robot/{body0}>")
    u.emit(d + 1, f"rel physics:body1 = </robot/{body1}>")
    if axis:
        u.emit(d + 1, f'uniform token physics:axis = "{axis}"')
    u.emit(d + 1, f"point3f physics:localPos0 = {fmt(pos0)}")
    u.emit(d + 1, "point3f physics:localPos1 = (0, 0, 0)")
    if rot is not None:
        q = f"({rot.w:.6f}, {rot.x:.6f}, {rot.y:.6f}, {rot.z:.6f})"
        u.emit(d + 1, f"quatf physics:localRot0 = {q}")
        u.emit(d + 1, f"quatf physics:localRot1 = {q}")
    if limits:
        u.emit(d + 1, f"float physics:lowerLimit = {limits[0]:g}")
        u.emit(d + 1, f"float physics:upperLimit = {limits[1]:g}")
    if motor:
        u.emit(d + 1, f"float gearbox:motor:maxForce = {motor[0]}")
        u.emit(d + 1, f"float gearbox:motor:maxVelocity = {motor[1]}")
    u.emit(d, "}")


def camera(u, d, at_, rotate, range_m):
    """The robot's one camera, watching what it cleans."""
    sensor(u, d, "camera_link", "camera", at_, rotate, [
        'token gearbox:sensor:render = "geometry"',
        "int gearbox:sensor:width = 160",
        "int gearbox:sensor:height = 120",
        "float gearbox:sensor:vfov_deg = 60",
        f"float gearbox:sensor:range_m = {range_m}",
        "float gearbox:sensor:rate_hz = 1",
    ])


def looking(d):
    """rotateXYZ degrees turning a camera's +X along the direction d."""
    d = Vector(d).normalized()
    return (0.0, math.degrees(math.asin(-d.z)), math.degrees(math.atan2(d.y, d.x)))


# Geometry


def swept(outline, frames, tile=0.5, caps=True):
    """A closed (u, v) outline carried through frames of (origin, u axis, v
    axis), capped, its faces turned out."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    rings = [[bm.verts.new(o + du * pu + dv * pv) for pu, pv in outline] for o, du, dv in frames]
    run = [0.0]
    for (a, _, _), (c, _, _) in zip(frames, frames[1:]):
        run.append(run[-1] + (c - a).length)
    n = len(outline)
    for k in range(len(frames) - 1):
        for i in range(n):
            j = (i + 1) % n
            face = bm.faces.new((rings[k][i], rings[k][j], rings[k + 1][j], rings[k + 1][i]))
            for loop, (p, q) in zip(face.loops, ((i, k), (j, k), (j, k + 1), (i, k + 1))):
                loop[uv].uv = (p / n * 0.3 / tile, run[q] / tile)
    if caps:
        for ring in (rings[0], rings[-1]):
            bm.faces.new(ring)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


def arc_frames(centre, radial, axial, radius, angles, offset=0.0):
    """Frames round an arc about `centre`: at each angle the point `radius`
    out along `radial(angle)` and `offset` along `axial`, u radial, v axial."""
    return [(centre + radial(a) * radius + axial * offset, radial(a), axial) for a in angles]


def span(a0, a1, step):
    """Angles from a0 to a1 no more than `step` apart."""
    n = max(1, math.ceil(abs(a1 - a0) / step))
    return [a0 + (a1 - a0) * i / n for i in range(n + 1)]


def tufts(x0, x1, step, per, core, length, w=0.014):
    """A brush's bristle tufts along its X from x0 to x1: rings `step`
    apart of `per` tapered square stalks from the core out to `length(x)`,
    each ring turned half a tuft on."""
    bm = bmesh.new()
    n = int(round((x1 - x0) / step))
    for k in range(n + 1):
        x = x0 + (x1 - x0) * k / n
        reach = length(x)
        for i in range(per):
            a = 2 * math.pi * (i + 0.5 * (k % 2)) / per
            d, t = Vector((0, math.cos(a), math.sin(a))), Vector((0, -math.sin(a), math.cos(a)))
            ring = []
            for r, h in ((core * 0.95, w / 2), (reach, w * 0.32)):
                c = Vector((x, 0, 0)) + d * r
                ring.append([bm.verts.new(c + X * sx * h + t * sy * h) for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))])
            lo, hi = ring
            for j in range(4):
                bm.faces.new((lo[j], lo[(j + 1) % 4], hi[(j + 1) % 4], hi[j]))
            bm.faces.new(hi)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    kit.project_uv(bm, 0.2)
    return bm


def helix_fins(x0, x1, fins, turns, core, reach, thick=0.006, steps=160):
    """Microfibre fins wound round a brush's X axis from x0 to x1: `fins`
    flaps from the core out to `reach`, each `turns` turns along it."""
    bm = bmesh.new()
    for f in range(fins):
        sections = []
        for i in range(steps + 1):
            x = x0 + (x1 - x0) * i / steps
            a = 2 * math.pi * (f / fins + turns * i / steps)
            d, t = Vector((0, math.cos(a), math.sin(a))), Vector((0, -math.sin(a), math.cos(a)))
            c = Vector((x, 0, 0))
            sections.append([bm.verts.new(c + d * r + t * s * thick / 2) for r, s in ((core, -1), (reach, -1), (reach, 1), (core, 1))])
        for a, c in zip(sections, sections[1:]):
            for j in range(4):
                bm.faces.new((a[j], a[(j + 1) % 4], c[(j + 1) % 4], c[j]))
        bm.faces.new(sections[0][::-1])
        bm.faces.new(sections[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    kit.project_uv(bm, 0.2)
    return bm


def flanged_wheel(part, m, r, w, flange):
    """A double-flanged rail wheel on an axle along the builder's Z, its
    tread `w` wide at radius r, flanges out to `flange`."""
    h = w / 2
    profile = [(0.0, -h - 0.012), (flange, -h - 0.012), (flange, -h), (r, -h + 0.006), (r, h - 0.006), (flange, h), (flange, h + 0.012), (0.0, h + 0.012)]
    part.add("steel", turned(profile, 24), m)
    part.add("black", cyl(0.35 * r, -h - 0.03, h + 0.03, 12), m)
    mk.bolts(part, Matrix(m) @ Matrix.Translation((0, 0, h + 0.012)), [(0.24 * r * math.cos(k * math.pi / 3), 0.24 * r * math.sin(k * math.pi / 3)) for k in range(6)],
             0.012 * r / 0.15)


def v_wheel(part, m, r, w):
    """A V-groove guide wheel on an axle along the builder's Z."""
    h = w / 2
    profile = [(0.0, -h), (r, -h), (r, -0.25 * h), (0.75 * r, 0.0), (r, 0.25 * h), (r, h), (0.0, h)]
    part.add("plastic", turned(profile, 18), m)
    part.add("steel", cyl(0.3 * r, -h - 0.004, h + 0.004, 10), m)


def gusset(w, h, t=0.016):
    """A tapered gusset plate `w` wide at its foot and `h` tall, in the YZ
    plane about the origin, `t` thick along X."""
    bm = kit.prism([(-w / 2, 0.0), (w / 2, 0.0), (0.12 * w, h), (-0.12 * w, h)], -t / 2, t / 2, 0.3)
    bm.transform(Matrix(((0, 0, 1, 0), (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 0, 1))))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


def beacon(part, m, r):
    """An amber beacon on the builder's +Z: base, lens and cap."""
    part.add("black", cyl(1.2 * r, 0.0, 0.3 * r, 16), m)
    part.add("amber", cyl(r, 0.3 * r, 1.6 * r, 16), m)
    part.add("black", cyl(1.05 * r, 1.6 * r, 1.8 * r, 16), m)


def estop(part, m, k):
    """An emergency stop on the builder's +Z: its yellow box and red mushroom head."""
    part.add("yellow", box((0.07 * k, 0.07 * k, 0.05 * k), (0, 0, 0.025 * k), tile=0.2, bevel=0.005 * k), m)
    part.add("red", turned([(0.0, 0.05 * k), (0.018 * k, 0.05 * k), (0.018 * k, 0.062 * k), (0.03 * k, 0.07 * k), (0.03 * k, 0.08 * k), (0.0, 0.085 * k)], 16), m)


def battery(part, m, size):
    """A battery pack about the builder's origin: cabinet, vent slots on its
    ±Y faces, a handle and label on +X, its terminal glands on top."""
    w, d, h = size
    mk.cover(part, m, size, key="grey", vents=5, bevel=0.01)
    part.add("steel", rod((w / 2 + 0.012, -0.25 * d, 0.1 * h), (w / 2 + 0.012, 0.25 * d, 0.1 * h), 0.008, 8), m)
    for y in (-0.25 * d, 0.25 * d):
        part.add("steel", box((0.024, 0.012, 0.012), (w / 2 + 0.006, y, 0.1 * h), tile=0.2), m)
    part.add("yellow", box((0.003, 0.4 * d, 0.25 * h), (w / 2 + 0.0015, 0, -0.22 * h), tile=0.2), m)
    for x in (-0.25 * w, 0.25 * w):
        part.add("plastic", cyl(0.012, h / 2, h / 2 + 0.02, 10), Matrix(m) @ Matrix.Translation((x, 0, 0)))


def camera_head(part, m, k):
    """A small camera looking along the builder's -Z: housing, lens and a
    dust wiper on its window."""
    part.add("grey", box((0.06 * k, 0.05 * k, 0.07 * k), (0, 0, 0.035 * k), tile=0.2, bevel=0.005 * k), m)
    mk.lens_ring(part, m, 0.01 * k, 0.018 * k)
    part.add("black", box((0.004 * k, 0.035 * k, 0.003 * k), (0.012 * k, 0, -0.008 * k), tile=0.2), m)


# The arch washer


def arch(length, width=10.0, bay=3.0):
    """An arch washer's sizes for a greenhouse `length` long and `width`
    wide, its arches `bay` apart, in the greenhouse's frame: radii from the
    vault's axis on (y 0, z curb), angles `a` from the -Y curb over the top."""
    t = buildings.arch_washer_track(width, length)
    r = width / 2
    g = SimpleNamespace(length=length, r=r, curb=buildings.arch_curb(width), track=t)
    g.bays = int(round(length / bay))
    g.bay = length / g.bays
    g.x, g.first = t.dock, g.bay / 2
    g.span = g.x - g.first
    g.top = t.head + 0.32
    g.ri, g.ro, g.half = r + 1.35, r + 1.7, 0.45
    g.foot = math.asin((g.top + 0.03 - g.curb) / g.ri)
    g.plate = r + 1.25
    g.cond = r + 1.295
    g.cp = (r + 1.03, r + 1.06)
    g.core, g.bristle, g.shape, g.brush_half = 0.05, 0.15, 0.15, 1.3
    ends = g.bristle + g.shape * (1 - math.sin(math.pi * (0.5 - g.brush_half / g.bay)))
    g.axis = r + t.crest + t.strap + 0.025 + ends
    g.beam = (g.axis + 0.27, g.axis + 0.35)
    g.lo, g.hi = math.radians(10.0), math.radians(170.0)
    deepest = r + t.crest * math.sin(g.lo) ** 0.5 - 0.03 + g.bristle
    g.stroke = round(g.axis - deepest + 0.012, 3)
    g.guide = 0.58
    g.column = round(g.cp[1] + 0.31 + g.stroke + 0.02 - g.beam[1] - 0.02, 3)
    return g


def arch_press(g, a):
    """How far the arch washer's head presses in at sweep angle `a` to lay
    its brush 3 cm into the cushion's crest."""
    t = g.track
    return g.axis - (g.r + t.crest * max(math.sin(a), 0.0) ** 0.5 - 0.03 + g.bristle)


def radial_x(a):
    return Vector((0.0, -math.cos(a), math.sin(a)))


def arch_local(g, a, radius, x=None):
    """The frame at angle `a` and `radius` round the vault's axis at the
    portal's plane: X along the vault, Y on round the arch, Z out."""
    m = Matrix.Rotation(math.pi / 2 - a, 4, "X")
    m.translation = Vector((g.x if x is None else x, 0.0, g.curb)) + radial_x(a) * radius
    return m


def girder_part(g):
    """The portal: the arched box truss of four chords and their lacing over
    the vault, the carriage's V-edged track plate, rack and conductor rail
    under its inner face, end stops and flags; the feet gusseted onto two
    bogies, each on two double-flanged wheels with a gear motor, hold-down
    claws, rail sweepers, buffers, limit switches and a battery pack; the
    charging shoes and the position reader on the +Y bogie, the control
    cabinet on the -Y one, the beacon on the crown."""
    t, x = g.track, g.x
    p = Part("portal", (x, 0.0, g.curb))
    c = Vector((x, 0.0, g.curb))
    angles = span(g.foot, math.pi - g.foot, math.radians(2.0))
    for radius in (g.ri, g.ro):
        for s in (-1, 1):
            p.add("alu", swept(kit.circle(0.045, 10), arc_frames(c, radial_x, X, radius, angles, s * g.half)))
    nodes = span(g.foot, math.pi - g.foot, math.radians(6.4))
    for k, a in enumerate(nodes):
        for s in (-1, 1):
            p.add("alu", rod(c + radial_x(a) * g.ri + X * s * g.half, c + radial_x(a) * g.ro + X * s * g.half, 0.022, 6))
        for radius in (g.ri, g.ro):
            p.add("alu", rod(c + radial_x(a) * radius - X * g.half, c + radial_x(a) * radius + X * g.half, 0.02, 6))
        if k + 1 < len(nodes):
            b = nodes[k + 1]
            for s in (-1, 1):
                lo, hi = (a, b) if (k % 2 == 0) == (s > 0) else (b, a)
                p.add("alu", rod(c + radial_x(lo) * g.ri + X * s * g.half, c + radial_x(hi) * g.ro + X * s * g.half, 0.018, 6))
            p.add("alu", rod(c + radial_x(a) * g.ro - X * g.half, c + radial_x(b) * g.ro + X * g.half, 0.016, 6))
    # The carriage's track plate on standoffs, its V edges, the rack and the conductor rail.
    tp = span(g.lo - math.radians(5.0), g.hi + math.radians(5.0), math.radians(1.0))
    p.add("steel", swept(mk.rect(0.32, 0.012), arc_frames(c, radial_x, X, g.plate, tp)))
    for s in (-1, 1):
        p.add("steel", swept(kit.circle(0.012, 8), arc_frames(c, radial_x, X, g.plate, tp, s * 0.16)))
    p.add("steel", swept([(-0.008, -0.02), (-0.008, 0.02), (0.0, 0.02), (0.0, -0.02)], arc_frames(c, radial_x, X, g.plate - 0.006, tp)))
    teeth = int((tp[-1] - tp[0]) * (g.plate - 0.03) / 0.03)
    rack = bmesh.new()
    for k in range(teeth):
        a = tp[0] + (tp[-1] - tp[0]) * (k + 0.5) / teeth
        kit.merge(rack, box((0.04, 0.012, 0.021), (0, 0, -0.0245), tile=0.1), arch_local(g, a, g.plate))
    p.add("steel", rack)
    lift = g.cond - g.plate
    for a in nodes:
        if tp[0] < a < tp[-1]:
            p.add("black", box((0.05, 0.04, g.ri - g.plate - 0.03), (0, 0, (g.ri - g.plate) / 2), tile=0.2), arch_local(g, a, g.plate))
            p.add("black", box((0.06, 0.03, 0.04), (0.3, 0, lift + 0.035), tile=0.2), arch_local(g, a, g.plate))
    p.add("grey", swept(mk.conductor_outline(0.03, 0.05, -1), [(o + X * 0.3, X, du) for o, du, _ in arc_frames(c, radial_x, X, g.cond, tp)]))
    for a, d in ((tp[0], 1), (tp[-1], -1)):
        m = arch_local(g, a, g.plate)
        p.add("black", box((0.22, 0.04, 0.06), (0, d * 0.02, -0.04), tile=0.2, bevel=0.003), m)
        for s in (-1, 1):
            mk.bumper(p, m @ at((s * 0.08, d * 0.04, -0.045), (0, d, 0)), 0.014, 0.04)
        p.add("yellow", box((0.012, 0.06, 0.05), (-0.22, d * 0.06, -0.05), tile=0.2), m)
    # The feet and their bogies.
    for s in (-1, 1):
        y = s * t.y
        a = g.foot if s < 0 else math.pi - g.foot
        foot = c + radial_x(a) * (g.ri + g.ro) / 2
        p.add("black", box((2 * g.half + 0.12, 0.44, 0.06), (x, y, g.top + 0.03), tile=0.3, bevel=0.004))
        for sx in (-1, 1):
            p.add("paint", gusset(0.42, foot.z + 0.42 - g.top - 0.06), Matrix.Translation((x + sx * (g.half + 0.065), y, g.top + 0.06)))
        bogie(p, g, x, y, s)
    cab = Matrix.Translation((x + 0.66, -t.y, g.top + 0.25))
    mk.cover(p, cab, (0.2, 0.42, 0.5), key="paint", vents=4, bevel=0.012)
    p.add("plastic", rod((x + 0.66, -t.y + 0.1, g.top + 0.5), (x + 0.66, -t.y + 0.1, g.top + 0.92), 0.006, 6))
    p.add("black", cyl(0.012, g.top + 0.92, g.top + 0.94, 8), Matrix.Translation((x + 0.66, -t.y + 0.1, 0)))
    estop(p, cab @ at((0.1, -0.12, 0.05), (1, 0, 0)), 1.0)
    p.add("plastic", tube([Vector((x + 0.6, -t.y - 0.1, g.top + 0.03)), Vector((x + 0.35, -t.y - 0.1, g.top + 0.03)), Vector((x + 0.2, -t.y - 0.2, g.top + 0.01)),
                           Vector((x + 0.2, -t.y - 0.24, g.top - 0.015))], 0.012, 8))
    crown = c + radial_x(math.pi / 2) * (g.ro + 0.045)
    p.add("black", box((2 * g.half + 0.1, 0.2, 0.02), tuple(crown + Z * 0.01), tile=0.3))
    beacon(p, Matrix.Translation(crown + Z * 0.02 + X * 0.3), 0.05)
    return p


def bogie(p, g, x, y, s):
    """One of the portal's bogies on its rail at (x, y): side and end
    plates, two double-flanged wheels, the drive's gear motor on the outer
    side, hold-down claws under the rail head, sweepers, buffers, limit
    switches and a battery pack outside; on the +Y bogie the charging shoes
    and the position reader inside."""
    t = g.track
    hz, wz = t.head, t.head + 0.15
    for dy in (-0.12, 0.12):
        p.add("paint", box((1.44, 0.016, g.top - hz - 0.01), (x, y + dy, (g.top + hz + 0.01) / 2), tile=0.3, bevel=0.003))
    p.add("paint", box((1.48, 0.27, 0.02), (x, y, g.top - 0.01), tile=0.3))
    for dx in (-1, 1):
        p.add("paint", box((0.02, 0.26, g.top - hz - 0.01), (x + dx * 0.73, y, (g.top + hz + 0.01) / 2), tile=0.3))
        mk.bumper(p, at((x + dx * 0.74, y, wz), (dx, 0, 0)), 0.05, 0.12)
        p.add("plastic", box((0.01, 0.075, 0.04), (x + dx * 0.68, y, hz + 0.022), tile=0.2))
        p.add("black", box((0.03, 0.09, 0.03), (x + dx * 0.68, y, hz + 0.055), tile=0.2))
        for dy in (-1, 1):
            p.add("black", box((0.12, 0.09, 0.014), (x + dx * 0.3, y + dy * 0.065, hz - 0.055), tile=0.2))
            p.add("black", box((0.12, 0.015, 0.075), (x + dx * 0.3, y + dy * 0.105, hz - 0.017), tile=0.2))
        mk.switch(p, at((x + dx * 0.62, y + s * 0.17, hz + 0.015), (0, 0, -1)), 2.5)
        p.add("black", box((0.09, 0.07, 0.012), (x + dx * 0.62, y + s * 0.17, hz + 0.096), tile=0.2))
        p.add("black", box((0.09, 0.012, 0.07), (x + dx * 0.62, y + s * 0.134, hz + 0.065), tile=0.2))
    for dx in (-0.5, 0.5):
        flanged_wheel(p, at((x + dx, y, wz), (0, 1, 0)), 0.15, 0.057, 0.175)
        p.add("steel", rod((x + dx, y - 0.15, wz), (x + dx, y + 0.15, wz), 0.025, 10))
        for dy in (-1, 1):
            p.add("black", box((0.12, 0.03, 0.12), (x + dx, y + dy * 0.143, wz), tile=0.2, bevel=0.005))
    out = s * 0.158
    p.add("black", box((0.13, 0.06, 0.13), (x + 0.5, y + out + s * 0.03, wz), tile=0.2, bevel=0.006))
    mk.motor(p, at((x + 0.5, y + out + s * 0.06, wz), (0, -s, 0)), 0.09, 0.12, gear_len=0.05)
    battery(p, Matrix.Translation((x - 0.12, y + s * 0.265, (hz + 0.06 + g.top) / 2)) @ Matrix.Rotation(math.pi / 2 * s, 4, "Z"), (0.26, 0.86, g.top - hz - 0.08))
    if s > 0:
        bar_face = t.y - 0.45 + 0.15
        for dx in (0.12, 0.32):
            p.add("plastic", box((0.04, 0.04, 0.05), (x + dx, y - 0.148, 0.56), tile=0.2))
            p.add("steel", rod((x + dx, y - 0.168, 0.56), (x + dx, bar_face + 0.013, 0.56), 0.008, 8))
            p.add("copper", box((0.08, 0.012, 0.045), (x + dx, bar_face + 0.007, 0.56), tile=0.2))
        p.add("black", box((0.04, 0.12, 0.02), (x - 0.6, y - 0.18, hz + 0.02), tile=0.2))
        p.add("black", box((0.04, 0.02, 0.2), (x - 0.6, y - 0.23, hz - 0.09), tile=0.2))
        p.add("grey", box((0.08, 0.03, 0.05), (x - 0.6, t.y - 0.2 - 0.022, 0.21), tile=0.2, bevel=0.003))
        p.add("led", box((0.02, 0.002, 0.01), (x - 0.6, t.y - 0.2 - 0.006, 0.21), tile=0.2))


def carriage_part(g):
    """The carriage: its plate, four V-wheels on the track plate's edges,
    the sweep's pinion, gearbox and motor, the conductor rail's collector
    and junction box, the two guide housings the head's columns slide in
    with their rack motors, a scroll compressor drawing Mars air with its
    filter and the receiver tank feeding the head through a column, end
    buffers and a limit switch."""
    p = Part("carriage", (g.x, 0.0, g.curb))
    m = arch_local(g, g.hi, g.cp[0])
    th = carriage_core(p, m, g, (2.64, 0.62))
    # The guide housings the head's columns slide in, each with its rack's gear motor.
    for sx in (-1, 1):
        hx = sx * g.guide
        p.add("black", box((0.1, 0.12, 0.3), (hx, 0, th + 0.15), tile=0.3, bevel=0.005), m)
        for z in (th + 0.005, th + 0.295):
            p.add("plastic", box((0.11, 0.13, 0.01), (hx, 0, z), tile=0.2), m)
        p.add("black", box((0.06, 0.08, 0.08), (hx + sx * 0.08, 0, th + 0.2), tile=0.2, bevel=0.003), m)
        mk.motor(p, m @ at((hx + sx * 0.11, 0, th + 0.2), (-sx, 0, 0)), 0.06, 0.06)
    # The scroll compressor drawing Mars air through its filter, and the receiver it fills.
    cx = 1.0
    p.add("black", box((0.32, 0.2, 0.02), (cx, 0, th + 0.01), tile=0.2), m)
    p.add("paint", cyl(0.085, -0.14, 0.14, 20), m @ at((cx, 0.0, th + 0.11), (1, 0, 0)))
    p.add("black", cyl(0.07, -0.2, -0.14, 18), m @ at((cx, 0.0, th + 0.11), (1, 0, 0)))
    mk.motor(p, m @ at((cx + 0.14, 0.0, th + 0.11), (-1, 0, 0)), 0.1, 0.1)
    p.add("grey", cyl(0.05, th + 0.19, th + 0.3, 14), Matrix(m) @ Matrix.Translation((cx, 0, 0)))
    p.add("black", cyl(0.054, th + 0.3, th + 0.315, 14), Matrix(m) @ Matrix.Translation((cx, 0, 0)))
    p.add("tank", turned(heads(0.09, 0.4), 20), m @ at((-1.26, 0.0, th + 0.12), (1, 0, 0)))
    for dx in (-0.1, 0.1):
        p.add("black", box((0.03, 0.2, 0.06), (-1.06 + dx, 0, th + 0.03), tile=0.2), m)
    p.add("steel", cyl(0.012, 0.0, 0.05, 8), m @ at((-1.06, 0.0, th + 0.2), (0, 0, 1)))
    p.add("black", cyl(0.025, 0.05, 0.065, 12), m @ at((-1.06, 0.0, th + 0.2), (0, 0, 1)))
    p.add("red", cyl(0.01, 0.0, 0.04, 8), m @ at((-1.16, 0.0, th + 0.2), (0, 0, 1)))
    p.add("plastic", tube([m @ Vector((0.82, 0.07, th + 0.12)), m @ Vector((0.8, 0.2, th + 0.12)), m @ Vector((-0.8, 0.2, th + 0.12)),
                           m @ Vector((-0.86, 0.07, th + 0.12))], 0.01, 8))
    p.add("plastic", tube([m @ Vector((-0.95, -0.07, th + 0.17)), m @ Vector((-0.8, -0.1, th + 0.25)), m @ Vector((-0.66, -0.07, th + 0.25)),
                           m @ Vector((-g.guide - 0.05, -0.04, th + 0.25))], 0.01, 8))
    return p


def carriage_core(p, m, g, size):
    """What a washer's carriage runs on, in its frame `m` (X across its
    track plate, Y along it, Z out from the plate `g.cp` below the track
    plate `g.plate`): the plate `size` across and along, four V-wheels on the
    track plate's edges, the pinion on the rack with its gearbox and motor,
    the collector up into the conductor rail at `g.cond` and its junction
    box, buffers against the track plate's end stops and a limit switch.
    Returns the plate's thickness."""
    th = g.cp[1] - g.cp[0]
    wheel_z, cond_z = g.plate - g.cp[0], g.cond - 0.015 - g.cp[0]
    w, d = size
    p.add("black", box((w, d, th), (0, 0, th / 2), tile=0.4, bevel=0.004), m)
    for sx in (-1, 1):
        for ty in (-0.25, 0.25):
            v_wheel(p, m @ Matrix.Translation((sx * 0.192, ty, wheel_z)), 0.028, 0.024)
            p.add("steel", cyl(0.012, th, wheel_z - 0.012, 8), m @ Matrix.Translation((sx * 0.192, ty, 0)))
            p.add("black", box((0.06, 0.06, 0.03), (sx * 0.192, ty, th + 0.015), tile=0.2), m)
    zp = g.plate - 0.07 - g.cp[0]
    p.add("steel", mk.gear(0.035, 0.041, -0.015, 0.015, 14), m @ at((0, 0, zp), (1, 0, 0)))
    p.add("black", box((0.095, 0.1, zp + 0.03 - th), (0.0725, 0, (zp + 0.03 + th) / 2), tile=0.2, bevel=0.004), m)
    mk.motor(p, m @ at((0.07, 0.05, 0.09), (0, -1, 0)), 0.07, 0.08)
    p.add("plastic", box((0.04, 0.04, 0.03), (0.3, -0.1, th + 0.015), tile=0.2), m)
    p.add("black", box((0.012, 0.02, cond_z - 0.008 - th - 0.03), (0.3, -0.1, (cond_z - 0.008 + th + 0.03) / 2), tile=0.2), m)
    p.add("copper", box((0.01, 0.04, 0.011), (0.3, -0.1, cond_z - 0.0005), tile=0.2), m)
    p.add("grey", box((0.14, 0.1, 0.1), (0.42, 0.16, th + 0.05), tile=0.2, bevel=0.005), m)
    p.add("plastic", tube([m @ Vector((0.32, -0.1, th + 0.04)), m @ Vector((0.42, -0.02, th + 0.04)), m @ Vector((0.42, 0.11, th + 0.05))], 0.006, 6))
    for ty in (-1, 1):
        for sx in (-1, 1):
            p.add("black", box((0.05, 0.03, 0.15), (sx * 0.08, ty * (d / 2 + 0.015), th + 0.075), tile=0.2), m)
            mk.bumper(p, m @ at((sx * 0.08, ty * (d / 2 + 0.03), 0.17), (0, ty, 0)), 0.014, 0.035)
    p.add("black", box((0.04, 0.03, 0.16), (-0.22, d / 2 + 0.015, th + 0.08), tile=0.2), m)
    mk.switch(p, m @ at((-0.22, d / 2 + 0.03, 0.17), (0, 1, 0)), 1.5)
    return th


def head_part(g):
    """The head: its beam and end plates, the hood over the brush with dust
    skirts, the brush's bearings and its belt-driven motor, two air knives
    either side fed from a manifold through solenoid valves, the two guide
    columns with their racks, and the inspection camera."""
    o = arch_local(g, g.hi, g.axis)
    p = Part("head", o.translation)
    b0, b1 = g.beam[0] - g.axis, g.beam[1] - g.axis
    p.add("alu", bar((-1.45, 0, b0), (1.45, 0, b0), mk.rect(0.16, b1 - b0, 0.0), UP, 0.5), o)
    hx = g.brush_half + 0.04
    for sx in (-1, 1):
        p.add("black", box((0.015, 0.22, b1 + 0.08), (sx * hx, 0, (b1 - 0.08) / 2), tile=0.3), o)
        p.add("steel", cyl(0.045, 0.0, 0.035, 16), o @ at((sx * (hx - 0.0075), 0, 0), (-sx, 0, 0)))
        mk.bolts(p, o @ at((sx * (hx + 0.0075), 0, 0), (sx, 0, 0)), [(0.03 * math.cos(k * math.pi / 2), 0.03 * math.sin(k * math.pi / 2)) for k in range(4)], 0.005)
    p.add("black", box((0.03, 0.1, 0.36), (hx + 0.022, 0, 0.12), tile=0.2, bevel=0.01), o)
    p.add("black", box((0.06, 0.08, 0.06), (hx + 0.06, 0, 0.25), tile=0.2, bevel=0.004), o)
    mk.motor(p, o @ at((hx + 0.09, 0, 0.25), (-1, 0, 0)), 0.06, 0.07)
    hood = [(0.3 * math.cos(math.radians(a)), 0.3 * math.sin(math.radians(a))) for a in range(15, 166, 10)]
    hood = hood + [(0.296 * yy / 0.3, 0.296 * zz / 0.3) for yy, zz in reversed(hood)]
    p.add("paint", swept([(zz, yy) for yy, zz in hood], [(Vector((xx, 0, 0)), Z, Y) for xx in (-hx + 0.01, hx - 0.01)]), o)
    for sy in (-1, 1):
        yy, zz = 0.3 * math.cos(math.radians(15)), 0.3 * math.sin(math.radians(15))
        p.add("plastic", box((2 * hx - 0.04, 0.006, 0.1), (0, sy * yy, zz - 0.05), tile=0.3), o)
    for sy in (-1, 1):
        ky = sy * 0.36
        p.add("grey", box((2.6, 0.05, 0.11), (0, ky, 0.005), tile=0.3, bevel=0.008), o)
        p.add("steel", box((2.6, 0.016, 0.014), (0, ky - sy * 0.012, -0.05 + 0.007), tile=0.2), o)
        for xx in (-0.4, 0.0, 0.4):
            p.add("plastic", tube([o @ Vector((xx, ky, 0.06)), o @ Vector((xx, ky * 0.9, 0.2)), o @ Vector((xx, sy * 0.012, b1 + 0.02))], 0.009, 6))
        p.add("black", box((0.05, 0.05, 0.04), (sy * 0.25, 0.046, b1 + 0.02), tile=0.2, bevel=0.003), o)
    p.add("steel", bar((-0.45, 0, b1), (0.45, 0, b1), [(0.0, -0.02), (0.0, 0.02), (0.04, 0.02), (0.04, -0.02)], UP, 0.3), o)
    for sx in (-1, 1):
        cx = sx * g.guide
        p.add("black", box((0.12, 0.14, 0.02), (cx, 0, b1 + 0.01), tile=0.2), o)
        p.add("alu", box((0.05, 0.05, g.column), (cx, 0, b1 + 0.02 + g.column / 2), tile=0.3, bevel=0.002), o)
        p.add("steel", box((0.008, 0.02, g.column - 0.06), (cx + sx * 0.029, 0, b1 + 0.02 + g.column / 2), tile=0.2), o)
        p.add("black", box((0.06, 0.06, 0.012), (cx, 0, b1 + 0.026 + g.column), tile=0.2), o)
    camera_head(p, o @ at((0.9, -0.43, 0.02), (0, 0, 1)), 1.2)
    p.add("black", box((0.04, 0.08, 0.04), (0.9, -0.39, 0.06), tile=0.2), o)
    return p


def arch_brush_part(g):
    """The arch washer's brush: a core with hub flanges and bristle tufts
    shaped to a cushion, longest at its ends."""
    o = arch_local(g, g.hi, g.axis)
    p = Part("brush", o.translation)
    p.add("black", cyl(g.core, -g.brush_half - 0.03, g.brush_half + 0.03, 16), o @ at((0, 0, 0), (1, 0, 0)))
    for sx in (-1, 1):
        p.add("steel", cyl(0.07, -0.006, 0.006, 16), o @ at((sx * g.brush_half, 0, 0), (1, 0, 0)))
    length = lambda xx: g.bristle + g.shape * (1 - math.sin(math.pi * (0.5 + xx / g.bay)))
    p.add("bristle", tufts(-g.brush_half, g.brush_half, 0.035, 12, g.core, length), o)
    return p


def track_part(g):
    """The arch washer's position tape along the +Y plinth's inner face."""
    t = g.track
    p = Part("track", (g.x, t.y, t.head))
    y = t.y - 0.2 - 0.0015
    p.add("alu", box((t.x1 - t.x0, 0.003, 0.03), ((t.x0 + t.x1) / 2, y, 0.215), tile=1.0))
    p.add("black", box((t.x1 - t.x0 - 0.02, 0.001, 0.018), ((t.x0 + t.x1) / 2, y - 0.002, 0.215), tile=1.0))
    return p


def arch_washer(g, note, name, bound):
    """The arch washer's machine: track (held to the world), portal, carriage, head, brush."""
    u = Usda("/robot/Looks")
    t = g.track
    open_machine(u, "roof_washer", (("travel", "joint_position"), ("sweep", "joint_position"), ("press", "joint_position"), ("brush", "joint_velocity")),
                 "track", note, name)
    body(u, 1, "track", arch_anchor(g), 20, (1, 200, 200), link="base_link")
    visual(u, 2, name, "track", bound)
    end(u, 1)
    span_kg = 900.0
    body(u, 1, "portal", (g.x, 0.0, g.curb), span_kg, (span_kg * g.ro ** 2 / 2, 300, span_kg * g.ro ** 2 / 2), values=[f"float gearbox:value:range = {-g.span:.4f}"])
    for k, (a0, a1) in enumerate(zip(span(g.foot, math.pi - g.foot, math.pi / 12)[:-1], span(g.foot, math.pi - g.foot, math.pi / 12)[1:])):
        mid = (a0 + a1) / 2
        c = radial_x(mid) * (g.ri + g.ro) / 2
        chord = 2 * g.ro * math.sin((a1 - a0) / 2)
        hull(u, 2, f"girder{k:02d}", (0.0, c.y, c.z), (2 * g.half + 0.1, chord, g.ro - g.ri + 0.1), (90.0 - math.degrees(mid), 0.0, 0.0))
    for s, side in ((-1, "left"), (1, "right")):
        hull(u, 2, f"bogie_{side}", (0.0, s * t.y + s * 0.13, (t.head + 0.01 + g.top) / 2 - g.curb), (1.72, 0.66, g.top - t.head - 0.01))
    visual(u, 2, name, "portal", bound)
    end(u, 1)
    body(u, 1, "carriage", (g.x, 0.0, g.curb), 70, (70 * g.cp[0] ** 2, 35 * g.cp[0] ** 2, 35 * g.cp[0] ** 2), values=[f"float gearbox:value:range = {g.hi - g.lo:.5f}"])
    cc = radial_x(g.hi) * (g.cp[0] + 0.1)
    hull(u, 2, "frame", (0.0, cc.y, cc.z), (2.7, 0.72, 0.2), (90.0 - math.degrees(g.hi), 0.0, 0.0))
    visual(u, 2, name, "carriage", bound)
    end(u, 1)
    head_at = Vector((g.x, 0.0, g.curb)) + radial_x(g.hi) * g.axis
    body(u, 1, "head", tuple(head_at), 35, (25, 2, 25), values=[f"float gearbox:value:range = {g.stroke:g}"])
    hc = radial_x(g.hi) * 0.17
    hull(u, 2, "hood", (0.0, hc.y, hc.z), (2 * g.brush_half + 0.12, 0.78, 0.42), (90.0 - math.degrees(g.hi), 0.0, 0.0))
    visual(u, 2, name, "head", bound)
    eye = arch_local(g, g.hi, g.axis) @ Vector((0.9, -0.43, -0.02)) - head_at
    camera(u, 2, tuple(eye), looking(-radial_x(g.hi)), 2.0)
    end(u, 1)
    body(u, 1, "brush", tuple(head_at), 12, (0.15, 7, 7))
    visual(u, 2, name, "brush", bound)
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "track", arch_anchor(g))
    joint(u, 2, "Prismatic", "travel", "track", "portal", (0.0, -t.y, g.curb - t.head), "X", (-g.span, 0), (6000, 0.3))
    joint(u, 2, "Revolute", "sweep", "portal", "carriage", (0, 0, 0), "X", (0, math.degrees(g.hi - g.lo)), (9000, 0.15))
    press = Quaternion((1, 0, 0), math.radians(270.0) - g.hi)
    joint(u, 2, "Prismatic", "press", "carriage", "head", tuple(head_at - Vector((g.x, 0.0, g.curb))), "Z", (0, g.stroke), (2500, 0.1), press)
    joint(u, 2, "Revolute", "brush", "head", "brush", (0, 0, 0), "X", None, (150, 25))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def arch_anchor(g):
    """Where the arch washer's anchor holds its track, in the greenhouse's frame: the +Y rail's head at the dock."""
    return (g.x, g.track.y, g.track.head)


def arch_looks(g):
    return [track_part(g), girder_part(g), carriage_part(g), head_part(g), arch_brush_part(g)]


# The dome washer


def dome(radius):
    """A dome washer's sizes for a dome `radius` round, in the dome's frame:
    radii from the sphere's centre on its axis at the spring height,
    elevations `e` over its equator; the arch at rest over the portal."""
    ring = buildings.dome_washer_ring(radius)
    g = SimpleNamespace(R=radius, ring=ring, c=Vector((0.0, 0.0, ring.spring)), seat=ring.seat)
    g.ri, g.ro, g.half = radius + 0.85, radius + 1.25, 0.4
    g.plate, g.cond = radius + 0.75, radius + 0.795
    g.cp = (radius + 0.53, radius + 0.56)
    g.core, g.bristle, g.brush_half = 0.05, 0.2, 1.0
    g.axis = radius - 0.01 + g.bristle
    g.top = ring.head + 0.33
    g.e_in = math.asin((g.top + 0.06 - ring.spring) / g.ri)
    g.e_out = math.asin((g.top + 0.06 - ring.spring) / g.ro)
    g.e_top = math.radians(86.0)
    g.hi = math.radians(82.0)
    lowest = math.asin((ring.portal + 0.1 - ring.spring) / radius) + (g.brush_half + 0.05) / g.axis
    g.lo = math.radians(math.ceil(math.degrees(lowest) * 2) / 2)
    g.yoke = (ring.seat + 0.13, ring.spring + g.ri * math.sin(g.e_top), ring.spring + (g.ro + 0.06) * math.sin(g.e_top))
    return g


def radial_m(e):
    return Vector((math.cos(e), 0.0, math.sin(e)))


def dome_local(g, e, radius):
    """The frame at elevation `e` and `radius` on the arch at rest: X across
    the arch, Y up its meridian, Z out."""
    m = Matrix((Y, Vector((-math.sin(e), 0.0, math.cos(e))), radial_m(e))).transposed().to_4x4()
    m.translation = g.c + radial_m(e) * radius
    return m


def meridian_part(g):
    """The dome washer's arch: the slewing bearing's turning ring and
    turntable on the crown, the yoke, the meridian box truss of four chords
    and their lacing, its track plate, rack and conductor rail, end stops
    and flags, the control cabinet over the foot, the foot gusseted onto
    the bogie that drives round the ring girder, and the beacon."""
    ring = g.ring
    p = Part("arch", (0.0, 0.0, g.seat))
    s0 = g.seat
    p.add("steel", turned([(0.24, s0 + 0.05), (0.36, s0 + 0.05), (0.36, s0 + 0.1), (0.24, s0 + 0.1), (0.24, s0 + 0.05)], 48))
    p.add("black", turned([(0.0, s0 + 0.1), (0.42, s0 + 0.1), (0.42, s0 + 0.13), (0.0, s0 + 0.13)], 48))
    for k in range(16):
        a = 2 * math.pi * k / 16
        p.add("steel", mk.hexagon(0.016, s0 + 0.13, s0 + 0.142), Matrix.Translation((0.38 * math.cos(a), 0.38 * math.sin(a), 0)))
    # The yoke: a welded frame of square tubes either side, tied across,
    # from the turntable out to the chords' ends.
    z0, z_in, z_out = g.yoke
    corners = [(0.0, z0 + 0.03), (0.42, z0 + 0.03), (0.98, z_in - 0.08), (0.98, z_out + 0.08), (0.0, z_out + 0.08)]
    braces = [(0, 3), (1, 4), (1, 3)]
    yo = g.half + 0.03
    for s in (-1, 1):
        pts = [Vector((xx, s * yo, zz)) for xx, zz in corners]
        for a, b in zip(pts, pts[1:] + pts[:1]):
            p.add("paint", bar(a, b, mk.rect(0.06, 0.06), Y, 0.3))
        for i, j in braces:
            p.add("paint", bar(pts[i], pts[j], mk.rect(0.04, 0.04), Y, 0.3))
    for xx, zz in corners:
        p.add("paint", bar((xx, -yo, zz), (xx, yo, zz), mk.rect(0.05, 0.05), X, 0.3))
    for xx, zz in corners[:2]:
        for s in (-1, 1):
            p.add("paint", kit.prism([(0.0, 0.0), (0.14, 0.0), (0.0, 0.14)], -0.006, 0.006, 0.3), Matrix.Translation((xx, s * (yo - 0.03), z0 + 0.06)) @ Matrix.Rotation(math.pi / 2, 4, "X"))
    p.add("paint", box((0.5, 2 * yo + 0.06, 0.02), (0.25, 0, z_out + 0.12), tile=0.4))
    beacon(p, Matrix.Translation((0.25, 0.0, z_out + 0.13)), 0.05)
    # The truss.
    for radius, e0 in ((g.ri, g.e_in), (g.ro, g.e_out)):
        es = span(e0, g.e_top, math.radians(1.5))
        for s in (-1, 1):
            p.add("alu", swept(kit.circle(0.045, 10), [(g.c + radial_m(e) * radius + Y * s * g.half, radial_m(e), Y) for e in es]))
    nodes = span(g.e_in, g.e_top, math.radians(4.0))
    for k, e in enumerate(nodes):
        lo, hi = g.c + radial_m(e) * g.ri, g.c + radial_m(e) * g.ro
        for s in (-1, 1):
            p.add("alu", rod(lo + Y * s * g.half, hi + Y * s * g.half, 0.022, 6))
        for pt in (lo, hi):
            p.add("alu", rod(pt - Y * g.half, pt + Y * g.half, 0.02, 6))
        if k + 1 < len(nodes):
            f = nodes[k + 1]
            for s in (-1, 1):
                a, b = (e, f) if (k % 2 == 0) == (s > 0) else (f, e)
                p.add("alu", rod(g.c + radial_m(a) * g.ri + Y * s * g.half, g.c + radial_m(b) * g.ro + Y * s * g.half, 0.018, 6))
            p.add("alu", rod(hi - Y * g.half, g.c + radial_m(f) * g.ro + Y * g.half, 0.016, 6))
    # The carriage's track plate, its edges, rack, conductor rail and stops.
    tp = span(g.lo - math.radians(3.5), g.hi + math.radians(3.5), math.radians(1.0))
    frames = lambda radius, off=0.0: [(g.c + radial_m(e) * radius + Y * off, radial_m(e), Y) for e in tp]
    p.add("steel", swept(mk.rect(0.32, 0.012), frames(g.plate)))
    for s in (-1, 1):
        p.add("steel", swept(kit.circle(0.012, 8), frames(g.plate, s * 0.16)))
    p.add("steel", swept([(-0.008, -0.02), (-0.008, 0.02), (0.0, 0.02), (0.0, -0.02)], frames(g.plate - 0.006)))
    teeth = int((tp[-1] - tp[0]) * (g.plate - 0.03) / 0.03)
    rack = bmesh.new()
    for k in range(teeth):
        e = tp[0] + (tp[-1] - tp[0]) * (k + 0.5) / teeth
        kit.merge(rack, box((0.04, 0.012, 0.021), (0, 0, -0.0245), tile=0.1), dome_local(g, e, g.plate))
    p.add("steel", rack)
    lift = g.cond - g.plate
    for e in nodes:
        if tp[0] < e < tp[-1]:
            p.add("black", box((0.05, 0.04, g.ri - g.plate - 0.03), (0, 0, (g.ri - g.plate) / 2), tile=0.2), dome_local(g, e, g.plate))
            p.add("black", box((0.06, 0.03, 0.04), (0.3, 0, lift + 0.035), tile=0.2), dome_local(g, e, g.plate))
    p.add("grey", swept(mk.conductor_outline(0.03, 0.05, -1), [(o + Y * 0.3, Y, du) for o, du, _ in frames(g.cond)]))
    for e, d in ((tp[0], 1), (tp[-1], -1)):
        m = dome_local(g, e, g.plate)
        p.add("black", box((0.22, 0.04, 0.06), (0, d * 0.02, -0.04), tile=0.2, bevel=0.003), m)
        for s in (-1, 1):
            mk.bumper(p, m @ at((s * 0.08, d * 0.04, -0.045), (0, d, 0)), 0.014, 0.04)
        p.add("yellow", box((0.012, 0.06, 0.05), (-0.22, d * 0.06, -0.05), tile=0.2), m)
    # The control cabinet on the outer chords over the foot, its feeds.
    ec = math.radians(12.0)
    cab = dome_local(g, ec, g.ro + 0.045 + 0.2)
    mk.cover(p, cab, (0.62, 0.6, 0.36), key="paint", vents=4, bevel=0.012)
    for s in (-1, 1):
        p.add("black", box((0.04, 0.5, 0.06), (s * 0.25, 0, -0.2), tile=0.2), cab)
    estop(p, cab @ at((0.0, -0.3, 0.05), (0, -1, 0)), 1.0)
    p.add("amber", cyl(0.025, 0.0, 0.04, 12), cab @ at((0.2, 0.3, 0.1), (0, 1, 0)))
    p.add("plastic", rod(cab @ Vector((-0.2, 0.3, 0.12)), cab @ Vector((-0.2, 0.75, 0.12)), 0.006, 6))
    feed = dome_local(g, g.lo - math.radians(3.0), g.cond)
    p.add("plastic", tube([cab @ Vector((0.3, -0.25, -0.18)), cab @ Vector((0.34, -0.3, -0.3)), feed @ Vector((0.34, 0.0, 0.0)), feed @ Vector((0.3, 0.0, 0.0))], 0.01, 8))
    # The foot and its bogie on the ring rail.
    bog = Matrix.Translation((ring.r, 0.0, ring.head))
    lo_r = g.ri * math.cos(g.e_in) - ring.r
    hi_r = g.ro * math.cos(g.e_out) - ring.r
    p.add("black", box((hi_r - lo_r + 0.16, 2 * g.half + 0.12, 0.06), ((lo_r + hi_r) / 2, 0, g.top - ring.head + 0.03), tile=0.3, bevel=0.004), bog)
    for s in (-1, 1):
        p.add("paint", gusset(hi_r - lo_r + 0.1, 0.42), bog @ Matrix.Translation(((lo_r + hi_r) / 2, s * (g.half + 0.065), g.top - ring.head + 0.06)) @ Matrix.Rotation(math.pi / 2, 4, "Z"))
    dome_bogie(p, g, bog, cab)
    return p


def dome_bogie(p, g, m, cab):
    """The arch's foot bogie on the ring rail, in its frame `m` (X out, Y
    along the rail, Z up from its head): side, end and top plates, two
    double-flanged wheels, the slew drive's gear motor, hold-down claws,
    sweepers, the collector reaching round into the conductor rail's slot
    and its cable up to the cabinet, the storm-lock pin over its socket and
    the home switch over its flag."""
    ring = g.ring
    top = g.top - ring.head
    for dx in (-0.11, 0.11):
        p.add("paint", box((0.016, 1.1, top - 0.035), (dx, 0, (top - 0.03 + 0.005) / 2), tile=0.3, bevel=0.003), m)
    p.add("paint", box((0.3, 1.12, 0.03), (0, 0, top - 0.015), tile=0.3), m)
    for dy in (-1, 1):
        p.add("paint", box((0.22, 0.02, top - 0.035), (0, dy * 0.55, (top - 0.03 + 0.005) / 2), tile=0.3), m)
        flanged_wheel(p, m @ at((0, dy * 0.42, 0.12), (1, 0, 0)), 0.12, 0.057, 0.145)
        p.add("steel", rod(m @ Vector((-0.14, dy * 0.42, 0.12)), m @ Vector((0.14, dy * 0.42, 0.12)), 0.022, 10))
        for dx in (-1, 1):
            p.add("black", box((0.03, 0.1, 0.1), (dx * 0.133, dy * 0.42, 0.12), tile=0.2, bevel=0.004), m)
            p.add("black", box((0.09, 0.12, 0.014), (dx * 0.065, dy * 0.25, -0.055), tile=0.2), m)
            p.add("black", box((0.015, 0.12, 0.075), (dx * 0.105, dy * 0.25, -0.017), tile=0.2), m)
        p.add("plastic", box((0.075, 0.01, 0.04), (0, dy * 0.5, 0.022), tile=0.2), m)
        p.add("black", box((0.09, 0.03, 0.03), (0, dy * 0.5, 0.055), tile=0.2), m)
    p.add("black", box((0.06, 0.13, 0.13), (0.17, 0.42, 0.12), tile=0.2, bevel=0.006), m)
    mk.motor(p, m @ at((0.2, 0.42, 0.12), (-1, 0, 0)), 0.09, 0.12, gear_len=0.04)
    # Collector: out past the girder's outer web and down into the slot.
    zc = ring.under + 0.08 - ring.head
    p.add("black", box((0.23, 0.03, 0.02), (0.23, -0.3, 0.03), tile=0.2), m)
    p.add("black", box((0.02, 0.03, 0.03 - zc + 0.01), (0.335, -0.3, (0.03 + zc) / 2), tile=0.2), m)
    p.add("plastic", box((0.04, 0.03, 0.012), (0.31, -0.3, zc), tile=0.2), m)
    p.add("copper", box((0.012, 0.04, 0.012), (0.294, -0.3, zc), tile=0.2), m)
    p.add("plastic", tube([m @ Vector((0.33, -0.3, 0.04)), m @ Vector((0.33, -0.3, 0.6)), cab @ Vector((-0.3, -0.25, -0.18))], 0.01, 8))
    # Storm lock and home switch.
    p.add("black", box((0.1, 0.08, 0.16), (0.17, 0.0, 0.08), tile=0.2, bevel=0.005), m)
    p.add("steel", cyl(0.015, -0.035, 0.0, 10), Matrix(m) @ Matrix.Translation((0.18, 0.0, 0.0)))
    p.add("black", box((0.09, 0.05, 0.012), (-0.155, 0.3, 0.1), tile=0.2), m)
    mk.switch(p, m @ at((-0.19, 0.3, 0.09), (0, 0, -1)), 3.2)


def crown_part(g):
    """The slewing bearing's fixed ring on the crown pedestal's seat, its bolt circle and grease nipple."""
    p = Part("crown", (0.0, 0.0, g.seat))
    s0 = g.seat
    p.add("steel", turned([(0.26, s0), (0.37, s0), (0.37, s0 + 0.05), (0.26, s0 + 0.05), (0.26, s0)], 48))
    for k in range(20):
        a = 2 * math.pi * (k + 0.5) / 20
        p.add("black", mk.hexagon(0.014, s0 + 0.05, s0 + 0.06), Matrix.Translation((0.33 * math.cos(a), 0.33 * math.sin(a), 0)))
    p.add("steel", cyl(0.008, 0.0, 0.03, 8), at((0.37, 0.0, s0 + 0.025), (1, 0, 0)))
    return p


def dome_carriage_part(g):
    """The dome washer's carriage and head: what it runs on, a scroll
    compressor and its filter beside, the head's beam under it on hangers,
    the hood along the meridian over the brush with dust skirts, the brush's
    bearings and motor, an air knife either side, and the camera."""
    p = Part("carriage", tuple(g.c))
    m = dome_local(g, g.hi, g.cp[0])
    th = carriage_core(p, m, g, (0.9, 0.8))
    p.add("black", box((0.18, 0.34, 0.02), (-0.33, 0, th + 0.01), tile=0.2), m)
    p.add("paint", cyl(0.07, -0.14, 0.14, 18), m @ at((-0.33, 0.0, th + 0.09), (0, 1, 0)))
    p.add("black", cyl(0.06, -0.19, -0.14, 16), m @ at((-0.33, 0.0, th + 0.09), (0, 1, 0)))
    mk.motor(p, m @ at((-0.33, 0.14, th + 0.09), (0, -1, 0)), 0.09, 0.09)
    p.add("grey", cyl(0.045, th + 0.16, th + 0.2, 14), Matrix(m) @ Matrix.Translation((-0.33, -0.05, 0)))
    o = dome_local(g, g.hi, g.axis)
    b0, b1 = g.R + 0.44 - g.axis, g.R + 0.5 - g.axis
    p.add("alu", bar((0, -1.06, b0), (0, 1.06, b0), mk.rect(0.14, b1 - b0, 0.0), UP, 0.5), o)
    for yy in (-0.3, 0.3):
        p.add("black", box((0.1, 0.06, g.cp[0] - g.axis - b1), (0, yy, (g.cp[0] - g.axis + b1) / 2), tile=0.2), o)
    hy = g.brush_half + 0.04
    for sy in (-1, 1):
        p.add("black", box((0.2, 0.015, b1 + 0.08), (0, sy * hy, (b1 - 0.08) / 2), tile=0.3), o)
        p.add("steel", cyl(0.04, 0.0, 0.03, 16), o @ at((0, sy * (hy - 0.0075), 0), (0, -sy, 0)))
    p.add("black", box((0.09, 0.03, 0.3), (0, hy + 0.022, 0.1), tile=0.2, bevel=0.01), o)
    p.add("black", box((0.07, 0.06, 0.06), (0, hy + 0.06, 0.21), tile=0.2, bevel=0.004), o)
    mk.motor(p, o @ at((0, hy + 0.09, 0.21), (0, -1, 0)), 0.06, 0.07)
    hood = [(0.24 * math.cos(math.radians(a)), 0.24 * math.sin(math.radians(a))) for a in range(15, 166, 10)]
    hood = hood + [(0.236 * xx / 0.24, 0.236 * zz / 0.24) for xx, zz in reversed(hood)]
    p.add("paint", swept([(zz, xx) for xx, zz in hood], [(Vector((0, yy, 0)), Z, X) for yy in (-hy + 0.01, hy - 0.01)]), o)
    for sx in (-1, 1):
        xx, zz = 0.24 * math.cos(math.radians(15)), 0.24 * math.sin(math.radians(15))
        p.add("plastic", box((0.006, 2 * hy - 0.04, 0.09), (sx * xx, 0, zz - 0.045), tile=0.3), o)
        kx = sx * 0.3
        p.add("grey", box((0.05, 1.9, 0.1), (kx, 0, 0.01), tile=0.3, bevel=0.008), o)
        p.add("steel", box((0.016, 1.9, 0.014), (kx - sx * 0.012, 0, -0.033), tile=0.2), o)
        for yy in (-0.5, 0.5):
            p.add("plastic", tube([o @ Vector((kx, yy, 0.06)), o @ Vector((kx * 0.9, yy, 0.2)), o @ Vector((sx * 0.05, yy, b1 + 0.02))], 0.009, 6))
    p.add("steel", bar((0, -0.6, b1), (0, 0.6, b1), [(0.0, -0.02), (0.0, 0.02), (0.04, 0.02), (0.04, -0.02)], UP, 0.3), o)
    p.add("plastic", tube([m @ Vector((-0.33, -0.19, th + 0.09)), m @ Vector((-0.2, -0.32, th + 0.02)), o @ Vector((-0.02, -0.58, b1 + 0.02))], 0.01, 8))
    camera_head(p, o @ at((0.36, 0.7, 0.0), (0, 0, 1)), 1.2)
    p.add("black", box((0.06, 0.04, 0.03), (0.33, 0.7, 0.045), tile=0.2), o)
    return p


def dome_brush_part(g):
    """The dome washer's brush along the meridian: core, hub flanges, bristle tufts."""
    o = dome_local(g, g.hi, g.axis) @ Matrix.Rotation(math.pi / 2, 4, "Z")
    p = Part("brush", o.translation)
    p.add("black", cyl(g.core, -g.brush_half - 0.03, g.brush_half + 0.03, 16), o @ at((0, 0, 0), (1, 0, 0)))
    for sx in (-1, 1):
        p.add("steel", cyl(0.07, -0.006, 0.006, 16), o @ at((sx * g.brush_half, 0, 0), (1, 0, 0)))
    p.add("bristle", tufts(-g.brush_half, g.brush_half, 0.035, 14, g.core, lambda xx: g.bristle), o)
    return p


def dome_washer(g, note, name, bound):
    """The dome washer's machine: crown (held to the world), arch, carriage, brush."""
    u = Usda("/robot/Looks")
    ring = g.ring
    open_machine(u, "dome_washer", (("slew", "joint_position"), ("meridian", "joint_position"), ("brush", "joint_velocity")), "crown", note, name)
    body(u, 1, "crown", dome_anchor(g), 40, (2, 2, 4), link="base_link")
    visual(u, 2, name, "crown", bound)
    end(u, 1)
    kg = 650.0
    body(u, 1, "arch", (0.0, 0.0, g.seat), kg, (40000, 40000, kg * g.ro ** 2 / 3), values=["float gearbox:value:range = 6.28319"])
    base = Vector((0.0, 0.0, g.seat))
    es = span(g.e_in, g.e_top, math.radians(8.0))
    for k, (e0, e1) in enumerate(zip(es, es[1:])):
        mid = (e0 + e1) / 2
        c = g.c + radial_m(mid) * (g.ri + g.ro) / 2 - base
        hull(u, 2, f"truss{k:02d}", tuple(c), (2 * g.ro * math.sin((e1 - e0) / 2), 2 * g.half + 0.1, g.ro - g.ri + 0.1), (0.0, 90.0 - math.degrees(mid), 0.0))
    z0, z_in, z_out = g.yoke
    hull(u, 2, "yoke", (0.49, 0.0, (z0 + z_out + 0.1) / 2 - g.seat), (0.98, 2 * g.half + 0.08, z_out + 0.1 - z0))
    hull(u, 2, "bogie", (ring.r, 0.0, ring.head + 0.02 + (g.top + 0.06 - ring.head - 0.02) / 2 - g.seat), (0.62, 1.2, g.top + 0.06 - ring.head - 0.02))
    cab = dome_local(g, math.radians(12.0), g.ro + 0.045 + 0.2).translation - base
    hull(u, 2, "cabinet", tuple(cab), (0.62, 0.62, 0.4), (0.0, 90.0 - 12.0, 0.0))
    visual(u, 2, name, "arch", bound)
    end(u, 1)
    body(u, 1, "carriage", tuple(g.c), 90, (14000, 14000, 300), values=[f"float gearbox:value:range = {g.hi - g.lo:.5f}"])
    hc = radial_m(g.hi) * (g.R + 0.36)
    hull(u, 2, "head", tuple(hc), (2.1, 0.82, 0.42), (0.0, 90.0 - math.degrees(g.hi), 0.0))
    visual(u, 2, name, "carriage", bound)
    o = dome_local(g, g.hi, g.axis)
    camera(u, 2, tuple(o @ Vector((0.36, 0.7, -0.02)) - g.c), looking(-radial_m(g.hi)), 2.0)
    end(u, 1)
    brush_at = o.translation
    body(u, 1, "brush", tuple(brush_at), 10, (0.2, 3.5, 3.5))
    visual(u, 2, name, "brush", bound)
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "crown", dome_anchor(g))
    joint(u, 2, "Revolute", "slew", "crown", "arch", (0, 0, 0), "Z", (0, 360), (40000, 0.05))
    joint(u, 2, "Revolute", "meridian", "arch", "carriage", (0.0, 0.0, ring.spring - g.seat), "Y", (0, math.degrees(g.hi - g.lo)), (20000, 0.04))
    joint(u, 2, "Revolute", "brush", "carriage", "brush", tuple(brush_at - g.c), "X", None, (150, 25), Quaternion((0, 1, 0), -(math.pi / 2 + g.hi)))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def dome_anchor(g):
    """Where the dome washer's anchor holds its crown bearing, in the dome's frame."""
    return (0.0, 0.0, g.seat)


def dome_looks(g):
    return [crown_part(g), meridian_part(g), dome_carriage_part(g), dome_brush_part(g)]


# The panel washer


def panel(panels, tilt):
    """A panel washer's sizes for a row of `panels` tables tilted `tilt`
    degrees, in the row's frame: slope coordinates (s up the slope, y along
    the row, t out of the slope plane) about the row's centre line."""
    t = power.panel_washer_track(panels, tilt)
    g = SimpleNamespace(track=t, th=t.tilt)
    g.y, g.span = t.dock, t.dock - t.end
    g.core, g.reach = 0.035, 0.13
    g.axis = t.glass + g.reach - 0.003
    g.brush_half = t.half - 0.015
    g.beam = 0.42
    return g


def slope(g, s, tt, y):
    """The frame at slope coordinates (s, t, y): X up the slope, Y along the row, Z out of it."""
    th, t = g.th, g.track
    m = Matrix.Rotation(-th, 4, "Y")
    m.translation = Vector((s * math.cos(th) - tt * math.sin(th), y, t.zc + s * math.sin(th) + tt * math.cos(th)))
    return m


def frame_part(g):
    """The panel washer's frame: the beam across the row's slope; a drive
    unit over each rail with two driven running wheels, two side guide
    wheels on the rail's outer face, cheeks, a gear motor, bumpers; the
    brush's bearings, hood with skirts and motor; the PV panel and battery
    on the beam, the controller with antenna and e-stop, the beacon, the
    camera, the cable runs, the home sensor and the dock latch's pin."""
    t = g.track
    L = slope(g, 0.0, 0.0, g.y)
    p = Part("frame", (L @ Vector((0.0, 0.0, g.beam))))
    b0 = g.beam
    p.add("alu", bar((-2.42, 0, b0), (2.42, 0, b0), mk.rect(0.1, 0.12, 0.0), UP, 0.5), L)
    for s in (-1, 1):
        p.add("plastic", box((0.006, 0.102, 0.122), (s * 2.423, 0, b0 + 0.06), tile=0.2), L)
    for e in (-1, 1):
        sr = e * t.s
        mk.cover(p, L @ Matrix.Translation((e * 2.31, 0, 0.24)), (0.22, 0.6, 0.18), key="paint", vents=3, bevel=0.012)
        p.add("black", box((0.08, 0.1, b0 - 0.33), (e * 2.3, 0, (b0 + 0.33) / 2), tile=0.2), L)
        p.add("black", box((0.01, 0.58, 0.09), (sr - e * 0.045, 0, 0.105), tile=0.2), L)
        p.add("black", box((0.01, 0.58, 0.13), (sr + e * 0.105, 0, 0.095), tile=0.2), L)
        for yy in (-0.22, 0.22):
            mk.wheel(p, L @ at((sr, yy, t.top + 0.05), (1, 0, 0)), 0.05, 0.04)
            p.add("steel", rod(L @ Vector((sr - e * 0.05, yy, t.top + 0.05)), L @ Vector((sr + e * 0.1, yy, t.top + 0.05)), 0.008, 8))
            gx = sr + e * 0.06
            mk.wheel(p, L @ at((gx, yy, 0.01), (0, 0, 1)), 0.03, 0.025)
            p.add("black", box((0.07, 0.05, 0.015), (gx + e * 0.01, yy, t.top + 0.0125), tile=0.2), L)
            p.add("steel", cyl(0.006, 0.0225, t.top + 0.005, 6), L @ Matrix.Translation((gx, yy, 0)))
        mk.motor(p, L @ at((sr + e * 0.11, 0.22, t.top + 0.05), (-e, 0, 0)), 0.06, 0.07, gear_len=0.03)
        for sy in (-1, 1):
            mk.bumper(p, L @ at((e * 2.3, sy * 0.3, 0.24), (0, sy, 0)), 0.03, t.bumper - 0.3)
        p.add("black", box((0.03, 0.09, 0.08), (e * (g.brush_half + 0.02), 0, g.axis + 0.03), tile=0.2), L)
        p.add("steel", cyl(0.03, 0.0, 0.02, 12), L @ at((e * (g.brush_half + 0.005), 0, g.axis), (-e, 0, 0)))
    mk.switch(p, L @ at((2.3, 0.3, 0.17), (0, 1, 0)), 2.5)
    p.add("steel", cyl(0.008, 0.09, 0.15, 8), L @ Matrix.Translation((t.s - 0.045, 0.0, 0)))
    p.add("grey", box((0.02, 0.04, 0.03), (-t.s + 0.06, 0.0, 0.045), tile=0.2), L)
    p.add("black", box((0.04, 0.04, 0.08), (-t.s + 0.07, 0.0, 0.1), tile=0.2), L)
    mk.motor(p, L @ at((2.3, -0.12, 0.33), (0, 0, -1)), 0.05, 0.06)
    # The hood over the brush, its skirts and hangers.
    hood = [(0.17 * math.cos(math.radians(a)), 0.17 * math.sin(math.radians(a))) for a in range(15, 166, 10)]
    hood = hood + [(0.166 * yy / 0.17, 0.166 * zz / 0.17) for yy, zz in reversed(hood)]
    hood_m = L @ Matrix.Translation((0, 0, g.axis))
    p.add("paint", swept([(zz, yy) for yy, zz in hood], [(Vector((xx, 0, 0)), Z, Y) for xx in (-g.brush_half - 0.005, g.brush_half + 0.005)]), hood_m)
    for sy in (-1, 1):
        yy, zz = 0.17 * math.cos(math.radians(15)), 0.17 * math.sin(math.radians(15))
        p.add("plastic", box((2 * g.brush_half, 0.005, 0.08), (0, sy * yy, zz - 0.04), tile=0.3), hood_m)
    for xx in (-1.6, 0.0, 1.6):
        p.add("black", box((0.04, 0.03, b0 - g.axis - 0.168), (xx, 0, (b0 + g.axis + 0.168) / 2), tile=0.2), L)
    # The PV panel, battery, controller, beacon and camera.
    top = b0 + 0.12
    for xx in (0.6, 1.8):
        p.add("black", box((0.04, 0.04, 0.03), (xx, 0, top + 0.015), tile=0.2), L)
        p.add("alu", box((0.04, 0.66, 0.02), (xx, 0, top + 0.04), tile=0.2), L)
    p.add("pv", box((1.5, 0.62, 0.006), (1.2, 0, top + 0.06), tile=1.5), L)
    for sy in (-1, 1):
        p.add("alu", box((1.52, 0.02, 0.03), (1.2, sy * 0.32, top + 0.058), tile=0.3), L)
    for sx in (-1, 1):
        p.add("alu", box((0.02, 0.66, 0.03), (1.2 + sx * 0.76, 0, top + 0.058), tile=0.3), L)
    p.add("black", box((0.1, 0.08, 0.025), (1.2, 0.0, top + 0.0425), tile=0.2), L)
    battery(p, L @ Matrix.Translation((-1.45, 0, top + 0.11)), (1.0, 0.3, 0.22))
    ctl = L @ Matrix.Translation((-0.5, 0, top + 0.08))
    mk.cover(p, ctl, (0.5, 0.24, 0.16), key="paint", vents=3, bevel=0.01)
    p.add("plastic", rod(L @ Vector((-0.32, 0.08, top + 0.16)), L @ Vector((-0.32, 0.08, top + 0.5)), 0.005, 6))
    p.add("black", cyl(0.01, top + 0.5, top + 0.52, 8), L @ Matrix.Translation((-0.32, 0.08, 0)))
    estop(p, ctl @ at((0.1, -0.12, 0.0), (0, -1, 0)), 0.9)
    beacon(p, at(L @ Vector((2.3, 0.15, 0.33)), (0, 0, 1)), 0.035)
    eye = L @ Vector((2.0, -0.33, 0.29))
    camera_head(p, at(eye, (0, 1, 0.35)), 1.0)
    p.add("black", box((0.05, 0.04, 0.04), (2.0, -0.31, 0.29), tile=0.2), L)
    p.add("plastic", tube([L @ Vector((-2.3, 0.062, b0 + 0.06)), L @ Vector((2.3, 0.062, b0 + 0.06))], 0.012, 8))
    for e in (-1, 1):
        p.add("plastic", tube([L @ Vector((e * 2.3, 0.062, b0 + 0.06)), L @ Vector((e * 2.3, 0.33, 0.31)), L @ Vector((e * 2.43, 0.33, 0.13)),
                               L @ Vector((e * 2.5, 0.25, t.top + 0.06))], 0.01, 8))
    return p


def panel_brush_part(g):
    """The panel washer's brush: a core with end hubs and four microfibre fins wound round it."""
    o = slope(g, 0.0, g.axis, g.y)
    p = Part("brush", o.translation)
    p.add("black", cyl(g.core, -g.brush_half + 0.005, g.brush_half - 0.005, 14), o @ at((0, 0, 0), (1, 0, 0)))
    for s in (-1, 1):
        p.add("steel", cyl(0.06, -0.008, 0.008, 14), o @ at((s * (g.brush_half - 0.03), 0, 0), (1, 0, 0)))
    p.add("fibre", helix_fins(-g.brush_half + 0.04, g.brush_half - 0.04, 4, 2.5, g.core, g.reach), o)
    return p


def marks_part(g):
    """The panel washer's home magnets on the bottom rail's inner face, at the dock and at the row's far end."""
    t = g.track
    p = Part("track", slope(g, 0.0, 0.0, g.y).translation)
    for y in (t.dock, t.end):
        p.add("grey", box((0.01, 0.05, 0.03), (0, 0, 0)), slope(g, -t.s + 0.035, t.top - 0.025, y))
    return p


def panel_washer(g, note, name, bound):
    """The panel washer's machine: track (held to the world), frame, brush."""
    u = Usda("/robot/Looks")
    t = g.track
    open_machine(u, "panel_washer", (("travel", "joint_position"), ("brush", "joint_velocity")), "track", note, name)
    track_at = slope(g, 0.0, 0.0, g.y).translation
    body(u, 1, "track", tuple(track_at), 2, (0.1, 0.1, 0.1), link="base_link")
    visual(u, 2, name, "track", bound)
    end(u, 1)
    frame_at = slope(g, 0.0, g.beam, g.y).translation
    body(u, 1, "frame", tuple(frame_at), 60, (2.5, 120, 120), values=[f"float gearbox:value:range = {-g.span:.4f}"])
    turn = (0.0, -math.degrees(g.th), 0.0)
    rel = lambda s, tt: tuple(slope(g, s, tt, g.y).translation - frame_at)
    hull(u, 2, "beam", rel(0.0, 0.35), (4.4, 0.44, 0.46), turn)
    for e, side in ((-1, "low"), (1, "high")):
        hull(u, 2, f"drive_{side}", rel(e * 2.31, 0.21), (0.26, 0.66, 0.26), turn)
    visual(u, 2, name, "frame", bound)
    eye = slope(g, 2.0, 0.29, g.y - 0.33).translation
    camera(u, 2, tuple(eye - frame_at), looking((0.0, -1.0, -0.35)), 8.0)
    end(u, 1)
    brush_at = slope(g, 0.0, g.axis, g.y).translation
    body(u, 1, "brush", tuple(brush_at), 8, (0.05, 13, 13))
    visual(u, 2, name, "brush", bound)
    end(u, 1)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    joint(u, 2, "Fixed", "anchor", None, "track", panel_anchor(g))
    joint(u, 2, "Prismatic", "travel", "track", "frame", tuple(frame_at - track_at), "Y", (-g.span, 0), (600, 0.4))
    joint(u, 2, "Revolute", "brush", "frame", "brush", tuple(brush_at - frame_at), "X", None, (60, 20), Quaternion((0, 1, 0), -g.th))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def panel_anchor(g):
    """Where the panel washer's anchor holds its track, in the row's frame: the slope's centre line at the dock."""
    return tuple(slope(g, 0.0, 0.0, g.y).translation)


def panel_looks(g):
    return [marks_part(g), frame_part(g), panel_brush_part(g)]


# Building


def write(name, g, looks, machine, note, work):
    """Builds a washer's looks, writes them as `<name>_geom.usdc` and the
    machine as `<name>.usda` into `work`; returns the machine layer's path."""
    kept = (set(bpy.data.materials), set(bpy.data.images), set(bpy.data.meshes))
    mats = materials(kit.Looks(os.path.join(work, "maps")))
    scene = bpy.data.collections.new(f"{name}_geom")
    bpy.context.scene.collection.children.link(scene)
    objs = [part.finish(mats, scene) for part in looks(g)]
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
    path = os.path.join(work, f"{name}.usda")
    with open(path, "w") as f:
        f.write(machine(g, note, name, bound))
    return path


def models(arch_len, dome_r, panels, tilt, width=10.0, bay=3.0):
    """Each washer's sizes for modules built at these sizes: (name, model, looks, machine, note)."""
    return [
        ("arch_washer", arch(arch_len, width, bay), arch_looks, arch_washer, "the roof washer of an arch greenhouse"),
        ("dome_washer", dome(dome_r), dome_looks, dome_washer, "the glazing washer of a geodesic dome"),
        ("panel_washer", panel(panels, tilt), panel_looks, panel_washer, "the waterless washer of a solar row"),
    ]


def build(work, arch_len, dome_r, panels, tilt, width=10.0, bay=3.0):
    """Writes the three washers' machine layers and geometry into `work`:
    {name: machine layer path}, for arches `arch_len` long, `width` wide with
    arches `bay` apart, domes `dome_r` round and solar rows of `panels`
    tables tilted `tilt` degrees."""
    os.makedirs(work, exist_ok=True)
    return {name: write(name, g, looks, machine, note, work) for name, g, looks, machine, note in models(arch_len, dome_r, panels, tilt, width, bay)}
