"""Robot arms from their makers' descriptions, for any machine to carry.

    UR10E  Universal Robots' UR10e: six revolute joints, 1.3 m reach, 12.5 kg
           payload, 33.5 kg. Joint frames, limits, efforts, speeds, masses
           and inertias are those of Universal_Robots_ROS2_Description
           (ur_description 4.4.0: config/ur10e/*.yaml, urdf/ur_macro.xacro
           at commit 81f7c9f); the looks are its visual meshes
           (meshes/ur10e/visual/*.dae), BSD-3-Clause like the rest of the
           package, fetched once into .work/harvester/downloads/ur.

An arm stands on a body of its machine: `mount` places its base_link in that
body's frame, and the base's looks and mass belong to that body. `looks`
gives each link's Part in the link's own frame and the base's in base_link's;
`bodies` writes the links as bodies posed at a rest pose, colliding as hidden
cylinders; `joints` writes the revolute joints with motors at the arm's
efforts and speeds. Each joint takes a `builtin:joint_position` controller of
its name with a position 0..1 of its working range: the joint's frame on its
parent is turned to the low end of that range, so the joint reads 0 there and
the range's span at the top, and the link's `gearbox:value:position` holds
the rest pose. `frames` is the forward kinematics, `solve` an inverse.

`build_pedestal` writes an arm alone on a floor pedestal, held to the world
by an anchor joint with an empty body0 (`pedestal_anchor` gives its frame
for a fleet file to set where the pedestal stands).
"""
import math
import os
import xml.etree.ElementTree as ET
from dataclasses import dataclass

import bmesh
import bpy
import numpy as np
from mathutils import Euler, Matrix, Vector

import kit
import mechanics as mk
from usda import Usda, end, fmt, schemas

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
UR_MESHES = os.path.join(ROOT, ".work", "harvester", "downloads", "ur", "meshes")

# Key: (material name, sRGB colour, roughness, metal, map settings).
FINISHES = {
    "ur_cap": ("ur_cap", (0.64, 0.76, 0.84), 0.42, 0.0, {"seed": 81, "grime": 0.05}),
    "ur_tube": ("ur_tube", (0.80, 0.81, 0.82), 0.36, 0.7, {"seed": 82, "brush": 0.05, "grime": 0.05}),
    "ur_joint": ("ur_joint", (0.68, 0.69, 0.71), 0.42, 0.4, {"seed": 83, "grime": 0.06}),
    "ur_black": ("ur_black", (0.06, 0.06, 0.065), 0.6, 0.0, {"seed": 84, "grime": 0.04}),
}
# The meshes' own materials, by the name their COLLADA files give them.
SOURCE_KEYS = {"URBlue": "ur_cap", "LinkGrey": "ur_tube", "JointGrey": "ur_joint", "Black": "ur_black"}


@dataclass
class Link:
    """A link: its visual mesh and where it sits in the link's frame (xyz,
    rpy), its mass, centre of mass, inertia tensor (ixx, ixy, ixz, iyy, iyz,
    izz) in a frame turned `axes` (rpy), and the cylinders it collides as:
    (centre, axis X/Y/Z, radius, length) in the link's frame."""
    name: str
    mesh: str
    offset: tuple
    mass: float
    com: tuple
    inertia: tuple
    axes: tuple
    hulls: tuple = ()


@dataclass
class Joint:
    """A revolute joint about its frame's Z: the frame on the parent link
    (xyz, rpy), its limit (± degrees), effort (N m) and speed (deg/s)."""
    name: str
    xyz: tuple
    rpy: tuple
    limit: float
    effort: float
    speed: float


@dataclass
class Arm:
    """A serial arm: its base, the link each joint carries, the joints, the
    base_link → base inertia frame yaw, the last link → flange and flange →
    tool0 turns (rpy), and its ready pose (degrees): upper arm up, forearm
    level, tool down."""
    kind: str
    meshes: str
    licence: str
    base: Link
    links: tuple
    joints: tuple
    base_yaw: float
    flange: tuple
    tool: tuple
    ready: tuple

    @property
    def mass(self):
        return self.base.mass + sum(link.mass for link in self.links)


PI, HALF = math.pi, math.pi / 2

UR10E = Arm(
    kind="ur10e",
    meshes=os.path.join(UR_MESHES, "ur10e", "visual"),
    licence="UR10e meshes, joint frames and inertias: ur_description 4.4.0 (github.com/UniversalRobots/Universal_Robots_ROS2_Description "
            "at 81f7c9f), BSD-3-Clause",
    base=Link("base", "base.dae", ((0, 0, 0), (0, 0, PI)), 4.0, (0, 0, 0), (0.0061, 0, 0, 0.0061, 0, 0.01125), (0, 0, 0)),
    links=(
        Link("shoulder_link", "shoulder.dae", ((0, 0, 0), (0, 0, PI)), 7.369, (0.021, -0.027, 0.0),
             (0.03408, 0.00002, -0.00425, 0.03529, 0.00008, 0.02156), (HALF, 0, 0),
             (((0, 0, 0.006), "Z", 0.077, 0.176), ((0, -0.01, 0), "Y", 0.077, 0.17))),
        Link("upper_arm_link", "upperarm.dae", ((0, 0, 0.1762), (HALF, 0, -HALF)), 13.051, (-0.2327, 0.0, 0.158),
             (0.02814, 0.00005, -0.01561, 0.77068, 0.00002, 0.76943), (0, 0, 0),
             (((0, 0, 0.182), "Z", 0.077, 0.175), ((-0.306, 0, 0.176), "X", 0.058, 0.46), ((-0.6127, 0, 0.176), "Z", 0.06, 0.15))),
        Link("forearm_link", "forearm.dae", ((0, 0, 0.0393), (HALF, 0, -HALF)), 3.989, (-0.33155, 0.0, 0.068),
             (0.01014, 0.00008, 0.00916, 0.30928, 0.0, 0.30646), (0, 0, 0),
             (((0, 0, 0.0435), "Z", 0.059, 0.125), ((-0.29, 0, 0.0393), "X", 0.045, 0.46), ((-0.5716, 0, 0.06), "Z", 0.046, 0.09))),
        Link("wrist_1_link", "wrist1.dae", ((0, 0, -0.135), (HALF, 0, 0)), 2.1, (0.0, -0.018, 0.007),
             (0.00296, -0.00001, 0.0, 0.00222, -0.00024, 0.00258), (HALF, 0, 0),
             (((0, 0, -0.0105), "Z", 0.047, 0.115), ((0, -0.005, 0), "Y", 0.047, 0.124))),
        Link("wrist_2_link", "wrist2.dae", ((0, 0, -0.12), (0, 0, 0)), 1.98, (0.0, 0.018, -0.007),
             (0.00296, -0.00001, 0.0, 0.00222, -0.00024, 0.00258), (-HALF, 0, 0),
             (((0, 0, -0.0035), "Z", 0.047, 0.1), ((0, 0.005, 0), "Y", 0.047, 0.125))),
        Link("wrist_3_link", "wrist3.dae", ((0, -0.0005, -0.1168), (HALF, 0, 0)), 0.615, (0.0, 0.0, -0.026),
             (0.0004, 0.0, 0.0, 0.00041, 0.0, 0.00034), (0, 0, 0),
             (((0, 0, -0.0245), "Z", 0.045, 0.049),)),
    ),
    joints=(
        Joint("shoulder_pan", (0, 0, 0.1807), (0, 0, 0), 360.0, 330.0, 120.0),
        Joint("shoulder_lift", (0, 0, 0), (HALF, 0, 0), 360.0, 330.0, 120.0),
        Joint("elbow", (-0.6127, 0, 0), (0, 0, 0), 180.0, 150.0, 180.0),
        Joint("wrist_1", (-0.57155, 0, 0.17415), (0, 0, 0), 360.0, 54.0, 180.0),
        Joint("wrist_2", (0, -0.11985, 0), (HALF, 0, 0), 360.0, 54.0, 180.0),
        Joint("wrist_3", (0, 0.11655, 0), (HALF, PI, PI), 360.0, 54.0, 180.0),
    ),
    base_yaw=PI,
    flange=(0, -HALF, -HALF),
    tool=(HALF, 0, HALF),
    ready=(0.0, -90.0, 90.0, -90.0, -90.0, 0.0),
)


def pose(xyz, rpy):
    """The matrix of a URDF origin: translation, then roll, pitch, yaw about fixed axes."""
    return Matrix.Translation(Vector(xyz)) @ Euler(rpy, "XYZ").to_matrix().to_4x4()


def spin(deg):
    return Matrix.Rotation(math.radians(deg), 4, "Z")


def frames(arm, base, q):
    """Each link's frame for joint angles `q` (degrees), and the flange's and
    tool0's, as matrices into the frame `base` places base_link in."""
    out = {"base_link": base}
    m = base @ Matrix.Rotation(arm.base_yaw, 4, "Z")
    out["base_inertia"] = m
    for joint, link, angle in zip(arm.joints, arm.links, q):
        m = m @ pose(joint.xyz, joint.rpy) @ spin(angle)
        out[link.name] = m
    out["flange"] = m @ pose((0, 0, 0), arm.flange)
    out["tool0"] = out["flange"] @ pose((0, 0, 0), arm.tool)
    return out


def tool0(arm):
    """tool0 in the last link's frame."""
    return pose((0, 0, 0), arm.flange) @ pose((0, 0, 0), arm.tool)


def ranges(arm, rest, limits=None):
    """Each joint's working range (degrees): as given, else 170° either side
    of the rest pose within the joint's limits."""
    out = []
    for joint, angle in zip(arm.joints, rest):
        if limits and joint.name in limits:
            lo, hi = limits[joint.name]
        else:
            lo, hi = max(-joint.limit, angle - 170.0), min(joint.limit, angle + 170.0)
        if not lo <= angle <= hi or angle - lo >= 179.0:
            raise ValueError(f"{arm.kind} {joint.name}: rest {angle}° must lie in {lo}..{hi}° within 179° of its low end")
        out.append((lo, hi))
    return out


# Looks


def collada(path):
    """A COLLADA file's triangles as bmeshes per source material, in metres,
    with the file's node transforms applied."""
    ns = {"c": "http://www.collada.org/2005/11/COLLADASchema"}
    root = ET.parse(path).getroot()
    unit = float(root.find("c:asset/c:unit", ns).get("meter", "1"))
    geoms = {g.get("id"): g for g in root.iterfind(".//c:library_geometries/c:geometry", ns)}
    out = {}

    def floats(source):
        values = [float(v) for v in source.find("c:float_array", ns).text.split()]
        stride = int(source.find(".//c:accessor", ns).get("stride", "1"))
        return [values[i:i + stride] for i in range(0, len(values), stride)]

    def mesh(geom, m, bind):
        body = geom.find("c:mesh", ns)
        sources = {s.get("id"): s for s in body.findall("c:source", ns)}
        verts = body.find("c:vertices", ns)
        position = verts.find("c:input[@semantic='POSITION']", ns).get("source")[1:]
        points = [m @ Vector(p) for p in floats(sources[position])]
        for prim in body:
            tag = prim.tag.split("}")[1]
            if tag not in ("triangles", "polylist"):
                continue
            inputs = prim.findall("c:input", ns)
            stride = max(int(i.get("offset")) for i in inputs) + 1
            at = next(int(i.get("offset")) for i in inputs if i.get("semantic") == "VERTEX")
            p = [int(v) for v in prim.find("c:p", ns).text.split()]
            counts = prim.find("c:vcount", ns)
            counts = [int(v) for v in counts.text.split()] if counts is not None else [3] * int(prim.get("count"))
            symbol = bind.get(prim.get("material"), prim.get("material") or "")
            bm = out.setdefault(symbol.replace("-material", ""), bmesh.new())
            made = {}
            k = 0
            for n in counts:
                ids = [p[(k + i) * stride + at] for i in range(n)]
                k += n
                if len(set(ids)) < 3:
                    continue
                vs = []
                for i in ids:
                    if i not in made:
                        made[i] = bm.verts.new(points[i])
                    vs.append(made[i])
                try:
                    bm.faces.new(vs)
                except ValueError:
                    pass

    def walk(node, m):
        local = node.find("c:matrix", ns)
        if local is not None:
            v = [float(x) for x in local.text.split()]
            m = m @ Matrix([v[0:4], v[4:8], v[8:12], v[12:16]])
        for inst in node.findall("c:instance_geometry", ns):
            bind = {b.get("symbol"): b.get("target")[1:] for b in inst.iterfind(".//c:instance_material", ns)}
            mesh(geoms[inst.get("url")[1:]], m, bind)
        for child in node.findall("c:node", ns):
            walk(child, m)

    for scene in root.iterfind(".//c:library_visual_scenes/c:visual_scene", ns):
        for node in scene.findall("c:node", ns):
            walk(node, Matrix.Scale(unit, 4))
    for bm in out.values():
        bmesh.ops.remove_doubles(bm, verts=bm.verts[:], dist=1e-6)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
        kit.project_uv(bm, 0.3)
    return out


def materials(looks):
    return {key: looks.get(name, colour, rough, metal, maps=mk.finish(colour, rough, **maps)) for key, (name, colour, rough, metal, maps) in FINISHES.items()}


def link_part(arm, link, name, frame=None):
    """A link's mesh as a Part in the link's frame, or in `frame` taking the link's frame into another."""
    part = mk.Part(name, (0, 0, 0))
    m = (frame or Matrix.Identity(4)) @ pose(*link.offset)
    for source, bm in collada(os.path.join(arm.meshes, link.mesh)).items():
        part.add(SOURCE_KEYS.get(source, "ur_black"), bm, m)
    return part


def looks(arm, prefix=""):
    """The base's Part in base_link's frame, as `<prefix>base`, and each
    link's in its own frame, by link name."""
    parts = {"base": link_part(arm, arm.base, f"{prefix}{arm.kind}_base", Matrix.Rotation(arm.base_yaw, 4, "Z"))}
    for link in arm.links:
        parts[link.name] = link_part(arm, link, f"{prefix}{link.name}")
    return parts


# The machine layer


def quat(m):
    q = m.to_quaternion()
    return (q.w, q.x, q.y, q.z)


def principal(link):
    """A link's principal moments and the quaternion of their axes in the link's frame."""
    xx, xy, xz, yy, yz, zz = link.inertia
    tensor = np.array([[xx, xy, xz], [xy, yy, yz], [xz, yz, zz]])
    turn = np.array(Euler(link.axes, "XYZ").to_matrix())
    moments, axes = np.linalg.eigh(turn @ tensor @ turn.T)
    if np.linalg.det(axes) < 0:
        axes[:, 2] *= -1
    return tuple(float(v) for v in moments), quat(Matrix(axes.tolist()).to_4x4())


def visual(u, d, geom, part, bound, prim="visual"):
    """A body's look: its part of the geometry layer, bound to the machine's copy of the materials."""
    u.emit(d, f'def "{prim}" (')
    u.emit(d + 1, f"prepend references = @./{geom}_geom.usdc@</geom/{part}>")
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


def hull(u, d, name, centre, axis, radius, length):
    """A hidden cylinder the body collides as."""
    u.emit(d, f'def Cylinder "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsCollisionAPI"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, 'token visibility = "invisible"')
    u.emit(d + 1, 'uniform token purpose = "guide"')
    u.emit(d + 1, f'uniform token axis = "{axis}"')
    u.emit(d + 1, f"double radius = {radius}")
    u.emit(d + 1, f"double height = {length}")
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate"]')
    u.emit(d, "}")


def body_prim(u, d, name, m, mass, com, moments, axes, values=(), link=None, role="link"):
    """Opens a rigid body posed by `m` in the machine's frame; close it with `usda.end`."""
    u.emit(d, f'def Xform "{name}" (')
    u.emit(d + 1, 'prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]')
    u.emit(d, ")")
    u.emit(d, "{")
    u.emit(d + 1, f'token gearbox:link:name = "{link or name}"')
    u.emit(d + 1, f'token gearbox:link:role = "{role}"')
    for value in values:
        u.emit(d + 1, value)
    u.emit(d + 1, f"float physics:mass = {mass:g}")
    u.emit(d + 1, f"point3f physics:centerOfMass = {fmt(com)}")
    u.emit(d + 1, f"float3 physics:diagonalInertia = ({', '.join(f'{v:.6g}' for v in moments)})")
    u.emit(d + 1, f"quatf physics:principalAxes = ({', '.join(f'{v:.6f}' for v in axes)})")
    u.emit(d + 1, f"double3 xformOp:translate = {fmt(m.translation)}")
    u.emit(d + 1, f"quatf xformOp:orient = ({', '.join(f'{v:.6f}' for v in quat(m))})")
    u.emit(d + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:orient"]')


def bodies(u, d, arm, base, rest, geom, bound, prefix="", limits=None, extra=None):
    """The arm's links as bodies, posed at `rest` (degrees) with `base` placing
    base_link in the machine's frame; `extra` maps a link name to a function
    `(u, depth)` adding prims inside that body."""
    at = frames(arm, base, rest)
    for link, joint, angle, (lo, hi) in zip(arm.links, arm.joints, rest, ranges(arm, rest, limits)):
        moments, axes = principal(link)
        span = math.radians(hi - lo)
        body_prim(u, d, f"{prefix}{link.name}", at[link.name], link.mass, link.com, moments, axes, [
            f"float gearbox:value:range = {span:.5f}",
            f"float gearbox:value:position = {(angle - lo) / (hi - lo):.5f}",
        ])
        for k, (centre, axis, radius, length) in enumerate(link.hulls):
            hull(u, d + 1, f"hull{k}", centre, axis, radius, length)
        visual(u, d + 1, geom, f"{prefix}{link.name}", bound)
        if extra and link.name in extra:
            extra[link.name](u, d + 1)
        u.emit(d, "}")


def joints(u, d, arm, mount_body, mount, rest, prefix="", limits=None, root="/robot"):
    """The arm's revolute joints, the first on `mount_body` with `mount`
    placing base_link in that body's frame."""
    parent, offset = mount_body, mount @ Matrix.Rotation(arm.base_yaw, 4, "Z")
    for link, joint, (lo, hi) in zip(arm.links, arm.joints, ranges(arm, rest, limits)):
        frame = offset @ pose(joint.xyz, joint.rpy) @ spin(lo)
        u.emit(d, f'def PhysicsRevoluteJoint "{prefix}{joint.name}"')
        u.emit(d, "{")
        u.emit(d + 1, f"rel physics:body0 = <{root}/{parent}>")
        u.emit(d + 1, f"rel physics:body1 = <{root}/{prefix}{link.name}>")
        u.emit(d + 1, 'uniform token physics:axis = "Z"')
        u.emit(d + 1, f"point3f physics:localPos0 = {fmt(frame.translation)}")
        u.emit(d + 1, f"quatf physics:localRot0 = ({', '.join(f'{v:.6f}' for v in quat(frame))})")
        u.emit(d + 1, "point3f physics:localPos1 = (0, 0, 0)")
        u.emit(d + 1, "quatf physics:localRot1 = (1, 0, 0, 0)")
        u.emit(d + 1, "float physics:lowerLimit = 0")
        u.emit(d + 1, f"float physics:upperLimit = {hi - lo:g}")
        u.emit(d + 1, "bool physics:collisionEnabled = 0")
        u.emit(d + 1, f"float gearbox:motor:maxForce = {joint.effort:g}")
        u.emit(d + 1, f"float gearbox:motor:maxVelocity = {math.radians(joint.speed):.4f}")
        u.emit(d, "}")
        parent, offset = f"{prefix}{link.name}", Matrix.Identity(4)


def controllers(arm, prefix=""):
    """The arm's `builtin:joint_position` controllers, one per joint, by name."""
    return [f"{prefix}{joint.name}" for joint in arm.joints]


# Inverse kinematics


def solve(arm, base, target, seed, tip=None, limits=None, steps=400, rest=None):
    """Joint angles (degrees) putting `tip` (a frame in tool0's, tool0 itself
    by default) at the matrix `target`, from `seed`, within the working
    ranges; with `target` a Vector, only the tip's position. Returns the
    angles and the position and angle errors (m, degrees) left."""
    tip = tip or Matrix.Identity(4)
    span = ranges(arm, rest or arm.ready, limits)
    q = np.array(seed, dtype=float)
    full = isinstance(target, Matrix)
    goal = target.translation if full else Vector(target)

    def error(angles):
        m = frames(arm, base, list(angles))["tool0"] @ tip
        e = list(goal - m.translation)
        if full:
            r = (target.to_3x3() @ m.to_3x3().transposed()).to_quaternion()
            e += list(r.axis * r.angle * 0.3)
        return np.array(e)

    lam = 0.05
    for _ in range(steps):
        e = error(q)
        if np.linalg.norm(e[:3]) < 1e-4 and (not full or np.linalg.norm(e[3:]) < 1e-4):
            break
        jac = np.zeros((len(e), len(q)))
        for i in range(len(q)):
            dq = q.copy()
            dq[i] += 0.01
            jac[:, i] = (e - error(dq)) / math.radians(0.01)
        step = jac.T @ np.linalg.solve(jac @ jac.T + lam * lam * np.eye(len(e)), e)
        q = q + np.degrees(np.clip(step, -0.2, 0.2))
        q = np.array([min(max(v, lo), hi) for v, (lo, hi) in zip(q, span)])
    m = frames(arm, base, list(q))["tool0"] @ tip
    angle = math.degrees((target.to_3x3() @ m.to_3x3().transposed()).to_quaternion().angle) if full else 0.0
    return [float(v) for v in q], (goal - m.translation).length, angle


# Writing a machine


def snapshot():
    """The materials, images and meshes Blender holds now, for `tidy`."""
    return set(bpy.data.materials), set(bpy.data.images), set(bpy.data.meshes)


def tidy(kept):
    """Removes the materials, images and meshes made since `snapshot`."""
    for mesh in set(bpy.data.meshes) - kept[2]:
        bpy.data.meshes.remove(mesh)
    for mat in set(bpy.data.materials) - kept[0]:
        bpy.data.materials.remove(mat)
    for image in set(bpy.data.images) - kept[1]:
        bpy.data.images.remove(image)


def write_geom(parts, mats, geom):
    """Writes Parts as the geometry layer `geom`, removing the objects made
    for them; returns the material bindings each body's visual rebinds."""
    from vehicles import bindings, export_geom
    scene = bpy.data.collections.new(os.path.splitext(os.path.basename(geom))[0])
    bpy.context.scene.collection.children.link(scene)
    objs = [part.finish(mats, scene) for part in parts]
    export_geom(scene, geom)
    bound = bindings(geom)
    for obj in objs:
        bpy.data.objects.remove(obj)
    bpy.data.collections.remove(scene)
    return bound


def pedestal_anchor():
    """Where a pedestal arm's anchor holds its pedestal's foot, in the machine's frame."""
    return (0.0, 0.0, 0.0)


def plate(size, at):
    return kit.box(size, at, tile=0.3, bevel=0.003)


def pedestal_part(arm, height):
    """A floor pedestal: a base plate on four anchor bolts, a gusseted steel
    tube, its top plate under the arm's base, and the arm cable's gland."""
    p = mk.Part("pedestal", (0, 0, 0))
    p.add("grey", plate((0.4, 0.4, 0.02), (0, 0, 0.01)))
    for sx in (-1, 1):
        for sy in (-1, 1):
            p.add("steel", mk.hexagon(0.016, 0.02, 0.032), Matrix.Translation((sx * 0.16, sy * 0.16, 0)))
            p.add("steel", mk.cyl(0.008, 0.032, 0.05, 10), Matrix.Translation((sx * 0.16, sy * 0.16, 0)))
    p.add("grey", mk.turned([(0.0, 0.02), (0.1, 0.02), (0.1, height - 0.02), (0.0, height - 0.02)], 40))
    for k in range(4):
        gusset = kit.prism([(0.0, 0.0), (0.09, 0.0), (0.0, 0.16)], -0.006, 0.006, 0.3)
        p.add("grey", gusset, Matrix.Rotation(math.pi / 2 * k + math.pi / 4, 4, "Z") @ Matrix.Translation((0.1, 0, 0.02)) @ Matrix.Rotation(math.pi / 2, 4, "X"))
    p.add("grey", plate((0.24, 0.24, 0.02), (0, 0, height - 0.01)))
    for k in range(4):
        a = math.pi / 2 * k + math.pi / 4
        p.add("steel", mk.hexagon(0.009, height, height + 0.008), Matrix.Translation((0.1 * math.cos(a), 0.1 * math.sin(a), 0)))
    p.add("black", mk.cyl(0.016, 0.0, 0.02, 14), mk.at((0.1, 0, 0.3), (1, 0, 0)))
    return p


def pedestal(arm, name, height, bound, rest, limits=None):
    """The machine layer of an arm on a pedestal `height` tall held to the world."""
    u = Usda("/robot/Looks")
    names = controllers(arm)
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "scripts/mars_base/arms.py: {arm.kind} on a floor pedestal"')
    u.emit(2, f'string licence = "{arm.licence}"')
    u.emit(1, "}")
    u.emit(1, 'defaultPrim = "robot"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, "kilogramsPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base/arms.py and build again.")
    u.emit(0, "# Each joint takes a position 0..1 of its range: `gearbox machine set-value <joint> position 0.5 --machine <id>`.")
    u.emit(0, 'def Xform "robot" (')
    u.emit(1, f"prepend apiSchemas = {schemas(['GearboxMachineAPI', 'PhysicsArticulationRootAPI', *(f'GearboxControllerAPI:{c}' for c in names)])}")
    u.emit(0, ")")
    u.emit(0, "{")
    u.emit(1, 'token gearbox:machine:kind = "arm"')
    u.emit(1, "rel gearbox:machine:body = </robot/pedestal>")
    for c in names:
        u.emit(1, f'token gearbox:controller:{c}:type = "builtin:joint_position"')
        u.emit(1, f"rel gearbox:controller:{c}:target = </robot/Joints/{c}>")
    u.emit(1, 'def "Looks" (')
    u.emit(2, f"prepend references = @./{name}_geom.usdc@</geom/_materials>")
    u.emit(1, ")")
    u.emit(1, "{")
    u.emit(1, "}")
    mass = 40.0 + arm.base.mass
    body_prim(u, 1, "pedestal", Matrix.Identity(4), mass, (0, 0, height / 2), (mass * height * height / 12, mass * height * height / 12, mass * 0.02),
              (1.0, 0.0, 0.0, 0.0), link="base_link", role="base")
    hull(u, 2, "tube", (0, 0, height / 2), "Z", 0.1, height)
    hull(u, 2, "base", (0, 0, height + 0.05), "Z", 0.095, 0.1)
    visual(u, 2, name, "pedestal", bound)
    visual(u, 2, name, f"{arm.kind}_base", bound, "arm_base")
    end(u, 1)
    mount = Matrix.Translation((0, 0, height))
    bodies(u, 1, arm, mount, rest, name, bound, limits=limits)
    u.emit(1, 'def Scope "Joints"')
    u.emit(1, "{")
    u.emit(2, 'def PhysicsFixedJoint "anchor"')
    u.emit(2, "{")
    u.emit(3, "rel physics:body1 = </robot/pedestal>")
    u.emit(3, f"point3f physics:localPos0 = {fmt(pedestal_anchor())}")
    u.emit(3, "point3f physics:localPos1 = (0, 0, 0)")
    u.emit(2, "}")
    joints(u, 2, arm, "pedestal", mount, rest, limits=limits)
    u.emit(1, "}")
    u.emit(0, "}")
    return u.text()


def build_pedestal(work, arm=UR10E, height=0.8, name=None, rest=None, limits=None):
    """Builds an arm on a floor pedestal: `<name>_geom.usdc` and `<name>.usda`
    in `work`; returns {name: layer path}."""
    name = name or f"{arm.kind}_pedestal"
    rest = rest or arm.ready
    os.makedirs(work, exist_ok=True)
    kept = snapshot()
    looks_ = kit.Looks(os.path.join(work, "maps"))
    mats = mk.materials(looks_)
    mats.update(materials(looks_))
    parts = looks(arm)
    base = parts.pop("base")
    shifted = mk.Part(base.name, (0, 0, 0))
    for key, bm in base.pieces.items():
        shifted.add(key, bm, Matrix.Translation((0, 0, height)))
    bound = write_geom([pedestal_part(arm, height), shifted, *parts.values()], mats, os.path.join(work, f"{name}_geom.usdc"))
    tidy(kept)
    path = os.path.join(work, f"{name}.usda")
    with open(path, "w") as f:
        f.write(pedestal(arm, name, height, bound, rest, limits))
    return {name: path}
