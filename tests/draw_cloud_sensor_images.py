"""Check typed-cloud viewer positive controls and actual robot-camera frames."""

import json
from pathlib import Path
import sys
import unittest

from PIL import Image


STAGES = ("before", "opaque", "translucent", "clear")
TYPED_STAGES = ("before", "opaque", "translucent", "octree", "surfels", "mesh", "volume", "elevation", "clear")
ORDER = TYPED_STAGES


def magenta(image):
    return sum(r > 150 and b > 130 and g < r * 0.65 and g < b * 0.65
               for r, g, b in image.convert("RGB").getdata())


def verify_images(viewer, sensor):
    assert len({im.size for im in viewer.values()}) == 1, "viewer capture dimensions changed"
    assert all(im.size == (160, 120) for im in sensor.values()), "unexpected camera dimensions"
    stages = [s for s in ORDER if s in viewer]
    counts = {stage: magenta(viewer[stage]) for stage in stages}
    assert counts["before"] < 100, "baseline already contains the cloud color"
    assert counts["clear"] < 100, "clear did not remove the viewer cloud"
    for stage in [s for s in ("opaque", "translucent", "octree", "surfels", "mesh", "volume", "elevation") if s in viewer]:
        assert counts[stage] > 1000, f"no rendered cloud positive control at {stage}: {counts}"
    baseline = sensor["before"].convert("RGBA").tobytes()
    assert any(value > 5 for value in sensor["before"].convert("RGB").tobytes()), "black sensor baseline"
    deltas = {}
    for stage in stages[1:]:
        pixels = sensor[stage].convert("RGBA").tobytes()
        mean = sum(abs(a - b) for a, b in zip(baseline, pixels)) / len(baseline)
        assert mean <= 1.0, f"sensor changed at {stage}: {mean}"
        assert magenta(sensor[stage]) <= 10, f"cloud visible in sensor at {stage}"
        deltas[stage] = mean
    return {"viewer_magenta_pixels": counts, "sensor_mean_channel_deltas": deltas}


def verify(directory, backend="typed-cloud"):
    names = TYPED_STAGES if backend == "typed-cloud" else STAGES
    viewer = {s: Image.open(directory / f"viewer-{s}.png").convert("RGB") for s in names}
    sensor = {s: Image.open(directory / f"sensor-{s}.png").convert("RGBA") for s in names}
    metrics = json.loads((directory / "sensor-metrics.json").read_text())
    assert backend in ("typed-cloud", "legacy-compact")
    assert metrics["backend"] == backend and metrics["points"] == 1617
    assert [value["stage"] for value in metrics["stages"]] == list(names[1:])
    previous_sample = None
    for value in metrics["stages"]:
        assert value["sample"] > value["after_sample"] + 3, "camera sample did not advance"
        if previous_sample is not None:
            assert value["after_sample"] == previous_sample, "camera samples are not chained"
        previous_sample = value["sample"]
        if value["stage"] != "clear" and backend == "typed-cloud":
            visible = value["visibility"]
            assert visible["state"] == "visible" and visible["disposition"] == "drawn"
            if value["stage"] in ("opaque", "translucent"):
                assert visible["source_samples"] == visible["selected_items"] == metrics["points"]
    if backend == "typed-cloud":
        opaque, translucent = [s["visibility"]["target"]["dataset"] for s in metrics["stages"][:2]]
        assert opaque["incarnation"] == translucent["incarnation"]
        assert translucent["generation"] == opaque["generation"] + 1
    else:
        assert [s["legacy_style"] for s in metrics["stages"]] == [16, 1, None]
        assert all("visibility" not in s for s in metrics["stages"])
    report = {"passed": True, "backend": backend, "link": metrics["link"], **verify_images(viewer, sensor)}
    (directory / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


class ImageOracleTests(unittest.TestCase):
    def setUp(self):
        self.viewer = {s: Image.new("RGB", (80, 60), (40, 50, 60)) for s in STAGES}
        self.sensor = {s: Image.new("RGBA", (160, 120), (40, 50, 60, 255)) for s in STAGES}
        for stage in ("opaque", "translucent"):
            self.viewer[stage].paste((240, 30, 230), (10, 10, 70, 50))

    def test_positive_controls_and_clean_sensors(self):
        self.assertEqual(verify_images(self.viewer, self.sensor)["viewer_magenta_pixels"]["opaque"], 2400)

    def test_absent_viewer_cloud_is_not_sensor_acceptance(self):
        self.viewer["opaque"] = self.viewer["before"]
        with self.assertRaisesRegex(AssertionError, "positive control"):
            verify_images(self.viewer, self.sensor)

    def test_invisible_translucent_path_is_rejected(self):
        self.viewer["translucent"] = self.viewer["before"]
        with self.assertRaisesRegex(AssertionError, "positive control"):
            verify_images(self.viewer, self.sensor)

    def test_camera_contamination_is_rejected(self):
        self.sensor["opaque"].paste((255, 0, 255, 255), (0, 0, 160, 120))
        with self.assertRaisesRegex(AssertionError, "sensor changed"):
            verify_images(self.viewer, self.sensor)

    def test_failed_clear_is_rejected(self):
        self.viewer["clear"] = self.viewer["opaque"]
        with self.assertRaisesRegex(AssertionError, "clear did not remove"):
            verify_images(self.viewer, self.sensor)


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--test":
        unittest.main(argv=[sys.argv[0]])
    else:
        verify(Path(sys.argv[1]), sys.argv[2] if len(sys.argv) > 2 else "typed-cloud")
