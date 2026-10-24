"""Modelling kit for the Mars base: solids of revolution, sweeps, trusses and
boxes built with bmesh, UV-mapped in metres, with textured materials.

Runs inside Blender. Every builder returns a bmesh; `put` turns one into an
object in a building's collection with a material.
"""
import math
import os

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

TAU = 2.0 * math.pi
HERE = os.path.dirname(os.path.abspath(__file__))


# Materials


def srgb_to_linear(v):
    return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(v):
    return np.where(v <= 0.0031308, v * 12.92, 1.055 * np.power(np.maximum(v, 0.0), 1.0 / 2.4) - 0.055)


class Looks:
    """Principled materials over image textures, made once per name. A
    material's colour map is its texture's detail, luminance over its mean,
    times the material's colour, baked to `textures/<name>.jpg`."""

    def __init__(self, textures):
        self.textures = textures
        self.made = {}

    def image(self, path, colour_data):
        image = bpy.data.images.load(path, check_existing=True)
        if not colour_data:
            image.colorspace_settings.name = "Non-Color"
        return image

    def save(self, name, array, srgb):
        """Writes an (h, w), (h, w, 3) or, with alpha as a PNG, (h, w, 4)
        array in [0, 1] as `textures/<name>.jpg`."""
        if array.ndim == 2:
            array = np.repeat(array[..., None], 3, axis=-1)
        alpha = array.shape[-1] == 4
        out = os.path.join(self.textures, f"{name}.{'png' if alpha else 'jpg'}")
        h, w = array.shape[:2]
        rgb = np.clip(array[..., :3], 0.0, 1.0)
        rgb = linear_to_srgb(rgb) if srgb else rgb
        a = array[..., 3:4] if alpha else np.ones((h, w, 1), dtype=np.float32)
        # Blender's pixel rows run bottom to top.
        rgba = np.concatenate([rgb, a], axis=-1)[::-1].reshape(-1, 4)
        dst = bpy.data.images.new(f"{name}_out", w, h, alpha=alpha)
        dst.pixels.foreach_set(rgba.astype(np.float32).ravel())
        dst.filepath_raw = out
        dst.file_format = "PNG" if alpha else "JPEG"
        bpy.context.scene.render.image_settings.quality = 92
        dst.save()
        bpy.data.images.remove(dst)
        return self.image(out, srgb)

    def tinted(self, name, tex, colour, contrast):
        src = bpy.data.images.load(os.path.join(self.textures, tex, "diff.jpg"), check_existing=True)
        w, h = src.size
        px = np.empty(w * h * 4, dtype=np.float32)
        src.pixels.foreach_get(px)
        rgb = srgb_to_linear(px.reshape(h, w, 4)[::-1, :, :3])
        lum = rgb @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
        detail = np.power(lum / max(float(lum.mean()), 1e-4), contrast)
        return self.save(name, np.array(colour, dtype=np.float32) * detail[..., None], True)

    def get(self, name, colour=(0.8, 0.8, 0.8), rough=0.5, metal=0.0, tex=None, emit=None, alpha=1.0, contrast=0.8, maps=None):
        """A material; `tex` names a Poly Haven set to tint, `maps` a
        procedural (diff, nor, rough) triple from `patterns`."""
        if name in self.made:
            return self.made[name]
        # Colours are given as sRGB, as picked; materials hold them linear.
        colour = tuple(float(c) for c in srgb_to_linear(np.array(colour, dtype=np.float32)))
        if emit:
            emit = (tuple(float(c) for c in srgb_to_linear(np.array(emit[0], dtype=np.float32))), emit[1])
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        bsdf = nodes["Principled BSDF"]
        bsdf.inputs["Base Color"].default_value = (*colour, 1.0)
        bsdf.inputs["Roughness"].default_value = rough
        bsdf.inputs["Metallic"].default_value = metal
        diff = rough_map = nor = None
        if tex:
            folder = os.path.join(self.textures, tex)
            diff = self.tinted(name, tex, colour, contrast)
            rough_map = os.path.join(folder, "rough.jpg")
            rough_map = self.image(rough_map, False) if os.path.exists(rough_map) else None
            nor = os.path.join(folder, "nor.jpg")
            nor = self.image(nor, False) if os.path.exists(nor) else None
        if maps:
            d, n, r = maps() if callable(maps) else maps
            diff = self.save(name, d, True)
            nor = self.save(f"{name}_nor", n, False) if n is not None else None
            rough_map = self.save(f"{name}_rough", r, False) if r is not None else None
        if diff:
            node = nodes.new("ShaderNodeTexImage")
            node.image = diff
            links.new(node.outputs["Color"], bsdf.inputs["Base Color"])
            if diff.filepath.endswith(".png"):
                links.new(node.outputs["Alpha"], bsdf.inputs["Alpha"])
                mat.surface_render_method = "BLENDED"
        if rough_map:
            node = nodes.new("ShaderNodeTexImage")
            node.image = rough_map
            links.new(node.outputs["Color"], bsdf.inputs["Roughness"])
        if nor:
            node = nodes.new("ShaderNodeTexImage")
            node.image = nor
            normal = nodes.new("ShaderNodeNormalMap")
            links.new(node.outputs["Color"], normal.inputs["Color"])
            links.new(normal.outputs["Normal"], bsdf.inputs["Normal"])
        if emit:
            bsdf.inputs["Emission Color"].default_value = (*emit[0], 1.0)
            bsdf.inputs["Emission Strength"].default_value = emit[1]
        if alpha < 1.0:
            bsdf.inputs["Alpha"].default_value = alpha
            mat.surface_render_method = "BLENDED"
        self.made[name] = mat
        return mat


# Mesh builders


def lathe(profile, segments=48, tile=2.0, angle=TAU, start=0.0, swap=False):
    """A solid of revolution about Z from (radius, z) points, bottom to top.
    A zero radius closes the surface at a pole; `swap` turns the texture a
    quarter, so stripes run round rather than up."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    closed = abs(angle - TAU) < 1e-6
    steps = segments if closed else segments + 1
    angles = [start + angle * i / segments for i in range(steps)]
    along = [0.0]
    for (r0, z0), (r1, z1) in zip(profile, profile[1:]):
        along.append(along[-1] + math.hypot(r1 - r0, z1 - z0))
    reach = max(r for r, _ in profile) or 1.0
    rings = []
    for r, z in profile:
        if r <= 1e-6:
            rings.append([bm.verts.new((0.0, 0.0, z))] * steps)
        else:
            rings.append([bm.verts.new((r * math.cos(a), r * math.sin(a), z)) for a in angles])
    for k in range(len(profile) - 1):
        lo, hi = rings[k], rings[k + 1]
        for i in range(segments):
            j = (i + 1) % steps
            corners = [(lo[i], i, k), (lo[j], i + 1, k), (hi[j], i + 1, k + 1), (hi[i], i, k + 1)]
            seen, keep = set(), []
            for corner in corners:
                if corner[0] not in seen:
                    seen.add(corner[0])
                    keep.append(corner)
            if len(keep) < 3:
                continue
            face = bm.faces.new([c[0] for c in keep])
            for loop, (_, col, row) in zip(face.loops, keep):
                st = (angle * reach * col / segments / tile, along[row] / tile)
                loop[uv].uv = st[::-1] if swap else st
    return bm


def sphere(radius, at=(0, 0, 0), segments=24, rings=12, tile=2.0, low=-math.pi / 2):
    """A sphere, or a cap of one from latitude `low` up."""
    lats = [low + (math.pi / 2 - low) * i / rings for i in range(rings + 1)]
    profile = [(radius * math.cos(a), radius * math.sin(a)) for a in lats]
    profile[-1] = (0.0, radius)
    if low <= -math.pi / 2 + 1e-6:
        profile[0] = (0.0, -radius)
    return moved(lathe(profile, segments, tile), at)


def cone(r0, r1, h, at=(0, 0, 0), segments=24, tile=2.0, caps=True):
    profile = ([(0.0, 0.0)] if caps else []) + [(r0, 0.0), (r1, h)] + ([(0.0, h)] if caps and r1 > 0 else [])
    return moved(lathe(profile, segments, tile), at)


def disc(radius, z=0.0, segments=64, tile=2.0, inner=0.0):
    """A flat ring or disc facing up."""
    return lathe([(radius, z), (inner, z)], segments, tile)


def vault(x0, x1, radius, z0=0.0, segments=24, tile=2.0, squash=1.0):
    """Half a cylinder along X, its open side down at height z0, facing out."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    arc = [math.pi * i / segments for i in range(segments + 1)]
    rows = [[bm.verts.new((x, -radius * math.cos(a), z0 + squash * radius * math.sin(a))) for a in arc] for x in (x0, x1)]
    for i in range(segments):
        face = bm.faces.new([rows[0][i], rows[1][i], rows[1][i + 1], rows[0][i + 1]])
        for loop, (u, v) in zip(face.loops, ((x0, i), (x1, i), (x1, i + 1), (x0, i + 1))):
            loop[uv].uv = (u / tile, radius * math.pi * v / segments / tile)
    return bm


def half_disc(radius, x=0.0, z0=0.0, facing=1.0, segments=24, tile=2.0, squash=1.0):
    """An upright half disc in the plane X = x, round side up, facing ±X."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    arc = [math.pi * i / segments for i in range(segments + 1)]
    rim = [bm.verts.new((x, -radius * math.cos(a), z0 + squash * radius * math.sin(a))) for a in arc]
    face = bm.faces.new(rim if facing < 0 else rim[::-1])
    for loop in face.loops:
        loop[uv].uv = (loop.vert.co.y / tile, loop.vert.co.z / tile)
    return bm


def prism(points, z0, z1, tile=2.0):
    """A vertical prism over a counter-clockwise footprint."""
    bm = bmesh.new()
    lo = [bm.verts.new((x, y, z0)) for x, y in points]
    hi = [bm.verts.new((x, y, z1)) for x, y in points]
    for i in range(len(points)):
        j = (i + 1) % len(points)
        bm.faces.new([lo[i], lo[j], hi[j], hi[i]])
    bm.faces.new(hi)
    bm.faces.new(lo[::-1])
    project_uv(bm, tile)
    return bm


def frames(path):
    """Parallel-transport frames along a polyline: (point, tangent, normal, binormal)."""
    out = []
    tangent = (path[1] - path[0]).normalized()
    normal = tangent.orthogonal().normalized()
    if abs(tangent.z) < 0.9:
        normal = tangent.cross(Vector((0, 0, 1))).cross(tangent).normalized()
    for i, p in enumerate(path):
        if i + 1 < len(path):
            ahead = (path[i + 1] - p).normalized()
            behind = (p - path[i - 1]).normalized() if i else ahead
            t = (ahead + behind).normalized()
        else:
            t = (p - path[i - 1]).normalized()
        normal = (normal - t * normal.dot(t)).normalized()
        out.append((p, t, normal, t.cross(normal)))
    return out


def sweep(profile, path, tile=1.0, closed_profile=True, caps=True):
    """A 2D profile, (x along the normal, y along the binormal), swept along a path."""
    path = [Vector(p) for p in path]
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    rings = []
    for p, _, n, b in frames(path):
        rings.append([bm.verts.new(p + n * x + b * y) for x, y in profile])
    across = [0.0]
    for a, c in zip(profile, profile[1:] + (profile[:1] if closed_profile else [])):
        across.append(across[-1] + math.hypot(c[0] - a[0], c[1] - a[1]))
    run = [0.0]
    for a, c in zip(path, path[1:]):
        run.append(run[-1] + (c - a).length)
    m = len(profile)
    edges = m if closed_profile else m - 1
    for k in range(len(path) - 1):
        for i in range(edges):
            j = (i + 1) % m
            face = bm.faces.new([rings[k][i], rings[k][j], rings[k + 1][j], rings[k + 1][i]])
            for loop, (u, v) in zip(face.loops, ((i, k), (i + 1, k), (i + 1, k + 1), (i, k + 1))):
                loop[uv].uv = (across[u] / tile, run[v] / tile)
    if caps and closed_profile:
        for ring in (rings[0], rings[-1]):
            face = bm.faces.new(ring)
            for loop in face.loops:
                loop[uv].uv = (loop.vert.co.x / tile, loop.vert.co.y / tile)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return bm


def circle(radius, segments=12):
    return [(radius * math.cos(TAU * i / segments), radius * math.sin(TAU * i / segments)) for i in range(segments)]


def rect(w, h):
    return [(-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)]


def ibeam(w, h, t):
    """An I profile, w wide (flanges) and h deep."""
    return [(-h / 2, -w / 2), (-h / 2, w / 2), (-h / 2 + t, w / 2), (-h / 2 + t, t / 2), (h / 2 - t, t / 2), (h / 2 - t, w / 2),
            (h / 2, w / 2), (h / 2, -w / 2), (h / 2 - t, -w / 2), (h / 2 - t, -t / 2), (-h / 2 + t, -t / 2), (-h / 2 + t, -w / 2)]


def tube(path, radius, segments=12, tile=1.0, caps=True):
    return sweep(circle(radius, segments), path, tile, caps=caps)


def rod(a, b, radius, segments=8):
    return tube([Vector(a), Vector(b)], radius, segments)


def box(size, at=(0, 0, 0), rot=None, tile=2.0, bevel=0.0):
    """A box of size (x, y, z) centred at `at`, UVs projected per face in metres."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    if bevel > 0.0:
        bmesh.ops.bevel(bm, geom=bm.edges[:], offset=bevel, segments=2, affect="EDGES", profile=0.5)
    if rot:
        bmesh.ops.rotate(bm, matrix=Matrix(rot).to_4x4() if not isinstance(rot, Matrix) else rot, verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(at), verts=bm.verts)
    project_uv(bm, tile)
    return bm


def project_uv(bm, tile):
    """Box-projected UVs: each face takes the plane its normal faces most."""
    uv = bm.loops.layers.uv.get("st") or bm.loops.layers.uv.new("st")
    for face in bm.faces:
        n = face.normal
        axis = max(range(3), key=lambda i: abs(n[i]))
        a, b = [(1, 2), (0, 2), (0, 1)][axis]
        for loop in face.loops:
            co = loop.vert.co
            loop[uv].uv = (co[a] / tile, co[b] / tile)


def truss(a, b, width, depth, bays, chord, web):
    """A box truss from a to b: four chords and zig-zag webs on its faces."""
    a, b = Vector(a), Vector(b)
    axis = (b - a).normalized()
    side = axis.orthogonal().normalized()
    if abs(axis.z) < 0.9:
        side = axis.cross(Vector((0, 0, 1))).normalized()
    up = side.cross(axis).normalized()
    corners = [side * sx * width / 2 + up * sy * depth / 2 for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    bm = bmesh.new()
    for c in corners:
        merge(bm, rod(a + c, b + c, chord))
    for k in range(bays + 1):
        p = a + (b - a) * (k / bays)
        for i in range(4):
            merge(bm, rod(p + corners[i], p + corners[(i + 1) % 4], web))
        if k < bays:
            q = a + (b - a) * ((k + 1) / bays)
            for i in range(4):
                lo, hi = (p, q) if (k + i) % 2 == 0 else (q, p)
                merge(bm, rod(lo + corners[i], hi + corners[(i + 1) % 4], web))
    return bm


def merge(into, other, matrix=None):
    """Appends one bmesh's geometry to another."""
    mesh = bpy.data.meshes.new("tmp")
    other.to_mesh(mesh)
    if matrix is not None:
        mesh.transform(matrix)
    into.from_mesh(mesh)
    bpy.data.meshes.remove(mesh)
    other.free()
    return into


def moved(bm, at=(0, 0, 0), yaw=0.0, pitch=0.0, roll=0.0, scale=None):
    m = Matrix.Translation(Vector(at)) @ Matrix.Rotation(yaw, 4, "Z") @ Matrix.Rotation(pitch, 4, "Y") @ Matrix.Rotation(roll, 4, "X")
    if scale is not None:
        m = m @ Matrix.Diagonal((*scale, 1.0))
    bm.transform(m)
    return bm


# Objects


class Building:
    """A collection of parts; `finish` joins the parts that share a material,
    and joins the low-poly stand-ins given to `collider` into one hidden
    mesh that the physics world meets in place of the looks."""

    def __init__(self, name, parent=None, at=(0, 0, 0), yaw=0.0):
        self.name = name
        self.collection = bpy.data.collections.new(name)
        (parent.collection if parent else bpy.context.scene.collection).children.link(self.collection)
        self.root = bpy.data.objects.new(name, None)
        self.collection.objects.link(self.root)
        self.root.location = at
        self.root.rotation_euler = (0, 0, yaw)
        if parent:
            self.root.parent = parent.root
        self.parts = {}
        self.colliders = []
        self.places = []
        # The module file it exports to: its own unless it shares a kind.
        self.kind = "".join(f"_{c.lower()}" if c.isupper() and i else c.lower() for i, c in enumerate(name))

    def put(self, bm, mat, smooth=True, sharp=40.0):
        mesh = bpy.data.meshes.new(f"{self.name}_{mat.name}")
        bm.to_mesh(mesh)
        bm.free()
        if smooth:
            mesh.shade_smooth()
            mesh.set_sharp_from_angle(angle=math.radians(sharp))
        mesh.materials.append(mat)
        self.parts.setdefault(mat.name, []).append(mesh)

    def collider(self, bm):
        """A low-poly stand-in for the parts that should stop what moves."""
        self.colliders.append(bm)

    def finish(self):
        """One object per material, parented to the building's root."""
        for mat_name, meshes in self.parts.items():
            bm = bmesh.new()
            for mesh in meshes:
                bm.from_mesh(mesh)
            joined = meshes[0]
            bm.to_mesh(joined)
            bm.free()
            for mesh in meshes[1:]:
                bpy.data.meshes.remove(mesh)
            obj = bpy.data.objects.new(f"{self.name}_{mat_name}", joined)
            self.collection.objects.link(obj)
            obj.parent = self.root
        if self.colliders:
            bm = bmesh.new()
            for part in self.colliders:
                merge(bm, part)
            mesh = bpy.data.meshes.new(f"{self.name}_collider")
            bm.to_mesh(mesh)
            bm.free()
            obj = bpy.data.objects.new(f"{self.name}_collider", mesh)
            self.collection.objects.link(obj)
            obj.parent = self.root
            obj["gearbox_collide"] = True
            obj["gearbox_hidden"] = True
        self.parts, self.colliders = {}, []
        return self
