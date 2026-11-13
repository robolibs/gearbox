"""Native distance-field lab checks: `check DIR` judges the run and writes ACCEPTANCE.md."""

import json
import pathlib
import sys

from PIL import Image

UNKNOWN = (96, 40, 112)
DARKER = tuple(int(c * 0.55) for c in UNKNOWN)


def state(text):
    """`(state, source samples, generation)` from a runner visibility entry."""
    parts = text.split()
    return parts[0], int(parts[1]) if len(parts) > 1 else 0, int(parts[2]) if len(parts) > 2 else 0


def sample(answer):
    return answer["samples"][0]


def near(value, expected, tolerance):
    return value is not None and abs(value - expected) <= tolerance


def checker_pixels(path):
    raw = Image.open(path).convert("RGB").tobytes()
    tones = {UNKNOWN: 0, DARKER: 0}
    for pixel in (raw[i:i + 3] for i in range(0, len(raw), 3)):
        for tone in tones:
            if all(abs(pixel[k] - tone[k]) <= 10 for k in range(3)):
                tones[tone] += 1
    return tones


def check(run):
    lab = json.loads((run / "lab.json").read_text())
    probes = {name: sample(answer) for name, answer in lab["probes"].items()}
    cli = sample(json.loads((run / "field_cli.json").read_text()))
    visible = json.loads((run / "visible.json").read_text())
    tones = checker_pixels(run / "fields.png")
    errors = (run / "renderer_errors.txt").read_text().strip()
    terrain = json.loads((run / "terrain.json").read_text())
    shown = {k: state(v) for k, v in json.loads((run / "terrain_visible.json").read_text()).items()}
    switched = json.loads((run / "layer_reply.json").read_text())
    redrawn = {k: state(v) for k, v in json.loads((run / "terrain_after.json").read_text()).items()}
    gradient = probes["esdf_clear_of_box"]["gradient"]
    cases = [
        ("TSDF zero crossing reads zero at the surface", near(probes["tsdf_surface"]["value"], 0.0, 0.02), probes["tsdf_surface"]),
        ("TSDF just inside the surface reads its depth", near(probes["tsdf_just_inside"]["value"], -0.15, 0.02), probes["tsdf_just_inside"]),
        ("TSDF free space reads the truncation", near(probes["tsdf_free"]["value"], 0.3, 1e-3), probes["tsdf_free"]),
        ("TSDF unseen centre is unknown, not a distance", probes["tsdf_centre_unseen"]["value"] is None, probes["tsdf_centre_unseen"]),
        ("TSDF shadow is unknown", probes["tsdf_shadow"]["value"] is None, probes["tsdf_shadow"]),
        ("ESDF clearance beside the box", near(probes["esdf_clear_of_box"]["value"], 0.6, 0.01), probes["esdf_clear_of_box"]),
        ("ESDF gradient points away from the box", gradient is not None and near(gradient[0], 1.0, 0.05), gradient),
        ("ESDF unobserved space is unknown", probes["esdf_unobserved"]["support"] == "unknown", probes["esdf_unobserved"]),
        ("ESDF never-sent block is unallocated", probes["esdf_never_sent"]["support"] == "unallocated", probes["esdf_never_sent"]),
        ("CLI field query matches the client probe", near(cli["value"], probes["esdf_clear_of_box"]["value"], 1e-9), cli),
        ("every lab dataset became visible", all(state == "visible" for state in visible.values()), visible),
        ("unknown checker drawn in both tones", min(tones.values()) >= 200, {"%d,%d,%d" % k: v for k, v in tones.items()}),
        ("every terrain panel and the rolling map became visible", all(s[0] == "visible" for s in shown.values()), shown),
        ("the rolling map stores exactly a fresh map's samples", shown["rolling"][1] == terrain["rolling"]["fresh_samples"],
         [shown["rolling"][1], terrain["rolling"]["fresh_samples"]]),
        ("rolling shifts sent and removed tiles", terrain["rolling"]["sent_tiles"] > 0 and terrain["rolling"]["removed_tiles"] > 0, terrain["rolling"]),
        ("CLI layer switch is a new generation, drawn", switched["generation"] == 2 and redrawn["elevation"][:1] == ("visible",) and redrawn["elevation"][2] == 2,
         [switched, redrawn["elevation"]]),
        ("renderer log clean", not errors, errors[:400]),
    ]
    lines = ["# Distance-field lab (native)", "", "| Case | State | Measured |", "|---|---|---|"]
    for name, ok, measured in cases:
        lines.append(f"| {name} | {'PASS' if ok else 'FAIL'} | `{json.dumps(measured)}` |")
    (run / "ACCEPTANCE.md").write_text("\n".join(lines) + "\n")
    failed = [name for name, ok, _ in cases if not ok]
    print(json.dumps({"cases": len(cases), "failed": failed}))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(check(pathlib.Path(sys.argv[2])))
