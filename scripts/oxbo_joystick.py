#!/usr/bin/env python3
"""Load flatland + Oxbo, then drive it from a Linux joystick event device.

Run Gearbox first:

    make run

Then run:

    python scripts/oxbo_joystick.py

By default this reads ``/dev/input/warpout0`` and publishes to the USD machine
controller namespace ``oxbo``. It talks directly to the Linux ``input_event``
API, so it does not need pygame/evdev.
"""

from __future__ import annotations

import argparse
import fcntl
import os
import selectors
import struct
import time
from dataclasses import dataclass

import cbor2
import zenoh


DEFAULT_DEVICE = "/dev/input/warpout0"
FLATLAND_USD_PATH = "world/flatland.usd"
OXBO_USD_PATH = "bin/gearbox/assets/oxbo.usd"

EV_KEY = 0x01
EV_ABS = 0x03

ABS_X = 0x00
ABS_Y = 0x01
ABS_Z = 0x02
ABS_RX = 0x03
ABS_RY = 0x04
ABS_RZ = 0x05
ABS_HAT0X = 0x10
ABS_HAT0Y = 0x11

AXIS_CODES = {
    "ABS_X": ABS_X,
    "ABS_Y": ABS_Y,
    "ABS_Z": ABS_Z,
    "ABS_RX": ABS_RX,
    "ABS_RY": ABS_RY,
    "ABS_RZ": ABS_RZ,
    "ABS_HAT0X": ABS_HAT0X,
    "ABS_HAT0Y": ABS_HAT0Y,
}

# struct input_event { timeval sec/usec; unsigned short type/code; int value; }
INPUT_EVENT = struct.Struct("llHHi")
ABS_INFO = struct.Struct("iiiiii")


@dataclass(frozen=True)
class AbsInfo:
    minimum: int
    maximum: int
    flat: int


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def open_session() -> zenoh.Session:
    opened = zenoh.open(zenoh.Config())
    return opened.wait() if hasattr(opened, "wait") else opened


def put_cbor(session: zenoh.Session, key: str, payload: dict) -> None:
    session.put(
        key,
        cbor2.dumps(payload),
        congestion_control=zenoh.CongestionControl.BLOCK,
    )


def claim_machine_session(session: zenoh.Session, namespace: str, session_id: str) -> None:
    put_cbor(
        session,
        f"gearbox/machines/{namespace}/session",
        {"session_id": session_id},
    )


def clear_sim(session: zenoh.Session) -> None:
    put_cbor(session, "gearbox/sim/clear", {"pause_clock": False})


def load_flatland(session: zenoh.Session) -> None:
    put_cbor(
        session,
        "gearbox/usd/load/flatland_terrain",
        {
            "category": "terrain",
            "usd_path": FLATLAND_USD_PATH,
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "remove": False,
        },
    )


def load_oxbo(session: zenoh.Session, namespace: str) -> None:
    put_cbor(
        session,
        f"gearbox/usd/load/{namespace}",
        {
            "category": "machine",
            "usd_path": OXBO_USD_PATH,
            "namespace": namespace,
            "label": "oxbo.usd",
            "x": 0.0,
            "y": 0.0,
            "z": 0.0,
            "yaw_deg": 0.0,
            "remove": False,
        },
    )


def publish_cmd(
    session: zenoh.Session,
    namespace: str,
    session_id: str,
    speed: float,
    yaw_rate: float,
) -> None:
    put_cbor(
        session,
        f"gearbox/machines/{namespace}/cmd_vel",
        {
            "linear": [float(speed), 0.0, 0.0],
            "angular": [0.0, 0.0, float(yaw_rate)],
            "session_id": session_id,
        },
    )


def eviocgabs(axis_code: int) -> int:
    # #define EVIOCGABS(abs) _IOR('E', 0x40 + (abs), struct input_absinfo)
    return 0x80184540 + axis_code


def read_abs_info(fd: int, axis_code: int) -> AbsInfo | None:
    buf = bytearray(ABS_INFO.size)
    try:
        fcntl.ioctl(fd, eviocgabs(axis_code), buf, True)
    except OSError:
        return None
    _value, minimum, maximum, _fuzz, flat, _resolution = ABS_INFO.unpack(buf)
    if maximum <= minimum:
        return None
    return AbsInfo(minimum=minimum, maximum=maximum, flat=flat)


def normalize_axis(value: int, info: AbsInfo | None, deadzone: float) -> float:
    if info is not None:
        center = (info.minimum + info.maximum) * 0.5
        half_range = max(abs(info.maximum - center), abs(info.minimum - center), 1.0)
        normalized = (value - center) / half_range
        flat = max(deadzone, info.flat / half_range)
    elif -1 <= value <= 1:
        normalized = float(value)
        flat = deadzone
    elif 0 <= value <= 255:
        normalized = (float(value) - 127.5) / 127.5
        flat = deadzone
    else:
        normalized = float(value) / 32767.0
        flat = deadzone

    normalized = clamp(normalized, -1.0, 1.0)
    if abs(normalized) < flat:
        return 0.0
    return normalized


def parse_axis(name: str) -> int:
    upper = name.upper()
    if upper in AXIS_CODES:
        return AXIS_CODES[upper]
    return int(name, 0)


def read_events(fd: int) -> list[tuple[int, int, int]]:
    events: list[tuple[int, int, int]] = []
    while True:
        try:
            data = os.read(fd, INPUT_EVENT.size * 32)
        except BlockingIOError:
            break
        if not data:
            break
        for offset in range(0, len(data) - INPUT_EVENT.size + 1, INPUT_EVENT.size):
            _sec, _usec, event_type, code, value = INPUT_EVENT.unpack_from(data, offset)
            events.append((event_type, code, value))
        if len(data) < INPUT_EVENT.size * 32:
            break
    return events


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--device", default=DEFAULT_DEVICE)
    parser.add_argument("--namespace", default="oxbo")
    parser.add_argument("--steer-axis", default="ABS_X")
    parser.add_argument("--throttle-axis", default="ABS_Y")
    parser.add_argument("--max-speed", type=float, default=3.0)
    parser.add_argument("--max-yaw", type=float, default=0.9)
    parser.add_argument("--deadzone", type=float, default=0.08)
    parser.add_argument("--invert-steer", action="store_true")
    parser.add_argument("--invert-throttle", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--no-clear", action="store_true")
    parser.add_argument("--no-flatland", action="store_true")
    args = parser.parse_args()

    steer_axis = parse_axis(args.steer_axis)
    throttle_axis = parse_axis(args.throttle_axis)
    session_id = f"oxbo_joystick_{args.namespace}_{int(time.time() * 1000)}"

    session = open_session()
    fd = os.open(args.device, os.O_RDONLY | os.O_NONBLOCK)
    selector = selectors.DefaultSelector()
    selector.register(fd, selectors.EVENT_READ)

    abs_infos = {
        axis: read_abs_info(fd, axis)
        for axis in {steer_axis, throttle_axis, ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ}
    }
    axes: dict[int, float] = {}
    buttons: dict[int, int] = {}

    try:
        print(f"claiming machine session `{session_id}` for namespace `{args.namespace}`")
        claim_machine_session(session, args.namespace, session_id)
        if not args.no_clear:
            clear_sim(session)
            time.sleep(0.3)
        if not args.no_flatland:
            load_flatland(session)
        print(f"loading {OXBO_USD_PATH}")
        load_oxbo(session, args.namespace)
        time.sleep(0.5)
        claim_machine_session(session, args.namespace, session_id)

        print(f"reading joystick events from {args.device}")
        print(
            "controls: "
            f"{args.throttle_axis}=speed, {args.steer_axis}=turn, Ctrl-C to stop"
        )

        next_publish = 0.0
        last_print = 0.0
        while True:
            timeout = max(0.0, next_publish - time.monotonic())
            selector.select(timeout)
            for event_type, code, value in read_events(fd):
                if event_type == EV_ABS:
                    axes[code] = normalize_axis(value, abs_infos.get(code), args.deadzone)
                elif event_type == EV_KEY:
                    buttons[code] = value

            now = time.monotonic()
            if now < next_publish:
                continue
            next_publish = now + 0.05

            steer = axes.get(steer_axis, 0.0)
            throttle = axes.get(throttle_axis, 0.0)
            if args.invert_steer:
                steer = -steer
            if args.invert_throttle:
                throttle = -throttle

            speed = clamp(throttle * args.max_speed, -args.max_speed, args.max_speed)
            yaw_rate = clamp(steer * args.max_yaw, -args.max_yaw, args.max_yaw)
            publish_cmd(session, args.namespace, session_id, speed, yaw_rate)

            if now - last_print > 0.5:
                last_print = now
                print(
                    f"\rspeed={speed:+.2f} m/s  yaw={yaw_rate:+.2f} rad/s  "
                    f"buttons={sum(1 for v in buttons.values() if v)}",
                    end="",
                    flush=True,
                )
    except KeyboardInterrupt:
        print()
    finally:
        publish_cmd(session, args.namespace, session_id, 0.0, 0.0)
        selector.close()
        os.close(fd)
        session.close()
        print("stopped Oxbo joystick control")


if __name__ == "__main__":
    main()
