"""Compare native stroke detail before and after collinear subdivision."""

import json
from pathlib import Path
import sys

from PIL import Image


def mask(path):
    image = Image.open(path).convert("RGB")
    pixels = image.load()
    points = set()
    for y in range(image.height):
        for x in range(image.width):
            r, g, b = pixels[x, y]
            if r > 100 and b > 95 and g < r * .7 and g < b * .8:
                points.add((x, y))
    assert len(points) > 100, f"visible strokes missing: {path}"
    return points


def verify(directory):
    report = {}
    for view in ("near", "far"):
        for reverse in ("false", "true"):
            a = mask(directory / f"{view}-{reverse}-1.png")
            b = mask(directory / f"{view}-{reverse}-256.png")
            report[f"{view}-{reverse}"] = {"coarse_pixels": len(a), "dense_pixels": len(b), "mask_iou": len(a & b) / len(a | b)}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    for stage, result in report.items():
        assert result["mask_iou"] > .95, f"stroke pattern changes with subdivision: {stage}: {result}"
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
