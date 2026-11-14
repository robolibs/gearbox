"""Verify original-ordinal point selection in native two-view readbacks."""

import json
from pathlib import Path
import sys

from draw_multiview_images import classify


def verify(root):
    captures = json.loads((root / "captures.json").read_text())
    assert [capture["name"] for capture in captures] == [
        "indexed-initial", "indexed-restyled", "indexed-replacement-denied",
        "indexed-replacement-recovered", "indexed-cleared",
    ]
    metrics = []
    for capture in captures:
        for suffix, color in zip("ab", capture["colors"]):
            path = root / f"{capture['sequence']}-{suffix}.png"
            counts, components = classify(path)
            if color == "none":
                assert sum(counts.values()) < 20, (path, counts)
            else:
                assert counts[color] > 800, (path, "missing selected discs", counts)
                other = sum(value for key, value in counts.items() if key != color)
                assert other < counts[color] * 0.02 + 20, (path, "mixed generations", counts)
                assert len(components[color]) == 2, (path, "ordinal lookup failed", components)
                assert min(components[color]) > counts[color] * .3, (path, "missing selected cluster", components)
            metrics.append({"stage": capture["name"], "view": suffix, "pixels": counts, "components": components})
    result = {"passed": "ten native selected-index readbacks", "metrics": metrics}
    (root / "image-metrics.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
