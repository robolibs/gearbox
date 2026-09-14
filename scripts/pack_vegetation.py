#!/usr/bin/env python3
"""Pack Poly Haven plant clumps for gearbox's instanced vegetation.

Each pack takes chosen clump nodes out of a model's 1k glTF and writes
`meshes/<model>.bin` (VEG1: vertex count, index count, then per vertex
position, normal, uv, colour = (variant, patch salt, sparse share, gain), then
u32 indices, variants contiguous) plus `textures/<model>.png` (diffuse RGB,
alpha map A) into the fields' clumps package. Run from the repository root.
"""
import json
import struct
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path("bin/gearbox/assets/vegetation")
OUT = Path("bin/gearbox/src/fields/clumps")
# model: (nodes by name suffix, patch salt, share of plants outside patches)
PACKS = {
    "grass_bermuda_01": ([f"medium_{c}" for c in "abcdef"] + [f"small_{c}" for c in "abcdef"]
                         + ["dead_a", "dead_b", "flattened_a"], 1.0, 0.35),
    "grass_medium_01": ([f"tiny_{c}_LOD0" for c in "abcdef"], 2.0, 0.2),
    "shrub_sorrel_01": ([f"_{c}" for c in "ghijk"], 3.0, 0.05),
    "celandine_01": (["a_LOD0", "b_LOD0", "e_LOD0"], 4.0, 0.1),
    "weed_plant_02": (["c_LOD0", "d_LOD0", "e_LOD0"], 5.0, 0.3),
}
# Albedo gain lifts each model's opaque texels towards this linear luma, the
# brightness of the procedural sward, so clumps do not read as dark specks.
TARGET_LUMA = 0.18
TYPES = {5121: np.uint8, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}


def box_blur(image, radius):
    """Separable box blur of an H x W (x C) float array with edge clamping."""
    for axis in (0, 1):
        pad = [(0, 0)] * image.ndim
        pad[axis] = (radius + 1, radius)
        sums = np.cumsum(np.pad(image, pad, mode="edge"), axis=axis)
        size = image.shape[axis]
        image = (np.take(sums, range(2 * radius + 1, 2 * radius + 1 + size), axis=axis)
                 - np.take(sums, range(0, size), axis=axis)) / (2 * radius + 1)
    return image


def pad_colour(rgb, alpha):
    """Fill transparent texels with nearby opaque colour so mips stay green."""
    rgb, weight = rgb.astype(np.float32), (alpha > 127).astype(np.float32)
    out, filled = rgb.copy(), weight > 0
    for radius in (1, 3, 9, 27, 81, 243):
        num = box_blur(rgb * weight[..., None], radius)
        den = box_blur(weight, radius)
        grow = ~filled & (den > 0.01)
        out[grow] = num[grow] / den[grow][:, None]
        filled |= grow
    out[~filled] = rgb[weight > 0].mean(axis=0)
    return np.clip(out, 0, 255).astype(np.uint8)
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def accessor(gltf, blob, index):
    acc = gltf["accessors"][index]
    view = gltf["bufferViews"][acc["bufferView"]]
    start = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    dtype, width = TYPES[acc["componentType"]], WIDTH[acc["type"]]
    count = acc["count"] * width
    data = np.frombuffer(blob, dtype=dtype, count=count, offset=start)
    return data.reshape(acc["count"], width) if width > 1 else data


def albedo_gain(rgb, alpha):
    linear = (np.asarray(rgb, np.float32) / 255.0) ** 2.2
    luma = linear[np.asarray(alpha) > 127] @ np.array([0.30, 0.59, 0.11], np.float32)
    return float(np.clip(TARGET_LUMA / max(luma.mean(), 1e-4), 1.0, 3.0))


def pack(model, suffixes, salt, sparse):
    folder = ROOT / model
    rgb = Image.open(folder / "textures" / f"{model}_diff_1k.jpg").convert("RGB")
    alpha = Image.open(folder / "textures" / f"{model}_alpha_1k.png").convert("L").resize(rgb.size)
    gain = albedo_gain(rgb, alpha)
    print(f"  {model} albedo gain {gain:.2f}")
    gltf = json.loads((folder / f"{model}.gltf").read_text())
    blob = (folder / gltf["buffers"][0]["uri"]).read_bytes()
    nodes = {n["name"]: n for n in gltf["nodes"]}
    vertices, indices, base = [], [], 0
    for variant, suffix in enumerate(suffixes):
        name = next(k for k in nodes if k.endswith(suffix))
        prim = gltf["meshes"][nodes[name]["mesh"]]["primitives"][0]
        pos = accessor(gltf, blob, prim["attributes"]["POSITION"]).astype(np.float32)
        nor = accessor(gltf, blob, prim["attributes"]["NORMAL"]).astype(np.float32)
        uv = accessor(gltf, blob, prim["attributes"]["TEXCOORD_0"]).astype(np.float32)
        idx = accessor(gltf, blob, prim["indices"]).astype(np.uint32)
        # Stand the clump on the origin: centred in XZ, root at y = 0.
        pos = pos - np.array([(pos[:, 0].min() + pos[:, 0].max()) / 2, pos[:, 1].min(),
                              (pos[:, 2].min() + pos[:, 2].max()) / 2], np.float32)
        colour = np.tile(np.array([variant, salt, sparse, gain], np.float32), (len(pos), 1))
        vertices.append(np.hstack([pos, nor, uv, colour]))
        indices.append(idx + base)
        base += len(pos)
        print(f"  {model} variant {variant}: {name} {len(idx) // 3} tris")
    vertices, indices = np.vstack(vertices).astype(np.float32), np.concatenate(indices)
    (OUT / "meshes").mkdir(parents=True, exist_ok=True)
    (OUT / "textures").mkdir(parents=True, exist_ok=True)
    with open(OUT / "meshes" / f"{model}.bin", "wb") as out:
        out.write(b"VEG1" + struct.pack("<II", len(vertices), len(indices)))
        out.write(vertices.tobytes() + indices.astype(np.uint32).tobytes())
    rgba = np.dstack([pad_colour(np.asarray(rgb), np.asarray(alpha)), np.asarray(alpha)])
    Image.fromarray(rgba, "RGBA").save(OUT / "textures" / f"{model}.png", optimize=True)


for model, spec in PACKS.items():
    pack(model, *spec)
