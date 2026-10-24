"""Renders one module of the Mars base on its own, a machine package, or a
spot of the whole base, to look at before a build. Runs in Blender, headless:

    flatpak run --filesystem=$PWD org.blender.Blender -b --factory-startup \\
        --python scripts/mars_base/preview.py -- OUT module habitat.hab_tower '{"name": "Hab"}'
    ... --python scripts/mars_base/preview.py -- OUT package machines/usd/perseverance_nasa.usdz
    ... --python scripts/mars_base/preview.py -- OUT base X Y RADIUS [HEIGHT]

A module is built at the origin by `<file>.<builder>(looks, **kwargs)`, a
package imported as it stands; the base is built whole and looked at round
the spot X, Y (base metres). Views go to OUT as PNGs with a `stats.json` of
the geometry; textures the looks make go to OUT/textures, never into assets/.
"""
import json
import math
import os
import sys

import bpy
from mathutils import Vector

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import kit  # noqa: E402

TEXTURES = os.path.join(HERE, "..", "..", "assets", "mars_base", "textures")


def clear():
    for data in (bpy.data.objects, bpy.data.meshes, bpy.data.materials, bpy.data.collections, bpy.data.lights, bpy.data.cameras):
        for block in list(data):
            data.remove(block)


def visible_meshes():
    return [o for o in bpy.data.objects if o.type == "MESH" and not o.get("gearbox_hidden") and not o.name.startswith("preview_")]


def bounds(objs):
    pts = [o.matrix_world @ Vector(c) for o in objs for c in o.bound_box]
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    return lo, hi


def stage(size):
    """Ground, sun, sky and a camera, as the sim lights Mars at mid-morning;
    the hidden physics stand-ins stay out of the picture."""
    scene = bpy.context.scene
    for o in bpy.data.objects:
        if o.get("gearbox_hidden"):
            o.hide_render = True
    mesh = bpy.data.meshes.new("preview_ground")
    r = size
    mesh.from_pydata([(-r, -r, 0), (r, -r, 0), (r, r, 0), (-r, r, 0)], [], [(0, 1, 2, 3)])
    ground = bpy.data.objects.new("preview_ground", mesh)
    mat = bpy.data.materials.new("preview_ground")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (0.20, 0.11, 0.06, 1)
    bsdf.inputs["Roughness"].default_value = 0.95
    mesh.materials.append(mat)
    scene.collection.objects.link(ground)
    sun = bpy.data.objects.new("preview_sun", bpy.data.lights.new("preview_sun", "SUN"))
    sun.data.energy = 3.2
    sun.data.color = (1.0, 0.94, 0.86)
    sun.data.angle = math.radians(0.5)
    sun.rotation_euler = (math.radians(52), 0, math.radians(-40))
    scene.collection.objects.link(sun)
    world = bpy.data.worlds.get("World") or bpy.data.worlds.new("World")
    scene.world = world
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.52, 0.40, 0.31, 1)
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.8
    cam = bpy.data.objects.new("preview_cam", bpy.data.cameras.new("preview_cam"))
    cam.data.clip_end = 5000
    scene.collection.objects.link(cam)
    scene.camera = cam
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x, scene.render.resolution_y = 1280, 800
    scene.view_settings.view_transform = "AgX"
    return cam


def shoot(cam, out, name, eye, target, lens=32):
    cam.location = eye
    cam.data.lens = lens
    cam.rotation_euler = (Vector(target) - Vector(eye)).to_track_quat("-Z", "Y").to_euler()
    bpy.context.scene.render.filepath = os.path.join(out, f"{name}.png")
    bpy.ops.render.render(write_still=True)


def stats(objs, lo, hi):
    return {
        "objects": len(objs),
        "verts": sum(len(o.data.vertices) for o in objs),
        "faces": sum(len(o.data.polygons) for o in objs),
        "materials": sorted({m.name for o in objs for m in o.data.materials if m}),
        "min": [round(v, 3) for v in lo],
        "max": [round(v, 3) for v in hi],
        "colliders": [o.name for o in bpy.data.objects if o.get("gearbox_collide")],
    }


def module(out, target, kwargs):
    file, builder = target.split(".")
    looks = kit.Looks(TEXTURES, os.path.join(out, "textures"))
    fn = getattr(__import__(file), builder)
    fn(looks, **kwargs)
    return views(out, visible_meshes())


def package(out, path):
    bpy.ops.wm.usd_import(filepath=os.path.abspath(path))
    return views(out, visible_meshes())


def views(out, objs):
    """Four views round `objs`, one close and low on their side, one from above."""
    lo, hi = bounds(objs)
    centre, radius = (lo + hi) / 2, max((hi - lo).length / 2, 1.0)
    cam = stage(radius * 20)
    for k, az in enumerate((35, 125, 215, 305)):
        a = math.radians(az)
        eye = centre + Vector((math.cos(a), math.sin(a), 0.42)) * radius * 2.4
        shoot(cam, out, f"view{k}", eye, centre)
    a = math.radians(60)
    shoot(cam, out, "close", centre + Vector((math.cos(a), math.sin(a), 0.15)) * radius * 1.25 + Vector((0, 0, 1.0 - centre.z)), (centre.x, centre.y, 1.4), 24)
    shoot(cam, out, "top", centre + Vector((0.01, 0.0, radius * 3.2)), centre, 32)
    return stats(objs, lo, hi)


def base(out, x, y, radius, height):
    import layout
    looks = kit.Looks(TEXTURES, os.path.join(out, "textures"))
    layout.build(looks)
    objs = visible_meshes()
    cam = stage(2000)
    centre = Vector((x, y, 1.5))
    for k, az in enumerate((20, 110, 200, 290)):
        a = math.radians(az)
        shoot(cam, out, f"spot{k}", centre + Vector((math.cos(a) * radius, math.sin(a) * radius, height)), centre, 28)
    lo, hi = bounds(objs)
    return stats(objs, lo, hi)


def main():
    args = sys.argv[sys.argv.index("--") + 1:]
    out, mode = os.path.abspath(args[0]), args[1]
    os.makedirs(out, exist_ok=True)
    clear()
    if mode == "module":
        report = module(out, args[2], json.loads(args[3]) if len(args) > 3 else {})
    elif mode == "package":
        report = package(out, args[2])
    else:
        x, y, radius = (float(v) for v in args[2:5])
        report = base(out, x, y, radius, float(args[5]) if len(args) > 5 else 4.0)
    with open(os.path.join(out, "stats.json"), "w") as f:
        json.dump(report, f, indent=1)
    print("PREVIEW", json.dumps(report))


main()
