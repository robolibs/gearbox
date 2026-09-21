"""STL to painted USD meshes — shared by the `build_*_geom.py` scripts.

An STL is a soup of independent triangles with a normal each and no idea
which of them share a corner. Two things here turn that into something that
renders well:

* `weld` joins vertices by position *and* by which way the surface faces,
  with a crease angle. Panels keep their hard edges; a wheel rim comes out
  round.
* `emit` gives each mesh a UsdPreviewSurface rather than a flat colour, so it
  picks up the scene's lighting instead of reading as a paper cut-out.

`shells` splits a mesh into its connected pieces so a body that is one STL
can still be painted part by part.
"""

from __future__ import annotations

import math
import pathlib
import struct
import sys
from collections import defaultdict

from pxr import Gf, Sdf, Usd, UsdGeom, UsdShade, Vt

# Above this angle between two facets, the edge between them stays sharp.
# 40 degrees keeps panel creases while letting wheels and fillets round off.
CREASE_DEG = 40.0

# name -> (linear rgb, roughness, metallic). The AgileX palette: the URDFs
# author 0.15/0.15/0.17 for bodies and 0.96/0.45/0.08 orange for accents,
# converted here out of sRGB. Body is lifted off pure black so it reads as
# dark grey under scene light rather than as a silhouette.
SURFACES = {
    "body": ((0.030, 0.031, 0.036), 0.45, 0.15),
    "accent": ((0.912, 0.171, 0.010), 0.42, 0.00),
    "alloy": ((0.520, 0.530, 0.550), 0.30, 0.90),
    "tyre": ((0.012, 0.012, 0.013), 0.92, 0.00),
    "hub": ((0.280, 0.290, 0.300), 0.35, 0.75),
    # Clearpath's Husky yellow, 0.8/0.8/0 sRGB in its meshes.
    "husky_yellow": ((0.604, 0.604, 0.000), 0.50, 0.00),
    "husky_tyre": ((0.030, 0.030, 0.032), 0.90, 0.00),
}


def read_stl(path: pathlib.Path):
    """Triangles from a binary STL as (positions, facet normal) per face."""
    raw = path.read_bytes()
    count = struct.unpack_from("<I", raw, 80)[0]
    faces = []
    for t in range(count):
        base = 84 + t * 50
        nx, ny, nz = struct.unpack_from("<3f", raw, base)
        tri = [struct.unpack_from("<3f", raw, base + 12 + v * 12) for v in range(3)]
        faces.append((tri, (nx, ny, nz)))
    return faces


def turn(faces, rotate, offset=(0.0, 0.0, 0.0)):
    """Rotate every face and shift it.

    `rotate` maps one point; normals go through it too, without the offset,
    which is right as long as it is a proper axis-aligned rotation.
    """
    out = []
    for tri, n in faces:
        moved = []
        for p in tri:
            r = rotate(p)
            moved.append((r[0] + offset[0], r[1] + offset[1], r[2] + offset[2]))
        out.append((moved, rotate(n)))
    return out


def bounds(faces):
    lo = [1e9] * 3
    hi = [-1e9] * 3
    for tri, _n in faces:
        for p in tri:
            for a in range(3):
                lo[a] = min(lo[a], p[a])
                hi[a] = max(hi[a], p[a])
    return lo, hi


def centre(faces):
    lo, hi = bounds(faces)
    return [(lo[a] + hi[a]) / 2.0 for a in range(3)]


def key_of(p):
    return tuple(round(c * 1e4) for c in p)


def shells(faces):
    """Group face indices into connected shells, by shared corner position."""
    ids: dict[tuple, int] = {}
    face_corners = []
    for tri, _n in faces:
        corners = []
        for p in tri:
            k = key_of(p)
            if k not in ids:
                ids[k] = len(ids)
            corners.append(ids[k])
        face_corners.append(corners)

    parent = list(range(len(ids)))

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    for corners in face_corners:
        ra, rb, rc = find(corners[0]), find(corners[1]), find(corners[2])
        parent[ra] = rb
        parent[find(rb)] = rc

    groups = defaultdict(list)
    for i, corners in enumerate(face_corners):
        groups[find(corners[0])].append(i)
    return list(groups.values())


def weld(faces, members, crease_deg=CREASE_DEG):
    """Weld by position, then split again wherever the surface creases.

    Corners are first gathered by position. Each corner's adjacent facets are
    grouped so that facets within the crease angle of one another share a
    smoothed normal, and facets across a sharper edge get their own. That is
    what makes a rim round without rounding off the panel it is bolted to.
    """
    cos_crease = math.cos(math.radians(crease_deg))

    def norm(v):
        n = math.sqrt(sum(c * c for c in v))
        return (v[0] / n, v[1] / n, v[2] / n) if n > 1e-12 else (0.0, 0.0, 1.0)

    corners: dict[tuple, list[tuple[int, int]]] = defaultdict(list)
    for fi in members:
        for ci, p in enumerate(faces[fi][0]):
            corners[key_of(p)].append((fi, ci))

    points: list[tuple[float, float, float]] = []
    normals: list[tuple[float, float, float]] = []
    index_at: dict[tuple[int, int], int] = {}

    for key, group in corners.items():
        pos = faces[group[0][0]][0][group[0][1]]
        clusters: list[list[int]] = []
        cluster_normal: list[tuple[float, float, float]] = []
        for fi, _ci in group:
            fn = norm(faces[fi][1])
            for k, cn in enumerate(cluster_normal):
                if sum(a * b for a, b in zip(fn, cn)) >= cos_crease:
                    clusters[k].append(fi)
                    acc = [0.0, 0.0, 0.0]
                    for gfi in clusters[k]:
                        gn = norm(faces[gfi][1])
                        for a in range(3):
                            acc[a] += gn[a]
                    cluster_normal[k] = norm(acc)
                    break
            else:
                clusters.append([fi])
                cluster_normal.append(fn)

        for k, group_faces in enumerate(clusters):
            out = len(points)
            points.append(pos)
            normals.append(cluster_normal[k])
            for fi in group_faces:
                for ci in range(3):
                    if key_of(faces[fi][0][ci]) == key:
                        index_at[(fi, ci)] = out

    indices = [index_at[(fi, ci)] for fi in members for ci in range(3)]
    return points, normals, indices


def surface(stage, mesh_path, paint):
    """A UsdPreviewSurface so the mesh lights rather than reads as a cut-out.

    It lives *under* the mesh, not beside it. The machine USD references one
    mesh at a time, and a reference only brings its own subtree — a material
    kept alongside would be outside that scope and the binding dropped.
    """
    rgb, roughness, metallic = SURFACES[paint]
    mat = UsdShade.Material.Define(stage, f"{mesh_path}/mat")
    shader = UsdShade.Shader.Define(stage, f"{mesh_path}/mat/surface")
    shader.CreateIdAttr("UsdPreviewSurface")
    shader.CreateInput("diffuseColor", Sdf.ValueTypeNames.Color3f).Set(Gf.Vec3f(*rgb))
    shader.CreateInput("roughness", Sdf.ValueTypeNames.Float).Set(roughness)
    shader.CreateInput("metallic", Sdf.ValueTypeNames.Float).Set(metallic)
    mat.CreateSurfaceOutput().ConnectToSource(shader.ConnectableAPI(), "surface")
    return mat


def emit(stage, name, paint, faces, members):
    if not members:
        print(f"  {name:<26} {paint:<7} (empty, skipped)")
        return 0
    points, normals, indices = weld(faces, members)
    path = f"/geom/{name}"
    mesh = UsdGeom.Mesh.Define(stage, path)
    mesh.CreatePointsAttr(Vt.Vec3fArray(points))
    mesh.CreateNormalsAttr(Vt.Vec3fArray(normals))
    mesh.SetNormalsInterpolation(UsdGeom.Tokens.vertex)
    mesh.CreateFaceVertexIndicesAttr(Vt.IntArray(indices))
    mesh.CreateFaceVertexCountsAttr(Vt.IntArray([3] * len(members)))
    # Normals are authored, so the renderer must not try to derive its own.
    mesh.CreateSubdivisionSchemeAttr(UsdGeom.Tokens.none)
    mesh.CreateExtentAttr(Vt.Vec3fArray([
        Gf.Vec3f(*(min(p[a] for p in points) for a in range(3))),
        Gf.Vec3f(*(max(p[a] for p in points) for a in range(3))),
    ]))
    UsdShade.MaterialBindingAPI.Apply(mesh.GetPrim()).Bind(surface(stage, path, paint))
    print(f"  {name:<26} {paint:<7} {len(members):6d} tris  {len(points):6d} points")
    return len(members)


def check(name, got, want, tol=0.002):
    """Loud failure if a mesh lands somewhere other than where it belongs.

    The transforms in the build scripts were recovered by hand, and a silent
    change to one puts a machine back in the state where it looked right in a
    viewer and wrong in the simulator.
    """
    ok = True
    for axis, g, w in zip("xyzxyz", got, want):
        if abs(g - w) > tol:
            print(f"  !! {name}: {axis} extent {g:+.4f}, expected {w:+.4f}", file=sys.stderr)
            ok = False
    return ok


def new_stage(out: pathlib.Path):
    stage = Usd.Stage.CreateNew(str(out))
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage, 1.0)
    root = UsdGeom.Scope.Define(stage, "/geom")
    stage.SetDefaultPrim(root.GetPrim())
    return stage


def wheel_paint(faces, tyre_inner_r):
    """Split a wheel's faces into tyre and hub by distance from the axle (x)."""
    painted = defaultdict(list)
    for i, (tri, _n) in enumerate(faces):
        cy = sum(p[1] for p in tri) / 3.0
        cz = sum(p[2] for p in tri) / 3.0
        painted["tyre" if math.hypot(cy, cz) > tyre_inner_r else "hub"].append(i)
    return painted
