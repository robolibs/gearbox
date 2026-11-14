"""Check native ghost visibility across source-machine unload and reload."""

import json
from pathlib import Path
import sys

from PIL import Image


def mask(path):
    image = Image.open(path).convert("RGB")
    return {(i % image.width, i // image.width)
            for i, (r, g, b) in enumerate(image.getdata())
            if r > 75 and b > 60 and g < .65 * r and g < .75 * b}


def verify(directory):
    before = mask(directory / "01-unloaded.png")
    counts = {stage: len(mask(directory / name) - before)
              for stage, name in [("loaded", "02-loaded.png"), ("removed", "03-removed.png"), ("reloaded", "04-reloaded.png")]}
    (directory / "image-metrics.json").write_text(json.dumps(counts, indent=2) + "\n")
    assert counts["loaded"] > 100, f"ghosts did not appear: {counts}"
    assert counts["removed"] <= max(30, .05 * counts["loaded"]), f"ghosts survived missing source: {counts}"
    assert counts["reloaded"] > 100, f"retained ghosts did not recover: {counts}"
    print(json.dumps(counts))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
