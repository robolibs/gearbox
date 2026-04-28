#!/usr/bin/env python3
"""Tail a single vehicle's odom + fix topics.

Usage:
    python watch.py [vehicle]

`vehicle` defaults to `tractor_0` — the starter tractor."""

from __future__ import annotations

import sys
import time

import zenoh
import cbor2


def main() -> None:
    vehicle = sys.argv[1] if len(sys.argv) > 1 else "tractor_0"

    session = zenoh.open(zenoh.Config())
    print(f"watching {vehicle} — Ctrl-C to quit")

    def fmt_odom(d: dict) -> str:
        p = d.get("position", [0, 0, 0])
        v = d.get("linear_velocity", [0, 0, 0])
        speed = (v[0] ** 2 + v[1] ** 2 + v[2] ** 2) ** 0.5
        return f"pos=({p[0]:+7.2f},{p[1]:+7.2f},{p[2]:+7.2f})  |v|={speed:5.2f} m/s"

    def fmt_fix(d: dict) -> str:
        return (
            f"lat={d.get('latitude', 0):+11.6f}°  "
            f"lon={d.get('longitude', 0):+11.6f}°  "
            f"alt={d.get('altitude', 0):+8.3f} m"
        )

    def on_odom(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
            print(f"  odom  {fmt_odom(d)}")
        except Exception as e:  # noqa: BLE001
            print(f"  odom  <decode err: {e}>")

    def on_fix(sample: zenoh.Sample) -> None:
        try:
            d = cbor2.loads(bytes(sample.payload))
            print(f"  fix   {fmt_fix(d)}")
        except Exception as e:  # noqa: BLE001
            print(f"  fix   <decode err: {e}>")

    session.declare_subscriber(f"{vehicle}/odom", on_odom)
    session.declare_subscriber(f"{vehicle}/fix", on_fix)

    try:
        while True:
            time.sleep(1.0)
    except KeyboardInterrupt:
        pass
    finally:
        session.close()


if __name__ == "__main__":
    main()
