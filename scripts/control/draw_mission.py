#!/usr/bin/env python3
"""Closed-loop Husky route following with retained, telemetry-driven world drawings."""

import argparse
import collections
import csv
import json
import math
from pathlib import Path
import signal
import time

NS = "draw-mission"
PLAN, TRAIL, TARGET, FORECAST = "#68c6ed", "#ffb451", "#f3d66b", "#72d9af"
TRAIL_POINTS = 160


def route(origin, count=128):
    return [(origin[0] + 7 * math.cos(-math.pi / 2 + math.tau * i / count),
             origin[1] + 4.5 + 4.5 * math.sin(-math.pi / 2 + math.tau * i / count), .14)
            for i in range(count)]


def steering(points, x, y, heading, lookahead=1.6):
    nearest = min(range(len(points)), key=lambda i: math.hypot(points[i][0] - x, points[i][1] - y))
    target, along = nearest, 0.0
    for _ in range(len(points)):
        following = (target + 1) % len(points)
        along += math.dist(points[target][:2], points[following][:2])
        target = following
        if along >= lookahead:
            break
    dx, dy = points[target][0] - x, points[target][1] - y
    angle = math.atan2(math.sin(math.atan2(dy, dx) - heading), math.cos(math.atan2(dy, dx) - heading))
    error = math.hypot(points[nearest][0] - x, points[nearest][1] - y)
    if abs(angle) > 1.1:
        return nearest, target, error, 0.0, math.copysign(.65, angle)
    speed = .95 * max(.35, math.cos(angle))
    yaw = max(-.75, min(.75, 2 * speed * math.sin(angle) / max(.4, math.hypot(dx, dy))))
    return nearest, target, error, speed, yaw


def forecast(x, y, heading, speed, yaw, horizon=2.0, count=24):
    points = []
    for i in range(count + 1):
        dt = horizon * i / count
        if abs(yaw) < 1e-6:
            px, py = x + speed * dt * math.cos(heading), y + speed * dt * math.sin(heading)
        else:
            px = x + speed / yaw * (math.sin(heading + yaw * dt) - math.sin(heading))
            py = y - speed / yaw * (math.cos(heading + yaw * dt) - math.cos(heading))
        points.append((px, py, .22))
    return points


def draw_static(draw, machine, points, origin):
    frame = f"{machine}/odom"
    common = {"frame": frame, "drape": True, "glow": .22}
    draw.lines(NS, "planned-route", points, loop=True, style="ribbon_flow", color=PLAN,
               width=.10, alpha=.4, flow_speed=.65, spacing=1.5, **common)
    stations = [points[i] for i in range(0, len(points), len(points) // 4)]
    draw.points(NS, "checkpoints", stations, disc=True, style="beacon", radius=.28,
                beam_height=.45, labels=["01 / START", "02 / EAST", "03 / NORTH", "04 / WEST"],
                color=TARGET, label_icon="destination", **common)
    draw.text(NS, "title", [(origin[0], origin[1] + 11, .4)], text="HUSKY / ROUTE FOLLOWING",
              color=PLAN, size=19, label_icon="robot", **common)
    draw.text(NS, "legend", [(origin[0], origin[1] - 2, .3)],
              text="BLUE / plan     AMBER / actual trail     GREEN / command projection",
              color="white", size=14, **common)
    draw.badge(NS, "robot", machine, "HUSKY / AUTONOMOUS", height=1.15, color=PLAN,
               drape=False, props={"panel_finish": "black", "panel_width": "195"})


def draw_trail(draw, machine, trail):
    if len(trail) > 1:
        draw.lines(NS, "actual-trail", list(trail), style="light_wall", width=.028, beam_height=.45,
                   color=TRAIL, fade=.12, pulse_hz=.5, spacing=1.2, glow=.65,
                   frame=f"{machine}/odom", drape=True)


def draw_live(draw, machine, points, origin, state, command, speeds, distance, laps):
    nearest, target, error, speed, yaw = command
    common = {"frame": f"{machine}/odom", "drape": True, "glow": .22}
    draw.points(NS, "lookahead", [points[target]], style="orb", color=TARGET, radius=.12, **common)
    draw.lines(NS, "steering-link", [(state.x, state.y, .2), points[target]], style="plain", color=TARGET,
               dashed=True, dash=.18, width=-1.2, **common)
    draw.lines(NS, "command-projection", forecast(state.x, state.y, state.heading_rad, speed, yaw),
               style="tube_pulse", color=FORECAST, width=.045, fade=.65, **common)
    panels = {"frame": f"{machine}/odom", "drape": True, "color": PLAN,
              "props": {"panel_finish": "white", "panel_width": "235"}}
    draw.table(NS, "telemetry", (origin[0] + 10, origin[1] + 6, 1.3),
               [("Speed", f"{state.linear_speed:.2f} m/s"), ("Tracking error", f"{error:.2f} m"),
                ("Distance driven", f"{distance:.1f} m"), ("Completed laps", str(laps))],
               title="LIVE / ODOMETRY", **panels)
    draw.panel(NS, "speed-history", (origin[0] - 10, origin[1] + 6, 1.3), "sparkline",
               values=list(speeds), title="MEASURED SPEED", unit="m/s", lo=0, hi=1.2,
               frame=f"{machine}/odom", drape=True, color=TRAIL,
               props={"panel_finish": "black", "panel_width": "220"})


def run(instance, machine_name, output, seconds=0):
    from gearbox_client import Gearbox

    if not instance.startswith("draw-mission-") or machine_name != "mission_husky":
        raise ValueError("use the dedicated draw-mission viewer and mission_husky")
    output.mkdir(parents=True, exist_ok=True)
    gb = Gearbox(instance=instance)
    gb.wait_ready()
    machine = gb.machine(machine_name, timeout=30.0)
    machine.claim(hold_ms=800, take=False, client="draw-mission")
    running = True

    def stop(_signal, _frame):
        nonlocal running
        running = False

    for kind in (signal.SIGTERM, signal.SIGINT):
        signal.signal(kind, stop)
    try:
        state = machine.state(wait=10)
        if state is None:
            raise RuntimeError("no robot odometry")
        origin = state.x, state.y
        points = route(origin)
        draw_static(gb.draw, machine_name, points, origin)
        gb.draw.wait = False
        gb.play()
        trail, speeds = collections.deque(maxlen=TRAIL_POINTS), collections.deque(maxlen=90)
        start = last_fresh = time.monotonic()
        last_seen, previous = None, state
        last_draw = last_report = 0.0
        previous_index, laps, distance = 0, 0, 0.0
        with (output / "telemetry.csv").open("w", newline="") as file:
            writer = csv.writer(file)
            writer.writerow(["elapsed_s", "x", "y", "heading", "speed_mps", "error_m", "command_v", "command_w", "lap", "distance_m"])
            while running and (not seconds or time.monotonic() - start < seconds):
                tick = time.monotonic()
                state = machine.state()
                if state is not last_seen:
                    last_fresh, last_seen = tick, state
                if state is None or tick - last_fresh > 2:
                    if gb.clock().paused:
                        machine.stop()
                        time.sleep(.2)
                        continue
                    raise RuntimeError("odometry stopped; braking the robot")
                if gb.draw.errors:
                    raise RuntimeError(f"drawing delivery failed: {gb.draw.errors[-1]}")
                if not all(math.isfinite(v) for v in (state.x, state.y, state.heading_rad, state.linear_speed)):
                    raise RuntimeError("non-finite odometry; braking the robot")
                command = steering(points, state.x, state.y, state.heading_rad)
                index, target, error, speed, yaw = command
                if error > 4:
                    raise RuntimeError(f"robot left the route corridor ({error:.2f} m); braking")
                status = machine.cmd_vel(speed, yaw)
                if not status.ok:
                    raise RuntimeError(status.message)
                distance += math.hypot(state.x - previous.x, state.y - previous.y)
                previous = state
                if previous_index > len(points) * .75 and index < len(points) * .25:
                    laps += 1
                previous_index = index
                point = state.x - .55 * math.cos(state.heading_rad), state.y - .55 * math.sin(state.heading_rad), .06
                if not trail or math.dist(point[:2], trail[-1][:2]) > .1:
                    trail.append(point)
                live_trail = list(trail)
                if live_trail and math.dist(point, live_trail[-1]) > 1e-4:
                    live_trail.append(point)
                draw_trail(gb.draw, machine_name, live_trail)
                if tick - last_draw >= .25:
                    speeds.append(state.linear_speed)
                    draw_live(gb.draw, machine_name, points, origin, state, command, speeds, distance, laps)
                    last_draw = tick
                if tick - last_report >= .5:
                    report = {"instance": instance, "machine": machine_name, "elapsed_s": tick - start,
                              "x": state.x, "y": state.y, "heading": state.heading_rad, "speed_mps": state.linear_speed,
                              "error_m": error, "command_v": speed, "command_w": yaw, "lap": laps, "distance_m": distance}
                    writer.writerow([report[k] for k in ("elapsed_s", "x", "y", "heading", "speed_mps", "error_m", "command_v", "command_w", "lap", "distance_m")])
                    file.flush()
                    (output / "latest.json").write_text(json.dumps(report, indent=2) + "\n")
                    if not (output / "ready.json").exists():
                        (output / "ready.json").write_text(json.dumps(report) + "\n")
                    print(json.dumps(report), flush=True)
                    last_report = tick
                time.sleep(max(0, .05 - (time.monotonic() - tick)))
    finally:
        try:
            machine.stop()
        finally:
            machine.release()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("instance")
    parser.add_argument("--machine", default="mission_husky")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=0)
    args = parser.parse_args()
    run(args.instance, args.machine, args.output, args.seconds)
