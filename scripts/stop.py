#!/usr/bin/env python3
"""One-shot zero cmd_vel — instant stop.

Usage:
    python stop.py [vehicle]

`vehicle` defaults to `tractor_0`."""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    vehicle = sys.argv[1] if len(sys.argv) > 1 else "tractor_0"
    opened = zenoh.open(zenoh.Config())
    session = opened.wait() if hasattr(opened, "wait") else opened
    payload = cbor2.dumps({"linear": [0, 0, 0], "angular": [0, 0, 0]})
    session.put(f"{vehicle}/cmd_vel", payload)

    session_id = f"stop_{vehicle}_{int(time.time() * 1000)}"
    machine_payload = cbor2.dumps(
        {"linear": [0, 0, 0], "angular": [0, 0, 0], "session_id": session_id}
    )
    session.put(
        f"gearbox/machines/{vehicle}/session",
        cbor2.dumps({"session_id": session_id}),
    )
    session.put(f"gearbox/machines/{vehicle}/cmd_vel", machine_payload)
    print(f"sent zero twist to {vehicle}/cmd_vel and gearbox/machines/{vehicle}/cmd_vel")
    session.close()


if __name__ == "__main__":
    main()
