"""Verify complete generation colors in native multiview GPU readbacks."""

import json
from pathlib import Path
import sys

from PIL import Image


def classify(path):
    image = Image.open(path).convert("RGB")
    counts = {"red": 0, "blue": 0, "magenta": 0}
    masks = {color: set() for color in counts}
    for index, (r, g, b) in enumerate(image.get_flattened_data()):
        if r > 120 and r > g * 1.8 and r > b * 1.5:
            counts["red"] += 1
            masks["red"].add(index)
        if b > 120 and b > g * 1.6 and b > r * 1.5:
            counts["blue"] += 1
            masks["blue"].add(index)
        if min(r, b) > 120 and min(r, b) > g * 1.35 and 0.8 < r / b < 1.25:
            counts["magenta"] += 1
            masks["magenta"].add(index)
    components = {}
    for color, pixels in masks.items():
        sizes = []
        while pixels:
            queue = [pixels.pop()]
            size = 0
            while queue:
                index = queue.pop()
                size += 1
                neighbors = [index - image.width, index + image.width]
                if index % image.width:
                    neighbors.append(index - 1)
                if index % image.width != image.width - 1:
                    neighbors.append(index + 1)
                for neighbor in neighbors:
                    if neighbor in pixels:
                        pixels.remove(neighbor)
                        queue.append(neighbor)
            if size >= 200:
                sizes.append(size)
        components[color] = sorted(sizes)
    return counts, components


def verify(root):
    captures = json.loads((root / "captures.json").read_text())
    assert [capture["name"] for capture in captures] == [
        "initial", "one_page_delayed", "superseded_on_secondary", "recovered",
        "recreated", "clear_recreated", "allocation_denied", "allocation_recovered",
        "staging_denied", "staging_recovered",
    ], "missing native generation or admission transitions"
    metrics = []
    for capture in captures:
        for suffix, color in zip("ab", capture["colors"]):
            path = root / f"{capture['sequence']}-{suffix}.png"
            counts, components = classify(path)
            assert counts[color] > 800, (path, "missing complete discs", counts)
            other = sum(value for key, value in counts.items() if key != color)
            assert other < counts[color] * 0.02 + 20, (path, "mixed generations", counts)
            assert len(components[color]) == 2, (path, "missing or extra page disc", components)
            assert min(components[color]) > counts[color] * .3, (path, "incomplete page disc", components)
            metrics.append({"stage": capture["name"], "view": suffix, "color": color, "pixels": counts, "components": components[color]})
    report = {"passed": "twenty native per-view generation and quota readbacks", "metrics": metrics}
    (root / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
