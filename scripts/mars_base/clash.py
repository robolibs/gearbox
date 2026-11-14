"""Finds where the Mars base's buildings cut into each other. Runs in
Blender, headless:

    scripts/mars_base/clash.sh OUT.json

The base is built whole; for every two buildings whose visible meshes meet,
OUT.json lists [a, b, faces of a that meet b, low corner, high corner] of
those faces, most first, and `lows` the buildings reaching under -0.6 m.
Corridors seated in their collars, roads on the plaza and parts standing on
a pad meet by design; anything else is a misfit.
"""
import json
import os
import sys

import bmesh
import bpy
from mathutils.bvhtree import BVHTree

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import kit  # noqa: E402
import layout  # noqa: E402

TEXTURES = os.path.join(HERE, "..", "..", "assets", "mars_base", "textures")


def merged(building, deps):
    """The building's visible meshes in the base frame, as one bmesh."""
    bm = bmesh.new()
    for o in building.collection.objects:
        if o.type != "MESH" or o.get("gearbox_hidden"):
            continue
        part = bmesh.new()
        part.from_object(o, deps)
        part.transform(o.matrix_world)
        mesh = bpy.data.meshes.new("clash_part")
        part.to_mesh(mesh)
        bm.from_mesh(mesh)
        bpy.data.meshes.remove(mesh)
        part.free()
    bm.faces.ensure_lookup_table()
    return bm


def corners(points):
    return [round(min(p[i] for p in points), 1) for i in range(3)], [round(max(p[i] for p in points), 1) for i in range(3)]


def main():
    out = os.path.abspath(sys.argv[sys.argv.index("--") + 1])
    for data in (bpy.data.objects, bpy.data.meshes, bpy.data.materials, bpy.data.collections):
        for block in list(data):
            data.remove(block)
    _, built = layout.build(kit.Looks(TEXTURES, os.path.join(os.path.dirname(out), "textures")))
    deps = bpy.context.evaluated_depsgraph_get()
    solids = []
    for b in built:
        bm = merged(b, deps)
        if bm.verts:
            solids.append((b.name, bm, BVHTree.FromBMesh(bm), corners([v.co for v in bm.verts])))
    report = {"clashes": [], "lows": []}
    for k, (na, ba, ta, (la, ha)) in enumerate(solids):
        if la[2] < -0.6:
            report["lows"].append([na, la[2]])
        for nb, _, tb, (lb, hb) in solids[k + 1:]:
            if any(ha[i] < lb[i] or hb[i] < la[i] for i in range(3)):
                continue
            pairs = ta.overlap(tb)
            if pairs:
                lo, hi = corners([ba.faces[i].calc_center_median() for i, _ in pairs[:2000]])
                report["clashes"].append([na, nb, len(pairs), lo, hi])
    report["clashes"].sort(key=lambda c: -c[2])
    with open(out, "w") as f:
        json.dump(report, f, indent=1)
    print("CLASH", len(solids), "buildings", len(report["clashes"]), "meeting pairs")


main()
