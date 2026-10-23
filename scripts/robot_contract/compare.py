"""Compare a native Gearbox conformance run with an Isaac probe run.

    python3 compare.py --schedule S.json --native DIR --isaac DIR --out report.json

Structure (E01), command targets (E02) and actual motion are reported as
separate classes; tolerances come from the schedule and are never widened
here. Exit code 0 only when every required check passes.
"""

import argparse
import json
import math
import os
import sys


def load(path):
    with open(path) as f:
        return json.load(f)


def lines(path):
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def rotate(q, v):
    w, x, y, z = q
    tx, ty, tz = 2 * (y * v[2] - z * v[1]), 2 * (z * v[0] - x * v[2]), 2 * (x * v[1] - y * v[0])
    return [v[0] + w * tx + (y * tz - z * ty), v[1] + w * ty + (z * tx - x * tz), v[2] + w * tz + (x * ty - y * tx)]


class Report:
    def __init__(self):
        self.checks = []

    def check(self, cls, name, ok, **detail):
        self.checks.append({"class": cls, "check": name, "ok": bool(ok), **detail})
        return ok

    def scalar(self, cls, name, native, isaac, tol, relative=False):
        if native is None or isaac is None:
            return self.check(cls, name, native is None and isaac is None, native=native, isaac=isaac)
        diff = abs(native - isaac)
        bound = tol * max(abs(native), abs(isaac), 1e-12) if relative else tol
        return self.check(cls, name, diff <= bound, native=native, isaac=isaac, diff=diff, tolerance=bound)

    def vector(self, cls, name, native, isaac, tol):
        diff = max(abs(a - b) for a, b in zip(native, isaac))
        return self.check(cls, name, diff <= tol, native=native, isaac=isaac, diff=diff, tolerance=tol)

    def passed(self, cls):
        selected = [c for c in self.checks if c["class"] == cls]
        return bool(selected) and all(c["ok"] for c in selected)


def compare_structure(r, schedule, native, isaac, tol):
    machine = native["manifest"]
    usd, runtime = isaac["structure"]["usd"], isaac["structure"]["runtime"]
    length, rel = tol["structure_length_m"], tol["structure_scalar_relative"]
    r.check("structure", "overlay authors no physics", not isaac["overlay"]["mechanical_changes"], changes=isaac["overlay"]["mechanical_changes"])
    r.check("structure", "machine accepted", machine["accepted"], diagnostics=machine["diagnostics"])

    isaac_bodies = {b["path"]: b for b in usd["bodies"]}
    masses = dict(zip(runtime["link_names"], runtime["link_masses"]))
    inertias = dict(zip(runtime["link_names"], runtime["link_inertias"]))
    r.check("structure", "same body set", sorted(b["path"] for b in machine["bodies"]) == sorted(isaac_bodies), native=sorted(b["path"] for b in machine["bodies"]), isaac=sorted(isaac_bodies))
    for body in machine["bodies"]:
        other = isaac_bodies.get(body["path"])
        if other is None:
            continue
        name = body["path"].rsplit("/", 1)[-1]
        mass = body["mass"]["value"] if body["mass"] else None
        r.scalar("structure", f"{name} mass", mass, masses.get(name), rel, relative=True)
        if body["diagonal_inertia"] and name in inertias:
            diag = [inertias[name][0], inertias[name][4], inertias[name][8]]
            for axis, (a, b) in zip("xyz", zip(body["diagonal_inertia"]["value"], diag)):
                r.scalar("structure", f"{name} inertia {axis}", a, b, rel, relative=True)
        fallbacks = isaac["structure"].get("schema_fallbacks", {})
        for field, key in (("linear_damping", "physxRigidBody:linearDamping"), ("angular_damping", "physxRigidBody:angularDamping")):
            value = body.get(field)
            if value and value["source"] == "schema_default":
                r.scalar("structure", f"{name} {field} (schema fallback)", value["value"], fallbacks.get(f"PhysxRigidBodyAPI.{key}"), 1e-6)
        r.vector("structure", f"{name} initial position", body["initial_pose"]["translation"], other["initial_pose"]["translation"], length)
        r.vector("structure", f"{name} initial rotation", body["initial_pose"]["rotation_wxyz"], other["initial_pose"]["rotation_wxyz"], tol["structure_angle_rad"])

    isaac_joints = {j["path"]: j for j in usd["joints"]}
    kinds = {"PhysicsRevoluteJoint": "revolute", "PhysicsPrismaticJoint": "prismatic", "PhysicsFixedJoint": "fixed", "PhysicsSphericalJoint": "spherical", "PhysicsDistanceJoint": "distance", "PhysicsJoint": "d6"}
    for joint in machine["joints"]:
        other = isaac_joints.get(joint["path"])
        if not r.check("structure", f"{joint['name']} present in Isaac", other is not None):
            continue
        r.check("structure", f"{joint['name']} type", kinds.get(other["type"]) == joint["kind"], native=joint["kind"], isaac=other["type"])
        r.check("structure", f"{joint['name']} axis", joint["axis"] == other["axis"], native=joint["axis"], isaac=other["axis"])
        for side in ("body0", "body1"):
            resolved = joint[side]["resolved"]
            want = [] if resolved["kind"] == "world" else [resolved.get("body")]
            r.check("structure", f"{joint['name']} {side}", want == other[side], native=resolved, isaac=other[side])
        r.vector("structure", f"{joint['name']} local position 0", joint["local0"]["translation"], other["local_pos0"], length)
        r.vector("structure", f"{joint['name']} local position 1", joint["local1"]["translation"], other["local_pos1"], length)

    dof_index = {path[0] if isinstance(path, list) else path: i for i, path in enumerate(runtime["dof_paths"][0] if runtime["dof_paths"] and isinstance(runtime["dof_paths"][0], list) and len(runtime["dof_paths"]) == 1 else runtime["dof_paths"] or [])}
    if not dof_index:
        dof_index = {f"/{n}": i for i, n in enumerate(runtime["dof_names"])}
    r.check("structure", "same DOF names", sorted(d["name"] for d in machine["dofs"]) == sorted(runtime["dof_names"]), native=[d["name"] for d in machine["dofs"]], isaac=runtime["dof_names"])
    for dof in machine["dofs"]:
        i = dof_index.get(dof["joint"], runtime["dof_names"].index(dof["name"]) if dof["name"] in runtime["dof_names"] else None)
        if not r.check("structure", f"{dof['name']} reported by Isaac", i is not None):
            continue
        drive = dof["drive"]
        if drive:
            r.check("structure", f"{dof['name']} drive type", drive["type"]["value"] == runtime["drive_types"][i], native=drive["type"]["value"], isaac=runtime["drive_types"][i])
            r.scalar("structure", f"{dof['name']} stiffness (SI)", drive["stiffness"]["value"], runtime["stiffness"][i], rel, relative=True)
            r.scalar("structure", f"{dof['name']} damping (SI)", drive["damping"]["value"], runtime["damping"][i], rel, relative=True)
            r.scalar("structure", f"{dof['name']} max force (SI)", drive["max_force"]["value"], runtime["max_effort"][i], rel, relative=True)
        r.scalar("structure", f"{dof['name']} lower limit", dof["limit"]["lower"], runtime["limits"][0][i], length)
        r.scalar("structure", f"{dof['name']} upper limit", dof["limit"]["upper"], runtime["limits"][1][i], length)

    sites = {s["path"]: s for s in usd["sites"]}
    poses = {b["path"]: b["initial_pose"] for b in machine["bodies"]}
    for frame in machine["frames"]:
        owner = frame.get("owner", {}).get("body")
        if frame["path"] not in sites or owner is None or frame["path"] in poses:
            continue
        body = poses[owner]
        world = [a + b for a, b in zip(body["translation"], rotate(body["rotation_wxyz"], frame["transform"]["translation"]))]
        r.vector("structure", f"site {frame['name']} initial position", world, sites[frame["path"]]["initial_pose"]["translation"], length)

    gravity = native["run"].get("gravity") or {}
    isaac_gravity = isaac["structure"].get("gravity") or {}
    r.scalar("structure", "gravity magnitude", gravity.get("magnitude"), isaac_gravity.get("magnitude"), rel, relative=True)
    r.scalar("structure", "physics dt", 1.0 / native["run"]["physics_hz"], runtime["physics_dt"], 1e-9)


def active_commands(schedule):
    """Per DOF, the schedule command in force after each tick; a reset frees every DOF."""
    by_tick = {}
    for c in schedule["commands"]:
        by_tick.setdefault(c["tick"], []).append(c)
    resets = {e["tick"] for e in schedule.get("events", []) if e["event"] == "reset"}
    state, timeline = {}, {}
    for tick in range(schedule["ticks"]):
        if tick in resets:
            state = {dof: {"tick": tick, "dof": dof, "mode": "uncommanded"} for dof in state}
        for c in by_tick.get(tick, []):
            state[c["dof"]] = c
        timeline[tick + 1] = dict(state)
    return timeline


def series(trace, dof):
    return {row["tick"]: row for row in trace if row.get("dof") == dof}


def frames(trace, name):
    return {row["tick"]: row["translation"] for row in trace if row.get("frame") == name}


def compare_targets(r, schedule, native, isaac, tol):
    timeline = active_commands(schedule)
    dofs = sorted({c["dof"] for c in schedule["commands"]})
    for dof in dofs:
        a, b = series(native["trace"], dof), series(isaac["trace"], dof)
        r.check("targets", f"{dof} sampled every tick", len(a) == len(b) == schedule["ticks"], native=len(a), isaac=len(b))
        mode_mismatch, target_mismatch, capture = [], [], []
        for tick in range(1, schedule["ticks"] + 1):
            command = timeline[tick].get(dof)
            ra, rb = a.get(tick), b.get(tick)
            if ra is None or rb is None:
                continue
            if ra["mode"] != rb["mode"]:
                mode_mismatch.append({"tick": tick, "native": ra["mode"], "isaac": rb["mode"]})
                continue
            if command and command["mode"] == "hold":
                q_entry_a, q_entry_b = a.get(command["tick"], {}).get("q", 0.0), b.get(command["tick"], {}).get("q", 0.0)
                capture.append(abs(ra["target"] - q_entry_a) <= tol["target_absolute"] + 1e-12 and abs(rb["target"] - q_entry_b) <= tol["target_absolute"] + 1e-12)
                continue
            ta, tb = ra["target"], rb["target"]
            if (ta is None) != (tb is None) or (ta is not None and abs(ta - tb) > tol["target_absolute"]):
                target_mismatch.append({"tick": tick, "native": ta, "isaac": tb})
        r.check("targets", f"{dof} mode per tick", not mode_mismatch, first=mode_mismatch[:5], count=len(mode_mismatch))
        r.check("targets", f"{dof} target per tick", not target_mismatch, first=target_mismatch[:5], count=len(target_mismatch))
        if capture:
            r.check("targets", f"{dof} hold captures its own coordinate at entry", all(capture), ticks=len(capture))


def phase_ends(schedule):
    ticks = sorted({c["tick"] for c in schedule["commands"]} | {e["tick"] for e in schedule.get("events", [])}) + [schedule["ticks"]]
    return list(zip(ticks, ticks[1:]))


def compare_lifecycle(r, schedule, native, isaac, tol):
    """Pause preserves state and advances no tick; reset frees every DOF."""
    dt = 1.0 / schedule["physics_hz"]
    for event in schedule.get("events", []):
        tick = event["tick"]
        for side, run in (("native", native), ("isaac", isaac)):
            row = next((v for v in run["trace"] if v.get("event") == event["event"] and v.get("tick") == tick), None)
            if not r.check("lifecycle", f"{side} records {event['event']} at tick {tick}", row is not None):
                continue
            if event["event"] == "pause":
                r.check("lifecycle", f"{side} pause advances no physics tick", row["ticks_advanced"] == 0, ticks=row["ticks_advanced"])
                r.check("lifecycle", f"{side} pause moves nothing", row["q_before"] == row["q_after"], before=row["q_before"], after=row["q_after"])
                for dof in sorted({c["dof"] for c in schedule["commands"]}):
                    s = series(run["trace"], dof)
                    before = (s[tick]["q"] - s[tick - 3]["q"]) / (3 * dt)
                    after = (s[tick + 4]["q"] - s[tick + 1]["q"]) / (3 * dt)
                    bound = tol["velocity_relative"] * max(abs(before), 1e-3)
                    r.check("lifecycle", f"{side} {dof} resumes at its speed", abs(after - before) <= bound, before=before, after=after, tolerance=bound)
            elif event["event"] == "reset" and "reset_return_rad_or_m" in tol:
                end = next((t for t, _ in phase_ends(schedule) if t > tick), schedule["ticks"])
                for dof, target in sorted(schedule.get("load_time_targets", {}).items()):
                    q = series(run["trace"], dof)[end]["q"]
                    bound = tol["reset_return_rad_or_m"]
                    r.check("lifecycle", f"{side} reset returns {dof} to its load-time target", abs(q - target) <= bound, q=q, target=target, tolerance=bound)


def compare_motion(r, schedule, native, isaac, tol):
    timeline = active_commands(schedule)
    dt = 1.0 / schedule["physics_hz"]
    settle = tol["position_settle_rad_or_m"]
    dofs = sorted({c["dof"] for c in schedule["commands"]})
    for dof in dofs:
        a, b = series(native["trace"], dof), series(isaac["trace"], dof)
        for start, end in phase_ends(schedule):
            command = timeline[start + 1].get(dof)
            if command is None or end - start < 4:
                continue
            mode = command["mode"]
            label = f"{dof} {mode} [{start},{end})"
            qa = lambda t: a[t]["q"]
            qb = lambda t: b[t]["q"]
            if mode == "position":
                r.scalar("motion", f"{label} settles at the same coordinate", qa(end), qb(end), settle)
            elif mode == "velocity":
                window = max(start + 1, end - (end - start) // 3)
                ra, rb = (qa(end) - qa(window)) / ((end - window) * dt), (qb(end) - qb(window)) / ((end - window) * dt)
                r.scalar("motion", f"{label} rate", ra, rb, tol["velocity_relative"], relative=True)
            elif mode == "effort":
                n = min(30, (end - start) // 2)
                acc = lambda q: (q(start + 1 + 2 * n) - 2 * q(start + 1 + n) + q(start + 1)) / ((n * dt) ** 2)
                r.scalar("motion", f"{label} early acceleration", acc(qa), acc(qb), tol["effort_acceleration_relative"], relative=True)
            elif mode in ("free", "uncommanded"):
                early = min(start + 30, end)
                r.scalar("motion", f"{label} at tick {early}", qa(early), qb(early), settle)
                r.scalar("motion", f"{label} ends at the same coordinate", qa(end), qb(end), settle)
            elif mode == "hold":
                for side, q, rows in (("native", qa, a), ("isaac", qb, b)):
                    drift = abs(q(end - 1) - rows[start + 1]["target"])
                    r.check("motion", f"{label} {side} holds its captured target", drift <= tol["hold_drift_rad_or_m"], drift=drift, tolerance=tol["hold_drift_rad_or_m"])
    tool = schedule.get("tool")
    if tool:
        name = tool.rsplit("/", 1)[-1]
        fa, fb = frames(native["trace"], name), frames(isaac["trace"], name)
        r.check("motion", f"{name} sampled every tick", len(fa) == len(fb) == schedule["ticks"], native=len(fa), isaac=len(fb))
        for start, end in phase_ends(schedule):
            command = timeline[start + 1]
            if all(c["mode"] in ("position", "hold") for c in command.values()) and end in fa and end in fb:
                r.vector("motion", f"{name} position at tick {end}", fa[end], fb[end], settle)


def fit_gains(q, dt, start, end, target, target_velocity):
    see = sev = svv = sae = sav = 0.0
    for n in range(max(start, 1), min(end, len(q) - 1)):
        a = (q[n + 1] - 2 * q[n] + q[n - 1]) / (dt * dt)
        v = target_velocity - (q[n + 1] - q[n - 1]) / (2 * dt)
        e = 0.0 if target is None else target - q[n]
        see, sev, svv, sae, sav = see + e * e, sev + e * v, svv + v * v, sae + a * e, sav + a * v
    if target is None:
        return 0.0, sav / svv
    det = see * svv - sev * sev
    return (sae * svv - sav * sev) / det, (sav * see - sae * sev) / det


def compare_gains(r, schedule, native, isaac):
    dt = 1.0 / schedule["physics_hz"]
    expected, rel = schedule["expected"], schedule["tolerances"]["gain_relative"]
    position = next(c for c in schedule["commands"] if c["mode"] == "position")
    velocity = next(c for c in schedule["commands"] if c["mode"] == "velocity")
    (s0, s1), (d0, d1) = schedule["measure"]["stiffness_window"], schedule["measure"]["damping_window"]
    for side, run in (("native", native), ("isaac", isaac)):
        rows = series(run["trace"], position["dof"])
        q = [0.0] + [rows[t]["q"] for t in sorted(rows)]
        k, _ = fit_gains(q, dt, s0 + 2, s1, position["value"], 0.0)
        _, c = fit_gains(q, dt, d0 + 2, d1, None, velocity["value"])
        r.scalar("structure", f"{side} measured stiffness per rad", k, expected["stiffness_per_rad"], rel, relative=True)
        r.scalar("structure", f"{side} measured damping per rad", c, expected["damping_per_rad"], rel, relative=True)
    runtime = isaac["structure"]["runtime"]
    r.scalar("structure", "Isaac-reported stiffness per rad", runtime["stiffness"][0], expected["stiffness_per_rad"], 1e-5, relative=True)
    r.scalar("structure", "Isaac-reported damping per rad", runtime["damping"][0], expected["damping_per_rad"], 1e-5, relative=True)


def compare_mapping(r, schedule, native, isaac):
    tol = schedule["tolerances"]
    isaac_cases = {c["name"]: c for c in isaac["cases"]}
    for case in native["cases"]:
        other = isaac_cases.get(case["name"])
        if not r.check("mapping", f"{case['name']} evaluated by Isaac", other is not None):
            continue
        for k in ("linear_mps", "angular_rps"):
            r.scalar("mapping", f"{case['name']} native applied {k} equals requested", case["applied"][k], case["requested"][k], 1e-5)
        for role, dof in case["dofs"].items():
            theirs = other["dofs"].get(role)
            if not r.check("mapping", f"{case['name']} {role} emitted by Isaac", theirs is not None):
                continue
            r.check("mapping", f"{case['name']} {role} mode", dof["mode"] == theirs["mode"], native=dof["mode"], isaac=theirs["mode"])
            if dof["mode"] == "position":
                r.scalar("mapping", f"{case['name']} {role} target", dof["target"], theirs["target"], tol["steer_angle_rad"])
            else:
                r.scalar("mapping", f"{case['name']} {role} target", dof["target"], theirs["target"], tol["wheel_velocity_relative"], relative=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--schedule", required=True)
    parser.add_argument("--native", required=True)
    parser.add_argument("--isaac", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    schedule = load(args.schedule)
    report = Report()
    status = load(os.path.join(args.isaac, "status.json")) if os.path.isfile(os.path.join(args.isaac, "status.json")) else {"result": "missing"}
    runtime_ok = report.check("runtime", "Isaac probe completed", status.get("result") == "completed", status=status)
    if schedule.get("mapping"):
        native_file, isaac_file = os.path.join(args.native, "mapping.json"), os.path.join(args.isaac, "mapping.json")
        present = report.check("runtime", "both mappings present", os.path.isfile(native_file) and os.path.isfile(isaac_file), files=[native_file, isaac_file])
        if runtime_ok and present:
            compare_mapping(report, schedule, load(native_file), load(isaac_file))
        native_ok = False
    else:
        native_files = [os.path.join(args.native, f) for f in ("manifest.json", "run.json", "trace.jsonl")]
        native_ok = report.check("runtime", "native run present", all(os.path.isfile(f) for f in native_files), files=native_files)
    if runtime_ok and native_ok:
        native = {"manifest": load(native_files[0]), "run": load(native_files[1]), "trace": lines(native_files[2])}
        isaac = {
            "structure": load(os.path.join(args.isaac, "structure.json")),
            "overlay": load(os.path.join(args.isaac, "overlay.json")),
            "trace": lines(os.path.join(args.isaac, "trace.jsonl")),
        }
        report.check("runtime", "Isaac trace nonempty", len(isaac["trace"]) > 0, rows=len(isaac["trace"]))
        report.check("runtime", "native trace nonempty", len(native["trace"]) > 0, rows=len(native["trace"]))
        if "expected" in schedule:
            compare_gains(report, schedule, native, isaac)
        else:
            compare_structure(report, schedule, native, isaac, schedule["tolerances"])
            compare_targets(report, schedule, native, isaac, schedule["tolerances"])
            compare_motion(report, schedule, native, isaac, schedule["tolerances"])
            compare_lifecycle(report, schedule, native, isaac, schedule["tolerances"])
    classes = sorted({c["class"] for c in report.checks})
    summary = {cls: {"passed": report.passed(cls), "checks": sum(1 for c in report.checks if c["class"] == cls), "failed": [c["check"] for c in report.checks if c["class"] == cls and not c["ok"]]} for cls in classes}
    result = {"schedule": os.path.basename(args.schedule), "summary": summary, "checks": report.checks, "passed": all(s["passed"] for s in summary.values())}
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(result, f, indent=1, sort_keys=True)
    for cls, s in summary.items():
        print(f"[robot-contract] {result['schedule']} {cls}: {'pass' if s['passed'] else 'FAIL'} ({s['checks']} checks){'' if s['passed'] else ' failed: ' + '; '.join(s['failed'])}")
    sys.exit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
