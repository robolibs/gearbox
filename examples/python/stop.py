#!/usr/bin/env python3
"""Take over a machine and send it a zero twist — instant stop.

Usage:
    python scripts/stop.py [namespace]

`namespace` defaults to `oxbo`. The session is taken from whoever holds it,
zeroed, and released again."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox  # noqa: E402


def main() -> None:
    namespace = sys.argv[1] if len(sys.argv) > 1 else "oxbo"
    gb = Gearbox()
    machine = gb.machine(namespace, timeout=5.0)
    machine.claim(take=True, client="stop.py")
    machine.stop()
    machine.release()
    print(f"sent zero twist to machine `{namespace}` and released it")


if __name__ == "__main__":
    main()
