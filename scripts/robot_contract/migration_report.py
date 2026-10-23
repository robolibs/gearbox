"""Compare the compiled manifests of one machine before and after migration.

    python3 migration_report.py BEFORE.json AFTER.json REPORT.json

Reports acceptance, profile, every changed body/joint/DOF/frame record by
path, controller and owner changes, the control frame in world terms, and
the diagnostics that appeared or went away.
"""

import json
import math
import sys


def keyed(records, key="path"):
    return {r[key]: r for r in records}


def changes(before, after):
    out = []
    for path in sorted(set(before) | set(after)):
        a, b = before.get(path), after.get(path)
        if a != b:
            fields = sorted(k for k in set(a or {}) | set(b or {}) if (a or {}).get(k) != (b or {}).get(k))
            out.append({"path": path, "before": None if a is None else {k: a.get(k) for k in fields}, "after": None if b is None else {k: b.get(k) for k in fields}})
    return out


def rotate(q, v):
    w, x, y, z = q
    tx, ty, tz = 2 * (y * v[2] - z * v[1]), 2 * (z * v[0] - x * v[2]), 2 * (x * v[1] - y * v[0])
    return [v[0] + w * tx + (y * tz - z * ty), v[1] + w * ty + (z * tx - x * tz), v[2] + w * tz + (x * ty - y * tx)]


def mul(a, b):
    w1, x1, y1, z1 = a
    w2, x2, y2, z2 = b
    return [w1 * w2 - x1 * x2 - y1 * y2 - z1 * z2, w1 * x2 + x1 * w2 + y1 * z2 - z1 * y2, w1 * y2 - x1 * z2 + y1 * w2 + z1 * x2, w1 * z2 + x1 * y2 - y1 * x2 + z1 * w2]


def control_axes(m):
    """World forward and left of the control frame at load; None without one."""
    if not m.get("control_frame") or not m.get("control_body"):
        return None
    frame = next((f for f in m["frames"] if f["name"] == m["control_frame"]["frame"] or f.get("role") == "control"), None)
    if frame is None:
        return None
    body = keyed(m["bodies"])[m["control_body"]]
    q = mul(body["initial_pose"]["rotation_wxyz"], frame["transform"]["rotation_wxyz"])
    return {"forward": [round(c, 9) for c in rotate(q, [1, 0, 0])], "left": [round(c, 9) for c in rotate(q, [0, 1, 0])], "source": m["control_frame"]["source"]}


def diagnostics(m):
    return {f"{d['severity']}:{d['code']}:{d.get('path')}" for d in m["diagnostics"]}


def main():
    before_doc, after_doc = (json.load(open(p)) for p in sys.argv[1:3])
    before, after = before_doc["machines"][0], after_doc["machines"][0]
    gone, new = diagnostics(before) - diagnostics(after), diagnostics(after) - diagnostics(before)
    axes_before, axes_after = control_axes(before), control_axes(after)
    same_axes = bool(axes_before and axes_after) and all(
        math.isclose(a, b, abs_tol=1e-6) for k in ("forward", "left") for a, b in zip(axes_before[k], axes_after[k])
    )
    report = {
        "root": after["root"],
        "profile": [before["profile"], after["profile"]],
        "accepted": [before["accepted"], after["accepted"]],
        "counts": {k: [len(before[k]), len(after[k])] for k in ("bodies", "joints", "dofs", "frames", "controllers")},
        "bodies": changes(keyed(before["bodies"]), keyed(after["bodies"])),
        "joints": changes(keyed(before["joints"]), keyed(after["joints"])),
        "dofs": changes(keyed(before["dofs"], "name"), keyed(after["dofs"], "name")),
        "frames": changes(keyed(before["frames"]), keyed(after["frames"])),
        "controllers": changes(keyed(before["controllers"], "instance"), keyed(after["controllers"], "instance")),
        "owners": before.get("owners") == after.get("owners"),
        "interfaces_changed": [
            {"joint": a["joint"], "dof": a["name"], "before": a["interfaces"]["value"], "after": b["interfaces"]["value"]}
            for a, b in zip(before["dofs"], after["dofs"])
            if a["interfaces"]["value"] != b["interfaces"]["value"]
        ],
        "control_frame": {"before": axes_before, "after": axes_after, "same_world_axes": same_axes},
        "diagnostics_new": sorted(new),
        "diagnostics_gone": sorted(gone),
        "errors_after": sorted(d for d in diagnostics(after) if d.startswith("error:")),
    }
    with open(sys.argv[3], "w") as f:
        json.dump(report, f, indent=1, sort_keys=True)
    print(
        f"[migration] {report['root']}: accepted {report['accepted']}, counts {report['counts']}, "
        f"control axes kept {same_axes}, {len(report['interfaces_changed'])} DOFs change interfaces, "
        f"{len(new)} new / {len(gone)} gone diagnostics, {len(report['errors_after'])} errors"
    )


if __name__ == "__main__":
    main()
