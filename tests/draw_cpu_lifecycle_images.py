"""Check partial clear, surviving neighbors, deletion and recreation in both views."""

import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

from PIL import Image, ImageDraw
from draw_multiview_images import classify


STAGES = ["initial", "namespace-cleared", "deleted", "recreated"]
COLORS = ["red+blue", "blue", "none", "magenta"]


def verify(root, stages=STAGES, colors=COLORS):
    captures = json.loads((root / "captures.json").read_text())
    assert [item["name"] for item in captures] == stages
    metrics = []
    assert len(stages) == len(colors)
    for item, expected in zip(captures, colors):
        assert item["colors"] == [expected, expected]
        wanted = set() if expected == "none" else set(expected.split("+"))
        for suffix in "ab":
            path = root / f"{item['sequence']}-{suffix}.png"
            counts, components = classify(path)
            for color in wanted:
                assert counts[color] > 800 and len(components[color]) == 2, (path, "missing live neighbor/pages", counts, components)
            other = sum(n for kind, n in counts.items() if kind not in wanted)
            assert other < 20, (path, "retired or wrong-generation points", counts)
            metrics.append({"stage": item["name"], "view": suffix, "counts": counts, "components": components})
    result = {"passed": True, "captures": metrics}
    (root / "image-metrics.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


class OracleTests(unittest.TestCase):
    def fixture(self, root):
        captures = []
        rgb = {"red": (255, 20, 20), "blue": (20, 20, 255), "magenta": (255, 20, 255)}
        for sequence, (name, color) in enumerate(zip(STAGES, COLORS)):
            captures.append({"name": name, "sequence": sequence, "colors": [color, color]})
            for suffix in "ab":
                image = Image.new("RGB", (240, 100))
                draw = ImageDraw.Draw(image)
                for index, kind in enumerate(color.split("+")):
                    if kind == "none":
                        continue
                    for x in (30 + index * 120, 85 + index * 120):
                        draw.ellipse((x - 17, 33, x + 17, 67), fill=rgb[kind])
                image.save(root / f"{sequence}-{suffix}.png")
        (root / "captures.json").write_text(json.dumps(captures))

    def test_correct_stages_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with contextlib.redirect_stdout(io.StringIO()):
                verify(root)

    def test_ghost_or_lost_neighbor_fails(self):
        for source in ("0-a.png", "2-a.png"):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                with Image.open(root / source) as image:
                    image.save(root / "1-a.png")
                with self.assertRaises(AssertionError):
                    verify(root)


if __name__ == "__main__":
    if sys.argv[1:] == ["--test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        verify(Path(sys.argv[1]))
