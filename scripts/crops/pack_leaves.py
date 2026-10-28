#!/usr/bin/env python3
"""Cut single leaves out of ambientCG's CC0 leaf sets into gearbox's crop atlases.

Each atlas is a row of equal cells, one leaf to a cell, tip up, its base at
the bottom centre of the cell, scaled to fill the cell's height. RGB is the
photographed colour, A the leaf's opacity mask; transparent texels carry the
nearest leaf colour so mipmaps stay green. Run from the repository root with
the ambientCG zips unpacked under `target/crops/leaves/<set>/` (download them
from https://ambientcg.com/get?file=<set>_1K-PNG.zip).

    nix-shell -p 'python3.withPackages(p: [p.pillow p.numpy p.scipy])' \
        --run 'python3 scripts/crops/pack_leaves.py'
"""
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

SOURCE = Path("target/crops/leaves")
OUT = Path("bin/gearbox/assets/crops")
# atlas: (leaf set, unblemished leaves in reading order, cell width, cell height,
# stretch to fill, quarter turns anticlockwise that bring a leaf lying on its
# side tip up)
ATLASES = {
    "lettuce": ("LeafSet009", [2, 3, 0, 1], 256, 288, False),
    "tomato": ("LeafSet014", [0, 1, 2, 4], 160, 288, False),
    "potato": ("LeafSet024", [0, 1, 2, 4], 192, 288, False),
    # Wheat blades: an elongated leaf drawn out into a ribbon.
    "wheat": ("LeafSet022", [1, 2, 4, 5], 48, 512, True),
    "spinach": ("LeafSet018", [1, 2, 0, 1], 160, 288, False),
    "kale": ("LeafSet026", [0, 1, 2, 5], 256, 288, False, 1),
    "radish": ("LeafSet020", [0, 1, 2, 3], 80, 288, False),
    # Carrot fronds: a feathery conifer sprig stands in for the finely cut leaf.
    "carrot": ("LeafSet019", [1, 2, 0, 1], 112, 288, False, 1),
    "pepper": ("LeafSet003", [1, 0, 2, 3], 128, 288, False),
    "strawberry": ("LeafSet001", [0, 2, 4, 5], 160, 288, False),
    "bean": ("LeafSet023", [0, 1, 2, 3], 160, 288, False, 1),
    "sweet_potato": ("LeafSet004", [0, 1, 2, 5], 224, 288, False),
}


def leaves(name):
    folder = SOURCE / name
    colour = np.asarray(Image.open(folder / f"{name}_1K-PNG_Color.png").convert("RGB"))
    alpha = np.asarray(Image.open(folder / f"{name}_1K-PNG_Opacity.png").convert("L"))
    labels, count = ndimage.label(alpha > 127)
    boxes = ndimage.find_objects(labels)
    sizes = ndimage.sum(np.ones_like(labels), labels, range(1, count + 1))
    big = [i for i in range(count) if sizes[i] > 0.02 * alpha.size / 12]
    # Reading order: rows top to bottom, then left to right.
    big.sort(key=lambda i: (round(boxes[i][0].start / 120), boxes[i][1].start))
    return colour, alpha, labels, [(i + 1, boxes[i]) for i in big]


def fill_transparent(rgb, mask):
    """Each transparent texel takes the colour of the nearest opaque one."""
    _, (rows, cols) = ndimage.distance_transform_edt(~mask, return_indices=True)
    return rgb[rows, cols]


def pack(atlas, spec):
    name, picks, cell_w, cell_h, stretch, *turns = spec
    turns = turns[0] if turns else 0
    colour, alpha, labels, found = leaves(name)
    out = np.zeros((cell_h, cell_w * len(picks), 4), np.uint8)
    for cell, pick in enumerate(picks):
        label, (rows, cols) = found[pick]
        mask = labels[rows, cols] == label
        rgb = colour[rows, cols]
        a = np.where(mask, alpha[rows, cols], 0).astype(np.uint8)
        mask, rgb, a = (np.rot90(x, turns) for x in (mask, rgb, a))
        height, width = mask.shape
        scale = min(cell_h / height, cell_w / width)
        size = (cell_w, cell_h) if stretch else (max(1, round(width * scale)), max(1, round(height * scale)))
        leaf_rgb = Image.fromarray(fill_transparent(rgb, mask)).resize(size, Image.LANCZOS)
        leaf_a = Image.fromarray(a).resize(size, Image.LANCZOS)
        x = cell * cell_w + (cell_w - size[0]) // 2
        y = cell_h - size[1]
        out[y:y + size[1], x:x + size[0], :3] = np.asarray(leaf_rgb)
        out[y:y + size[1], x:x + size[0], 3] = np.asarray(leaf_a)
        print(f"  {atlas} cell {cell}: {name} leaf {pick}, {width}x{height} px")
    mask = out[..., 3] > 127
    out[..., :3] = fill_transparent(out[..., :3], mask)
    OUT.mkdir(parents=True, exist_ok=True)
    Image.fromarray(out, "RGBA").save(OUT / f"{atlas}_leaves.png", optimize=True)


for atlas, spec in ATLASES.items():
    pack(atlas, spec)
