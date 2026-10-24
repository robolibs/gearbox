"""Rocks strewn over the base's ground: Poly Haven's CC0 scans of arid
boulders and stones, fetched once into .work, cut down, and tinted to Mars
basalt under dust. Boulders are modules of their own with a convex hull to
drive into; stones scatter by the thousand through one point instancer.
"""
import json
import math
import os
import random
import urllib.request

import bmesh
import bpy
import numpy as np
from mathutils import Vector

from kit import Building, linear_to_srgb, srgb_to_linear
from weathering import kept

HERE = os.path.dirname(os.path.abspath(__file__))
STORE = os.path.join(HERE, "..", "..", ".work", "mars_base", "polyhaven")

# Scan id: (kind of rock, faces kept, collides).
SCANS = {
    "namaqualand_boulder_02": ("boulder", 5000, True),
    "namaqualand_boulder_03": ("boulder", 5000, True),
    "namaqualand_boulder_04": ("boulder", 5000, True),
    "namaqualand_boulder_05": ("boulder", 3000, True),
    "namaqualand_boulder_06": ("boulder", 3000, True),
    "namaqualand_boulders_01": ("stone", 1500, False),
    "rock_07": ("stone", 800, False),
    "stone_01": ("stone", 800, False),
    "moon_rock_02": ("stone", 800, False),
    "moon_rock_05": ("stone", 800, False),
}

# Basalt under a film of dust, as picked in sRGB.
BASALT = (0.30, 0.235, 0.20)


def download(url):
    """A URL's bytes, asked for under a name of our own as Poly Haven's API wants."""
    request = urllib.request.Request(url, headers={"User-Agent": "gearbox-mars-base/1.0"})
    with urllib.request.urlopen(request) as response:
        return response.read()


def fetch(scan, res="1k"):
    """The scan's glTF in the store, downloaded with its files if missing."""
    folder = os.path.join(STORE, scan)
    done = os.path.join(folder, f"{scan}_{res}.gltf")
    if os.path.exists(done):
        return done
    listing = json.loads(download(f"https://api.polyhaven.com/files/{scan}"))["gltf"][res]["gltf"]
    os.makedirs(os.path.join(folder, "textures"), exist_ok=True)
    for name, item in [*listing["include"].items(), (os.path.basename(listing["url"]), listing)]:
        with open(os.path.join(folder, name), "wb") as f:
            f.write(download(item["url"]))
    return done


def tinted(image, out, colour=BASALT, contrast=1.2):
    """The scan's colour photo turned to `colour`, its own light and shade kept."""
    w, h = image.size
    px = np.empty(w * h * 4, dtype=np.float32)
    image.pixels.foreach_get(px)
    rgb = srgb_to_linear(px.reshape(h, w, 4)[..., :3])
    lum = rgb @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    detail = np.power(lum / max(float(lum.mean()), 1e-4), contrast)
    col = np.clip(np.array(srgb_to_linear(np.array(colour, dtype=np.float32))) * detail[..., None], 0.0, 1.0)
    rgba = np.concatenate([linear_to_srgb(col), np.ones((h, w, 1), dtype=np.float32)], axis=-1)
    dst = bpy.data.images.new(os.path.basename(out), w, h, alpha=False)
    dst.pixels.foreach_set(rgba.astype(np.float32).ravel())
    dst.filepath_raw = out
    dst.file_format = "JPEG"
    dst.save()
    return dst


def look(scan, images, out_dir):
    """Basalt over the scan's own normals and roughness, its maps kept in `out_dir`."""
    mat = bpy.data.materials.new(f"rock_{scan}")
    mat.use_nodes = True
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    bsdf = nodes["Principled BSDF"]
    named = lambda part: next((i for i in images if part in i.name or part in os.path.basename(i.filepath)), None)
    node = nodes.new("ShaderNodeTexImage")
    node.image = tinted(named("_diff"), os.path.join(out_dir, f"rock_{scan}_colour.jpg"))
    links.new(node.outputs["Color"], bsdf.inputs["Base Color"])
    arm = named("_arm")
    if arm is not None:
        arm.colorspace_settings.name = "Non-Color"
        node = nodes.new("ShaderNodeTexImage")
        node.image = kept(arm, os.path.join(out_dir, f"rock_{scan}_arm.jpg"), 512)
        split = nodes.new("ShaderNodeSeparateColor")
        links.new(node.outputs["Color"], split.inputs["Color"])
        links.new(split.outputs["Green"], bsdf.inputs["Roughness"])
    nor = named("_nor_gl")
    if nor is not None:
        nor.colorspace_settings.name = "Non-Color"
        node = nodes.new("ShaderNodeTexImage")
        node.image = kept(nor, os.path.join(out_dir, f"rock_{scan}_normal.jpg"), 512)
        normal = nodes.new("ShaderNodeNormalMap")
        links.new(node.outputs["Color"], normal.inputs["Color"])
        links.new(normal.outputs["Normal"], bsdf.inputs["Normal"])
    bsdf.inputs["Metallic"].default_value = 0.0
    return mat


def clear(x, y, keep_out, lines):
    """Whether a point lies outside every box and further than its width from every line."""
    for x0, y0, x1, y1 in keep_out:
        if x0 <= x <= x1 and y0 <= y <= y1:
            return False
    for path, half in lines:
        for a, b in zip(path, path[1:]):
            ax, ay, bx, by = a[0], a[1], b[0], b[1]
            dx, dy = bx - ax, by - ay
            t = max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / max(dx * dx + dy * dy, 1e-9)))
            if math.hypot(x - (ax + t * dx), y - (ay + t * dy)) < half:
                return False
    return True


def scatter(scans, keep_out, lines, radius, seed=7, boulders=150, stones=6000):
    """Where rocks lie round the base: (scan, at, (tilt x, tilt y, yaw) in
    degrees, scale) for boulders and for stones, outside `keep_out` boxes
    (x0, y0, x1, y1) and clear of `lines` (path, half width), thinning
    toward the base's middle."""
    rng = random.Random(seed)
    out = {"boulder": [], "stone": []}
    for kind, count, spacing in (("boulder", boulders, 7.0), ("stone", stones, 1.2)):
        names = [s for s, (k, _, _) in SCANS.items() if k == kind and s in scans]
        placed = []
        tries = 0
        while len(placed) < count and tries < count * 40:
            tries += 1
            r = radius * math.sqrt(rng.random())
            if rng.random() > (r / radius) ** 0.6:
                continue
            a = rng.random() * math.tau
            x, y = r * math.cos(a), r * math.sin(a)
            if not clear(x, y, keep_out, lines) or any((x - px) ** 2 + (y - py) ** 2 < spacing ** 2 for px, py in placed[-400:]):
                continue
            placed.append((x, y))
            scan = rng.choice(names)
            size = scans[scan]
            scale = rng.uniform(0.35, 1.3) if kind == "boulder" else rng.uniform(0.6, 2.4)
            sink = -0.15 * size.z * scale
            out[kind].append((scan, (round(x, 3), round(y, 3), round(sink, 3)), (rng.uniform(-8, 8), rng.uniform(-8, 8), rng.uniform(0, 360)), round(scale, 3)))
    return out


def rock(scan, parent, out_dir):
    """One scan as a module about its own footprint: base at Z = 0, centred."""
    kind, faces, collides = SCANS[scan]
    before, images = set(bpy.data.objects), set(bpy.data.images)
    bpy.ops.import_scene.gltf(filepath=fetch(scan))
    new = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in new if o.type == "MESH"]
    bm = bmesh.new()
    for o in meshes:
        part = bmesh.new()
        part.from_mesh(o.data)
        part.transform(o.matrix_world)
        part.to_mesh(o.data)
        bm.from_mesh(o.data)
        part.free()
    lo = Vector((min(v.co.x for v in bm.verts), min(v.co.y for v in bm.verts), min(v.co.z for v in bm.verts)))
    hi = Vector((max(v.co.x for v in bm.verts), max(v.co.y for v in bm.verts), max(v.co.z for v in bm.verts)))
    bmesh.ops.translate(bm, vec=Vector((-(lo.x + hi.x) / 2, -(lo.y + hi.y) / 2, -lo.z)), verts=bm.verts)
    mesh = bpy.data.meshes.new(f"rock_{scan}")
    bm.to_mesh(mesh)
    bm.free()
    work = bpy.data.objects.new(f"rock_{scan}_work", mesh)
    bpy.context.scene.collection.objects.link(work)
    cut = work.modifiers.new("cut", "DECIMATE")
    cut.ratio = min(1.0, faces / max(len(mesh.polygons), 1))
    depsgraph = bpy.context.evaluated_depsgraph_get()
    low = bmesh.new()
    low.from_object(work, depsgraph)
    mat = look(scan, [i for i in bpy.data.images if i not in images], out_dir)
    b = Building(f"Rock_{scan}", parent)
    b.kind = f"rock_{scan}"
    b.natural = True
    if collides:
        hull = bmesh.new()
        step = max(1, len(low.verts) // 90)
        for v in list(low.verts)[::step]:
            hull.verts.new(v.co)
        bmesh.ops.convex_hull(hull, input=hull.verts)
        b.collider(hull)
    b.put(low, mat, sharp=180)
    bpy.data.objects.remove(work)
    bpy.data.meshes.remove(mesh)
    for o in new:
        if o.type == "MESH":
            data = o.data
            bpy.data.objects.remove(o)
            bpy.data.meshes.remove(data)
        else:
            bpy.data.objects.remove(o)
    b.finish()
    for o in b.collection.objects:
        if o.type == "MESH" and o.data.uv_layers:
            o.data.uv_layers[0].name = "st"
    return b, kind, (hi - lo)
