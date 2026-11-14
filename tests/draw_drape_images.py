"""Compare implicit and explicit origin-instance terrain placement."""

import json
import math
from pathlib import Path
import sys

from PIL import Image


def coherent_pixels(points):
    remaining = set(points)
    kept = set()
    while remaining:
        component = {remaining.pop()}
        pending = list(component)
        while pending:
            x, y = pending.pop()
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    point = x + dx, y + dy
                    if point in remaining:
                        remaining.remove(point)
                        component.add(point)
                        pending.append(point)
        if len(component) >= 32:
            kept.update(component)
    return kept


def mask(path, kind):
    image = Image.open(path).convert("RGB")
    pixels = image.load()
    points = set()
    for y in range(image.height):
        for x in range(image.width):
            r, g, b = pixels[x, y]
            matches = b > 100 and b > r * 1.5 and b > g * 1.1 if kind == "axes" else r > 100 and b > 95 and g < r * .7 and g < b * .8
            if matches:
                points.add((x, y))
    points = coherent_pixels(points)
    assert len(points) > 50, f"drawing missing: {path}"
    center = [sum(p[i] for p in points) / len(points) for i in (0, 1)]
    return points, center


def verify(directory):
    report = {}
    for kind in ("axes", "shape", "mesh"):
        a, ca = mask(directory / f"{kind}-false.png", kind)
        b, cb = mask(directory / f"{kind}-true.png", kind)
        report[kind] = {"implicit_pixels": len(a), "explicit_pixels": len(b),
                        "center_drift_px": math.dist(ca, cb), "mask_iou": len(a & b) / len(a | b)}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    for kind, result in report.items():
        assert result["center_drift_px"] < 1 and result["mask_iou"] > .98, f"{kind} origin-instance placement differs: {result}"
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
