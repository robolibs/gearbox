#!/usr/bin/env python3
"""One-shot zero cmd_vel — instant stop.

Usage:
    python stop.py [vehicle]

`vehicle` defaults to `tractor_0`."""

from __future__ import annotations

import sys

import zenoh
import cbor2


def main() -> None:
    vehicle = sys.argv[1] if len(sys.argv) > 1 else "tractor_0"
    session = zenoh.open(zenoh.Config())
    payload = cbor2.dumps({"linear": [0, 0, 0], "angular": [0, 0, 0]})
    session.put(f"{vehicle}/cmd_vel", payload)
    print(f"sent zero twist to {vehicle}/cmd_vel")
    session.close()


if __name__ == "__main__":
    main()
