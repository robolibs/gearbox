#!/usr/bin/env python3
"""Turn a bench-draw-data run into results.json and ACCEPTANCE.md.

Usage: bench_draw_data.py RUN_DIR PROFILE PROFILES_JSON
"""
import json
import pathlib
import sys


def lines(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def percentile(values, q):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round(q / 100 * (len(ordered) - 1))))
    return ordered[index]


def window_stats(frames, window):
    inside = [f for f in frames if window["start_unix_s"] <= f["unix_s"] <= window["end_unix_s"]]
    times = [f["cpu_ms"] for f in inside]
    lod = [f["lod"] for f in inside if f.get("lod")]
    return {
        "frames": len(inside),
        "seconds": round(window["end_unix_s"] - window["start_unix_s"], 2),
        "cpu_ms_p50": percentile(times, 50),
        "cpu_ms_p95": percentile(times, 95),
        "cpu_ms_p99": percentile(times, 99),
        "cpu_ms_max": max(times) if times else None,
        "gpu_ms": None,
        "gpu_ms_reason": inside[0]["gpu_ms_reason"] if inside else None,
        "selected_max": max((l["selected"] for l in lod), default=None),
        "coarse_frames": sum(1 for l in lod if l.get("coarse") or l.get("coarse_pages", 0) > 0),
        "owned_cpu_max": max((f["owned_cpu_bytes"] for f in inside), default=None),
        "owned_gpu_max": max((f["owned_gpu_bytes"] for f in inside), default=None),
        "rss_max": max((f["rss_bytes"] or 0 for f in inside), default=None),
        "msaa": sorted({f["msaa"] for f in inside if f.get("msaa") is not None}),
        "octree_cells_max": max((f.get("octree_visible_cells", 0) for f in inside), default=None),
    }


def at(frames, unix_s):
    """The first frame logged at or after `unix_s`."""
    return next((f for f in frames if f["unix_s"] >= unix_s), frames[-1] if frames else None)


def upload_between(frames, start, end):
    a, b = at(frames, start), at(frames, end)
    if not a or not b:
        return None
    return b["gpu_uploaded_bytes"] - a["gpu_uploaded_bytes"]


def main():
    run, profile_name, profiles = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
    profile = json.loads(pathlib.Path(profiles).read_text())[profile_name]
    cases = []

    def case(name, state, measured, thresholds=None, evidence=None):
        cases.append({"name": name, "state": state, "measured": measured, "thresholds": thresholds or {}, "evidence": evidence or []})

    for scenario in sorted(p for p in run.iterdir() if p.is_dir()):
        frames = lines(scenario / "frames.jsonl")
        windows = lines(scenario / "window.jsonl")
        events = lines(scenario / "events.jsonl")
        errors = (scenario / "renderer_errors.txt").read_text().strip() if (scenario / "renderer_errors.txt").exists() else ""
        name = scenario.name
        evidence = [str(scenario / f) for f in ("frames.jsonl", "events.jsonl", "viewer.log")]
        case(f"{name}: renderer log clean", "pass" if not errors else "fail", {"errors": errors.splitlines()[:5]}, evidence=[str(scenario / "viewer.log")])
        failed_run = scenario / "scenario_failed.json"
        case(f"{name}: scenario completed", "fail" if failed_run.exists() else "pass",
             json.loads(failed_run.read_text()) if failed_run.exists() else {}, evidence=[str(scenario / "events.jsonl")])
        stats = [window_stats(frames, w) for w in windows]
        expected_msaa = profile["msaa_samples"]
        for index, s in enumerate(stats):
            label = f"{name}: window {index + 1}" if len(stats) > 1 else name
            settings_ok = s["msaa"] == [expected_msaa]
            case(f"{label}: profile settings applied", "pass" if settings_ok else "fail", {"msaa": s["msaa"]}, {"msaa": expected_msaa}, evidence)
            budget_ok = (s["selected_max"] or 0) <= profile["visible_points"] and (s["owned_cpu_max"] or 0) <= profile["cpu_bytes"] and (s["owned_gpu_max"] or 0) <= profile["gpu_bytes"]
            case(f"{label}: budgets held", "pass" if budget_ok else "fail", {k: s[k] for k in ("selected_max", "owned_cpu_max", "owned_gpu_max", "rss_max")},
                 {"visible_points": profile["visible_points"], "cpu_bytes": profile["cpu_bytes"], "gpu_bytes": profile["gpu_bytes"]}, evidence)
            timed = "frame_p95_ms" in profile and name in ("cloud-1m", "octree-100k", "cloud-5m-paged", "cloud-100k-live", "mixed")
            if timed:
                passed = s["cpu_ms_p95"] is not None and s["cpu_ms_p95"] <= profile["frame_p95_ms"] and s["cpu_ms_p99"] <= profile["frame_p99_ms"]
                case(f"{label}: frame time", "pass" if passed else "fail", s, {"p95_ms": profile["frame_p95_ms"], "p99_ms": profile["frame_p99_ms"]}, evidence)
            else:
                case(f"{label}: frame time (report)", "report", s, evidence=evidence)
        visible = [e for e in events if e["event"] == "visible"]
        if name == "cloud-1m" and visible and "static_retained_ready_ms" in profile:
            ready = visible[0]["retained_to_visible_ms"]
            case("cloud-1m: retained to visible", "pass" if ready <= profile["static_retained_ready_ms"] else "fail", {"ms": ready}, {"ms": profile["static_retained_ready_ms"]}, evidence)
        committed = [e for e in events if e["event"] == "committed"]
        for begin in (e for e in events if e["event"] == "delta_begin"):
            initial = upload_between(frames, committed[0]["unix_s"] - 1, committed[0]["unix_s"] + 8) if committed else None
            edit = upload_between(frames, begin["unix_s"], begin["unix_s"] + 3)
            share = begin["pages"] / begin["total_pages"]
            passed = initial is not None and edit is not None and initial > 0 and edit <= max(3 * share * initial, 2 * begin["page_bytes"] + (1 << 20))
            case(f"{name}: sparse edit uploads", "pass" if passed else "fail",
                 {"edit_bytes": edit, "initial_bytes": initial, "edit_share_of_pages": round(share, 4)}, {"at_most": "3x the edited page share of the initial upload"}, evidence)
        for begin in (e for e in events if e["event"] == "restyle_begin"):
            a, b = at(frames, begin["unix_s"] - 0.05), at(frames, begin["unix_s"] + 3)
            if a and b:
                positions = b["uploads"]["positions"] - a["uploads"]["positions"]
                indices = b["uploads"]["indices"] - a["uploads"]["indices"]
                styles = b["uploads"]["styles"] - a["uploads"]["styles"]
                case(f"{name}: presentation-only edit", "pass" if positions == 0 and indices == 0 and styles > 0 else "fail",
                     {"position_uploads": positions, "index_uploads": indices, "style_uploads": styles}, {"position_uploads": 0, "index_uploads": 0}, evidence)
        updates = [e for e in events if e["event"] == "update"]
        if updates and windows:
            w = windows[0]
            inside = [u for u in updates if w["start_unix_s"] <= u["unix_s"] <= w["end_unix_s"]]
            latencies = [u["stored_to_visible_ms"] for u in inside if u["stored_to_visible_ms"] is not None]
            missed = sum(1 for u in inside if u["stored_to_visible_ms"] is None)
            p95 = percentile(latencies, 95)
            limit = profile.get("live_stored_visible_p95_ms")
            case(f"{name}: stored to visible", "pass" if p95 is not None and (limit is None or p95 <= limit) else "fail",
                 {"updates": len(inside), "p50_ms": percentile(latencies, 50), "p95_ms": p95, "p99_ms": percentile(latencies, 99),
                  "not_visible_within_three_periods": missed, "poll_resolution_ms": 20}, {"p95_ms": limit}, evidence)
        cleared = [e for e in events if e["event"] == "cleared"]
        if cleared:
            idle = [at(frames, e["unix_s"]) for e in cleared]
            gpu = [f["owned_gpu_bytes"] for f in idle if f]
            cpu = [f["owned_cpu_bytes"] for f in idle if f]
            growth_gpu, growth_cpu = gpu[-1] - gpu[0], cpu[-1] - cpu[0]
            passed = len(cleared) >= 20 and growth_gpu <= (1 << 20) and growth_cpu <= (4 << 20)
            case(f"{name}: 20 load/clear cycles return to idle", "pass" if passed else "fail",
                 {"cycles": len(cleared), "idle_gpu_first_last": [gpu[0], gpu[-1]], "idle_cpu_first_last": [cpu[0], cpu[-1]]},
                 {"gpu_growth_bytes": 1 << 20, "cpu_growth_bytes": 4 << 20}, evidence)
        for reject in (e for e in events if e["event"] == "reject"):
            passed = not reject["big_accepted"] and reject["previous_generation_retained"] and reject["previous_generation_visible"]
            case(f"{name}: oversize upload rejected, previous generation kept", "pass" if passed else "fail", reject, {"big_accepted": False}, evidence)
        for status in (e for e in events if e["event"] == "status"):
            paging = ((status.get("status") or {}).get("paging"))
            case(f"{name}: page residency reported", "pass" if paging else "fail", {"paging": paging}, evidence=evidence)
            coarse = stats[0]["coarse_frames"] if stats else 0
            case(f"{name}: coarse fallback during motion", "pass" if coarse > 0 else "fail", {"coarse_frames": coarse}, evidence=evidence)
        if name == "mixed" and len(stats) == 2 and stats[0]["cpu_ms_p95"] and stats[1]["cpu_ms_p95"]:
            case("mixed: added overhead (report)", "report", {"base_p95_ms": stats[0]["cpu_ms_p95"], "mixed_p95_ms": stats[1]["cpu_ms_p95"],
                 "added_p95_ms": round(stats[1]["cpu_ms_p95"] - stats[0]["cpu_ms_p95"], 3)}, evidence=evidence)

    failed = [c for c in cases if c["state"] == "fail"]
    manifest = [json.loads(l) for l in (run / "manifest.json").read_text().splitlines() if l.strip()]
    results = {"profile": profile_name, "manifest": manifest, "passed": not failed, "cases": cases}
    (run / "results.json").write_text(json.dumps(results, indent=2))
    rows = "\n".join(
        f"| {c['name']} | {c['state'].upper()} | {json.dumps(c['measured'])[:160]} | {json.dumps(c['thresholds'])[:80]} |" for c in cases
    )
    head = manifest[0] if manifest else {}
    (run / "ACCEPTANCE.md").write_text(
        f"# Bench {head.get('run_id')} ({profile_name})\n\n"
        f"Device: {head.get('device')}. Size {head.get('size')}, MSAA {head.get('msaa')}, uncapped headless, "
        f"{head.get('warmup_s')} s warmup, {head.get('measure_s')} s measured. Competing instances at start: "
        f"{head.get('competing_instances')}; GPU at start: {head.get('gpu_utilization_at_start')}. "
        "CPU frame time is wall time between main-world frames; GPU timestamp queries are not enabled.\n\n"
        f"Result: {'all gates passed' if not failed else f'{len(failed)} gate(s) failed'}.\n\n"
        "| Case | State | Measured | Threshold |\n|---|---|---|---|\n" + rows + "\n"
    )
    print(json.dumps({"profile": profile_name, "cases": len(cases), "failed": [c["name"] for c in failed]}))


if __name__ == "__main__":
    main()
