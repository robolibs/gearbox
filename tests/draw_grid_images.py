"""Compare updated terrain-grid geometry with freshly built native geometry."""

import json
import math
from pathlib import Path
import sys

from PIL import Image


def mask(path):
    image = Image.open(path).convert("RGB")
    points = {(x, y) for y in range(image.height) for x in range(image.width)
              if (lambda c: c[0] > 100 and c[2] > 95 and c[1] < c[0] * .75 and c[1] < c[2] * .9)(image.getpixel((x, y)))}
    assert len(points) > 100, f"visible grid missing: {path}"
    center = [sum(p[i] for p in points) / len(points) for i in (0, 1)]
    return points, center


def verify(directory):
    stages = ["01-world", "02-frame-updated", "03-frame-fresh", "04-anchor-updated", "05-anchor-fresh", "06-height-updated", "07-height-fresh"]
    captures = [mask(directory / f"{stage}.png") for stage in stages]
    report = {"markers": {stage: {"pixels": len(points), "center": center} for stage, (points, center) in zip(stages, captures)}, "comparisons": {}}
    for name, a, b in [("frame", 1, 2), ("anchor", 3, 4), ("height", 5, 6)]:
        pa, ca = captures[a]
        pb, cb = captures[b]
        report["comparisons"][name] = {"center_drift_px": math.dist(ca, cb), "mask_iou": len(pa & pb) / len(pa | pb)}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    assert math.dist(captures[0][1], captures[2][1]) > 30, "frame change did not move the grid"
    assert math.dist(captures[4][1], captures[6][1]) > 20, "pose height did not raise the grid"
    for name, result in report["comparisons"].items():
        assert result["center_drift_px"] <= 2 and result["mask_iou"] > .97, f"{name} update reused stale geometry: {result}"
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
