"""Native compact legacy lifecycle viewer positive controls."""

import json
import math
from pathlib import Path
import sys
import unittest
from PIL import Image
from draw_cloud_frame_images import components, verify_motion, STAGES as FRAME_STAGES, IDS


STAGES = ("00-empty", "01-initial", "02-pose", "03-color", "04-glass", "05-effect-only",
          "06-label-fallback", "07-decorated-fallback", "08-compact-restored", "09-patch",
          "10-rejected-patch", "11-extend", "12-delete", "13-namespaces", "14-scoped-clear",
          "15-clear", "16-ttl-paused", "17-ttl-expired")
EMPTY = {"00-empty", "12-delete", "15-clear", "17-ttl-expired"}


def marker(image, cyan=False):
    selected_pixels = set()
    for i, (r, g, b) in enumerate(image.convert("RGB").getdata()):
        high, low = (g, r) if cyan else (r, g)
        selected = high > 150 and b > 150 and low + 40 < min(high, b)
        if selected:
            selected_pixels.add((i % image.width, i // image.width))
    pixels = []
    while selected_pixels:
        todo = [selected_pixels.pop()]
        group = []
        while todo:
            x, y = todo.pop()
            group.append((x, y))
            for point in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                if point in selected_pixels:
                    selected_pixels.remove(point)
                    todo.append(point)
        if len(group) >= 12:
            pixels.extend(group)
    return {"pixels": len(pixels), "center": [sum(p[k] for p in pixels) / len(pixels) for k in (0, 1)] if pixels else [0, 0]}


def verify_markers(markers):
    for stage in STAGES:
        if stage in EMPTY:
            assert markers[stage]["pixels"] <= 8, f"clear left points at {stage}"
        else:
            assert markers[stage]["pixels"] > 100, f"missing visible positive control at {stage}"
    center = lambda a, b: math.dist(markers[a]["center"], markers[b]["center"])
    assert center("01-initial", "02-pose") > 30, "pose did not move points"
    assert center("02-pose", "03-color") < 2, "color moved points"
    assert center("01-initial", "06-label-fallback") < 2, "generic fallback moved points"
    assert center("01-initial", "08-compact-restored") < 2, "compact restore moved points"
    assert center("09-patch", "10-rejected-patch") < 0.25, "rejected patch changed points"
    assert abs(markers["09-patch"]["pixels"] - markers["10-rejected-patch"]["pixels"]) <= 4
    assert markers["13-namespaces"]["pixels"] > 1.5 * markers["14-scoped-clear"]["pixels"], "scoped clear removed wrong subset"


def verify(directory):
    markers = {s: marker(Image.open(directory / f"{s}.png"), s == "03-color") for s in STAGES}
    (directory / "marker-diagnostics.json").write_text(json.dumps(markers, indent=2) + "\n")
    verify_markers(markers)
    frames = {s: components(Image.open(directory / f"frame-{s}.png")) for s in FRAME_STAGES}
    poses = [json.loads((directory / f"frame-{s}.machine.json").read_text()) for s in FRAME_STAGES[:2]]
    moved = math.hypot(poses[1]["x"] - poses[0]["x"], poses[1]["y"] - poses[0]["y"])
    secondary = {s: components(Image.open(directory / f"{i + 2}-b.png")) for i, s in enumerate(FRAME_STAGES)}
    (directory / "frame-diagnostics.json").write_text(json.dumps({"primary": frames, "secondary": secondary}, indent=2) + "\n")
    verify_motion(frames, moved)
    verify_motion(secondary, moved)
    cleared = components(Image.open(directory / "frame-06-clear.png"))
    assert all(cleared[name]["pixels"] <= 8 for name in IDS), "frame clear left points"
    reports = [json.loads((directory / f"frame-{s}.json").read_text()) for s in FRAME_STAGES]
    assert all(m["uploaded"][:2] == reports[0]["uploaded"][:2] for m in reports)
    report = {"passed": True, "markers": markers, "frames": frames, "secondary_1x": secondary,
              "machine_moved_m": moved, "frame_initial_uploads": reports[0]["uploaded"], "frame_final_uploads": reports[-1]["uploaded"]}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


class OracleTests(unittest.TestCase):
    def test_tonemapped_cyan_is_distinct_from_neutral_background(self):
        self.assertEqual(marker(Image.new("RGB", (20, 20), (125, 205, 205)), True)["pixels"], 400)
        self.assertEqual(marker(Image.new("RGB", (20, 20), (205, 205, 205)), True)["pixels"], 0)
        self.assertEqual(marker(Image.new("RGB", (20, 20), (205, 125, 205)))["pixels"], 400)
        self.assertEqual(marker(Image.new("RGB", (20, 20), (205, 205, 205)))["pixels"], 0)

    def test_isolated_background_pixels_do_not_move_the_marker(self):
        image = Image.new("RGB", (60, 60), (60, 70, 40))
        image.paste((125, 205, 205), (5, 5, 25, 25))
        expected = marker(image, True)
        image.putpixel((50, 50), (0, 255, 255))
        self.assertEqual(marker(image, True), expected)

    def fixture(self):
        out = {s: {"pixels": 300, "center": [50, 50]} for s in STAGES}
        for s in EMPTY: out[s] = {"pixels": 0, "center": [0, 0]}
        for s in ("02-pose", "03-color"): out[s]["center"] = [100, 50]
        out["13-namespaces"]["pixels"] = 600
        return out

    def test_visible_transitions_and_empty_cleanup(self):
        verify_markers(self.fixture())

    def test_missing_fallback_failed_clear_and_ignored_pose_fail(self):
        for name, value in [("06-label-fallback", {"pixels":0,"center":[0,0]}),
                            ("12-delete", {"pixels":300,"center":[50,50]}),
                            ("02-pose", {"pixels":300,"center":[50,50]})]:
            markers = self.fixture()
            markers[name] = value
            with self.assertRaises(AssertionError): verify_markers(markers)


if __name__ == "__main__":
    if sys.argv[1:] == ["--test"]: unittest.main(argv=[sys.argv[0]])
    else: verify(Path(sys.argv[1]))
