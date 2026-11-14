#!/usr/bin/env python3
"""Scatter bales and drive one USD tractor to them.

Usage:
    python scripts/bale_run.py [machine_namespace] [n_bales] [field_size] [seed]

Defaults: namespace=robot, n_bales=50, field_size=300, seed=42.

Spawns ``bin/gearbox/assets/tractor.usd`` under the given namespace and
collects the bales greedily, nearest first. This is the single-machine case
of ``bale_run_multi.py`` and shares its driving loop.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from bale_run_multi import RobotProxy, run  # noqa: E402


def main() -> None:
    namespace = sys.argv[1] if len(sys.argv) > 1 else "robot"
    n_bales = int(sys.argv[2]) if len(sys.argv) > 2 else 50
    field = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 42
    run([RobotProxy(0, namespace)], n_bales, field, seed)


if __name__ == "__main__":
    main()
