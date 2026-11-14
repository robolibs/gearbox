"""Check native auxiliary-budget failure, recovery, resize and clear images."""

import json
from pathlib import Path
import sys
import unittest
import tempfile
import contextlib
import io
from PIL import Image, ImageDraw
from draw_multiview_images import classify


def verify(root):
    captures = json.loads((root / "captures.json").read_text())
    assert [item["name"] for item in captures] == ["opaque", "denied", "recovered", "resized", "disabled", "cleared"]
    metrics = []
    for item in captures:
        for suffix, color in zip("ab", item["colors"]):
            path = root / f"{item['sequence']}-{suffix}.png"
            counts, components = classify(path)
            if color == "none":
                assert sum(counts.values()) < 20, (path, "clear left colored samples", counts)
            else:
                assert counts[color] > 800 and len(components[color]) == 2, (path, "missing complete points", counts, components)
                assert sum(n for kind, n in counts.items() if kind != color) < counts[color] * .02 + 20, (path, "mixed generation", counts)
            if suffix == "b" and item["name"] in ("resized", "disabled", "cleared"):
                assert Image.open(path).size == (960, 540), "secondary target was not really resized"
            metrics.append({"stage": item["name"], "view": suffix, "counts": counts, "components": components})
    result = {"passed": True, "captures": metrics}
    (root / "image-metrics.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


class OracleTests(unittest.TestCase):
    def fixture(self, root):
        captures = []
        for sequence, name in enumerate(["opaque", "denied", "recovered", "resized", "disabled", "cleared"]):
            color = "red" if sequence < 2 else "magenta" if sequence < 5 else "none"
            captures.append({"name": name, "sequence": sequence, "colors": [color, color]})
            for suffix in "ab":
                size = (960, 540) if suffix == "b" and sequence >= 3 else (128, 128)
                image = Image.new("RGB", size)
                if color != "none":
                    draw = ImageDraw.Draw(image)
                    for x in (35, 90):
                        draw.ellipse((x - 17, 43, x + 17, 77), fill=(255, 20, 20) if color == "red" else (255, 20, 255))
                image.save(root / f"{sequence}-{suffix}.png")
        (root / "captures.json").write_text(json.dumps(captures))

    def test_complete_stages_pass_and_wrong_generation_or_missing_points_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with contextlib.redirect_stdout(io.StringIO()):
                verify(root)
            for replacement in ("0-a.png", "5-a.png"):
                with Image.open(root / replacement) as image:
                    image.save(root / "2-a.png")
                with self.assertRaises(AssertionError):
                    verify(root)

    def test_unresized_secondary_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with Image.open(root / "2-b.png") as image:
                image.save(root / "3-b.png")
            with self.assertRaises(AssertionError):
                verify(root)


if __name__ == "__main__":
    if sys.argv[1:] == ["--test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        verify(Path(sys.argv[1]))
