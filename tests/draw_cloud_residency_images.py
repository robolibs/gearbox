"""Verify selected source-bank eviction and reload in native views."""

import json
import math
from pathlib import Path
import sys

from draw_multiview_images import classify


def verify_fine(view):
    assert view["coarse"] is False and view["preparing"] is False, view
    assert 12_288 < view["selected"] <= 1_000_000, view
    assert view["budget_limited_pages"] == 0, view
    assert view["valid"] == 6_291_456, view
    assert view["represented"] == view["culled"] == 3_145_728, view
    assert view["verified_ordinals"] is True, view
    error = view["max_error_px"]
    assert isinstance(error, (int, float)) and math.isfinite(error), view
    assert 0 <= error <= 1.5 * 1.15, view


def self_test():
    good = dict(coarse=False, preparing=False, selected=200_000,
                budget_limited_pages=0,
                valid=6_291_456, represented=3_145_728, culled=3_145_728,
                verified_ordinals=True, max_error_px=1.7)
    verify_fine(good)
    invalid = [("coarse", True), ("preparing", True), ("selected", 0),
               ("selected", 12_288), ("selected", 1_000_001),
               ("budget_limited_pages", 1),
               ("valid", 0), ("represented", 0), ("culled", 0),
               ("verified_ordinals", False), ("max_error_px", None),
               ("max_error_px", float("nan")), ("max_error_px", float("inf")),
               ("max_error_px", -1.0), ("max_error_px", 1.726)]
    for field, value in invalid:
        try:
            verify_fine(good | {field: value})
        except AssertionError:
            continue
        raise AssertionError(("invalid fine cut accepted", field, value))
    print("fine-cut quality oracle: valid cut and 15 rejection cases passed")


def verify(root):
    captures = json.loads((root / "captures.json").read_text())
    expected = ["resident-left", "coarse-right", "direct-right", "coarse-left", "direct-left",
                "evicted-center", "resident-right", "evicted-again",
                "reloaded-left", "divergent-left-right", "divergent-right-left", "residency-cleared"]
    assert [capture["name"] for capture in captures] == expected
    metrics = []
    for capture in captures:
        for suffix, color in zip("ab", capture["colors"]):
            path = root / f"{capture['sequence']}-{suffix}.png"
            counts, _ = classify(path)
            if color == "none":
                assert sum(counts.values()) < 20, (path, "evicted bank remains", counts)
            else:
                assert counts[color] > 800, (path, "resident bank missing", counts)
                assert counts["blue"] + counts["magenta"] < counts[color] * 0.02 + 20, (path, counts)
            metrics.append({"stage": capture["name"], "view": suffix, "pixels": counts})
    result = json.loads((root / "result.json").read_text())
    coarse = [capture for capture in captures if capture["name"].startswith("coarse-")]
    assert len(result["transitions"]) == len(coarse) == 2
    for capture, transition in zip(coarse, result["transitions"]):
        assert transition["sequence"] == capture["sequence"]
        assert transition["allocations"]["coarse_points"] == 12_288
        assert transition["allocations"]["coarse_positions"] == 12_288 * 12
        assert transition["allocations"]["coarse_attributes"] == 0
        assert all(view["coarse"] and view["selected"] == 12_288 for view in transition["lod"])
    for full in result["stages"]:
        for view in full["lod"]:
            if view["represented"]:
                verify_fine(view)
    mixed_captures = [capture for capture in captures if capture["name"].startswith("divergent-")]
    assert len(result["divergent"]) == len(mixed_captures) == 2
    for capture, mixed in zip(mixed_captures, result["divergent"]):
        assert mixed["sequence"] == capture["sequence"]
        shares = []
        banks = (0, 1) if capture["name"] == "divergent-left-right" else (1, 0)
        for view, bank in zip(mixed["lod"], banks):
            assert view["source_page_bounds"] == [bank * 48, (bank + 1) * 48 - 1], view
            assert view["pages"] == 48, view
            assert view["coarse"] is False and view["preparing"] is False, view
            assert 0 < view["full_pages"] < 48, view
            assert view["full_pages"] + view["coarse_pages"] == 48, view
            assert view["valid"] == 6_291_456, view
            assert view["represented"] == view["culled"] == 3_145_728, view
            assert 12_288 < view["selected"] <= 1_000_000, view
            assert view["verified_ordinals"] is True, view
            shares.append(view["full_pages"])
        assert abs(shares[0] - shares[1]) <= 1, shares
        assert mixed["allocations"]["pages"] == sum(shares) + mixed["allocations"]["coarse_pages"], mixed
        assert mixed["allocations"]["positions"] == sum(shares) * 65_536 * 12 + mixed["allocations"]["coarse_positions"], mixed
        assert mixed["allocations"]["attributes"] == 0 and mixed["allocations"]["coarse_attributes"] == 0, mixed
        assert mixed["allocations"]["coarse_points"] == (96 - sum(shares)) * 256, mixed
        assert mixed["allocations"]["gpu_bytes"] <= result["limits"]["gpu_bytes"], mixed
    report = {"passed": "24 native residency, transition and divergent-view readbacks", "metrics": metrics}
    (root / "image-metrics.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    if sys.argv[1] == "--self-test":
        self_test()
    else:
        verify(Path(sys.argv[1]))
