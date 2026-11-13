"""Compare OBJ normals and concave faces against explicit reference triangles."""

import json
from pathlib import Path
import sys

from PIL import Image, ImageFilter

from draw_drape_images import coherent_pixels


def verify(directory):
    images = {mode: Image.open(directory / f"{mode}.png").convert("RGB")
              for mode in ("reference", "flat", "authored", "smooth", "concave-reference", "concave", "concave-authored")}
    assert len({image.size for image in images.values()}) == 1, "capture dimensions changed"
    masks = {}
    for mode, image in images.items():
        masks[mode] = coherent_pixels({(x, y) for y in range(image.height) for x in range(image.width)
            if (lambda c: c[0] > 60 and c[2] > 55 and c[1] < c[0] * .8 and c[1] < c[2] * .9)(image.getpixel((x, y)))})
        assert len(masks[mode]) > 1000, f"mesh is missing or too small: {mode}"
    report = {}
    for name, modes in [("reference", ("flat", "authored", "smooth")),
                        ("concave-reference", ("concave", "concave-authored"))]:
        reference = images[name]
        mask = Image.new("L", reference.size)
        for point in masks[name]:
            mask.putpixel(point, 255)
        mask = mask.filter(ImageFilter.MinFilter(7))
        interior = [point for point in masks[name] if mask.getpixel(point)]
        assert len(interior) > 1000, f"mesh has no stable interior: {name}"
        for mode in modes:
            differences = [sum(abs(a - b) for a, b in zip(reference.getpixel(p), images[mode].getpixel(p))) / 3 for p in interior]
            report[mode] = {
                "mask_iou": len(masks[mode] & masks[name]) / len(masks[mode] | masks[name]),
                "mean_channel_error": sum(differences) / len(differences),
                "interior_pixels": len(interior),
            }
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    for mode in ("flat", "authored"):
        assert report[mode]["mask_iou"] > .98 and report[mode]["mean_channel_error"] < 1.5, f"{mode} normals changed: {report[mode]}"
    assert report["smooth"]["mean_channel_error"] > 3, "smoothing was lost; flat and smooth shading are indistinguishable"
    for mode in ("concave", "concave-authored"):
        assert report[mode]["mask_iou"] > .98 and report[mode]["mean_channel_error"] < 1.5, f"{mode} topology changed: {report[mode]}"
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
