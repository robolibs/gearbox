#!/usr/bin/env python3
"""Build the de Marke cow house as a self-contained Gearbox asset.

    python scripts/build_demarke_barn.py

Reads the Isaac Sim export at `SOURCE` and writes

    bin/gearbox/assets/demarke/barn.usdc
    bin/gearbox/assets/demarke/Textures/*.png

The export is left untouched; both outputs are generated and git-ignored.

Loading the export directly into Gearbox went wrong in three ways, and each
one is dealt with here rather than in the loader:

*It is a web of references.* Every mesh prim references its own part file
under `Assets/.../Geometries/` and places it with a transform, 451 layers in
all. usd_bevy's composition tripped on that and spawned every part twice —
once raw, piled near the origin, and once arranged — so the barn appeared
with its own kit of construction parts next to it. The stage is flattened
into one layer with no references left for anything to mis-compose.

*Its textures do not fit.* All 176 are 4096 square: 11 GiB uncompressed,
more than the GPU has, and the renderer gave up with an out-of-memory error
and dropped every material. Colour maps are brought down to 2048 and
roughness maps to 1024 — under 2 GiB with mipmaps — and the flattened layer
points at those copies.

*It has no lights.* Gearbox lights the world with one sun, and under a roof
that is black. A row of lamps goes in along each alley.

It also drops a 320 m skybox mesh that swallowed the scene bounds, the eight
VRay cameras, which would otherwise be spawned as real cameras, and rolls up
the main gate at each end of the centre alley so machines can drive in.
"""

from __future__ import annotations

import pathlib
import sys
import time

from PIL import Image
from pxr import Gf, Sdf, Usd, UsdGeom, UsdLux

SOURCE = pathlib.Path(
    "/home/bresilla/data/code/other/isaacsim-de-marke/de marke/cow_house.usd"
)
OUT_DIR = pathlib.Path(__file__).resolve().parents[1] / "bin/gearbox/assets/demarke"
OUT_USD = OUT_DIR / "barn.usdc"
OUT_TEX = OUT_DIR / "Textures"

TOP = "/Root/deMarke_FINAL_cleaned"

# The main gates are rolled up. Each end of the centre alley has a sectional
# door of eight slats under a housing; the slats go, the housing and the top
# slat stay as the rolled-up edge, so the door reads as open rather than
# missing. The north end also carries the whole door a second time as one
# mesh, `GATE_Original_02`, on top of its slats.
SOUTH_GATE = ["GATE_ELEMENT_009", "GATE_ELEMENT_010", "GATE_ELELEMT_004",
              "GATE_ELEMENT_011", "GATE_ELEMENT_012", "GATE_ELEMENT_013",
              "GATE_ELEMENT_014"]
NORTH_GATE = ["GATE_ELEMENT_01", "GATE_ELEMENT_02", "GATE_ELELEMT_03",
              "GATE_ELEMENT_04", "GATE_ELEMENT_05", "GATE_ELEMENT_06",
              "GATE_ELEMENT_07", "GATE_Original_02"]
# `Object005` is a 13 m rail hanging in the air 1.6 m outside the east wall
# — the outer edge of the south-east canopy, exported without its posts.
STRAY = ["Object005"]
DROP = [f"{TOP}/{name}" for name in ["_671Background", *SOUTH_GATE, *NORTH_GATE, *STRAY]]

# Texture edge lengths after downscaling, by what the map is for.
TEXTURE_SIZE = {"BaseColor": 2048, "Roughness": 1024}
TEXTURE_SIZE_DEFAULT = 1024

# Lamps, in the barn's own frame and units (centimetres, z up). One row
# down each of the three alleys, hung just under the eaves.
LAMP_X_CM = (2490.0, 3088.0, 3940.0)
LAMP_Y_CM = range(600, 6401, 1600)
LAMP_Z_CM = 420.0
LAMP_INTENSITY = 60.0
LAMP_RADIUS_CM = 25.0


def deactivate(stage: Usd.Stage) -> int:
    """Switch off the skybox and the cameras. Returns how many prims."""
    count = 0
    for path in DROP:
        prim = stage.GetPrimAtPath(path)
        if prim:
            prim.SetActive(False)
            count += 1
    for prim in list(stage.Traverse()):
        if prim.GetTypeName() == "Camera":
            # The camera prims sit under an Xform of their own; drop that so
            # the target Xform beside it goes too.
            holder = prim.GetParent()
            (holder if holder.GetTypeName() == "Xform" else prim).SetActive(False)
            count += 1
    return count


def add_lamps(stage: Usd.Stage) -> int:
    scope = UsdGeom.Scope.Define(stage, f"{TOP}/Lamps")
    count = 0
    for x in LAMP_X_CM:
        for y in LAMP_Y_CM:
            lamp = UsdLux.SphereLight.Define(stage, scope.GetPath().AppendChild(f"lamp_{count:02d}"))
            lamp.CreateIntensityAttr(LAMP_INTENSITY)
            lamp.CreateRadiusAttr(LAMP_RADIUS_CM)
            lamp.CreateColorAttr(Gf.Vec3f(1.0, 0.96, 0.90))
            UsdGeom.Xformable(lamp).AddTranslateOp().Set(Gf.Vec3d(x, y, LAMP_Z_CM))
            count += 1
    return count


def downscale_textures(layer: Sdf.Layer) -> tuple[int, int]:
    """Copy every texture the layer uses into OUT_TEX at a size the GPU can
    hold, and point the layer at the copy. Returns (rewritten, resized)."""
    OUT_TEX.mkdir(parents=True, exist_ok=True)
    rewritten = resized = 0
    seen: dict[str, str] = {}

    def visit(path):
        nonlocal rewritten, resized
        spec = layer.GetObjectAtPath(path)
        if not isinstance(spec, Sdf.AttributeSpec) or spec.typeName != Sdf.ValueTypeNames.Asset:
            return
        value = spec.default
        if not value or not value.path:
            return
        source = pathlib.Path(value.resolvedPath or value.path)
        if not source.is_absolute():
            source = SOURCE.parent / source
        if source.suffix.lower() not in (".png", ".jpg", ".jpeg", ".exr", ".tga"):
            return
        target_name = source.name
        if target_name not in seen:
            target = OUT_TEX / target_name
            if not target.exists() or target.stat().st_mtime < source.stat().st_mtime:
                kind = next((k for k in TEXTURE_SIZE if k in target_name), None)
                edge = TEXTURE_SIZE.get(kind, TEXTURE_SIZE_DEFAULT)
                with Image.open(source) as image:
                    if max(image.size) > edge:
                        image = image.resize((edge, edge), Image.Resampling.LANCZOS)
                    image.save(target, optimize=False)
                resized += 1
            seen[target_name] = f"Textures/{target_name}"
        spec.default = Sdf.AssetPath(seen[target_name])
        rewritten += 1

    layer.Traverse(Sdf.Path.absoluteRootPath, visit)
    return rewritten, resized


def main() -> int:
    if not SOURCE.exists():
        print(f"source not found: {SOURCE}", file=sys.stderr)
        return 1
    started = time.time()

    stage = Usd.Stage.Open(str(SOURCE))
    stage.SetEditTarget(stage.GetSessionLayer())
    print(f"  switched off {deactivate(stage)} prims (skybox, cameras)")
    print(f"  added {add_lamps(stage)} lamps")

    flat = stage.Flatten()
    flat.defaultPrim = "Root"
    print(f"  flattened {len(stage.GetUsedLayers())} layers into one")

    rewritten, resized = downscale_textures(flat)
    print(f"  {rewritten} texture references now under Textures/, {resized} files resized")

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    flat.Export(str(OUT_USD))
    check = Usd.Stage.Open(str(OUT_USD))
    meshes = sum(1 for p in check.Traverse() if p.IsA(UsdGeom.Mesh))
    size_mb = OUT_USD.stat().st_size / 1048576
    tex_mb = sum(f.stat().st_size for f in OUT_TEX.iterdir()) / 1048576
    print(f"\nwrote {OUT_USD}  ({size_mb:.0f} MB, {meshes} meshes)")
    print(f"      {OUT_TEX}/  ({tex_mb:.0f} MB on disk)")
    print(f"  {time.time() - started:.0f} s")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
