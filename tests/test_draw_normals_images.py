import contextlib
import io
from pathlib import Path
import tempfile
import unittest

from PIL import Image, ImageDraw

from draw_normals_images import verify


class NormalCaptureTests(unittest.TestCase):
    def captures(self, directory, flat=(200, 60, 180), smooth=(180, 50, 160)):
        for mode, color in [("reference", (200, 60, 180)), ("authored", (200, 60, 180)), ("flat", flat), ("smooth", smooth)]:
            image = Image.new("RGB", (100, 100), (40, 80, 30))
            ImageDraw.Draw(image).rectangle((10, 10, 90, 90), fill=color)
            image.save(directory / f"{mode}.png")
        for mode in ("concave-reference", "concave", "concave-authored"):
            image = Image.new("RGB", (100, 100), (40, 80, 30))
            ImageDraw.Draw(image).polygon([(10, 10), (90, 10), (90, 35), (35, 35),
                                          (35, 65), (90, 65), (90, 90), (10, 90)], fill=(200, 60, 180))
            image.save(directory / f"{mode}.png")

    def test_matching_hard_normals_and_distinct_smoothing_pass(self):
        with tempfile.TemporaryDirectory() as path, contextlib.redirect_stdout(io.StringIO()):
            directory = Path(path)
            self.captures(directory)
            verify(directory)

    def test_wrong_normals_and_lost_smoothing_fail(self):
        with tempfile.TemporaryDirectory() as path:
            directory = Path(path)
            self.captures(directory, flat=(180, 50, 160))
            with self.assertRaisesRegex(AssertionError, "flat normals changed"):
                verify(directory)
            self.captures(directory, smooth=(200, 60, 180))
            with self.assertRaisesRegex(AssertionError, "smoothing was lost"):
                verify(directory)

    def test_filled_concave_notch_fails(self):
        with tempfile.TemporaryDirectory() as path:
            directory = Path(path)
            self.captures(directory)
            image = Image.open(directory / "concave.png")
            ImageDraw.Draw(image).rectangle((35, 35, 90, 65), fill=(200, 60, 180))
            image.save(directory / "concave.png")
            with self.assertRaisesRegex(AssertionError, "concave topology changed"):
                verify(directory)


if __name__ == "__main__":
    unittest.main()
