"""Native compact/generic point rendering parity."""

import json
from pathlib import Path
import sys
import unittest
from PIL import Image


def mask(image, baseline=None):
    pixels = list(image.getdata())
    background = list(baseline.getdata()) if baseline is not None else [(0, 0, 0)] * len(pixels)
    result = set()
    for i, ((r, g, b), (br, bg, bb)) in enumerate(zip(pixels, background)):
        dr, dg, db = r - br, g - bg, b - bb
        if (r > 80 and b > 65 and g < r * .65 and g < b * .8) or (
                dr > 3 and db > 3 and dr > dg * 1.5 and db > dg * 1.3):
            result.add(i)
    return result


def compare(a, b, baseline=None):
    assert a.size == b.size
    ma, mb = mask(a, baseline), mask(b, baseline)
    assert min(len(ma), len(mb)) > 16, f"missing point positive control: compact={len(ma)}, generic={len(mb)}"
    union = ma | mb
    overlap = len(ma & mb) / len(union)
    pa, pb = list(a.getdata()), list(b.getdata())
    difference = sum(abs(pa[i][c] - pb[i][c]) for i in union for c in range(3)) / (3 * len(union))
    width = a.width
    def nearby(source, target):
        return sum(any(0 <= i % width + dx < width and 0 <= i // width + dy < a.height
                       and i + dy * width + dx in target
                       for dy in (-1, 0, 1) for dx in (-1, 0, 1)) for i in source) / len(source)
    boundary = min(nearby(ma, mb), nearby(mb, ma))
    centers = [(sum(i % width for i in m) / len(m), sum(i // width for i in m) / len(m)) for m in (ma, mb)]
    center_error = sum((a - b) ** 2 for a, b in zip(*centers)) ** .5
    area_error = abs(len(ma) - len(mb)) / max(len(ma), len(mb))
    assert boundary > .99 and center_error < .25 and area_error < .03, f"point shape differs: boundary={boundary}, center={center_error}, area={area_error}"
    assert difference < 2.0, f"point shading differs: {difference}"
    return {"compact_pixels": len(ma), "generic_pixels": len(mb), "mask_iou": overlap, "boundary_match": boundary,
            "center_error_px": center_error, "area_error": area_error, "mean_channel_error": difference}


def compare_overlap(a, b, baseline):
    assert a.size == b.size == baseline.size
    pa, pb, pc = [list(image.getdata()) for image in (a, b, baseline)]
    def colored(pixels):
        return {i for i, (r, g, blue) in enumerate(pixels) if blue > 100 and blue - min(r, g) > 35}
    ma, mb = colored(pa), colored(pb)
    assert min(len(ma), len(mb)) > 100, "missing overlapping transparent point positive control"
    union = ma | mb
    difference = sum(abs(pa[i][c] - pb[i][c]) for i in union for c in range(3)) / (3 * len(union))
    baseline_difference = sum(abs(pb[i][c] - pc[i][c]) for i in mb for c in range(3)) / (3 * len(mb))
    assert baseline_difference > 10, "overlap fixture matches empty background"
    assert ma == mb and difference < 0.1, f"transparent point order differs: {difference}"
    return {"compact_pixels": len(ma), "generic_pixels": len(mb), "mean_channel_error": difference,
            "baseline_mean_channel_difference": baseline_difference}


def verify(directory):
    report = {}
    baseline = Image.open(directory / "clear.png").convert("RGB")
    for case in range(20):
        images = [Image.open(directory / f"case-{case:02d}-{kind}.png").convert("RGB") for kind in ("compact", "generic")]
        try:
            report[str(case)] = compare(*images, baseline)
        except AssertionError as error:
            report[str(case)] = {"error": str(error), "compact_pixels": len(mask(images[0], baseline)), "generic_pixels": len(mask(images[1], baseline))}
    try:
        images = [Image.open(directory / f"case-20-{kind}.png").convert("RGB") for kind in ("compact", "generic")]
        report["20"] = compare_overlap(*images, baseline)
    except AssertionError as error:
        report["20"] = {"error": str(error)}
    assert not mask(baseline), "clear left point pixels"
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    assert all("error" not in value for value in report.values()), "native point comparisons failed; inspect image-metrics.json"


class OracleTests(unittest.TestCase):
    def fixture(self, color=(230, 30, 170), shift=0):
        image = Image.new("RGB", (24, 24))
        for y in range(5, 15):
            for x in range(5 + shift, 15 + shift):
                image.putpixel((x, y), color)
        return image

    def test_identical_visible_points_pass(self):
        result = compare(self.fixture(), self.fixture())
        self.assertEqual(result["mask_iou"], 1.0)
        self.assertEqual(result["mean_channel_error"], 0.0)

    def test_missing_shifted_and_recolored_points_fail(self):
        for other in (Image.new("RGB", (24, 24)), self.fixture(shift=3), self.fixture((160, 30, 230))):
            with self.assertRaises(AssertionError):
                compare(self.fixture(), other)

    def test_overlap_requires_visible_matching_blend_order(self):
        empty = Image.new("RGB", (24, 24), (30, 30, 30))
        a = Image.new("RGB", (24, 24), (100, 170, 220))
        b = Image.new("RGB", (24, 24), (170, 100, 220))
        self.assertEqual(compare_overlap(a, a, empty)["mean_channel_error"], 0.0)
        for other in (b, empty):
            with self.assertRaises(AssertionError):
                compare_overlap(a, other, empty)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        verify(Path(sys.argv[1]))
