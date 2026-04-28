#!/usr/bin/env python3
"""Wildcard-subscribe to every `*_*/odom` topic and print live vehicles.

Run this with the simulator open — every vehicle prints once per
frame so you can see exactly which `<robot_name>_<instance>` keys
the API is exposing without grepping through gearbox source."""

from __future__ import annotations

import time

import zenoh
import cbor2


def main() -> None:
    session = zenoh.open(zenoh.Config())
    print("listening on **/odom — Ctrl-C to quit")

    seen: set[str] = set()

    def on_sample(sample: zenoh.Sample) -> None:
        key = str(sample.key_expr)
        if key in seen:
            return
        seen.add(key)
        try:
            payload = cbor2.loads(bytes(sample.payload))
        except Exception as e:  # noqa: BLE001
            payload = f"<decode err: {e}>"
        print(f"NEW {key}  →  {payload}")

    session.declare_subscriber("**/odom", on_sample)

    try:
        while True:
            time.sleep(1.0)
    except KeyboardInterrupt:
        pass
    finally:
        session.close()


if __name__ == "__main__":
    main()
