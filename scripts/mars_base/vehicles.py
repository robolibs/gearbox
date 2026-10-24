"""Drivable vehicles made from NASA's public-domain models: the model's
wheels are cut out of its mesh as their own bodies on revolute axles, the
rest is the chassis, and a skid-steer drive controller takes `cmd_vel`.

Frame, as the other wheeled machines: +X left, +Y astern, +Z up, the
machine driving toward -Y, the tyres touching Z = 0.

Runs inside Blender: it imports the glTF, splits it, exports the parts as a
geometry layer, and writes the machine beside it.
"""
import os
from dataclasses import dataclass

import bmesh
import bpy
from mathutils import Vector

from usda import Usda, fmt, schemas


@dataclass
class Vehicle:
    """What a NASA model needs to drive: its axles along Y, the hubs of the
    wheels on each side across X, their radius and tread, and masses."""
    source: str
    kind: str
    note: str
    axles: tuple
    hubs: tuple
    hub_z: float
    radius: float
    tread: float
    chassis_kg: float
    wheel_kg: float
    body: tuple


SEV = Vehicle(
    source="SEV.glb", kind="sev_rover", note="NASA's Space Exploration Vehicle, a pressurised rover on six pairs of wheels",
    axles=(-0.83, 0.87, 2.57), hubs=(-1.9, -1.45, 1.45, 1.9), hub_z=-1.86, radius=0.385, tread=0.2,
    chassis_kg=2600.0, wheel_kg=60.0, body=((-2.0, -2.5, -1.4), (2.0, 3.3, 1.7)),
)


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


def split(v, obj):
    """The model's faces, sorted into the chassis and each side's wheel pair
    of every axle, by where each loose piece's centre lies."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bm.transform(obj.matrix_world)
    bm.verts.ensure_lookup_table()
    pairs = [(sum(v.hubs[:2]) / 2, v.hubs[:2]), (sum(v.hubs[2:]) / 2, v.hubs[2:])]
    owner = {}
    for part in loose_parts(bm):
        c = sum((bm.verts[i].co for i in part), Vector()) / len(part)
        label = "chassis"
        for k, y in enumerate(v.axles):
            for side, (x, hubs) in enumerate(pairs):
                across = min(hubs) - v.tread <= c.x <= max(hubs) + v.tread
                round_ = (c.y - y) ** 2 + (c.z - v.hub_z) ** 2 <= (v.radius * 1.1) ** 2
                if across and round_:
                    label = f"wheel_{k}_{'left' if x > 0 else 'right'}"
        for i in part:
            owner[i] = label
    return bm, owner


def piece(bm, owner, label, offset):
    """One labelled piece as its own mesh, moved by `offset`."""
    sub = bm.copy()
    sub.verts.ensure_lookup_table()
    drop = [vert for vert in sub.verts if owner.get(vert.index) != label]
    bmesh.ops.delete(sub, geom=drop, context="VERTS")
    bmesh.ops.translate(sub, vec=offset, verts=sub.verts)
    mesh = bpy.data.meshes.new(label)
    sub.to_mesh(mesh)
    sub.free()
    return mesh


def build(v, nasa_dir, out_dir, work_dir):
    """Writes `<kind>_geom.usdc` and `<kind>.usda` into `work_dir`; returns the machine layer's path."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=os.path.join(nasa_dir, v.source))
    new = [o for o in bpy.data.objects if o not in before]
    model = next(o for o in new if o.type == "MESH")
    bm, owner = split(v, model)
    lift = v.radius - v.hub_z
    scene = bpy.data.collections.new(f"{v.kind}_geom")
    bpy.context.scene.collection.children.link(scene)
    meshes = {"chassis": piece(bm, owner, "chassis", Vector((0, 0, lift)))}
    hub_x = {"left": sum(v.hubs[2:]) / 2, "right": sum(v.hubs[:2]) / 2}
    for side, x in hub_x.items():
        meshes[f"wheel_{side}"] = piece(bm, owner, f"wheel_0_{side}", Vector((-x, -v.axles[0], -v.hub_z)))
    for name, mesh in meshes.items():
        for mat in model.data.materials:
            mesh.materials.append(mat)
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
    for obj in list(scene.objects):
        bpy.data.objects.remove(obj)
    bpy.data.collections.remove(scene)
    for obj in new:
        bpy.data.objects.remove(obj)
    path = os.path.join(work_dir, f"{v.kind}.usda")
    with open(path, "w") as f:
        f.write(machine(v, hub_x, lift))
    return path


def machine(v, hub_x, lift):
    """The machine layer: chassis and wheel bodies, axles, and the drive."""
    u = Usda("")
    wheels = [(f"wheel_{k}_{side}", side, x, y) for k, y in enumerate(v.axles) for side, x in hub_x.items()]
    track = hub_x["left"] - hub_x["right"]
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
    u.emit(1, "rel gearbox:machine:role:poweredWheelJoints = [" + ", ".join(f"</robot/Joints/axle_{n[6:]}>" for n, *_ in wheels) + "]")
    u.emit(1, "bool gearbox:controller:drive:enabled = true")
    u.emit(1, 'token gearbox:controller:drive:type = "builtin:diff_drive_cmd_vel"')
    u.emit(1, "float gearbox:controller:drive:updateRateHz = 60")
    u.emit(1, 'token gearbox:controller:drive:namespacePolicy = "machine_id"')
    u.emit(1, 'token gearbox:controller:drive:commandInterface = "cmd_vel"')
    u.emit(1, 'token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity", "joint_state"]')
    u.emit(1, 'token gearbox:controller:drive:frameConvention = "usd_z_up"')
    u.emit(1, f"float gearbox:controller:drive:wheelRadius = {v.radius}")
    u.emit(1, f"float gearbox:controller:drive:trackWidth = {track:.4f}")
    u.emit(1, f"float gearbox:controller:drive:wheelBase = {base:.4f}")
    u.emit(1, 'token[] gearbox:controller:drive:usesRoles = ["poweredWheelJoints"]')
    u.emit(1, "rel gearbox:controller:drive:target = </robot>")
    u.emit(1, "rel gearbox:controller:drive:body = </robot/chassis>")
    (x0, y0, z0), (x1, y1, z1) = v.body
    z0, z1 = z0 + lift, z1 + lift
    u.emit(1, 'def Xform "chassis" (')
    u.emit(2, 'prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI"]')
    u.emit(1, ")")
    u.emit(1, "{")
    # The control frame's +X is where the machine drives: the chassis's -Y.
    u.emit(2, 'def Xform "control_frame"')
    u.emit(2, "{")
    u.emit(3, "quatd xformOp:orient = (0.7071067811865476, 0, 0, -0.7071067811865476)")
    u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:orient"]')
    u.emit(2, "}")
    u.emit(2, f"float physics:mass = {v.chassis_kg}")
    sx, sy, sz = x1 - x0, y1 - y0, z1 - z0
    u.emit(2, f"vector3f physics:diagonalInertia = {fmt((v.chassis_kg * (sy * sy + sz * sz) / 12, v.chassis_kg * (sx * sx + sz * sz) / 12, v.chassis_kg * (sx * sx + sy * sy) / 12))}")
    u.emit(2, 'def "visual" (')
    u.emit(3, f"prepend references = @./{v.kind}_geom.usdc@</geom/chassis>")
    u.emit(2, ")")
    u.emit(2, "{")
    u.emit(2, "}")
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
    u.emit(1, "}")
    inertia_axle = v.wheel_kg * v.radius * v.radius / 2
    inertia_side = v.wheel_kg * (3 * v.radius * v.radius + v.tread * v.tread) / 12
    width = max(v.hubs) - min(h for h in v.hubs if h > 0) + v.tread
    for name, side, x, y in wheels:
        u.emit(1, f'def Xform "{name}" (')
        u.emit(2, 'prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI"]')
        u.emit(1, ")")
        u.emit(1, "{")
        u.emit(2, f"float physics:mass = {v.wheel_kg}")
        u.emit(2, f"vector3f physics:diagonalInertia = {fmt((inertia_axle, inertia_side, inertia_side))}")
        u.emit(2, f"double3 xformOp:translate = {fmt((x, y, v.radius))}")
        u.emit(2, 'uniform token[] xformOpOrder = ["xformOp:translate"]')
        u.emit(2, 'def "visual" (')
        u.emit(3, f"prepend references = @./{v.kind}_geom.usdc@</geom/wheel_{side}>")
        u.emit(2, ")")
        u.emit(2, "{")
        u.emit(2, "}")
        u.emit(2, 'def Cylinder "tire_collider" (')
        u.emit(3, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
        u.emit(2, ")")
        u.emit(2, "{")
        u.emit(3, 'token visibility = "invisible"')
        u.emit(3, 'uniform token axis = "X"')
        u.emit(3, f"double radius = {v.radius}")
        u.emit(3, f"double height = {width:.3f}")
        u.emit(2, "}")
        u.emit(1, "}")
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    for name, side, x, y in wheels:
        u.emit(2, f'def PhysicsRevoluteJoint "axle_{name[6:]}"')
        u.emit(2, "{")
        u.emit(3, "rel physics:body0 = </robot/chassis>")
        u.emit(3, f"rel physics:body1 = </robot/{name}>")
        u.emit(3, f"point3f physics:localPos0 = {fmt((x, y, v.radius))}")
        u.emit(3, "point3f physics:localPos1 = (0, 0, 0)")
        u.emit(3, "quatf physics:localRot0 = (1, 0, 0, 0)")
        u.emit(3, "quatf physics:localRot1 = (1, 0, 0, 0)")
        u.emit(3, 'uniform token physics:axis = "X"')
        u.emit(2, "}")
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()
