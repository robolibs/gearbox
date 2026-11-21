"""CLI entrypoint for `rungear` (and `gearbox rungear`)."""

from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
from pathlib import Path

import rerun as rr

from .bridge import RungearBridge, RungearConfig


def _find_rerun_binary() -> str | None:
    """Find a working native `rerun` viewer binary on PATH.

    In Nix devshells that include both `python3Packages.rerun-sdk` and
    `pkgs.rerun`, the Python env's `bin/rerun` wrapper can shadow the native
    viewer binary and fail with `ModuleNotFoundError: rerun_cli`. Walk PATH
    and return the first `rerun` that answers `--version` cleanly, preferring
    non-Python store paths.
    """
    candidates: list[Path] = []
    seen: set[str] = set()
    for raw_dir in os.environ.get("PATH", "").split(os.pathsep):
        if not raw_dir:
            continue
        cand = Path(raw_dir) / "rerun"
        key = str(cand)
        if key in seen or not cand.is_file() or not os.access(cand, os.X_OK):
            continue
        seen.add(key)
        candidates.append(cand)
    # Prefer native `rerun` over Python env wrappers.
    candidates.sort(key=lambda p: ("python" in str(p).lower() or "rerun-sdk" in str(p).lower()))
    for cand in candidates:
        try:
            res = subprocess.run([str(cand), "--version"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3)
            if res.returncode == 0:
                return str(cand)
        except OSError:
            continue
    return None


def _ensure_display_env() -> None:
    """Detect a live Wayland socket if the shell didn't export one."""
    if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
        return
    rt = Path(os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}")
    wls = sorted(p.name for p in rt.glob("wayland-*") if not p.name.endswith(".lock"))
    if wls:
        os.environ["WAYLAND_DISPLAY"] = wls[0]
        os.environ["XDG_RUNTIME_DIR"] = str(rt)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="rungear",
        description="Subscribe to a running Gearbox simulator over peerbus shared memory and visualize in Rerun.",
    )
    p.add_argument("--instance", help="Gearbox instance name or DID (default: first live instance)")
    p.add_argument("--machine", action="append", default=[], help="Filter to specific machine id(s) (repeatable)")
    p.add_argument("--sensor", action="append", default=[], help="Filter to specific sensor link(s) (repeatable)")
    p.add_argument("--color-by", choices=("height", "range"), default="height", help="LiDAR point coloring (default: height)")
    p.add_argument("--point-radius", type=float, default=0.035, help="LiDAR 3D point radius in metres (default: 0.035)")
    p.add_argument("--no-tf", action="store_true", help="Do not subscribe to /tf dynamic link poses")
    p.add_argument("--no-state", action="store_true", help="Do not subscribe to /state root poses")
    p.add_argument("--no-cameras", action="store_true", help="Skip logging camera images/depth")
    p.add_argument("--no-lidar", action="store_true", help="Skip logging LiDAR point clouds")
    out = p.add_mutually_exclusive_group()
    out.add_argument("--connect", metavar="URL", help="Connect to an existing Rerun viewer via gRPC (e.g. rerun+http://127.0.0.1:9876/proxy)")
    out.add_argument("--save", metavar="FILE.rrd", help="Record directly to a .rrd file instead of spawning a viewer")
    out.add_argument("--serve", action="store_true", help="Serve Rerun gRPC stream without spawning a viewer")
    p.add_argument("--port", type=int, default=9876, help="Rerun gRPC port when spawning/serving (default: 9876)")
    p.add_argument("--app-id", default="rungear", help="Rerun recording application id (default: rungear)")
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    _ensure_display_env()

    rr.init(args.app_id)
    if args.save:
        rr.save(args.save)
        print(f"[rungear] recording to {args.save}", flush=True)
    elif args.connect:
        rr.connect_grpc(args.connect)
        print(f"[rungear] connected to Rerun viewer at {args.connect}", flush=True)
    elif args.serve:
        uri = rr.serve_grpc(grpc_port=args.port)
        print(f"[rungear] serving Rerun gRPC at {uri}", flush=True)
    else:
        exe = _find_rerun_binary()
        rr.spawn(port=args.port, memory_limit="50%", executable_path=exe)
        print(f"[rungear] spawned Rerun viewer ({exe or 'rerun'}) on port {args.port}", flush=True)

    cfg = RungearConfig(
        instance=args.instance,
        machines=set(args.machine),
        sensors=set(args.sensor),
        enable_tf=not args.no_tf,
        enable_state=not args.no_state,
        enable_cameras=not args.no_cameras,
        enable_lidar=not args.no_lidar,
        lidar_color_by=args.color_by,
        lidar_point_radius=args.point_radius,
    )
    bridge = RungearBridge(cfg)

    def _on_signal(_sig, _frame):
        bridge.stop()

    signal.signal(signal.SIGINT, _on_signal)
    signal.signal(signal.SIGTERM, _on_signal)

    bridge.run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
