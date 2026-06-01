#!/usr/bin/env python3
"""One-shot zero USD-machine cmd_vel — instant stop.

Usage:
    python stop.py [namespace]

`namespace` defaults to `oxbo`."""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    namespace = sys.argv[1] if len(sys.argv) > 1 else "oxbo"
    opened = zenoh.open(zenoh.Config())
    session = opened.wait() if hasattr(opened, "wait") else opened

    session_id = f"stop_{namespace}_{int(time.time() * 1000)}"
    machine_payload = cbor2.dumps(
        {"linear": [0, 0, 0], "angular": [0, 0, 0], "session_id": session_id}
    )
    session.put(
        f"gearbox/machines/{namespace}/session",
        cbor2.dumps({"session_id": session_id}),
    )
    session.put(f"gearbox/machines/{namespace}/cmd_vel", machine_payload)
    print(f"sent zero twist to gearbox/machines/{namespace}/cmd_vel")
    session.close()


if __name__ == "__main__":
    main()
