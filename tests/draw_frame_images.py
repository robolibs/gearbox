"""Check controlled snapshot-marker positions in native frame captures."""

import json
import math
from pathlib import Path
import sys

from PIL import Image


def marker(path):
    image = Image.open(path).convert("RGB")
    pixels = image.load()
    points = []
    for y in range(image.height):
        for x in range(image.width):
            r, g, b = pixels[x, y]
            if r > 130 and b > 100 and g < r * .8 and g < b * .9:
                points.append((x, y))
    assert len(points) > 25, f"snapshot marker not found in {path}"
    bounds = [min(p[0] for p in points), min(p[1] for p in points), max(p[0] for p in points), max(p[1] for p in points)]
    return {"pixels": len(points), "bounds": bounds, "center": [(bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2]}


def verify(directory):
    stages = ["01-initial", "02-frame-moved", "03-snapshot-restyled", "04-snapshot-local-pose", "05-snapshot-replaced"]
    result = {stage: marker(directory / f"{stage}.png") for stage in stages}
    distance = lambda a, b: math.dist(result[stages[a]]["center"], result[stages[b]]["center"])
    before = json.loads((directory / f"{stages[0]}.machine.json").read_text())
    after = json.loads((directory / f"{stages[1]}.machine.json").read_text())
    moved = math.hypot(after["x"] - before["x"], after["y"] - before["y"])
    assert moved > 2.5, f"machine did not move enough: {moved} m"
    assert distance(0, 1) <= 2, f"snapshot moved with robot: {distance(0, 1)} px"
    assert distance(1, 2) <= 2, f"restyling moved snapshot: {distance(1, 2)} px"
    assert distance(2, 3) > 4, "pose update did not reposition inside captured frame"
    assert distance(3, 4) > 10, "replacement did not recapture the moved frame"
    report = {"passed": "native snapshot position transitions", "machine_moved_m": moved, "markers": result}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
