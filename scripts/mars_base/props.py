"""Everything else a working base has lying about it: cargo, comms masts,
dishes, a weather station, marker poles, tyre tracks, and NASA's own models."""
import math
import os
import random

import bmesh
import bpy
from mathutils import Matrix, Vector

from buildings import mats
from kit import (TAU, Building, box, cone, lathe, merge, moved, rod, sphere, truss)


def container(b, m, at, yaw, skin="container_orange", high=False):
    """A 20 ft ISO container with corner castings and door bars."""
    h = 2.9 if high else 2.59
    b.put(moved(box((6.06, 2.44, h), (0, 0, h / 2), tile=2.4), at, yaw), m[skin], smooth=False)
    cast = bmesh.new()
    for x in (-3.0, 3.0):
        for y in (-1.19, 1.19):
            for z in (0.08, h - 0.08):
                merge(cast, box((0.18, 0.18, 0.18), (x, y, z)))
    for y in (-0.8, -0.3, 0.3, 0.8):
        merge(cast, rod((3.05, y, 0.2), (3.05, y, h - 0.2), 0.025, 6))
    b.put(moved(cast, at, yaw), m["dark"], smooth=False)
    b.collider(moved(box((6.06, 2.44, h), (0, 0, h / 2)), at, yaw))


def crates(b, m, at, yaw, seed=0, count=6):
    """A stack of cargo cases, pallet-wide, some on top of others."""
    rng = random.Random(seed)
    for k in range(count):
        w, d, h = rng.choice(((1.2, 1.0, 0.9), (1.2, 0.8, 0.6), (0.8, 0.6, 0.6), (1.6, 1.2, 1.1)))
        x, y = (k % 3) * 1.4, (k // 3 % 2) * 1.3
        z = 0.0 if k < 5 else 1.0
        skin = rng.choice(("hull", "hull_grey", "hull_orange", "container_white"))
        turn = yaw + rng.uniform(-0.1, 0.1)
        b.put(moved(box((w, d, h), (x, y, z + h / 2), tile=1.0, bevel=0.03), at, turn), m[skin], smooth=False)
        b.collider(moved(box((w, d, h), (x, y, z + h / 2)), at, turn))


def barrels(b, m, at, seed=0, count=8):
    rng = random.Random(seed)
    for k in range(count):
        p = Vector(at) + Vector(((k % 4) * 0.7, (k // 4) * 0.7, 0))
        skin = rng.choice(("hull", "hull_orange", "pipe_blue", "hull_grey"))
        drum = lathe([(0.0, 0.0), (0.29, 0.0), (0.3, 0.02), (0.3, 0.3), (0.31, 0.31), (0.3, 0.32), (0.3, 0.58), (0.31, 0.59), (0.3, 0.6), (0.3, 0.86), (0.28, 0.88), (0.0, 0.88)], 20, 1.0)
        b.put(moved(drum, tuple(p)), m[skin])


def cable_reel(b, m, at, yaw):
    reel = bmesh.new()
    for s in (-0.55, 0.55):
        merge(reel, moved(lathe([(0.0, -0.04), (1.0, -0.04), (1.0, 0.04), (0.0, 0.04)], 24, 1.0), (0, 0, s)))
    b.put(moved(reel, tuple(Vector(at) + Vector((0, 0, 1.0))), yaw, math.pi / 2), m["hull_orange"])
    b.put(moved(lathe([(0.75, -0.5), (0.75, 0.5)], 24, 0.5), tuple(Vector(at) + Vector((0, 0, 1.0))), yaw, math.pi / 2), m["cable"])


def lattice_mast(looks, name, at=(0, 0, 0), height=32.0, parent=None):
    """A guyed lattice comms mast with panel antennas and beacons."""
    m = mats(looks)
    b = Building(name, parent, at, 0.0)
    b.put(truss((0, 0, 0.3), (0, 0, height), 1.1, 1.1, int(height / 1.5), 0.06, 0.03), m["hull_orange"])
    b.put(box((2.0, 2.0, 0.4), (0, 0, 0.2)), m["concrete"], smooth=False)
    for k in range(3):
        a = TAU * k / 3
        c = Vector((math.cos(a), math.sin(a), 0))
        anchor = c * height * 0.55
        b.put(rod(Vector((0, 0, height * 0.9)), anchor, 0.015, 4), m["steel"])
        b.put(rod(Vector((0, 0, height * 0.5)), anchor, 0.015, 4), m["steel"])
        b.put(moved(box((0.8, 0.8, 0.5), (0, 0, 0.25)), tuple(anchor)), m["concrete"], smooth=False)
        b.put(moved(box((0.25, 0.5, 1.6), (0, 0, height - 3.0), bevel=0.05), tuple(c * 0.8)), m["white_frame"], smooth=False)
    for z in (height * 0.5, height + 0.3):
        b.put(sphere(0.2, (0, 0, z), 10, 5), m["red"])
    b.put(rod((0, 0, height), (0, 0, height + 4.0), 0.04, 6), m["steel"])
    b.collider(box((1.2, 1.2, height), (0, 0, height / 2)))
    return b.finish()


def dish(looks, name, diameter=8.0, at=(0, 0, 0), yaw=0.0, elevation=45.0, parent=None):
    """A steerable parabolic antenna on a pedestal, with a quadripod feed."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    r = diameter / 2
    f = diameter * 0.4
    hub = Vector((0, 0, r + 2.0))
    b.put(lathe([(1.2, 0.0), (1.0, r + 1.0), (0.0, r + 1.2)], 24, 1.0), m["white_frame"], sharp=40)
    b.put(moved(box((1.8, 1.4, 1.2), (0, 0, 0)), tuple(hub)), m["hull"], smooth=False)
    tilt = math.radians(90.0 - elevation)
    rot = Matrix.Translation(hub + Vector((0.6, 0, 0))) @ Matrix.Rotation(math.pi / 2 - tilt, 4, "Y")
    bowl = lathe([(0.0, 0.0)] + [(r * t, (r * t) ** 2 / (4 * f)) for t in [i / 12 for i in range(1, 13)]], 48, 2.0)
    back = lathe([(r * t, (r * t) ** 2 / (4 * f) - 0.08) for t in [i / 12 for i in range(12, 0, -1)]] + [(0.0, -0.08)], 48, 2.0)
    bowl.transform(rot)
    back.transform(rot)
    b.put(bowl, m["white_frame"])
    b.put(back, m["steel"])
    feed = rot @ Vector((0, 0, f))
    for k in range(4):
        a = TAU * k / 4 + TAU / 8
        rim = rot @ Vector((r * 0.92 * math.cos(a), r * 0.92 * math.sin(a), (r * 0.92) ** 2 / (4 * f)))
        b.put(rod(rim, feed, 0.04, 6), m["white_frame"])
    b.put(moved(cone(0.25, 0.12, 0.6, segments=12), tuple(feed)), m["hull_grey"])
    b.collider(lathe([(0.0, 0.0), (1.2, 0.0), (1.0, r + 1.2), (0.0, r + 1.2)], 8, 2.0))
    return b.finish()


def weather_station(looks, name, at=(0, 0, 0), parent=None):
    m = mats(looks)
    b = Building(name, parent, at, 0.0)
    b.put(rod((0, 0, 0), (0, 0, 6.0), 0.05, 8), m["steel"])
    for z in (1.5, 3.0, 4.5):
        b.put(rod((0, -0.6, z), (0, 0.6, z), 0.02, 6), m["steel"])
        b.put(moved(box((0.15, 0.1, 0.25), (0, 0, 0)), (0, 0.6, z)), m["white_frame"], smooth=False)
    for k in range(3):
        a = TAU * k / 3
        b.put(rod((0, 0, 6.0), (0.35 * math.cos(a), 0.35 * math.sin(a), 6.0), 0.01, 4), m["steel"])
        b.put(sphere(0.07, (0.35 * math.cos(a), 0.35 * math.sin(a), 6.0), 8, 4), m["white_frame"])
    b.put(box((0.6, 0.4, 0.5), (0, 0.3, 0.8), bevel=0.02), m["hull"], smooth=False)
    b.put(moved(box((0.04, 0.6, 0.4), (0, 0, 0)), (0, -0.4, 2.2), 0.0, -0.5), m["solar"], smooth=False)
    return b.finish()


def marker_poles(b, m, path, every=12.0):
    """Orange-and-white route poles along a path."""
    path = [Vector(p) for p in path]
    for a, c in zip(path, path[1:]):
        run = c - a
        side = Vector((-run.y, run.x, 0)).normalized() * 4.2
        for k in range(int(run.length / every) + 1):
            p = a + run * (k * every / run.length)
            for s in (-1, 1):
                base = p + side * s
                b.put(rod(base, base + Vector((0, 0, 0.8)), 0.04, 6), m["hazard"])
                b.put(rod(base + Vector((0, 0, 0.8)), base + Vector((0, 0, 1.6)), 0.04, 6), m["paint"])


def tracks(looks, name, path, width=7.0, parent=None):
    """Compacted regolith with tyre ruts, a ribbon laid along a path."""
    m = mats(looks)
    b = Building(name, parent, (0, 0, 0), 0.0)
    path = [Vector(p) for p in path]
    pts = []
    for a, c in zip(path, path[1:]):
        n = max(1, int((c - a).length / 6.0))
        pts += [a.lerp(c, k / n) for k in range(n)]
    pts.append(path[-1])
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("st")
    rows, run = [], 0.0
    for i, p in enumerate(pts):
        ahead = (pts[min(i + 1, len(pts) - 1)] - pts[max(i - 1, 0)]).normalized()
        side = Vector((-ahead.y, ahead.x, 0))
        if i:
            run += (p - pts[i - 1]).length
        rows.append(([bm.verts.new(p + side * (width / 2) * s + Vector((0, 0, 0.03))) for s in (-1, 1)], run))
    for (a, ra), (c, rc) in zip(rows, rows[1:]):
        face = bm.faces.new([a[0], a[1], c[1], c[0]])
        for loop, st in zip(face.loops, ((0.0, ra / width), (1.0, ra / width), (1.0, rc / width), (0.0, rc / width))):
            loop[uv].uv = st
    b.put(bm, m["tracks"], smooth=False)
    return b.finish()


def nasa_model(path, name, parent, at, yaw=0.0, height=None, textures=None, solid=None):
    """Imports one of NASA's public-domain glTF models as a building, its
    parts joined per material, standing on the ground at `at`; `height`
    rescales a model authored at the wrong size. Its packed images are
    written to `textures/<name>/`; `solid`, (radius, height), stands a
    cylinder for it in the physics world."""
    b = Building(name, parent, at, yaw)
    collection, root = b.collection, b.root
    before = set(bpy.data.objects)
    images = set(bpy.data.images)
    bpy.ops.import_scene.gltf(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in new if o.type == "MESH"]
    for o in new:
        for c in list(o.users_collection):
            c.objects.unlink(o)
        collection.objects.link(o)
    if textures:
        folder = os.path.join(textures, b.kind)
        os.makedirs(folder, exist_ok=True)
        for image in set(bpy.data.images) - images:
            if image.packed_file or not image.filepath:
                # Saved as the bytes they were packed in, under the matching extension.
                ext = {"WEBP": "webp", "JPEG": "jpg"}.get(image.file_format, "png")
                image.filepath_raw = os.path.join(folder, f"{bpy.path.clean_name(image.name)}.{ext}")
                image.save()
                if image.packed_file:
                    image.unpack(method="REMOVE")
    depsgraph = bpy.context.evaluated_depsgraph_get()
    by_mat = {}
    lo, hi = 1e9, -1e9
    for o in meshes:
        mesh = bpy.data.meshes.new_from_object(o.evaluated_get(depsgraph))
        mesh.transform(o.matrix_world)
        for v in mesh.vertices:
            lo, hi = min(lo, v.co.z), max(hi, v.co.z)
        for i, slot in enumerate(mesh.materials):
            by_mat.setdefault(slot.name if slot else "none", []).append((mesh, i))
        if not mesh.materials:
            by_mat.setdefault("none", []).append((mesh, 0))
    scale = height / max(hi - lo, 1e-6) if height else 1.0
    root.scale = (scale, scale, scale)
    for mat_name, parts in by_mat.items():
        bm = bmesh.new()
        for mesh, index in parts:
            sub = bmesh.new()
            sub.from_mesh(mesh)
            bmesh.ops.delete(sub, geom=[f for f in sub.faces if f.material_index != index], context="FACES")
            for f in sub.faces:
                f.material_index = 0
            tmp = bpy.data.meshes.new("tmp")
            sub.to_mesh(tmp)
            sub.free()
            bm.from_mesh(tmp)
            bpy.data.meshes.remove(tmp)
        bmesh.ops.translate(bm, vec=(0, 0, -lo), verts=bm.verts)
        out = bpy.data.meshes.new(f"{name}_{mat_name}")
        bm.to_mesh(out)
        bm.free()
        if mat_name in bpy.data.materials:
            out.materials.append(bpy.data.materials[mat_name])
        obj = bpy.data.objects.new(f"{name}_{mat_name}", out)
        collection.objects.link(obj)
        obj.parent = root
    for mesh in {p[0] for parts in by_mat.values() for p in parts}:
        bpy.data.meshes.remove(mesh)
    for o in new:
        bpy.data.objects.remove(o)
    if solid:
        radius, tall = solid
        b.collider(lathe([(0.0, 0.0), (radius / scale, 0.0), (radius / scale, tall / scale), (0.0, tall / scale)], 12, 2.0))
    return b.finish()
