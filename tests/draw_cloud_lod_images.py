"""Verify native paged-cloud scale and camera readbacks."""

import json
from pathlib import Path
import sys

from draw_multiview_images import classify


def verify(root):
    captures = json.loads((root / "captures.json").read_text())
    expected = [f"lod-{count}-{pose}" for count in (100_000, 1_000_000, 5_000_000)
                for pose in ("near", "far", "grazing", "motion-1", "motion-2", "motion-3", "clear")]
    assert [capture["name"] for capture in captures] == expected
    metrics = []
    for capture in captures:
        for suffix, color in zip("ab", capture["colors"]):
            path = root / f"{capture['sequence']}-{suffix}.png"
            counts, _ = classify(path)
            if color == "none":
                assert sum(counts.values()) < 20, (path, "retired cloud remains", counts)
            else:
                assert counts[color] > 800, (path, "cloud disappeared", counts)
                assert counts["blue"] + counts["magenta"] < counts[color] * 0.02 + 20, (path, counts)
            metrics.append({"stage": capture["name"], "view": suffix, "pixels": counts})
    result = json.loads((root / "result.json").read_text())
    for case in result["cases"]:
        audit = case["motion_audit"]
        assert audit["active"] is False and audit["frames"] >= 20, audit
        assert audit["failed_frames"] == 0, audit
        assert audit["skipped_frames"] == 0, audit
        assert audit["frames"] == audit["end_frame"] - audit["start_frame"], audit
        assert audit["expected_sources"] == [case["source_points"]] * 2, audit
        assert audit["max_gpu_bytes"] <= result["limits"]["gpu_bytes"], audit
        assert audit["max_cpu_bytes"] <= result["limits"]["cpu_bytes"], audit
        assert audit["max_frame_upload"] <= result["limits"]["upload_bytes_per_frame"], audit
        for view in audit["views"]:
            assert view["frames"] == audit["frames"], view
            assert view["missing"] == view["empty"] == view["invalid"] == 0, view
            assert view["camera_changes"] >= 12 and view["selection_changes"] >= 2, view
            assert view["completions"] > 0 and view["invalid_completions"] == 0, view
    report = {"passed": "42 native 100k/1M/5M static and motion readbacks", "metrics": metrics}
    (root / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
