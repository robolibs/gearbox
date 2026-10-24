"""Drivable rovers made from NASA's public-domain models, cut into the
bodies they move as: the chassis, and on each wheel module a suspension arm
hinged on the chassis, an upright steering on the arm's end and a pair of
twin wheels turning on the upright. An ackermann controller steers every
module and drives every wheel from `cmd_vel`.

Frame, as the other wheeled machines: +X left, +Y astern, +Z up, the
machine driving toward -Y, the tyres touching Z = 0.

Runs inside Blender: it imports the glTF, cuts it, exports the parts as a
geometry layer, and writes the machine beside it.
"""
import os
from dataclasses import dataclass

import bmesh
import bpy
import numpy as np
from mathutils import Vector

import weathering
from usda import Usda, fmt, schemas

POS = ("front", "middle", "rear")
SIDES = (("left", 1.0), ("right", -1.0))


@dataclass
class Rover:
    """Where a NASA model's wheel modules are, in the model's own frame: the
    hub Y of each, front first, the hub's |X| and Z (its steering axis stands
    through it), the |X| faces of each of the twin wheels, where the arms
    hinge on the chassis, and the box an upright's parts stand in. Masses in kg."""
    source: str
    kind: str
    note: str
    axles: tuple
    hub: tuple
    span: tuple
    radius: float
    pivot: tuple
    upright_x: float
    top_z: float
    arm_top: float
    rim: float
    masses: tuple
    body: tuple
    steer_deg: float = 60.0
    travel_deg: float = 12.0


SEV = Rover(
    source="SEV_full.glb", kind="sev_rover",
    note="NASA's Space Exploration Vehicle, a pressurised rover on six steered, sprung pairs of wheels",
    axles=(-0.83, 0.87, 2.57), hub=(1.70, -1.86), span=((1.38, 1.60), (1.80, 2.02)), radius=0.38,
    pivot=(0.69, -1.25), upright_x=1.45, top_z=-0.75, arm_top=-0.9, rim=0.30,
    masses=(2400.0, 25.0, 40.0, 60.0), body=((-1.55, -2.5, -1.4), (1.55, 3.3, 1.7)),
)


def modules(v):
    """Each wheel module: its name, side sign, hub Y."""
    return [(f"{POS[k]}_{side}", s, y) for k, y in enumerate(v.axles) for side, s in SIDES]


def loose_parts(bm):
    """Vertex index sets of each connected piece of a mesh."""
    seen, parts = set(), []
    for v in bm.verts:
        if v.index in seen:
            continue
        stack, part = [v], []
        seen.add(v.index)
        while stack:
            a = stack.pop()
            part.append(a.index)
            for e in a.link_edges:
                b = e.other_vert(a)
                if b.index not in seen:
                    seen.add(b.index)
                    stack.append(b)
        parts.append(part)
    return parts


def label(v, lo, hi):
    """Which body a loose piece with bounds `lo`..`hi` moves with."""
    c = (lo + hi) / 2
    s = 1.0 if c.x > 0 else -1.0
    near, far = sorted((s * lo.x, s * hi.x))
    for name, side, y in modules(v):
        if side != s:
            continue
        # A wheel's pieces are rings round its hub, or lie inside its tyre, within one wheel's width.
        ring = abs(c.y - y) < 0.06 and abs(c.z - v.hub[1]) < 0.08
        reach = max((yy - y) ** 2 + (zz - v.hub[1]) ** 2 for yy in (lo.y, hi.y) for zz in (lo.z, hi.z)) ** 0.5
        if (ring or reach <= v.radius + 0.02) and any(a - 0.02 <= near and far <= b + 0.02 for a, b in v.span):
            return f"wheel_{name}"
        if abs(c.y - y) < 0.45 and near >= v.upright_x and hi.z <= v.top_z and lo.z >= v.hub[1] - v.radius:
            return f"upright_{name}"
        if abs(c.y - y) < 0.35 and near >= v.pivot[0] - 0.05 and far <= v.upright_x + 0.15 and lo.z >= v.hub[1] - 0.1 and hi.z <= v.arm_top:
            return f"arm_{name}"
    return "chassis"


def split(v, obj):
    """The model's faces sorted into the bodies, by each loose piece's bounds."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bm.transform(obj.matrix_world)
    bm.verts.ensure_lookup_table()
    owner = {}
    for part in loose_parts(bm):
        cos = [bm.verts[i].co for i in part]
        lo = Vector((min(c.x for c in cos), min(c.y for c in cos), min(c.z for c in cos)))
        hi = Vector((max(c.x for c in cos), max(c.y for c in cos), max(c.z for c in cos)))
        name = label(v, lo, hi)
        for i in part:
            owner[i] = name
    return bm, owner


def mark_wheels(v, obj, bm, owner):
    """`wheel` and `tread` attributes on the model: 1 on the wheels, and on
    the tyres' rubber, their vertices out past the rims."""
    hubs = {name: y for name, _, y in modules(v)}
    wheel = np.zeros(len(bm.verts), dtype=np.float32)
    tread = np.zeros(len(bm.verts), dtype=np.float32)
    for vert in bm.verts:
        name = owner.get(vert.index, "")
        if name.startswith("wheel_"):
            y = hubs[name[len("wheel_"):]]
            wheel[vert.index] = 1.0
            tread[vert.index] = ((vert.co.y - y) ** 2 + (vert.co.z - v.hub[1]) ** 2) ** 0.5 >= v.rim
    obj.data.attributes.new("wheel", "FLOAT", "POINT").data.foreach_set("value", wheel)
    obj.data.attributes.new("tread", "FLOAT", "POINT").data.foreach_set("value", tread)


def piece(bm, owner, name, origin):
    """One body's faces as its own mesh, about the body's origin."""
    sub = bm.copy()
    sub.verts.ensure_lookup_table()
    bmesh.ops.delete(sub, geom=[vert for vert in sub.verts if owner.get(vert.index) != name], context="VERTS")
    bmesh.ops.translate(sub, vec=-origin, verts=sub.verts)
    mesh = bpy.data.meshes.new(name)
    sub.to_mesh(mesh)
    sub.free()
    return mesh


def origins(v):
    """Each body's origin in the model's frame: arms at their hinge, uprights
    and wheels at the hub, the chassis on the ground under its middle."""
    out = {"chassis": Vector((0.0, 0.0, v.hub[1] - v.radius))}
    for name, s, y in modules(v):
        out[f"arm_{name}"] = Vector((s * v.pivot[0], y, v.pivot[1]))
        out[f"upright_{name}"] = Vector((s * v.hub[0], y, v.hub[1]))
        out[f"wheel_{name}"] = Vector((s * v.hub[0], y, v.hub[1]))
    return out


def bindings(geom):
    """Each part's material binding, and its subsets', as (part, child path, material name)."""
    from pxr import Usd, UsdShade
    stage = Usd.Stage.Open(geom)
    found = []
    for part in stage.GetPrimAtPath("/geom").GetChildren():
        for prim in [part, *part.GetChildren()]:
            target = UsdShade.MaterialBindingAPI(prim).GetDirectBinding().GetMaterialPath()
            if not target.isEmpty:
                found.append((part.GetName(), "" if prim == part else prim.GetName(), target.name))
    return found


def build(v, nasa_dir, out_dir, work_dir):
    """Writes `<kind>_geom.usdc` and `<kind>.usda` into `work_dir`; returns the machine layer's path."""
    before = set(bpy.data.objects)
    kept = (set(bpy.data.materials), set(bpy.data.images))
    bpy.ops.import_scene.gltf(filepath=os.path.join(nasa_dir, v.source))
    new = [o for o in bpy.data.objects if o not in before]
    model = next(o for o in new if o.type == "MESH")
    bm, owner = split(v, model)
    mark_wheels(v, model, bm, owner)
    look = weathering.bake(model, model.data.materials[0], v.hub[1] - v.radius, os.path.join(work_dir, "bake"), v.kind)
    scene = bpy.data.collections.new(f"{v.kind}_geom")
    bpy.context.scene.collection.children.link(scene)
    for name, origin in origins(v).items():
        mesh = piece(bm, owner, name, origin)
        mesh.materials.append(look)
        scene.objects.link(bpy.data.objects.new(name, mesh))
    bm.free()
    os.makedirs(work_dir, exist_ok=True)
    geom = os.path.join(work_dir, f"{v.kind}_geom.usdc")
    bpy.ops.wm.usd_export(
        filepath=geom, check_existing=False, collection=scene.name, export_materials=True, generate_preview_surface=True,
        export_textures_mode="NEW", overwrite_textures=True, relative_paths=True, root_prim_path="/geom", merge_parent_xform=True,
        export_uvmaps=True, rename_uvmaps=True, export_normals=True, convert_scene_units="METERS", meters_per_unit=1.0,
        export_lights=False, export_cameras=False, use_instancing=False,
    )
    from pxr import Usd, UsdGeom
    stage = Usd.Stage.Open(geom)
    stage.SetDefaultPrim(stage.GetPrimAtPath("/geom"))
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage, 1.0)
    stage.GetRootLayer().Save()
    del stage
    bound = bindings(geom)
    meshes = [obj.data for obj in [*scene.objects, *new] if obj.type == "MESH"]
    for obj in [*scene.objects, *new]:
        bpy.data.objects.remove(obj)
    bpy.data.collections.remove(scene)
    for mesh in meshes:
        bpy.data.meshes.remove(mesh)
    for mat in set(bpy.data.materials) - kept[0]:
        bpy.data.materials.remove(mat)
    for image in set(bpy.data.images) - kept[1]:
        bpy.data.images.remove(image)
    path = os.path.join(work_dir, f"{v.kind}.usda")
    with open(path, "w") as f:
        f.write(machine(v, bound))
    return path


def box_inertia(mass, size):
    x, y, z = size
    return (mass * (y * y + z * z) / 12, mass * (x * x + z * z) / 12, mass * (x * x + y * y) / 12)


def drive(u, d, kind, stiffness, damping, max_force=None):
    """A joint's angular drive; gains per degree, as USD physics authors them."""
    u.emit(d, 'uniform token drive:angular:physics:type = "force"')
    u.emit(d, f"float drive:angular:physics:stiffness = {stiffness}")
    u.emit(d, f"float drive:angular:physics:damping = {damping}")
    if max_force:
        u.emit(d, f"float drive:angular:physics:maxForce = {max_force}")
    if kind == "position":
        u.emit(d, "float drive:angular:physics:targetPosition = 0")
    else:
        u.emit(d, "float drive:angular:physics:targetVelocity = 0")


def joint(u, d, name, body0, body1, at, axis, limit, drive_args):
    u.emit(d, f'def PhysicsRevoluteJoint "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsDriveAPI:angular"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, f"rel physics:body0 = </robot/{body0}>")
    u.emit(d + 1, f"rel physics:body1 = </robot/{body1}>")
    u.emit(d + 1, f'uniform token physics:axis = "{axis}"')
    u.emit(d + 1, f"point3f physics:localPos0 = {fmt(at)}")
    u.emit(d + 1, "point3f physics:localPos1 = (0, 0, 0)")
    u.emit(d + 1, "quatf physics:localRot0 = (1, 0, 0, 0)")
    u.emit(d + 1, "quatf physics:localRot1 = (1, 0, 0, 0)")
    if limit is not None:
        u.emit(d + 1, f"float physics:lowerLimit = {-limit}")
        u.emit(d + 1, f"float physics:upperLimit = {limit}")
    drive(u, d + 1, *drive_args)
    u.emit(d, "}")


def visual(u, d, kind, part, bound):
    """A body's look: its part of the geometry layer, bound to the machine's own copy of the materials."""
    u.emit(d, 'def "visual" (')
    u.emit(d + 1, f"prepend references = @./{kind}_geom.usdc@</geom/{part}>")
    u.emit(d, ")")
    u.emit(d, "{")
    for owner, child, material in bound:
        if owner != part:
            continue
        if child:
            u.emit(d + 1, f'over "{child}"')
            u.emit(d + 1, "{")
            u.emit(d + 2, f"rel material:binding = </robot/Looks/{material}>")
            u.emit(d + 1, "}")
        else:
            u.emit(d + 1, f"rel material:binding = </robot/Looks/{material}>")
    u.emit(d, "}")


def rigid(u, name, at, mass, inertia, link, role, extra=()):
    u.emit(1, f'def Xform "{name}" (')
    u.emit(2, 'prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]')
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(2, f'token gearbox:link:name = "{link}"')
    u.emit(2, f'token gearbox:link:role = "{role}"')
    u.emit(2, f"float physics:mass = {mass}")
    u.emit(2, f"vector3f physics:diagonalInertia = {fmt(inertia)}")
    for line in extra:
        u.emit(2, line)
    u.emit(2, f"double3 xformOp:translate = {fmt(at)}")
    u.emit(2, 'uniform token[] xformOpOrder = ["xformOp:translate"]')


def sensor(u, name, kind, at, rotate, attrs):
    u.emit(2, f'def Xform "{name}" (prepend apiSchemas = ["GearboxLinkAPI"])')
    u.emit(2, "{")
    u.emit(3, 'token gearbox:link:role = "sensor"')
    u.emit(3, f'token gearbox:sensor:kind = "{kind}"')
    for attr in attrs:
        u.emit(3, attr)
    u.emit(3, f"double3 xformOp:translate = {fmt(at)}")
    u.emit(3, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
    u.emit(2, "}")


def machine(v, bound):
    """The machine layer: the bodies, their joints, drives and the controller."""
    u = Usda("")
    o = origins(v)
    ground = o["chassis"]
    at = {name: p - ground for name, p in o.items()}
    chassis_kg, arm_kg, upright_kg, wheel_kg = v.masses
    mods = modules(v)
    joints = lambda prefix: ", ".join(f"</robot/Joints/{prefix}_{name}>" for name, _, _ in mods)
    spin, steer, arm = joints("spin"), joints("steer"), joints("arm")
    track = 2 * v.hub[0]
    base = max(v.axles) - min(v.axles)
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, 'upAxis = "Z"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "scripts/mars_base/vehicles.py: {v.note}"')
    u.emit(1, "}")
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/vehicles.py and build again. Drive it with")
    u.emit(0, "# `gearbox machine move --machine <id> ...` or `gearbox machine drive --machine <id>`.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'GearboxControllerAPI:drive'])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, 'token gearbox:machine:interfaceVersion = "0.1"')
    u.emit(1, f'token gearbox:machine:kind = "{v.kind}"')
    u.emit(1, 'token gearbox:machine:idPolicy = "prim_path"')
    u.emit(1, 'token gearbox:machine:upAxis = "Z"')
    u.emit(1, "rel gearbox:machine:body = </robot/chassis>")
    u.emit(1, "rel gearbox:machine:controlFrame = </robot/chassis/control_frame>")
    u.emit(1, f"rel gearbox:machine:role:poweredWheelJoints = [{spin}]")
    u.emit(1, f"rel gearbox:machine:role:brakeJoints = [{spin}]")
    u.emit(1, f"rel gearbox:machine:role:steeringJoints = [{steer}]")
    u.emit(1, f"rel gearbox:machine:role:suspensionJoints = [{arm}]")
    u.emit(1, "bool gearbox:controller:drive:enabled = true")
    u.emit(1, 'token gearbox:controller:drive:type = "builtin:ackermann_cmd_vel"')
    u.emit(1, 'token gearbox:controller:drive:steeringGeometry = "six_wheel_ackermann"')
    u.emit(1, "float gearbox:controller:drive:frontSteerMultiplier = 1")
    u.emit(1, "float gearbox:controller:drive:middleSteerMultiplier = 0")
    u.emit(1, "float gearbox:controller:drive:rearSteerMultiplier = -1")
    u.emit(1, "float gearbox:controller:drive:maxSteerDeg = 45")
    u.emit(1, "float gearbox:controller:drive:maxWheelTorqueNm = 800")
    u.emit(1, "float gearbox:controller:drive:maxPowerKw = 30")
    u.emit(1, 'token gearbox:controller:drive:zeroCommand = "brake"')
    u.emit(1, "float gearbox:controller:drive:updateRateHz = 60")
    u.emit(1, 'token gearbox:controller:drive:namespacePolicy = "machine_id"')
    u.emit(1, 'token gearbox:controller:drive:commandInterface = "cmd_vel"')
    u.emit(1, 'token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity", "joint_state"]')
    u.emit(1, 'token gearbox:controller:drive:frameConvention = "usd_z_up"')
    u.emit(1, f"float gearbox:controller:drive:wheelRadius = {v.radius}")
    u.emit(1, f"float gearbox:controller:drive:trackWidth = {track:.4f}")
    u.emit(1, f"float gearbox:controller:drive:frontTrackWidth = {track:.4f}")
    u.emit(1, f"float gearbox:controller:drive:rearTrackWidth = {track:.4f}")
    u.emit(1, f"float gearbox:controller:drive:wheelBase = {base:.4f}")
    u.emit(1, f"rel gearbox:controller:drive:driveWheelJoints = [{spin}]")
    u.emit(1, f"rel gearbox:controller:drive:steerJoints = [{steer}]")
    u.emit(1, "rel gearbox:controller:drive:target = </robot>")
    u.emit(1, "rel gearbox:controller:drive:body = </robot/chassis>")
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{v.kind}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")

    (x0, y0, z0), (x1, y1, z1) = v.body
    z0, z1 = z0 - ground.z, z1 - ground.z
    rigid(u, "chassis", (0, 0, 0), chassis_kg, box_inertia(chassis_kg, (x1 - x0, y1 - y0, z1 - z0)), "base_link", "base")
    # The control frame's +X is where the machine drives: the chassis's -Y.
    u.emit(2, 'def Xform "control_frame"')
    u.emit(2, "{")
    u.emit(3, "quatd xformOp:orient = (0.7071067811865476, 0, 0, -0.7071067811865476)")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:orient"]')
    u.emit(2, "}")
    visual(u, 2, v.kind, "chassis", bound)
    u.emit(2, 'def Mesh "body_collider" (')
    u.emit(3, 'prepend apiSchemas = ["PhysicsCollisionAPI", "PhysicsMeshCollisionAPI"]')
    u.emit(2, ")")
    u.emit(2, "{")
    u.emit(3, 'token visibility = "invisible"')
    u.emit(3, 'uniform token physics:approximation = "convexHull"')
    u.emit(3, 'uniform token subdivisionScheme = "none"')
    corners = [(x, y, z) for z in (z0, z1) for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1))]
    u.emit(3, "point3f[] points = [" + ", ".join(fmt(c) for c in corners) + "]")
    u.emit(3, "int[] faceVertexCounts = [4, 4, 4, 4, 4, 4]")
    u.emit(3, "int[] faceVertexIndices = [0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 2, 3, 7, 6, 1, 2, 6, 5, 3, 0, 4, 7]")
    u.emit(3, f"float3[] extent = [{fmt((x0, y0, z0))}, {fmt((x1, y1, z1))}]")
    u.emit(2, "}")
    nose = (0.0, y0 + 0.1, 2.2)
    sensor(u, "imu_link", "imu", (0, 0, 1.2), (0, 0, 0), ["float gearbox:sensor:rate_hz = 100"])
    sensor(u, "front_camera", "camera", nose, (0, 10, -90), [
        'token gearbox:sensor:channels = "color_depth"',
        "int gearbox:sensor:width = 320",
        "int gearbox:sensor:height = 240",
        "float gearbox:sensor:vfov_deg = 60",
        "float gearbox:sensor:range_m = 40",
        "float gearbox:sensor:rate_hz = 10",
    ])
    sensor(u, "lidar", "lidar", (0.0, y0 + 0.9, z1 + 0.05), (0, 0, -90), [
        "int gearbox:sensor:rows = 16",
        "int gearbox:sensor:columns = 360",
        "float gearbox:sensor:hfov_deg = 360",
        "float gearbox:sensor:vfov_deg = 30",
        "float gearbox:sensor:range_m = 60",
        "float gearbox:sensor:rate_hz = 10",
    ])
    u.emit(1, "}")

    width = v.span[-1][1] - v.span[0][0]
    for name, s, y in mods:
        rigid(u, f"arm_{name}", at[f"arm_{name}"], arm_kg, box_inertia(arm_kg, (0.9, 0.45, 0.5)), f"arm_{name}", "link")
        visual(u, 2, v.kind, f"arm_{name}", bound)
        u.emit(1, "}")
        rigid(u, f"upright_{name}", at[f"upright_{name}"], upright_kg, box_inertia(upright_kg, (0.45, 0.45, 0.9)), f"upright_{name}", "steer")
        visual(u, 2, v.kind, f"upright_{name}", bound)
        u.emit(1, "}")
        r = v.radius
        rigid(u, f"wheel_{name}", at[f"wheel_{name}"], wheel_kg,
              (wheel_kg * r * r / 2, wheel_kg * (3 * r * r + width * width) / 12, wheel_kg * (3 * r * r + width * width) / 12),
              f"wheel_{name}", "wheel", [f"float gearbox:value:tyre_width_m = {width:.3f}"])
        visual(u, 2, v.kind, f"wheel_{name}", bound)
        u.emit(2, 'def Cylinder "tire_collider" (')
        u.emit(3, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
        u.emit(2, ")")
        u.emit(2, "{")
        u.emit(3, 'token visibility = "invisible"')
        u.emit(3, 'uniform token axis = "X"')
        u.emit(3, f"double radius = {r}")
        u.emit(3, f"double height = {width:.3f}")
        u.emit(2, "}")
        u.emit(1, "}")

    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    for name, s, y in mods:
        joint(u, 2, f"arm_{name}", "chassis", f"arm_{name}", at[f"arm_{name}"], "Y", v.travel_deg, ("position", 1000, 60))
        joint(u, 2, f"steer_{name}", f"arm_{name}", f"upright_{name}", at[f"upright_{name}"] - at[f"arm_{name}"], "Z",
              v.steer_deg, ("position", 1500, 60, 4000))
        joint(u, 2, f"spin_{name}", f"upright_{name}", f"wheel_{name}", (0, 0, 0), "X", None, ("velocity", 0, 60, 800))
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()
