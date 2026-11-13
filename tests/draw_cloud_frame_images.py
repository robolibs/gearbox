"""Verify native typed cloud placement and metadata-only GPU updates."""

import copy
import json
import math
from pathlib import Path
import sys
import unittest

from PIL import Image


STAGES = ["01-initial", "02-frame-moved", "03-snapshot-restyled", "04-snapshot-local-pose", "05-capture-replaced"]
IDS = ("world", "odom", "live", "snapshot")


def components(image):
    selected = {name: set() for name in IDS}
    pixels = image.convert("RGB").load()
    for y in range(image.height):
        for x in range(image.width):
            r, g, b = pixels[x, y]
            name = None
            if r > 190 and g > 190 and abs(r - g) <= 8 and b < min(r, g) * .75:
                name = "world"
            elif g > 165 and b > 190 and r < g * .8:
                name = "odom"
            elif g > 180 and r < g * .8 and b < g * .8:
                name = "live"
            elif r > 180 and b > 180 and g < min(r, b) * .8:
                name = "snapshot"
            if name:
                selected[name].add((x, y))
    result = {}
    for name, remaining in selected.items():
        largest = []
        while remaining:
            todo = [remaining.pop()]
            group = []
            while todo:
                x, y = todo.pop()
                group.append((x, y))
                for point in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                    if point in remaining:
                        remaining.remove(point)
                        todo.append(point)
            if len(group) > len(largest):
                largest = group
        result[name] = {"pixels": len(largest), "center": [sum(p[i] for p in largest) / len(largest) for i in range(2)] if largest else None}
    return result


def verify_motion(markers, moved):
    assert moved > 2.5, f"robot did not move: {moved} m"
    for stage in STAGES:
        for name in IDS:
            assert markers[stage][name]["pixels"] > 40, f"missing {name} at {stage}"
    def distance(name, a, b):
        return math.dist(markers[STAGES[a]][name]["center"], markers[STAGES[b]][name]["center"])
    for name in ("world", "odom"):
        for stage in range(1, 5):
            assert distance(name, 0, stage) <= 2, f"{name} drifted at {STAGES[stage]}"
    assert distance("live", 0, 1) > 10, "live cloud did not follow the robot"
    for stage in range(2, 5):
        assert distance("live", 1, stage) <= 2, "unrelated metadata edit moved live cloud"
    assert distance("snapshot", 0, 1) <= 2, "snapshot followed the robot"
    assert distance("snapshot", 1, 2) <= 2, "restyling moved the snapshot"
    assert distance("snapshot", 2, 3) > 4, "local pose delta was ignored"
    assert distance("snapshot", 3, 4) > 10, "capture transform delta was ignored"


def verify(directory):
    markers = {stage: components(Image.open(directory / f"{stage}.png")) for stage in STAGES}
    poses = [json.loads((directory / f"{stage}.machine.json").read_text()) for stage in STAGES[:2]]
    moved = math.hypot(poses[1]["x"] - poses[0]["x"], poses[1]["y"] - poses[0]["y"])
    verify_motion(markers, moved)
    references = {}
    for reference, stage in [("06-reference-initial", STAGES[0]), ("07-reference-local", STAGES[3]), ("08-reference-capture", STAGES[4])]:
        reference_marker = components(Image.open(directory / f"{reference}.png"))["world"]
        assert reference_marker["pixels"] > 40, f"missing world reference at {reference}"
        distance = math.dist(reference_marker["center"], markers[stage]["snapshot"]["center"])
        assert distance <= 2, f"snapshot does not match explicit world reference at {stage}: {distance} px"
        references[reference] = {"marker": reference_marker, "snapshot_error_px": distance}
    cleared = components(Image.open(directory / "09-clear.png"))
    assert all(value["pixels"] <= 8 for value in cleared.values()), f"clear left a marker: {cleared}"
    stages = [json.loads((directory / f"{stage}.json").read_text()) for stage in STAGES]
    initial = stages[0]["metrics"]["uploads"]
    assert initial["positions"] == 108 * 16 and initial["attributes"] == 108 * 32
    for i, stage in enumerate(stages):
        uploads = stage["metrics"]["uploads"]
        assert uploads["positions"] == initial["positions"] and uploads["attributes"] == initial["attributes"], "point records were reuploaded"
        assert [d["key"]["id"] for d in stage["datasets"]] == list(IDS)
        for d in stage["datasets"]:
            visible = d["visibility"]
            assert visible["state"] == "visible" and visible["disposition"] == "drawn"
            assert visible["source_samples"] == visible["selected_items"] == 27
            expected = [1, 1, 2, 3, 4][i] if d["key"]["id"] == "snapshot" else 1
            assert visible["target"]["dataset"]["generation"] == expected
    captures = [s["datasets"][3]["scan"]["frame"]["capture_transform"] for s in stages]
    assert captures[:4] == [captures[0]] * 4
    assert captures[4]["stamp"]["sample"] == 2 and captures[4]["pose"] != captures[0]["pose"]
    assert stages[1]["metrics"]["uploads"]["placements"] > initial["placements"]
    assert stages[1]["metrics"]["uploads"]["styles"] == initial["styles"]
    clear = json.loads((directory / "09-clear.json").read_text())
    assert all(clear["allocations"][key] == 0 for key in ("positions", "attributes", "styles", "placements", "points", "pages", "gpu_bytes"))
    report = {"passed": True, "machine_moved_m": moved, "markers": markers, "world_references": references, "clear": cleared, "initial_uploads": initial, "final_uploads": clear["uploads"]}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


class ComponentOracleTests(unittest.TestCase):
    def test_yellow_green_terrain_does_not_replace_a_yellow_marker(self):
        image = Image.new("RGB", (120, 100), (60, 70, 40))
        image.paste((197, 219, 147), (0, 0, 120, 30))
        image.paste((197, 219, 147), (70, 40, 110, 80))
        image.paste((255, 255, 0), (50, 60, 64, 74))
        marker = components(image)["world"]
        self.assertEqual(marker["pixels"], 196)
        self.assertEqual(marker["center"], [56.5, 66.5])

    def test_missing_marker_and_tonemapped_yellow_are_distinct(self):
        for color in [(197, 219, 147), (215, 215, 215)]:
            self.assertEqual(components(Image.new("RGB", (20, 20), color))["world"]["pixels"], 0)
        self.assertEqual(components(Image.new("RGB", (20, 20), (215, 215, 131)))["world"]["pixels"], 400)


class MotionOracleTests(unittest.TestCase):
    def setUp(self):
        self.markers = {s: {name: {"pixels": 120, "center": [50, 50]} for name in IDS} for s in STAGES}
        for s in STAGES[1:]:
            self.markers[s]["live"]["center"] = [80, 50]
        self.markers[STAGES[3]]["snapshot"]["center"] = [60, 50]
        self.markers[STAGES[4]]["snapshot"]["center"] = [90, 50]

    def test_valid_transitions(self):
        verify_motion(self.markers, 3.0)

    def test_fixed_live_marker_is_rejected(self):
        self.markers[STAGES[1]]["live"]["center"] = [50, 50]
        with self.assertRaisesRegex(AssertionError, "did not follow"):
            verify_motion(self.markers, 3.0)

    def test_drifting_world_and_odom_are_rejected(self):
        for name in ("world", "odom"):
            values = copy.deepcopy(self.markers)
            values[STAGES[1]][name]["center"] = [60, 50]
            with self.assertRaisesRegex(AssertionError, "drifted"):
                verify_motion(values, 3.0)

    def test_snapshot_follow_and_ignored_edits_are_rejected(self):
        for stage, point, message in [(1, [80, 50], "followed"), (2, [55, 50], "restyling"), (3, [50, 50], "local pose"), (4, [60, 50], "capture transform")]:
            values = copy.deepcopy(self.markers)
            values[STAGES[stage]]["snapshot"]["center"] = point
            with self.assertRaisesRegex(AssertionError, message):
                verify_motion(values, 3.0)

    def test_missing_marker_is_rejected(self):
        self.markers[STAGES[0]]["world"]["pixels"] = 0
        with self.assertRaisesRegex(AssertionError, "missing"):
            verify_motion(self.markers, 3.0)

    def test_color_components_have_separate_centers(self):
        image = Image.new("RGB", (80, 80))
        for name, color, box in zip(IDS, [(255, 255, 0), (0, 200, 255), (0, 255, 0), (255, 0, 255)], [(0, 0, 10, 10), (20, 0, 30, 10), (0, 20, 10, 30), (20, 20, 30, 30)]):
            image.paste(color, box)
        result = components(image)
        self.assertTrue(all(value["pixels"] == 100 for value in result.values()))
        self.assertEqual(result["snapshot"]["center"], [24.5, 24.5])


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--test":
        unittest.main(argv=[sys.argv[0]])
    else:
        verify(Path(sys.argv[1]))
