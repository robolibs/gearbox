"""Write the run manifest of one robot-contract conformance run.

    python3 evidence.py --run DIR --schedules NAME... --launcher PATH --isaac-root DIR

Records source and dependency revisions, executable and input hashes,
the Isaac runtime, per-schedule settings and results, and evidence paths.
"""

import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys

FIXTURES = "tests/fixtures/robot_contract"


def sha256(path):
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return None


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def git(*args):
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def newest(pattern):
    paths = [p for p in glob.glob(pattern) if os.path.isfile(p) and os.access(p, os.X_OK) and not p.endswith(".d")]
    return max(paths, key=os.path.getmtime) if paths else None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", required=True)
    parser.add_argument("--schedules", nargs="+", required=True)
    parser.add_argument("--launcher", required=True)
    parser.add_argument("--isaac-root", required=True)
    args = parser.parse_args()

    test_binary = newest("target/debug/deps/gearbox-*")
    schedules = {}
    for name in args.schedules:
        path = os.path.join(FIXTURES, "schedules", name + ".json")
        schedule = load(path) or {}
        inputs = {os.path.basename(path): sha256(path)}
        for key in ("fixture", "overlay"):
            if schedule.get(key):
                inputs[schedule[key]] = sha256(os.path.join(FIXTURES, schedule[key]))
        for case in schedule.get("cases", []):
            inputs[case["fixture"]] = sha256(os.path.join(FIXTURES, case["fixture"]))
        isaac = os.path.join(args.run, "isaac", name)
        structure = load(os.path.join(isaac, "structure.json")) or {}
        runtime = structure.get("runtime", {})
        report = load(os.path.join(args.run, f"compare-{name}.json")) or {}
        schedules[name] = {
            "inputs_sha256": inputs,
            "physics_hz": schedule.get("physics_hz"),
            "ticks": schedule.get("ticks") or schedule.get("settle_ticks"),
            "tolerances": schedule.get("tolerances"),
            "isaac_status": load(os.path.join(isaac, "status.json")),
            "isaac_engine": runtime.get("engine"),
            "isaac_physics_dt": runtime.get("physics_dt"),
            "gravity": structure.get("gravity"),
            "initial_dof_positions": runtime.get("dof_positions"),
            "comparison": report.get("summary"),
            "passed": report.get("passed"),
            "evidence": sorted(os.path.relpath(p, args.run) for p in glob.glob(os.path.join(args.run, "**", "*"), recursive=True) if name in p and os.path.isfile(p)),
        }
    preflight = next((load(p) for p in sorted(glob.glob(os.path.join(args.run, "isaac", "*", "preflight.json")))), None)
    manifest = {
        "plan": "plans/001-shared-robot-and-control-contract.md",
        "source": {
            "commit": git("rev-parse", "HEAD"),
            "dirty_paths": len((git("status", "--porcelain") or "").splitlines()),
            "machines_submodule": git("-C", "assets/machines", "rev-parse", "HEAD"),
            "cargo_lock_sha256": sha256("Cargo.lock"),
        },
        "executables": {
            "native_test_binary": test_binary,
            "native_test_binary_sha256": sha256(test_binary) if test_binary else None,
            "isaac_launcher": args.launcher,
            "isaac_launcher_sha256": sha256(args.launcher),
            "isaac_probe_sha256": sha256("scripts/robot_contract/isaac_probe.py"),
            "compare_sha256": sha256("scripts/robot_contract/compare.py"),
        },
        "isaac": {
            "root": args.isaac_root,
            "version_file": (open(os.path.join(args.isaac_root, "VERSION")).read().strip() if os.path.isfile(os.path.join(args.isaac_root, "VERSION")) else None),
            "preflight": preflight,
        },
        "commands": {
            "native": "cargo test -p gearbox-sim --bin gearbox -- <conformance filters> with GEARBOX_CONFORMANCE_OUT=<run>/native",
            "isaac": "PROBE_SCHEDULE=<schedule> PROBE_OUT=<run>/isaac/<name> timeout --kill-after=15 <limit> isaacsim --no-window --no-ros-env --exec scripts/robot_contract/isaac_probe.py",
            "compare": "python3 scripts/robot_contract/compare.py --schedule <schedule> --native <run>/native/<name> --isaac <run>/isaac/<name> --out <run>/compare-<name>.json",
        },
        "gates": {"E01": "e02_arm structure", "E02": "e02_arm targets and motion", "lifecycle": "e02_lifecycle pause and reset", "E03": "e03_mapping", "drive_units": "e01_accel"},
        "schedules": schedules,
        "passed": all(s["passed"] for s in schedules.values()),
    }
    with open(os.path.join(args.run, "run.json"), "w") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
    print(f"[robot-contract] run manifest {os.path.join(args.run, 'run.json')}: {'pass' if manifest['passed'] else 'FAIL'}")
    sys.exit(0 if manifest["passed"] else 1)


if __name__ == "__main__":
    main()
